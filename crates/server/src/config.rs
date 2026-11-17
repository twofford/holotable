//! Server settings, read from environment variables at startup.

use std::{net::SocketAddr, path::PathBuf};

#[derive(Debug, Clone)]
pub struct Config {
    /// `HOLOTABLE_ADDR`, default `127.0.0.1:3000`.
    pub addr: SocketAddr,
    /// `HOLOTABLE_STATIC_DIR`, default `client/dist`.
    pub static_dir: PathBuf,
    /// `HOLOTABLE_DATABASE_URL`, default `sqlite://data/holotable.db`.
    pub database_url: String,
    /// `HOLOTABLE_DATA_DIR`, default `data`. Uploaded files go in `campaigns/` here.
    pub data_dir: PathBuf,
    /// `HOLOTABLE_SECURE_COOKIES`: `true` or `false`.
    ///
    /// A `Secure` cookie is only sent over HTTPS. That's required in production,
    /// but it breaks sign-in over plain `http://` on your own machine (Safari
    /// won't keep Secure cookies there). So the default follows the build:
    /// `false` for debug builds (`cargo run`), `true` for release builds
    /// (`cargo build --release`, which is what gets deployed).
    pub secure_cookies: bool,
}

impl Config {
    pub fn from_env() -> Self {
        let var =
            |name: &str, default: &str| std::env::var(name).unwrap_or_else(|_| default.to_owned());
        Self {
            addr: var("HOLOTABLE_ADDR", "127.0.0.1:3000")
                .parse()
                .expect("HOLOTABLE_ADDR must be an address like 127.0.0.1:3000"),
            static_dir: PathBuf::from(var("HOLOTABLE_STATIC_DIR", "client/dist")),
            database_url: var("HOLOTABLE_DATABASE_URL", "sqlite://data/holotable.db"),
            data_dir: PathBuf::from(var("HOLOTABLE_DATA_DIR", "data")),
            secure_cookies: match std::env::var("HOLOTABLE_SECURE_COOKIES") {
                Ok(v) => v == "true" || v == "1",
                // `cfg!(debug_assertions)` is true in debug builds, false in release.
                Err(_) => !cfg!(debug_assertions),
            },
        }
    }
}
