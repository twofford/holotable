//! Random secret tokens: session cookies and invite links.
//!
//! A token is 256 random bits, written as 64 hex characters. The server stores
//! only its SHA-256 hash. 256 random bits can't be guessed, so (unlike a
//! password) a fast hash is enough; what matters is that a copy of the database
//! doesn't contain anything you could paste into a cookie or a link.

use std::fmt;

use sha2::{Digest, Sha256};

#[derive(Clone, PartialEq, Eq)]
pub struct SecretToken(String);

/// `Debug` is written by hand so that `{:?}` in a log line or a panic message
/// prints a placeholder, never the token (spec NFR-005).
impl fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretToken(<redacted>)")
    }
}

impl SecretToken {
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("the OS random number generator works");
        Self(hex::encode(bytes))
    }

    /// Accepts a value only if it has the shape of one of our tokens: 64 hex
    /// characters. Anything else is rejected without touching the database.
    pub fn parse(value: &str) -> Option<Self> {
        let ok = value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit());
        ok.then(|| Self(value.to_ascii_lowercase()))
    }

    /// The hash that the database stores.
    pub fn hash(&self) -> Vec<u8> {
        Sha256::digest(self.0.as_bytes()).to_vec()
    }

    /// The token itself, for the one place it must leave the server: the
    /// cookie or the invite link. The name makes those places easy to find.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_and_well_formed() {
        let a = SecretToken::generate();
        let b = SecretToken::generate();
        assert_ne!(a, b);
        assert_eq!(SecretToken::parse(a.expose()), Some(a.clone()));
        assert_eq!(a.hash().len(), 32);
    }

    #[test]
    fn parse_rejects_wrong_shapes() {
        assert!(SecretToken::parse("").is_none());
        assert!(SecretToken::parse("abc").is_none());
        assert!(SecretToken::parse(&"z".repeat(64)).is_none());
    }

    #[test]
    fn debug_output_hides_the_token() {
        assert_eq!(
            format!("{:?}", SecretToken::generate()),
            "SecretToken(<redacted>)"
        );
    }
}
