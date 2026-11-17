//! Sign-in sessions.
//!
//! On sign-in the server makes a random token, puts it in the user's cookie,
//! and stores only its SHA-256 hash. Each request hashes the cookie's token and
//! looks the hash up. The token is 256 random bits, so unlike a password it
//! can't be guessed, and a fast hash like SHA-256 is enough.

use axum_extra::extract::cookie::{Cookie, SameSite};
use holotable_core::api::UserInfo;
use sqlx::SqlitePool;

use crate::{db, error::AppError};

pub const COOKIE_NAME: &str = "holotable_session";
/// Sessions last 30 days from their last use (spec ACC-005).
pub const SESSION_SECS: i64 = 30 * 24 * 60 * 60;
/// Extend a session at most once a day, so reads don't turn into writes.
const EXTEND_AFTER_SECS: i64 = 24 * 60 * 60;

/// A session token: the shared secret-token type (see `tokens.rs`).
pub use crate::tokens::SecretToken as SessionToken;

/// The user columns a session lookup returns.
struct UserRow {
    id: i64,
    email: String,
    display_name: String,
}

impl From<UserRow> for UserInfo {
    fn from(r: UserRow) -> Self {
        UserInfo {
            id: r.id,
            email: r.email,
            display_name: r.display_name,
        }
    }
}

/// Starts a new session for a user and returns its token.
pub async fn create(db: &SqlitePool, user_id: i64) -> Result<SessionToken, AppError> {
    let token = SessionToken::generate();
    let now = db::now();
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, created_at, expires_at) VALUES (?, ?, ?, ?)",
    )
    .bind(token.hash())
    .bind(user_id)
    .bind(now)
    .bind(now + SESSION_SECS)
    .execute(db)
    .await?;
    Ok(token)
}

/// Finds the user for a token, if the session exists and hasn't expired.
/// Slides the expiry forward when the session is in use (ACC-005).
pub async fn lookup(db: &SqlitePool, token: &SessionToken) -> Result<Option<UserInfo>, AppError> {
    let now = db::now();
    let hash = token.hash();
    // `query_as` turns the row into a tuple, one element per column. The `?`
    // placeholders are filled by `bind`, so values are never pasted into the
    // SQL text, which is what prevents SQL injection (spec NFR-003).
    let found: Option<(i64, String, String, i64)> = sqlx::query_as(
        "SELECT u.id, u.email, u.display_name, s.expires_at
         FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = ? AND s.expires_at > ?",
    )
    .bind(&hash)
    .bind(now)
    .fetch_optional(db)
    .await?;

    let Some((id, email, display_name, expires_at)) = found else {
        return Ok(None);
    };
    if expires_at - now < SESSION_SECS - EXTEND_AFTER_SECS {
        sqlx::query("UPDATE sessions SET expires_at = ? WHERE token_hash = ?")
            .bind(now + SESSION_SECS)
            .bind(&hash)
            .execute(db)
            .await?;
    }
    Ok(Some(
        UserRow {
            id,
            email,
            display_name,
        }
        .into(),
    ))
}

/// Ends one session (sign-out).
pub async fn delete(db: &SqlitePool, token: &SessionToken) -> Result<(), AppError> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
        .bind(token.hash())
        .execute(db)
        .await?;
    Ok(())
}

/// Ends all of a user's sessions except `keep` (after a password change, ACC-008).
pub async fn delete_others(
    db: &SqlitePool,
    user_id: i64,
    keep: &SessionToken,
) -> Result<u64, AppError> {
    let result = sqlx::query("DELETE FROM sessions WHERE user_id = ? AND token_hash != ?")
        .bind(user_id)
        .bind(keep.hash())
        .execute(db)
        .await?;
    Ok(result.rows_affected())
}

/// Removes expired sessions. Runs hourly in the background.
pub async fn delete_expired(db: &SqlitePool) -> Result<u64, AppError> {
    let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
        .bind(db::now())
        .execute(db)
        .await?;
    Ok(result.rows_affected())
}

/// The cookie that carries a token (spec ACC-003).
/// - `HttpOnly`: page JavaScript can't read it, so injected scripts can't steal it.
/// - `SameSite=Lax`: other sites' pages can't make the browser send it with
///   their form posts or fetches.
/// - `Secure`: HTTPS only (see `Config::secure_cookies`).
/// - `Path=/`: sent with every request to this server.
/// - `Max-Age`: the browser keeps it for 400 days (the most browsers allow).
///   The server decides when a session really ends, by its `expires_at`, so the
///   cookie doesn't need updating each time the session is extended.
pub fn cookie(token: &SessionToken, secure: bool) -> Cookie<'static> {
    Cookie::build((COOKIE_NAME, token.expose().to_owned()))
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(secure)
        .path("/")
        .max_age(time::Duration::days(400))
        .build()
}

/// A cookie that tells the browser to delete the session cookie.
pub fn removal_cookie(secure: bool) -> Cookie<'static> {
    let mut c = Cookie::build((COOKIE_NAME, ""))
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(secure)
        .path("/")
        .build();
    c.make_removal();
    c
}
