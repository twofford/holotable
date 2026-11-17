//! Cross-site request protection (spec NFR-002).
//!
//! Browsers send an `Origin` header with every POST, PATCH and DELETE, and with
//! WebSocket upgrades. If it doesn't match the server's own address, another
//! site's page is making the request in the user's name: refuse it. Together
//! with `SameSite=Lax` cookies, this stops cross-site request forgery and
//! cross-site WebSocket hijacking.
//!
//! Requests without an `Origin` header (curl, scripts) are allowed: they can't
//! carry a victim's browser cookies anyway.

use axum::{
    extract::Request,
    http::{Method, header},
    middleware::Next,
    response::Response,
};

use crate::error::AppError;

/// Middleware: runs before every handler. `next.run(req)` passes the request on.
pub async fn check_origin(req: Request, next: Next) -> Result<Response, AppError> {
    let is_websocket = req
        .headers()
        .get(header::UPGRADE)
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"websocket"));
    let changes_state = !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);

    if (changes_state || is_websocket)
        && let Some(origin) = req.headers().get(header::ORIGIN)
    {
        let host = req
            .headers()
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        let origin = origin.to_str().unwrap_or("");
        if !origin_matches_host(origin, host) {
            tracing::warn!(origin, host, "request from another origin refused");
            return Err(AppError::Forbidden(
                "That request isn't allowed.".to_owned(),
            ));
        }
    }
    Ok(next.run(req).await)
}

/// `true` if `origin` (like `https://example.com:8443`) names the same host and
/// port as the `Host` header (like `example.com:8443`).
fn origin_matches_host(origin: &str, host: &str) -> bool {
    let without_scheme = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"));
    !host.is_empty() && without_scheme.is_some_and(|o| o.eq_ignore_ascii_case(host))
}

#[cfg(test)]
mod tests {
    use super::origin_matches_host;

    #[test]
    fn matching() {
        assert!(origin_matches_host(
            "http://127.0.0.1:3000",
            "127.0.0.1:3000"
        ));
        assert!(origin_matches_host(
            "https://Holotable.example",
            "holotable.example"
        ));
    }

    #[test]
    fn not_matching() {
        assert!(!origin_matches_host(
            "https://evil.example",
            "holotable.example"
        ));
        assert!(!origin_matches_host(
            "http://127.0.0.1:5173",
            "127.0.0.1:3000"
        ));
        assert!(!origin_matches_host("null", "holotable.example"));
        assert!(!origin_matches_host("https://holotable.example", ""));
    }
}
