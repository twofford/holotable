//! The Holotable server, as a library.
//!
//! `main.rs` is a thin wrapper that reads the config and starts listening.
//! Everything else lives here, so integration tests in `tests/` can build the
//! same app and send it requests without opening a network port.

pub mod auth;
pub mod campaigns;
pub mod config;
pub mod db;
pub mod error;
pub mod live;
pub mod origin;
pub mod storage;
pub mod tokens;
pub mod ws;

use std::{path::Path, sync::Arc, time::Duration};

use axum::{Router, middleware, routing::get};
use sqlx::SqlitePool;
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

use crate::{auth::rate_limit::RateLimiter, config::Config, live::Hub, storage::Storage};

/// Shared state that every handler can reach with `State<AppState>`.
///
/// axum clones it for each request, so it must be cheap to clone: `SqlitePool`
/// is already a handle to shared connections, and the rest sits behind `Arc`
/// (a reference-counted pointer: cloning it just bumps a counter).
#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub config: Arc<Config>,
    pub auth_limiter: Arc<RateLimiter>,
    pub storage: Arc<Storage>,
    /// Open connections and running scene rooms (M4).
    pub hub: Arc<Hub>,
}

impl AppState {
    pub fn new(db: SqlitePool, config: Config) -> Self {
        let storage = Arc::new(Storage::new(&config.data_dir));
        Self {
            hub: Arc::new(Hub::new(db.clone(), storage.clone())),
            db,
            storage,
            config: Arc::new(config),
            // 10 attempts per key per 15 minutes (spec ACC-007).
            auth_limiter: Arc::new(RateLimiter::new(10, Duration::from_secs(15 * 60))),
        }
    }
}

/// Builds the router.
pub fn app(state: AppState) -> Router {
    // Unknown paths fall back to index.html, so client-side routes still load.
    let static_dir: &Path = &state.config.static_dir;
    let static_files =
        ServeDir::new(static_dir).fallback(ServeFile::new(static_dir.join("index.html")));

    Router::new()
        .merge(auth::routes())
        .merge(campaigns::routes(state.clone()))
        .route("/api/health", get(|| async { "ok" }))
        .route("/ws", get(ws::upgrade))
        .fallback_service(static_files)
        // Layers wrap everything added above them. The last one added runs first.
        .layer(middleware::from_fn(origin::check_origin))
        .layer(TraceLayer::new_for_http())
        // `with_state` hands the state to every route, and turns the
        // `Router<AppState>` into a plain `Router` that's ready to serve.
        .with_state(state)
}

/// Deletes expired sessions every hour, for as long as the server runs.
pub fn spawn_session_cleanup(db: SqlitePool) {
    // `tokio::spawn` runs the future as its own task; it keeps going after this
    // function returns. `move` gives the task ownership of `db`.
    tokio::spawn(async move {
        let mut every_hour = tokio::time::interval(Duration::from_secs(60 * 60));
        loop {
            every_hour.tick().await;
            match auth::session::delete_expired(&db).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(removed = n, "expired sessions removed"),
                Err(e) => tracing::warn!(error = %e, "expired-session cleanup failed"),
            }
        }
    });
}
