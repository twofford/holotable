//! Helpers shared by the integration tests.
//!
//! Each file in `tests/` is its own crate; `mod common;` at the top of a test
//! file pulls this module in. (A `common/mod.rs` folder, not `common.rs`, so
//! cargo doesn't treat it as a test file of its own.)

#![allow(dead_code)] // Not every test file uses every helper.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use holotable_server::{AppState, app, config::Config, db};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tempfile::TempDir;
use tower::ServiceExt;

pub const HOST: &str = "holotable.test";
pub const ORIGIN: &str = "http://holotable.test";
pub const PASSWORD: &str = "correct horse battery";

/// A running app on a fresh in-memory database and an empty temporary data
/// folder. The folder is deleted when the `TestApp` is dropped.
pub struct TestApp {
    pub app: Router,
    pub db: SqlitePool,
    pub data_dir: TempDir,
}

pub async fn test_app() -> TestApp {
    let pool = db::connect_in_memory().await.expect("in-memory database");
    let data_dir = tempfile::tempdir().expect("temporary folder");
    let config = Config {
        addr: "127.0.0.1:0".parse().unwrap(),
        static_dir: "does-not-exist".into(),
        database_url: "sqlite::memory:".into(),
        data_dir: data_dir.path().to_owned(),
        secure_cookies: true,
    };
    TestApp {
        app: app(AppState::new(pool.clone(), config)),
        db: pool,
        data_dir,
    }
}

/// What a test needs from a response.
pub struct Reply {
    pub status: StatusCode,
    pub set_cookie: Option<String>,
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
    pub body: Value,
}

impl Reply {
    /// The `name=value` part of `Set-Cookie`, ready to send back in a `Cookie` header.
    pub fn cookie(&self) -> String {
        let raw = self.set_cookie.as_deref().expect("response sets a cookie");
        raw.split(';').next().unwrap().to_owned()
    }

    pub fn error_code(&self) -> &str {
        self.body["error"].as_str().unwrap_or("")
    }

    pub fn id(&self) -> i64 {
        self.body["id"].as_i64().expect("response has an id")
    }
}

/// A request under construction. Defaults: sent from the app's own origin,
/// no cookie, no body.
pub struct Req<'a> {
    method: &'a str,
    path: String,
    cookie: Option<&'a str>,
    origin: Option<&'a str>,
    body: Body,
    content_type: Option<&'a str>,
}

pub fn req<'a>(method: &'a str, path: impl Into<String>) -> Req<'a> {
    Req {
        method,
        path: path.into(),
        cookie: None,
        origin: Some(ORIGIN),
        body: Body::empty(),
        content_type: None,
    }
}

impl<'a> Req<'a> {
    pub fn cookie(mut self, cookie: &'a str) -> Self {
        self.cookie = Some(cookie);
        self
    }

    pub fn origin(mut self, origin: Option<&'a str>) -> Self {
        self.origin = origin;
        self
    }

    pub fn json(mut self, value: Value) -> Self {
        self.body = Body::from(value.to_string());
        self.content_type = Some("application/json");
        self
    }

    pub fn bytes(mut self, bytes: Vec<u8>) -> Self {
        self.body = Body::from(bytes);
        self.content_type = Some("application/octet-stream");
        self
    }

    pub async fn send(self, app: &Router) -> Reply {
        let mut builder = Request::builder()
            .method(self.method)
            .uri(&self.path)
            .header(header::HOST, HOST);
        if let Some(origin) = self.origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        if let Some(cookie) = self.cookie {
            builder = builder.header(header::COOKIE, cookie);
        }
        if let Some(ct) = self.content_type {
            builder = builder.header(header::CONTENT_TYPE, ct);
        }
        // `oneshot` consumes the service, so send to a clone. Cloning a Router is cheap.
        let res = app
            .clone()
            .oneshot(builder.body(self.body).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let header_str = |name| {
            res.headers()
                .get(name)
                .map(|v: &header::HeaderValue| v.to_str().unwrap().to_owned())
        };
        let set_cookie = header_str(header::SET_COOKIE);
        let content_type = header_str(header::CONTENT_TYPE);
        let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Reply {
            status,
            set_cookie,
            content_type,
            bytes,
            body,
        }
    }
}

/// Creates an account and returns its session cookie.
pub async fn sign_up_as(app: &Router, email: &str, display_name: &str) -> String {
    let reply = req("POST", "/api/auth/signup")
        .json(json!({ "email": email, "password": PASSWORD, "displayName": display_name }))
        .send(app)
        .await;
    assert_eq!(
        reply.status,
        StatusCode::CREATED,
        "sign-up failed: {}",
        reply.body
    );
    reply.cookie()
}
