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
/// `also_oldest` widens it for articles unread elsewhere (the run snapshot).
pub fn unread_scan_since(
    conn: &Connection,
    server_id: &str,
    since: &str,
    feed_id: Option<&str>,
    also_oldest: Option<&str>,
) -> Result<String, AppError> {
    let oldest: Option<String> = conn.query_row(
        "SELECT MIN(first_seen_at) FROM cloud_read_marks
         WHERE server_id = ?1 AND read = 0 AND (?2 IS NULL OR feed_id = ?2)",
        params![server_id, feed_id],
        |r| r.get(0),
    )?;
    let oldest = match (oldest, also_oldest) {
        (Some(a), Some(b)) => Some(a.min(b.to_string())),
        (a, b) => a.or_else(|| b.map(str::to_string)),
    };
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

/// This device's read state for one server as this client run first saw it.
/// "Hide read" keeps listing what was unread then, even after it is read,
/// so items read during a run disappear only at the next launch.
#[derive(Debug, Default)]
pub struct Snapshot {
    since: String,
    /// article id -> (read, feed id, first seen at)
    marks: HashMap<String, (bool, String, String)>,
}

impl Snapshot {
    pub fn load(conn: &Connection, server_id: &str, since: &str) -> Result<Self, AppError> {
        let mut stmt = conn.prepare(
            "SELECT article_id, read, feed_id, first_seen_at FROM cloud_read_marks
             WHERE server_id = ?1",
        )?;
        let marks = stmt
            .query_map([server_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    (r.get::<_, i64>(1)? != 0, r.get(2)?, r.get(3)?),
                ))
            })?
            .collect::<Result<_, _>>()?;
        Ok(Self {
            since: since.to_string(),
            marks,
        })
    }

    pub fn since(&self) -> &str {
        &self.since
    }

    fn unread(&self, id: &str, first_seen_at: &str) -> bool {
        match self.marks.get(id) {
            Some((read, ..)) => !read,
            None => first_seen_at > self.since.as_str(),
        }
    }

    /// Oldest article explicitly unread at launch, optionally in one feed.
    pub fn oldest_unread(&self, feed_id: Option<&str>) -> Option<&str> {
        self.marks
            .values()
            .filter(|(read, feed, _)| !read && feed_id.is_none_or(|f| f == feed))
            .map(|(.., first)| first.as_str())
            .min()
    }

    /// Whether "hide read" lists an article: unread now, or unread when this
    /// run started.
    pub fn listed(
        &self,
        since: &str,
        id: &str,
        first_seen_at: &str,
        mark: Option<&(bool, String)>,
    ) -> bool {
        self.unread(id, first_seen_at) || read_at(since, first_seen_at, mark).is_none()
    }
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
        assert_eq!(unread_scan_since(&conn, SID, T0, None, None).unwrap(), T0);
        assert_eq!(
            set_read(&conn, SID, T0, &art("o1", OLD), false, NOW).unwrap(),
            None
        );
        assert_eq!(
            unread_scan_since(&conn, SID, T0, Some("f"), None).unwrap(),
            "2025-12-31T23:59:59.999Z"
        );
        assert_eq!(
            unread_scan_since(&conn, SID, T0, Some("g"), None).unwrap(),
            T0
        );
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

    #[test]
    fn snapshot_lists_what_was_unread_at_launch() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open_at(&dir.path().join("r.sqlite")).unwrap();
        advance(&mut conn, SID, T0).unwrap();
        set_read(&conn, SID, T0, &art("read_new", NEW), true, T0).unwrap();
        set_read(&conn, SID, T0, &art("unread_old", OLD), false, T0).unwrap();
        let snap = Snapshot::load(&conn, SID, T0).unwrap();
        assert_eq!(snap.oldest_unread(Some("f")), Some(OLD));
        assert_eq!(snap.oldest_unread(Some("g")), None);

        // This run reads everything, per feed and then globally.
        set_read(&conn, SID, T0, &art("unread_old", OLD), true, NOW).unwrap();
        let feed = [
            ("read_new".into(), NEW.into()),
            ("fresh".into(), NEW.into()),
        ];
        mark_feed_read(&mut conn, SID, T0, "f", &feed, NOW).unwrap();
        advance(&mut conn, SID, NOW).unwrap();
        assert!(
            unread_scan_since(&conn, SID, T0, Some("f"), snap.oldest_unread(Some("f")))
                .unwrap()
                .as_str()
                < OLD
        );

        let listed = |id: &str, first: &str| {
            let m = marks(&conn, SID, &[id]).unwrap();
            snap.listed(NOW, id, first, m.get(id))
        };
        assert!(listed("fresh", NEW));
        assert!(listed("unread_old", OLD));
        assert!(!listed("read_new", NEW));
        assert!(!listed("other_old", OLD));

        // Next launch: everything read is hidden.
        let next = Snapshot::load(&conn, SID, NOW).unwrap();
        assert!(!next.listed(NOW, "fresh", NEW, None));
        assert!(!next.listed(NOW, "unread_old", OLD, None));
    }
}
