//! Password hashing with argon2id.
//!
//! Argon2 is slow and memory-hungry on purpose, so that guessing passwords from
//! a stolen database is expensive. That also means a hash takes tens of
//! milliseconds of CPU, which would stall every other request sharing that
//! tokio worker thread. So each hash runs on tokio's separate pool for blocking
//! work, through `spawn_blocking`.

use std::sync::LazyLock;

use argon2::{
    Argon2,
    password_hash::{Error as HashError, PasswordHasher, PasswordVerifier, phc::PasswordHash},
};

use crate::error::AppError;

/// Hashes a new password into a PHC string such as `$argon2id$v=19$m=19456,...`.
///
/// Takes `String`, not `&str`: the closure runs on another thread and may
/// outlive this call, so it must own its data (`move`), not borrow it.
pub async fn hash(password: String) -> Result<String, AppError> {
    tokio::task::spawn_blocking(move || hash_blocking(&password)).await?
}

/// Checks `password` against a stored hash.
///
/// With `None` (no such account), it checks against a dummy hash anyway, so a
/// wrong email takes as long as a wrong password and can't be told apart by
/// timing (spec ACC-004). Always returns `false` in that case.
pub async fn verify(password: String, stored: Option<String>) -> Result<bool, AppError> {
    tokio::task::spawn_blocking(move || {
        let (hash, real) = match &stored {
            Some(h) => (h.as_str(), true),
            None => (DUMMY_HASH.as_str(), false),
        };
        let parsed = PasswordHash::new(hash)
            .map_err(|e| AppError::Internal(format!("stored hash is invalid: {e}")))?;
        match Argon2::default().verify_password(password.as_bytes(), &parsed) {
            Ok(()) => Ok(real),
            Err(HashError::PasswordInvalid) => Ok(false),
            Err(e) => Err(AppError::Internal(format!("password check failed: {e}"))),
        }
    })
    .await?
}

fn hash_blocking(password: &str) -> Result<String, AppError> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| AppError::Internal(format!("password hashing failed: {e}")))
}

/// A real hash of a throwaway password, made on first use. `LazyLock` runs the
/// closure once, the first time it's read, even if several threads race to it.
static DUMMY_HASH: LazyLock<String> =
    LazyLock::new(|| hash_blocking("not a real password, only for timing").expect("dummy hash"));

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn right_password_verifies_and_wrong_does_not() {
        let stored = hash("correct horse battery".into()).await.unwrap();
        assert!(stored.starts_with("$argon2id$"));
        assert!(
            verify("correct horse battery".into(), Some(stored.clone()))
                .await
                .unwrap()
        );
        assert!(
            !verify("wrong horse battery".into(), Some(stored))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn missing_account_never_verifies() {
        assert!(
            !verify("not a real password, only for timing".into(), None)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn same_password_gets_different_salts() {
        let a = hash("same password!".into()).await.unwrap();
        let b = hash("same password!".into()).await.unwrap();
        assert_ne!(a, b);
    }
}
