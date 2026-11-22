//! Feed / article queries. Every write that touches articles keeps
//! `id`, `first_seen_at` and `sort_at` stable once minted.

use super::{now_rfc3339, StorageError, StorageResult};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const MIN_REFRESH_SECS: i64 = 120;
pub const MAX_REFRESH_SECS: i64 = 86_400;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FeedRow {
    pub id: String,
    pub url: String,
    pub title: String,
    /// OPML folder path, segments joined by `/`; empty = top level.
    pub folder: String,
    pub enabled: bool,
    pub refresh_interval_secs: i64,
    pub added_at: String,
    pub last_attempt_at: Option<String>,
    pub last_success_at: Option<String>,
    pub next_fetch_at: Option<String>,
    pub failure_count: i64,
    pub last_error: Option<String>,
    pub article_count: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArticleRow {
    pub id: String,
    pub feed_id: String,
    pub feed_title: String,
    pub guid: String,
    pub title: String,
    pub url: Option<String>,
    pub published_at: Option<String>,
    pub first_seen_at: String,
    #[serde(skip)]
    pub sort_at: String,
    pub summary_text: String,
    pub content_html: Option<String>,
    pub paper_url: Option<String>,
    pub updated_at: String,
}

/// A feed the scheduler should poll now.
#[derive(Debug, Clone)]
pub struct DueFeed {
    pub id: String,
    pub url: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub failure_count: i64,
    pub refresh_interval_secs: i64,
}

/// One normalized feed entry handed to [`record_success`].
#[derive(Debug, Clone, Default)]
pub struct IncomingItem {
    pub guid: String,
    pub title: String,
    pub url: Option<String>,
    pub published_at: Option<String>,
    pub summary_text: String,
    pub content_html: Option<String>,
    pub paper_url: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UpsertStats {
    pub inserted: usize,
    pub updated: usize,
    pub unchanged: usize,
}

const FEED_COLUMNS: &str =
    "f.id, f.url, f.title, f.folder, f.enabled, f.refresh_interval_secs, f.added_at,
    f.last_attempt_at, f.last_success_at, f.next_fetch_at, f.failure_count, f.last_error,
    (SELECT COUNT(*) FROM articles a WHERE a.feed_id = f.id)";

const ARTICLE_COLUMNS: &str = "a.id, a.feed_id, f.title, a.guid, a.title, a.url, a.published_at,
    a.first_seen_at, a.sort_at, a.summary_text, a.content_html, a.paper_url, a.updated_at";

fn map_feed(row: &Row<'_>) -> rusqlite::Result<FeedRow> {
    Ok(FeedRow {
        id: row.get(0)?,
        url: row.get(1)?,
        title: row.get(2)?,
        folder: row.get(3)?,
        enabled: row.get::<_, i64>(4)? != 0,
        refresh_interval_secs: row.get(5)?,
        added_at: row.get(6)?,
        last_attempt_at: row.get(7)?,
        last_success_at: row.get(8)?,
        next_fetch_at: row.get(9)?,
        failure_count: row.get(10)?,
        last_error: row.get(11)?,
        article_count: row.get(12)?,
    })
}

fn map_article(row: &Row<'_>) -> rusqlite::Result<ArticleRow> {
    Ok(ArticleRow {
        id: row.get(0)?,
        feed_id: row.get(1)?,
        feed_title: row.get(2)?,
        guid: row.get(3)?,
        title: row.get(4)?,
        url: row.get(5)?,
        published_at: row.get(6)?,
        first_seen_at: row.get(7)?,
        sort_at: row.get(8)?,
        summary_text: row.get(9)?,
        content_html: row.get(10)?,
        paper_url: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

pub fn server_id(conn: &Connection) -> StorageResult<String> {
    Ok(
        conn.query_row("SELECT value FROM meta WHERE key = 'server_id'", [], |r| {
            r.get(0)
        })?,
    )
}

pub fn check_interval(secs: i64) -> StorageResult<i64> {
    if (MIN_REFRESH_SECS..=MAX_REFRESH_SECS).contains(&secs) {
        Ok(secs)
    } else {
        Err(StorageError(format!(
            "refresh interval must be between {MIN_REFRESH_SECS} and {MAX_REFRESH_SECS} seconds"
        )))
    }
}

pub fn add_feed(
    conn: &Connection,
    url: &str,
    title: &str,
    interval_secs: i64,
) -> StorageResult<FeedRow> {
    let interval = check_interval(interval_secs)?;
    let taken: i64 = conn.query_row("SELECT COUNT(*) FROM feeds WHERE url = ?1", [url], |r| {
        r.get(0)
    })?;
    if taken > 0 {
        return Err(StorageError(format!("feed already exists: {url}")));
    }
    let id = Uuid::new_v4().to_string();
    let now = now_rfc3339();
    conn.execute(
        "INSERT INTO feeds (id, url, title, refresh_interval_secs, added_at, next_fetch_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        params![id, url, title, interval, now],
    )?;
    get_feed(conn, &id)?.ok_or_else(|| StorageError("feed vanished after insert".into()))
}

pub fn list_feeds(conn: &Connection, enabled_only: bool) -> StorageResult<Vec<FeedRow>> {
    let sql = format!(
        "SELECT {FEED_COLUMNS} FROM feeds f {} ORDER BY f.added_at ASC, f.id ASC",
        if enabled_only {
            "WHERE f.enabled = 1"
        } else {
            ""
        }
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([], map_feed)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_feed(conn: &Connection, id: &str) -> StorageResult<Option<FeedRow>> {
    let sql = format!("SELECT {FEED_COLUMNS} FROM feeds f WHERE f.id = ?1");
    Ok(conn.query_row(&sql, [id], map_feed).optional()?)
}

/// Disabling stops polling and keeps history and `feedId`.
pub fn set_enabled(conn: &Connection, id: &str, enabled: bool) -> StorageResult<bool> {
    let changed = conn.execute(
        "UPDATE feeds SET enabled = ?1,
             next_fetch_at = CASE WHEN ?1 = 1 THEN ?2 ELSE next_fetch_at END
         WHERE id = ?3",
        params![i64::from(enabled), now_rfc3339(), id],
    )?;
    Ok(changed > 0)
}

pub fn set_interval(conn: &Connection, id: &str, secs: i64) -> StorageResult<bool> {
    let secs = check_interval(secs)?;
    let changed = conn.execute(
        "UPDATE feeds SET refresh_interval_secs = ?1 WHERE id = ?2",
        params![secs, id],
    )?;
    Ok(changed > 0)
}

pub fn set_title(conn: &Connection, id: &str, title: &str) -> StorageResult<bool> {
    let title = title.trim();
    if title.is_empty() {
        return Err(StorageError("title must not be empty".into()));
    }
    let changed = conn.execute(
        "UPDATE feeds SET title = ?1 WHERE id = ?2",
        params![title, id],
    )?;
    Ok(changed > 0)
}

pub const MAX_FOLDER_CHARS: usize = 200;

/// Trim each `/`-separated segment, collapse whitespace and drop empty ones.
pub fn normalize_folder(raw: &str) -> StorageResult<String> {
    let folder = raw
        .split('/')
        .map(|seg| seg.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|seg| !seg.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    if folder.chars().count() > MAX_FOLDER_CHARS {
        return Err(StorageError(format!(
            "folder must be at most {MAX_FOLDER_CHARS} characters"
        )));
    }
    Ok(folder)
}

pub fn set_folder(conn: &Connection, id: &str, folder: &str) -> StorageResult<bool> {
    let folder = normalize_folder(folder)?;
    let changed = conn.execute(
        "UPDATE feeds SET folder = ?1 WHERE id = ?2",
        params![folder, id],
    )?;
    Ok(changed > 0)
}

/// Mark a feed due so the running scheduler polls it on its next tick.
pub fn schedule_now(conn: &Connection, id: &str) -> StorageResult<bool> {
    let changed = conn.execute(
        "UPDATE feeds SET next_fetch_at = ?1 WHERE id = ?2",
        params![now_rfc3339(), id],
    )?;
    Ok(changed > 0)
}

pub fn due_feeds(conn: &Connection, now: &str, limit: usize) -> StorageResult<Vec<DueFeed>> {
    let mut stmt = conn.prepare(
        "SELECT id, url, etag, last_modified, failure_count, refresh_interval_secs
         FROM feeds
         WHERE enabled = 1 AND (next_fetch_at IS NULL OR next_fetch_at <= ?1)
         ORDER BY next_fetch_at ASC
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![now, limit as i64], |row| {
            Ok(DueFeed {
                id: row.get(0)?,
                url: row.get(1)?,
                etag: row.get(2)?,
                last_modified: row.get(3)?,
                failure_count: row.get(4)?,
                refresh_interval_secs: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn record_attempt(conn: &Connection, id: &str, at: &str) -> StorageResult<()> {
    conn.execute(
        "UPDATE feeds SET last_attempt_at = ?1 WHERE id = ?2",
        params![at, id],
    )?;
    Ok(())
}

/// HTTP 304: no article writes, only bookkeeping.
pub fn record_not_modified(
    conn: &Connection,
    id: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
    next_fetch_at: &str,
) -> StorageResult<()> {
    conn.execute(
        "UPDATE feeds SET last_success_at = ?1, next_fetch_at = ?2, failure_count = 0,
             last_error = NULL, etag = COALESCE(?3, etag),
             last_modified = COALESCE(?4, last_modified)
         WHERE id = ?5",
        params![now_rfc3339(), next_fetch_at, etag, last_modified, id],
    )?;
    Ok(())
}

/// Failure keeps every archived article and the previous success time.
pub fn record_failure(
    conn: &Connection,
    id: &str,
    error: &str,
    next_fetch_at: &str,
) -> StorageResult<()> {
    conn.execute(
        "UPDATE feeds SET failure_count = failure_count + 1, last_error = ?1, next_fetch_at = ?2
         WHERE id = ?3",
        params![error, next_fetch_at, id],
    )?;
    Ok(())
}

/// Canonical UTC millisecond RFC 3339, or `None` for unparseable dates.
pub fn canonical_time(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| super::to_rfc3339(dt.with_timezone(&chrono::Utc)))
}

fn content_hash(item: &IncomingItem, published_at: Option<&str>) -> String {
    let mut h = Sha256::new();
    for part in [
        Some(item.title.as_str()),
        item.url.as_deref(),
        published_at,
        Some(item.summary_text.as_str()),
        item.content_html.as_deref(),
        item.paper_url.as_deref(),
    ] {
        match part {
            Some(s) => {
                h.update([1u8]);
                h.update((s.len() as u64).to_le_bytes());
                h.update(s.as_bytes());
            }
            None => h.update([0u8]),
        }
    }
    hex::encode(h.finalize())
}

/// Idempotent archive write for one successful fetch, in a single transaction.
/// Never prunes: the archive keeps every entry it has seen.
pub fn record_success(
    conn: &mut Connection,
    feed_id: &str,
    feed_title: Option<&str>,
    items: &[IncomingItem],
    etag: Option<&str>,
    last_modified: Option<&str>,
    next_fetch_at: &str,
) -> StorageResult<UpsertStats> {
    let tx = conn.transaction()?;
    let now = now_rfc3339();
    let mut stats = UpsertStats::default();
    {
        let mut find =
            tx.prepare("SELECT id, content_sha256 FROM articles WHERE feed_id = ?1 AND guid = ?2")?;
        let mut insert = tx.prepare(
            "INSERT INTO articles (id, feed_id, guid, title, url, published_at, first_seen_at,
                 sort_at, summary_text, content_html, paper_url, content_sha256, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?7)",
        )?;
        let mut update = tx.prepare(
            "UPDATE articles SET title = ?1, url = ?2, published_at = ?3, summary_text = ?4,
                 content_html = ?5, paper_url = ?6, content_sha256 = ?7, updated_at = ?8
             WHERE id = ?9",
        )?;
        for item in items {
            let guid = item.guid.trim();
            if guid.is_empty() {
                continue;
            }
            let published_at = canonical_time(item.published_at.as_deref());
            let hash = content_hash(item, published_at.as_deref());
            let existing: Option<(String, String)> = find
                .query_row(params![feed_id, guid], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            match existing {
                None => {
                    let sort_at = published_at.clone().unwrap_or_else(|| now.clone());
                    insert.execute(params![
                        Uuid::now_v7().to_string(),
                        feed_id,
                        guid,
                        item.title,
                        item.url,
                        published_at,
                        now,
                        sort_at,
                        item.summary_text,
                        item.content_html,
                        item.paper_url,
                        hash,
                    ])?;
                    stats.inserted += 1;
                }
                Some((_, old_hash)) if old_hash == hash => stats.unchanged += 1,
                Some((id, _)) => {
                    update.execute(params![
                        item.title,
                        item.url,
                        published_at,
                        item.summary_text,
                        item.content_html,
                        item.paper_url,
                        hash,
                        now,
                        id,
                    ])?;
                    stats.updated += 1;
                }
            }
        }
    }
    tx.execute(
        "UPDATE feeds SET last_success_at = ?1, next_fetch_at = ?2, failure_count = 0,
             last_error = NULL, etag = ?3, last_modified = ?4,
             title = CASE WHEN ?5 IS NOT NULL AND ?5 != '' AND (title = '' OR title = url) THEN ?5 ELSE title END
         WHERE id = ?6",
        params![now, next_fetch_at, etag, last_modified, feed_title, feed_id],
    )?;
    tx.commit()?;
    Ok(stats)
}

/// Keyset page ordered by `(sort_at DESC, id DESC)`. Returns `limit` rows and
/// whether more exist. `include_content` drops `content_html` for list views.
pub fn list_articles(
    conn: &Connection,
    feed_id: Option<&str>,
    after: Option<(&str, &str)>,
    limit: usize,
    include_content: bool,
) -> StorageResult<(Vec<ArticleRow>, bool)> {
    let sql = format!(
        "SELECT {ARTICLE_COLUMNS}
         FROM articles a JOIN feeds f ON f.id = a.feed_id
         WHERE (?1 IS NULL OR a.feed_id = ?1)
           AND (?2 IS NULL OR a.sort_at < ?2 OR (a.sort_at = ?2 AND a.id < ?3))
         ORDER BY a.sort_at DESC, a.id DESC
         LIMIT ?4"
    );
    let (sort_at, id) = match after {
        Some((s, i)) => (Some(s), Some(i)),
        None => (None, None),
    };
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt
        .query_map(
            params![feed_id, sort_at, id, (limit + 1) as i64],
            map_article,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    if !include_content {
        for row in &mut rows {
            row.content_html = None;
        }
    }
    Ok((rows, has_more))
}

pub fn get_article(conn: &Connection, id: &str) -> StorageResult<Option<ArticleRow>> {
    let sql = format!(
        "SELECT {ARTICLE_COLUMNS} FROM articles a JOIN feeds f ON f.id = a.feed_id WHERE a.id = ?1"
    );
    Ok(conn.query_row(&sql, [id], map_article).optional()?)
}

pub fn article_count(conn: &Connection) -> StorageResult<i64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM articles", [], |r| r.get(0))?)
}
