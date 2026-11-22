//! Schema V2 (V1 plus `feeds.folder`). Article ids are minted once and never regenerated; see the
//! design doc §3 before changing anything here.

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

pub const SCHEMA_VERSION: &str = "2";

const SCHEMA_V1: &str = "
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS feeds (
    id TEXT PRIMARY KEY,
    url TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
    refresh_interval_secs INTEGER NOT NULL DEFAULT 600
        CHECK (refresh_interval_secs BETWEEN 120 AND 86400),
    added_at TEXT NOT NULL,
    last_attempt_at TEXT,
    last_success_at TEXT,
    next_fetch_at TEXT,
    failure_count INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    etag TEXT,
    last_modified TEXT
);

CREATE TABLE IF NOT EXISTS articles (
    id TEXT PRIMARY KEY,
    feed_id TEXT NOT NULL REFERENCES feeds(id) ON DELETE RESTRICT,
    guid TEXT NOT NULL,
    title TEXT NOT NULL,
    url TEXT,
    published_at TEXT,
    first_seen_at TEXT NOT NULL,
    sort_at TEXT NOT NULL,
    summary_text TEXT NOT NULL DEFAULT '',
    content_html TEXT,
    paper_url TEXT,
    content_sha256 TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (feed_id, guid)
);

CREATE INDEX IF NOT EXISTS idx_articles_timeline
ON articles(sort_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_articles_feed_timeline
ON articles(feed_id, sort_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_feeds_due
ON feeds(enabled, next_fetch_at);
";

pub fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    Ok(())
}

/// Create tables on first open and mint the persistent `server_id`.
pub fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute_batch(SCHEMA_V1)?;
    let version: Option<String> = tx
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    match version.as_deref() {
        None => {
            tx.execute(
                "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)",
                [SCHEMA_VERSION],
            )?;
        }
        Some("1") => {
            tx.execute(
                "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
                [SCHEMA_VERSION],
            )?;
        }
        Some(SCHEMA_VERSION) => {}
        Some(other) => {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "unsupported schema_version {other}; this build understands {SCHEMA_VERSION}"
            )));
        }
    }
    let has_folder: bool = tx.query_row(
        "SELECT COUNT(*) > 0 FROM pragma_table_info('feeds') WHERE name = 'folder'",
        [],
        |r| r.get(0),
    )?;
    if !has_folder {
        tx.execute_batch("ALTER TABLE feeds ADD COLUMN folder TEXT NOT NULL DEFAULT ''")?;
    }
    tx.execute(
        "INSERT OR IGNORE INTO meta (key, value) VALUES ('server_id', ?1)",
        params![Uuid::new_v4().to_string()],
    )?;
    tx.commit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrades_v1_archives_in_place() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA_V1).unwrap();
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('schema_version', '1')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO feeds (id, url, title, added_at) VALUES ('f', 'https://a.example/', 'A', 'now')",
            [],
        )
        .unwrap();
        migrate(&mut conn).unwrap();
        migrate(&mut conn).unwrap();
        let (version, folder): (String, String) = conn
            .query_row(
                "SELECT (SELECT value FROM meta WHERE key = 'schema_version'), folder FROM feeds",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((version.as_str(), folder.as_str()), (SCHEMA_VERSION, ""));
    }
}
