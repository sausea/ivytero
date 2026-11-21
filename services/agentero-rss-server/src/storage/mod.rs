//! `rss-archive.sqlite`: feed collection config, archived feed entries and
//! fetch bookkeeping. No reader state (star / read / tags / AI / vault).

mod migrations;
pub mod queries;

use rusqlite::Connection;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

pub use migrations::SCHEMA_VERSION;
pub use queries::*;

#[derive(Debug)]
pub struct StorageError(pub String);

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StorageError {}

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        Self(e.to_string())
    }
}

pub type StorageResult<T> = Result<T, StorageError>;

/// One serialized connection. A personal archive does not need a pool; WAL
/// lets the admin CLI read and write alongside the server process.
#[derive(Clone)]
pub struct Archive {
    conn: Arc<Mutex<Connection>>,
}

impl Archive {
    pub fn open(path: &Path) -> StorageResult<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .map_err(|e| StorageError(format!("create {}: {e}", parent.display())))?;
        }
        let mut conn = Connection::open(path)?;
        migrations::configure(&conn)?;
        migrations::migrate(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Run a blocking query off the async runtime.
    pub async fn run<T, F>(&self, f: F) -> StorageResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> StorageResult<T> + Send + 'static,
    {
        let archive = self.clone();
        tokio::task::spawn_blocking(move || f(&mut archive.lock()))
            .await
            .map_err(|e| StorageError(format!("storage task: {e}")))?
    }
}

pub fn now_rfc3339() -> String {
    to_rfc3339(chrono::Utc::now())
}

pub fn to_rfc3339(at: chrono::DateTime<chrono::Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
