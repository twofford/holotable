//! The SQLite database.

use std::{str::FromStr, time::Duration};

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

/// Opens (creating if needed) the database at `url` and applies migrations.
pub async fn connect(url: &str) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        // SQLite ignores REFERENCES ... ON DELETE CASCADE unless this is on.
        .foreign_keys(true)
        // Write-ahead logging: readers don't block the writer (spec NFR-015).
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        // Wait up to 5 s for a lock instead of failing straight away.
        .busy_timeout(Duration::from_secs(5));

    // SQLite creates the file but not the folder it goes in.
    if let Some(dir) = options.get_filename().parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).map_err(sqlx::Error::Io)?;
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

/// A private, empty, in-memory database, for tests.
///
/// An in-memory SQLite database lives only as long as its connection, so the
/// pool keeps exactly one connection open forever.
pub async fn connect_in_memory() -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .idle_timeout(None)
        .max_lifetime(None)
        .connect_with(options)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

/// Applies any migrations in `crates/server/migrations` that haven't run yet.
///
/// `migrate!` reads the .sql files at compile time and builds them into the
/// binary, so the deployed server needs no migration files beside it. sqlx
/// records each applied migration in a `_sqlx_migrations` table.
async fn migrate(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}

/// The current time as Unix seconds, matching the INTEGER time columns.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after 1970")
        .as_secs() as i64
}
