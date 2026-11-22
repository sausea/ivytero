//! Reader state for cloud articles, kept on this device only: the archive
//! never stores read marks. Per server, an article is read when the archive
//! first saw it at or before the `unread_since` watermark; a mark records
//! only the articles that differ from that default.

use crate::error::AppError;
use crate::paths;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn db_path() -> PathBuf {
    paths::agentero_data_dir().join("rss-cloud-reads.sqlite")
}

pub fn open() -> Result<Connection, AppError> {
    open_at(&db_path())
}

fn open_at(path: &Path) -> Result<Connection, AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS cloud_read_watermarks (
             server_id TEXT PRIMARY KEY,
             unread_since TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS cloud_read_marks (
             server_id TEXT NOT NULL,
             article_id TEXT NOT NULL,
             feed_id TEXT NOT NULL,
             first_seen_at TEXT NOT NULL,
             read INTEGER NOT NULL CHECK (read IN (0, 1)),
             marked_at TEXT NOT NULL,
             PRIMARY KEY (server_id, article_id)
         );
         CREATE INDEX IF NOT EXISTS idx_cloud_read_marks_feed
             ON cloud_read_marks(server_id, feed_id);",
    )?;
    Ok(conn)
}

pub fn unread_since(conn: &Connection, server_id: &str) -> Result<Option<String>, AppError> {
    Ok(conn
        .query_row(
            "SELECT unread_since FROM cloud_read_watermarks WHERE server_id = ?1",
            [server_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// Move the watermark to `now`: everything seen so far becomes read.
pub fn advance(conn: &mut Connection, server_id: &str, now: &str) -> Result<(), AppError> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO cloud_read_watermarks (server_id, unread_since) VALUES (?1, ?2)
         ON CONFLICT (server_id) DO UPDATE SET unread_since = excluded.unread_since",
        params![server_id, now],
    )?;
    tx.execute(
        "DELETE FROM cloud_read_marks WHERE server_id = ?1",
        [server_id],
    )?;
    tx.commit()?;
    Ok(())
}

/// `(read, marked_at)` for the given articles that carry a mark.
pub fn marks(
    conn: &Connection,
    server_id: &str,
    ids: &[&str],
) -> Result<HashMap<String, (bool, String)>, AppError> {
    let mut stmt = conn.prepare_cached(
        "SELECT read, marked_at FROM cloud_read_marks WHERE server_id = ?1 AND article_id = ?2",
    )?;
    let mut out = HashMap::new();
    for id in ids {
        if let Some((read, at)) = stmt
            .query_row(params![server_id, id], |r| {
                Ok((r.get::<_, i64>(0)? != 0, r.get::<_, String>(1)?))
            })
            .optional()?
        {
            out.insert((*id).to_string(), (read, at));
        }
    }
    Ok(out)
}

/// Lower bound for an unread-only scan: the watermark, or just before the
/// oldest article explicitly marked unread so it is not filtered out.
pub fn unread_scan_since(
    conn: &Connection,
    server_id: &str,
    since: &str,
    feed_id: Option<&str>,
) -> Result<String, AppError> {
    let oldest: Option<String> = conn.query_row(
        "SELECT MIN(first_seen_at) FROM cloud_read_marks
         WHERE server_id = ?1 AND read = 0 AND (?2 IS NULL OR feed_id = ?2)",
        params![server_id, feed_id],
        |r| r.get(0),
    )?;
    let Some(oldest) = oldest.filter(|o| o.as_str() <= since) else {
        return Ok(since.to_string());
    };
    Ok(chrono::DateTime::parse_from_rfc3339(&oldest)
        .map(|t| {
            (t.with_timezone(&chrono::Utc) - chrono::Duration::milliseconds(1))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        })
        .unwrap_or_else(|_| since.to_string()))
}

/// `read_at` shown for an article: the mark time, the watermark, or unread.
pub fn read_at(since: &str, first_seen_at: &str, mark: Option<&(bool, String)>) -> Option<String> {
    match mark {
        Some((true, at)) => Some(at.clone()),
        Some((false, _)) => None,
        None => (first_seen_at <= since).then(|| since.to_string()),
    }
}

pub struct Article<'a> {
    pub id: &'a str,
    pub feed_id: &'a str,
    pub first_seen_at: &'a str,
}

pub fn set_read(
    conn: &Connection,
    server_id: &str,
    since: &str,
    article: &Article<'_>,
    read: bool,
    now: &str,
) -> Result<Option<String>, AppError> {
    if read == (article.first_seen_at <= since) {
        conn.execute(
            "DELETE FROM cloud_read_marks WHERE server_id = ?1 AND article_id = ?2",
            params![server_id, article.id],
        )?;
        return Ok(read.then(|| since.to_string()));
    }
    conn.execute(
        "INSERT INTO cloud_read_marks (server_id, article_id, feed_id, first_seen_at, read, marked_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (server_id, article_id) DO UPDATE SET
             read = excluded.read, marked_at = excluded.marked_at",
        params![server_id, article.id, article.feed_id, article.first_seen_at, read, now],
    )?;
    Ok(read.then(|| now.to_string()))
}

/// Mark every listed article of one feed read (they are all newer than `since`).
pub fn mark_feed_read(
    conn: &mut Connection,
    server_id: &str,
    since: &str,
    feed_id: &str,
    articles: &[(String, String)],
    now: &str,
) -> Result<(), AppError> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM cloud_read_marks WHERE server_id = ?1 AND feed_id = ?2 AND read = 0",
        params![server_id, feed_id],
    )?;
    for (id, first_seen_at) in articles {
        set_read(
            &tx,
            server_id,
            since,
            &Article {
                id,
                feed_id,
                first_seen_at,
            },
            true,
            now,
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Per feed: archive count after `since`, corrected by this device's marks.
pub fn unread_counts(
    conn: &Connection,
    server_id: &str,
    since: &str,
    new_counts: &HashMap<String, i64>,
) -> Result<HashMap<String, i64>, AppError> {
    let mut out = new_counts.clone();
    let mut stmt = conn.prepare(
        "SELECT feed_id,
                SUM(CASE WHEN read = 1 AND first_seen_at > ?2 THEN 1 ELSE 0 END),
                SUM(CASE WHEN read = 0 AND first_seen_at <= ?2 THEN 1 ELSE 0 END)
         FROM cloud_read_marks WHERE server_id = ?1 GROUP BY feed_id",
    )?;
    let rows = stmt.query_map(params![server_id, since], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (feed_id, read_new, unread_old) = row?;
        if let Some(n) = out.get_mut(&feed_id) {
            *n = (*n - read_new + unread_old).max(0);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SID: &str = "srv";
    const T0: &str = "2026-01-02T00:00:00.000Z";
    const OLD: &str = "2026-01-01T00:00:00.000Z";
    const NEW: &str = "2026-01-03T00:00:00.000Z";
    const NOW: &str = "2026-01-04T00:00:00.000Z";

    fn art<'a>(id: &'a str, first_seen_at: &'a str) -> Article<'a> {
        Article {
            id,
            feed_id: "f",
            first_seen_at,
        }
    }

    #[test]
    fn marks_only_store_deviations_and_correct_counts() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open_at(&dir.path().join("r.sqlite")).unwrap();
        assert_eq!(unread_since(&conn, SID).unwrap(), None);
        advance(&mut conn, SID, T0).unwrap();
        assert_eq!(unread_since(&conn, SID).unwrap().as_deref(), Some(T0));

        assert_eq!(read_at(T0, OLD, None).as_deref(), Some(T0));
        assert_eq!(read_at(T0, NEW, None), None);

        // New article read, old article back to unread; reverting deletes marks.
        assert_eq!(
            set_read(&conn, SID, T0, &art("n1", NEW), true, NOW)
                .unwrap()
                .as_deref(),
            Some(NOW)
        );
        assert_eq!(unread_scan_since(&conn, SID, T0, None).unwrap(), T0);
        assert_eq!(
            set_read(&conn, SID, T0, &art("o1", OLD), false, NOW).unwrap(),
            None
        );
        assert_eq!(
            unread_scan_since(&conn, SID, T0, Some("f")).unwrap(),
            "2025-12-31T23:59:59.999Z"
        );
        assert_eq!(unread_scan_since(&conn, SID, T0, Some("g")).unwrap(), T0);
        let counts = unread_counts(&conn, SID, T0, &HashMap::from([("f".into(), 3)])).unwrap();
        assert_eq!(counts["f"], 3);
        let m = marks(&conn, SID, &["n1", "o1", "x"]).unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(
            set_read(&conn, SID, T0, &art("o1", OLD), true, NOW)
                .unwrap()
                .as_deref(),
            Some(T0)
        );
        let counts = unread_counts(&conn, SID, T0, &HashMap::from([("f".into(), 3)])).unwrap();
        assert_eq!(counts["f"], 2);

        set_read(&conn, SID, T0, &art("o2", OLD), false, NOW).unwrap();
        let feed = [
            ("n1".into(), NEW.into()),
            ("n2".into(), NEW.into()),
            ("n3".into(), NEW.into()),
        ];
        mark_feed_read(&mut conn, SID, T0, "f", &feed, NOW).unwrap();
        let counts = unread_counts(&conn, SID, T0, &HashMap::from([("f".into(), 3)])).unwrap();
        assert_eq!(counts["f"], 0);

        advance(&mut conn, SID, NOW).unwrap();
        assert!(marks(&conn, SID, &["n2"]).unwrap().is_empty());
        assert_eq!(read_at(NOW, NEW, None).as_deref(), Some(NOW));
    }
}
