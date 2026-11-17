//! JSON types for the HTTP API, shared with the client through ts-rs.
//!
//! Also holds the rules for valid input (email, password, display name). They
//! live here, not in the server, so the client can show the same rules and,
//! later, run them through WASM.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where `cargo test` writes the generated TypeScript (see `protocol.rs`).
const EXPORT_DIR: &str = "../../../client/src/protocol/generated/";

/// Shortest allowed password, in characters (spec ACC-002).
pub const MIN_PASSWORD_CHARS: usize = 10;
/// Longest allowed password, in bytes. Hashing is deliberately slow, so a
/// megabyte-long "password" would be a cheap way to tie up the server.
pub const MAX_PASSWORD_BYTES: usize = 1024;
/// Longest allowed display name, in characters.
pub const MAX_DISPLAY_NAME_CHARS: usize = 50;
/// Longest allowed email address, in bytes (the limit in the email standards).
pub const MAX_EMAIL_BYTES: usize = 254;

/// A signed-in user, as the client sees them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct UserInfo {
    /// Database row id. `i64` would become `bigint` in TypeScript, so this tells
    /// ts-rs to use `number`; ids will never come near 2^53.
    #[ts(type = "number")]
    pub id: i64,
    pub email: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct SignUpRequest {
    pub email: String,
    pub password: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct SignInRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct UpdateProfileRequest {
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// What went wrong, as a stable code the client can switch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = EXPORT_DIR)]
pub enum ApiErrorCode {
    InvalidInput,
    EmailTaken,
    BadCredentials,
    Unauthorized,
    Forbidden,
    NotFound,
    /// The request clashes with the current state (e.g. removing the last GM).
    Conflict,
    /// An upload is too big, or the campaign is out of space.
    TooLarge,
    RateLimited,
    Internal,
}

/// The body of every error response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = EXPORT_DIR)]
pub struct ApiError {
    pub error: ApiErrorCode,
    /// A sentence for people. Never includes secrets or internal details.
    pub message: String,
}

/// Trims and lower-cases an email address and checks its basic shape.
///
/// This is deliberately loose (something@something.something): the only real
/// check of an address is sending mail to it, which comes with password reset.
/// Returns the normalized address, or a message for the user.
pub fn normalize_email(raw: &str) -> Result<String, &'static str> {
    let email = raw.trim().to_lowercase();
    if email.len() > MAX_EMAIL_BYTES {
        return Err("That email address is too long.");
    }
    // `split_once` splits at the first '@' and returns None if there is none.
    let Some((local, domain)) = email.split_once('@') else {
        return Err("Enter an email address like name@example.com.");
    };
    let shape_ok = !local.is_empty()
        && !domain.contains('@')
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !email.chars().any(char::is_whitespace);
    if shape_ok {
        Ok(email)
    } else {
        Err("Enter an email address like name@example.com.")
    }
}

/// Checks a new password's length. Any characters are allowed.
pub fn check_new_password(password: &str) -> Result<(), &'static str> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        Err("Use at least 10 characters for your password.")
    } else if password.len() > MAX_PASSWORD_BYTES {
        Err("That password is too long.")
    } else {
        Ok(())
    }
}

/// Trims a display name and checks it isn't empty or too long.
pub fn normalize_display_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.trim();
    if name.is_empty() {
        Err("Enter a display name.")
    } else if name.chars().count() > MAX_DISPLAY_NAME_CHARS {
        Err("Use 50 characters or fewer for your display name.")
    } else if name.chars().any(char::is_control) {
        Err("Your display name can't contain control characters.")
    } else {
        Ok(name.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_is_trimmed_and_lowercased() {
        assert_eq!(
            normalize_email("  Ada@Example.COM ").unwrap(),
            "ada@example.com"
        );
    }

    #[test]
    fn bad_emails_are_rejected() {
        for bad in [
            "",
            "ada",
            "ada@",
            "@example.com",
            "ada@example",
            "ada@@example.com",
            "a da@example.com",
            "ada@.com",
        ] {
            assert!(normalize_email(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn password_length_counts_characters_not_bytes() {
        assert!(check_new_password("short").is_err());
        assert!(check_new_password("0123456789").is_ok());
        // 10 characters, 20 bytes: still long enough.
        assert!(check_new_password("éééééééééé").is_ok());
        assert!(check_new_password(&"x".repeat(MAX_PASSWORD_BYTES + 1)).is_err());
    }

    #[test]
    fn display_name_rules() {
        assert_eq!(normalize_display_name("  Vex ").unwrap(), "Vex");
        assert!(normalize_display_name("   ").is_err());
        assert!(normalize_display_name(&"x".repeat(51)).is_err());
        assert!(normalize_display_name("bad\u{0}name").is_err());
    }

    #[test]
    fn user_info_is_camel_case() {
        let user = UserInfo {
            id: 1,
            email: "a@b.co".into(),
            display_name: "A".into(),
        };
        assert_eq!(
            serde_json::to_string(&user).unwrap(),
            r#"{"id":1,"email":"a@b.co","displayName":"A"}"#
        );
    }
}
