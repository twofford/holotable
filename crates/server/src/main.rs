//! Holotable server entry point. See `lib.rs` for the app itself.
//!
//! Configuration comes from environment variables (see `config.rs`):
//! `HOLOTABLE_ADDR`, `HOLOTABLE_STATIC_DIR`, `HOLOTABLE_DATABASE_URL`,
//! `HOLOTABLE_SECURE_COOKIES`, and `RUST_LOG` for log filtering.

use std::net::SocketAddr;

use holotable_server::{AppState, app, config::Config, db, spawn_session_cleanup};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("holotable=info,holotable_server=info,tower_http=info")
        }))
        .init();

    let config = Config::from_env();
    if !config.static_dir.join("index.html").exists() {
        tracing::warn!(
            dir = %config.static_dir.display(),
            "no index.html found; run `npm run build` in client/ first, or use the Vite dev server"
        );
    }
    if !config.secure_cookies {
        tracing::warn!(
            "session cookies are not marked Secure; fine for local http, not for a public server"
        );
    }

    let pool = db::connect(&config.database_url).await?;
    tracing::info!(database = %config.database_url, "database ready");
    spawn_session_cleanup(pool.clone());

    let addr = config.addr;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("listening on http://{addr}");
    // `with_connect_info` makes each client's address available to handlers
    // (the `ClientIp` extractor uses it for rate limiting).
    let service =
        app(AppState::new(pool, config)).into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, service)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
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
