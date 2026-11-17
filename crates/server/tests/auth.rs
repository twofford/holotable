//! Integration tests for accounts (M2).
//!
//! Files in `tests/` are compiled as separate crates that use the server as a
//! library, like any other user of it would. Each test builds the full app on a
//! fresh in-memory database and sends it requests with `oneshot`, which runs
//! one request through the router without opening a network port.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::{HOST, ORIGIN, PASSWORD, Reply, req, test_app};
use holotable_server::db;
use serde_json::json;
use tower::ServiceExt;

async fn sign_up(app: &axum::Router, email: &str) -> Reply {
    let reply = req("POST", "/api/auth/signup")
        .json(json!({ "email": email, "password": PASSWORD, "displayName": "Vex" }))
        .send(app)
        .await;
    assert_eq!(
        reply.status,
        StatusCode::CREATED,
        "sign-up failed: {}",
        reply.body
    );
    reply
}

async fn sign_in(app: &axum::Router, email: &str, password: &str) -> Reply {
    req("POST", "/api/auth/signin")
        .json(json!({ "email": email, "password": password }))
        .send(app)
        .await
}

/// Shorthand for the many requests in this file that are just method, path,
/// optional cookie and optional JSON body.
async fn send(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Option<serde_json::Value>,
) -> Reply {
    send_from(app, method, path, cookie, body, Some(ORIGIN)).await
}

async fn send_from(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Option<serde_json::Value>,
    origin: Option<&str>,
) -> Reply {
    let mut r = req(method, path).origin(origin);
    if let Some(c) = cookie {
        r = r.cookie(c);
    }
    if let Some(b) = body {
        r = r.json(b);
    }
    r.send(app).await
}

#[tokio::test]
async fn sign_up_sets_a_safe_cookie_and_signs_in() {
    let t = test_app().await;
    let app = &t.app;
    let reply = sign_up(app, "vex@example.com").await;
    assert_eq!(reply.body["email"], "vex@example.com");
    assert_eq!(reply.body["displayName"], "Vex");

    let cookie = reply.set_cookie.clone().unwrap();
    for attr in ["HttpOnly", "SameSite=Lax", "Secure", "Path=/"] {
        assert!(cookie.contains(attr), "cookie is missing {attr}: {cookie}");
    }

    let me = send(app, "GET", "/api/auth/me", Some(&reply.cookie()), None).await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.body["email"], "vex@example.com");
}

#[tokio::test]
async fn me_without_a_session_is_401() {
    let t = test_app().await;
    let app = &t.app;
    let reply = send(app, "GET", "/api/auth/me", None, None).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.error_code(), "unauthorized");

    let junk = send(
        app,
        "GET",
        "/api/auth/me",
        Some("holotable_session=abc"),
        None,
    )
    .await;
    assert_eq!(junk.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn emails_are_normalized_and_unique() {
    let t = test_app().await;
    let app = &t.app;
    let first = sign_up(app, "  Ada@Example.COM ").await;
    assert_eq!(first.body["email"], "ada@example.com");

    let again = send(
        app,
        "POST",
        "/api/auth/signup",
        None,
        Some(json!({ "email": "ada@example.com", "password": PASSWORD, "displayName": "Other" })),
    )
    .await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.error_code(), "email_taken");
}

#[tokio::test]
async fn sign_up_checks_its_input() {
    let t = test_app().await;
    let app = &t.app;
    for body in [
        json!({ "email": "ada@example.com", "password": "short", "displayName": "Ada" }),
        json!({ "email": "not-an-email", "password": PASSWORD, "displayName": "Ada" }),
        json!({ "email": "ada@example.com", "password": PASSWORD, "displayName": "   " }),
    ] {
        let reply = send(app, "POST", "/api/auth/signup", None, Some(body.clone())).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(reply.error_code(), "invalid_input");
    }
}

#[tokio::test]
async fn wrong_password_and_unknown_email_look_the_same() {
    let t = test_app().await;
    let app = &t.app;
    sign_up(app, "vex@example.com").await;

    let ok = sign_in(app, "VEX@example.com", PASSWORD).await;
    assert_eq!(ok.status, StatusCode::OK);
    assert!(ok.set_cookie.is_some());

    let wrong_password = sign_in(app, "vex@example.com", "not the password").await;
    let unknown_email = sign_in(app, "nobody@example.com", PASSWORD).await;
    assert_eq!(wrong_password.status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong_password.body, unknown_email.body);
    assert_eq!(wrong_password.error_code(), "bad_credentials");
    assert!(wrong_password.set_cookie.is_none());
}

#[tokio::test]
async fn sign_out_ends_the_session() {
    let t = test_app().await;
    let app = &t.app;
    let cookie = sign_up(app, "vex@example.com").await.cookie();

    let out = send(app, "POST", "/api/auth/signout", Some(&cookie), None).await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert!(
        out.set_cookie.unwrap().contains("Max-Age=0"),
        "cookie should be removed"
    );

    let me = send(app, "GET", "/api/auth/me", Some(&cookie), None).await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn expired_sessions_are_rejected_and_used_ones_slide() {
    let t = test_app().await;
    let (app, pool) = (&t.app, &t.db);
    let cookie = sign_up(app, "vex@example.com").await.cookie();
    let now = db::now();

    // A session with 10 days left is pushed back out to 30 on use.
    sqlx::query("UPDATE sessions SET expires_at = ?")
        .bind(now + 10 * 86_400)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        send(app, "GET", "/api/auth/me", Some(&cookie), None)
            .await
            .status,
        StatusCode::OK
    );
    let expires_at: i64 = sqlx::query_scalar("SELECT expires_at FROM sessions")
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(
        expires_at >= now + 30 * 86_400 - 5,
        "session should be extended to 30 days"
    );

    // An expired session doesn't work.
    sqlx::query("UPDATE sessions SET expires_at = ?")
        .bind(now - 1)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        send(app, "GET", "/api/auth/me", Some(&cookie), None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn secrets_are_never_stored_in_plain_text() {
    let t = test_app().await;
    let (app, pool) = (&t.app, &t.db);
    let cookie = sign_up(app, "vex@example.com").await.cookie();
    let token = cookie.split_once('=').unwrap().1;

    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users")
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(hash.starts_with("$argon2id$"));
    assert!(!hash.contains(PASSWORD));

    let stored: Vec<u8> = sqlx::query_scalar("SELECT token_hash FROM sessions")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(stored.len(), 32, "a SHA-256 hash");
    assert_ne!(stored, token.as_bytes());
}

#[tokio::test]
async fn changing_the_password_ends_other_sessions() {
    let t = test_app().await;
    let app = &t.app;
    let here = sign_up(app, "vex@example.com").await.cookie();
    let elsewhere = sign_in(app, "vex@example.com", PASSWORD).await.cookie();
    let new_password = "a much better passphrase";

    let wrong = send(
        app,
        "POST",
        "/api/account/password",
        Some(&here),
        Some(json!({ "currentPassword": "not it at all", "newPassword": new_password })),
    )
    .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);

    let changed = send(
        app,
        "POST",
        "/api/account/password",
        Some(&here),
        Some(json!({ "currentPassword": PASSWORD, "newPassword": new_password })),
    )
    .await;
    assert_eq!(changed.status, StatusCode::NO_CONTENT);

    assert_eq!(
        send(app, "GET", "/api/auth/me", Some(&here), None)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        send(app, "GET", "/api/auth/me", Some(&elsewhere), None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        sign_in(app, "vex@example.com", PASSWORD).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        sign_in(app, "vex@example.com", new_password).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn display_name_can_be_changed() {
    let t = test_app().await;
    let app = &t.app;
    let cookie = sign_up(app, "vex@example.com").await.cookie();
    let reply = send(
        app,
        "PATCH",
        "/api/account",
        Some(&cookie),
        Some(json!({ "displayName": " Nyx " })),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body["displayName"], "Nyx");
    assert_eq!(
        send(app, "GET", "/api/auth/me", Some(&cookie), None)
            .await
            .body["displayName"],
        "Nyx"
    );
}

#[tokio::test]
async fn requests_from_other_sites_are_refused() {
    let t = test_app().await;
    let app = &t.app;
    let cookie = sign_up(app, "vex@example.com").await.cookie();

    let evil = send_from(
        app,
        "PATCH",
        "/api/account",
        Some(&cookie),
        Some(json!({ "displayName": "Pwned" })),
        Some("https://evil.example"),
    )
    .await;
    assert_eq!(evil.status, StatusCode::FORBIDDEN);
    assert_eq!(evil.error_code(), "forbidden");

    // Without an Origin header (curl, scripts) the request is allowed.
    let no_origin = send_from(
        app,
        "PATCH",
        "/api/account",
        Some(&cookie),
        Some(json!({ "displayName": "Nyx" })),
        None,
    )
    .await;
    assert_eq!(no_origin.status, StatusCode::OK);
}

#[tokio::test]
async fn sign_in_is_rate_limited() {
    let t = test_app().await;
    let app = &t.app;
    for _ in 0..10 {
        assert_eq!(
            sign_in(app, "vex@example.com", "wrong password")
                .await
                .status,
            StatusCode::UNAUTHORIZED
        );
    }
    let blocked = sign_in(app, "vex@example.com", "wrong password").await;
    assert_eq!(blocked.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(blocked.error_code(), "rate_limited");
}

#[tokio::test]
async fn websocket_needs_a_session() {
    let t = test_app().await;
    let app = &t.app;
    let req = Request::builder()
        .uri("/ws")
        .header(header::HOST, HOST)
        .header(header::ORIGIN, ORIGIN)
        .header(header::CONNECTION, "upgrade")
        .header(header::UPGRADE, "websocket")
        .header(header::SEC_WEBSOCKET_VERSION, "13")
        .header(header::SEC_WEBSOCKET_KEY, "dGhlIHNhbXBsZSBub25jZQ==")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
