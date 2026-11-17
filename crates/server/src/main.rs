//! Holotable server entry point.
//!
//! M0: serves the built client as static files and runs a WebSocket ping/pong.
//! Configuration comes from environment variables:
//! - `HOLOTABLE_ADDR` (default `127.0.0.1:3000`)
//! - `HOLOTABLE_STATIC_DIR` (default `client/dist`)
//! - `RUST_LOG` (default `holotable=info,tower_http=info`)

mod ws;

use std::{net::SocketAddr, path::PathBuf};

use axum::{Router, routing::get};
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("holotable=info,tower_http=info")),
        )
        .init();

    let addr: SocketAddr = std::env::var("HOLOTABLE_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:3000".into())
        .parse()
        .expect("HOLOTABLE_ADDR must be an address like 127.0.0.1:3000");
    let static_dir = PathBuf::from(
        std::env::var("HOLOTABLE_STATIC_DIR").unwrap_or_else(|_| "client/dist".into()),
    );
    if !static_dir.join("index.html").exists() {
        tracing::warn!(
            dir = %static_dir.display(),
            "no index.html found; run `npm run build` in client/ first, or use the Vite dev server"
        );
    }

    let app = app(static_dir);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("listening on http://{addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
}

/// Builds the router. Kept separate from `main` so that tests can build it too.
fn app(static_dir: PathBuf) -> Router {
    // Unknown paths fall back to index.html, so client-side routes still load.
    let index = static_dir.join("index.html");
    let static_files = ServeDir::new(static_dir).fallback(ServeFile::new(index));

    Router::new()
        .route("/api/health", get(|| async { "ok" }))
        .route("/ws", get(ws::upgrade))
        .fallback_service(static_files)
        .layer(TraceLayer::new_for_http())
}

/// Resolves when the process gets Ctrl-C or (on Unix) SIGTERM.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to listen for Ctrl-C");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to listen for SIGTERM")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutting down");
}
