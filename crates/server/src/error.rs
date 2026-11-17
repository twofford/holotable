//! One error type for every handler.
//!
//! Handlers return `Result<_, AppError>`. Because `AppError` implements
//! `IntoResponse`, axum turns an `Err` into a JSON error response, and `?`
//! works on database and task errors thanks to the `#[from]` conversions.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use holotable_core::api::{ApiError, ApiErrorCode};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// The request's data broke a rule. The message is shown to the user.
    #[error("{0}")]
    InvalidInput(String),
    #[error("An account with this email already exists.")]
    EmailTaken,
    /// Deliberately vague: it must not reveal whether the email exists (ACC-004).
    #[error("Wrong email or password.")]
    BadCredentials,
    #[error("You're not signed in.")]
    Unauthorized,
    /// Signed in, but not allowed to do this. The message says why.
    #[error("{0}")]
    Forbidden(String),
    /// Also used for things the user isn't allowed to know exist (CMP-011).
    #[error("{0}")]
    NotFound(String),
    /// The request clashes with the current state. The message says how.
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    TooLarge(String),
    #[error("Too many attempts. Wait a few minutes and try again.")]
    RateLimited,

    // Everything below is a server problem. The details go to the log, and the
    // user only sees a generic message.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("file storage error: {0}")]
    Io(#[from] std::io::Error),
    #[error("background task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    fn status_and_code(&self) -> (StatusCode, ApiErrorCode) {
        use ApiErrorCode as C;
        match self {
            AppError::InvalidInput(_) => (StatusCode::BAD_REQUEST, C::InvalidInput),
            AppError::EmailTaken => (StatusCode::CONFLICT, C::EmailTaken),
            AppError::BadCredentials => (StatusCode::UNAUTHORIZED, C::BadCredentials),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, C::Unauthorized),
            AppError::Forbidden(_) => (StatusCode::FORBIDDEN, C::Forbidden),
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, C::NotFound),
            AppError::Conflict(_) => (StatusCode::CONFLICT, C::Conflict),
            AppError::TooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, C::TooLarge),
            AppError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, C::RateLimited),
            AppError::Database(_) | AppError::Io(_) | AppError::Task(_) | AppError::Internal(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, C::Internal)
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = self.status_and_code();
        let message = if status.is_server_error() {
            tracing::error!(error = %self, "request failed");
            "Something went wrong on the server.".to_owned()
        } else {
            self.to_string()
        };
        (
            status,
            Json(ApiError {
                error: code,
                message,
            }),
        )
            .into_response()
    }
}

/// A 403 for a player trying something only GMs may do.
pub fn gm_only() -> AppError {
    AppError::Forbidden("Only the campaign's GMs can do that.".to_owned())
}

/// A 404 with the standard message.
pub fn not_found() -> AppError {
    AppError::NotFound("Not found.".to_owned())
}

/// Lets handlers write `normalize_email(..).map_err(invalid)?`.
pub fn invalid(message: &str) -> AppError {
    AppError::InvalidInput(message.to_owned())
}
