//! Extractors: values a handler can ask for in its argument list.
//!
//! A handler that takes `CurrentUser` only runs for signed-in users; for anyone
//! else axum calls `from_request_parts`, gets `Err(AppError::Unauthorized)`,
//! and sends that as the response. That keeps the sign-in check in one place
//! instead of repeating it in every handler.

use std::net::{IpAddr, SocketAddr};

use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::request::Parts,
};
use axum_extra::extract::CookieJar;
use holotable_core::api::UserInfo;

use super::session::{self, COOKIE_NAME, SessionToken};
use crate::{AppState, error::AppError};

/// The signed-in user making the request, and the session they used.
pub struct CurrentUser {
    pub user: UserInfo,
    pub token: SessionToken,
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);
        let token = session_token(&jar).ok_or(AppError::Unauthorized)?;
        let user = session::lookup(&state.db, &token)
            .await?
            .ok_or(AppError::Unauthorized)?;
        Ok(CurrentUser { user, token })
    }
}

/// Reads the session token from the request's cookies, if there is a valid-looking one.
pub fn session_token(jar: &CookieJar) -> Option<SessionToken> {
    jar.get(COOKIE_NAME)
        .and_then(|c| SessionToken::parse(c.value()))
}

/// The client's IP address, for rate limiting.
///
/// `None` when the connection info isn't available, as in tests that call the
/// router directly. Behind Caddy this will be Caddy's address; reading the real
/// client IP from `X-Forwarded-For` comes with deployment.
pub struct ClientIp(pub Option<IpAddr>);

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    // `Infallible`: this extractor can't fail; it just may find nothing.
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let ip = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.ip());
        Ok(ClientIp(ip))
    }
}
