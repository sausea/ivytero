//! Local daily RSS bookmarks (design P1). The vault Markdown file is the
//! source of truth; the SQLite index under the app data dir is rebuildable
//! from `RSS Key` lines and is never sent to the cloud archive.

pub mod daily_markdown;

use crate::error::AppError;
use crate::features::paper::discovery::feeds::cloud::{self, FeedSource};
use crate::paths;
use chrono::{NaiveDate, NaiveDateTime};
use daily_markdown as md;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

const MAX_TAGS: usize = 20;
const MAX_TAG_CHARS: usize = 40;
const LOCAL_SERVER_ID: &str = "local";

#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RssBookmarkInput {
    pub source: FeedSource,
    pub article_id: String,
    pub title: String,
    pub feed_title: String,
    pub url: Option<String>,
    pub published_at: Option<String>,
    pub summary_text: String,
    pub body_markdown: Option<String>,
    pub tags: Vec<String>,
    /// Client-local day the user confirmed, `YYYY-MM-DD`.
    pub archive_date: String,
    /// Client-local time, `YYYY-MM-DD HH:MM`.
    pub bookmarked_at: String,
}

#[derive(Debug, Clone, Serialize, specta::Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RssBookmark {
    pub server_id: String,
    pub article_id: String,
    pub note_path: String,
    pub archive_date: String,
    pub bookmarked_at: String,
    pub tags: Vec<String>,
    pub title: String,
    pub feed_title: String,
    pub url: Option<String>,
    pub content_status: String,
    pub ai_status: String,
}

fn bad(code: &str) -> AppError {
    AppError::message(format!("feeds.{code}"))
}

static FILE_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> =
    LazyLock::new(Default::default);
static RECONCILED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

fn file_lock(path: &Path) -> Arc<Mutex<()>> {
    let mut map = FILE_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    map.entry(path.to_path_buf()).or_default().clone()
}

fn vault_key(vault: &Path) -> String {
    let id = crate::fs::canonicalize_best_effort(vault)
        .to_string_lossy()
        .replace('\\', "/");
    hex::encode(Sha256::digest(id.as_bytes()))
}

fn resolve_vault(vault_path: &str) -> Result<PathBuf, AppError> {
    if vault_path.trim().is_empty() {
        return Err(bad("bookmark_no_vault"));
    }
    crate::fs::resolve_vault(vault_path).map_err(|_| bad("bookmark_no_vault"))
}

pub async fn server_id_for(source: FeedSource) -> Result<String, AppError> {
    match source {
        FeedSource::Local => Ok(LOCAL_SERVER_ID.to_string()),
        FeedSource::Cloud => cloud::server_id().await,
    }
}

pub fn normalize_tags(raw: &[String]) -> Result<Vec<String>, AppError> {
    let mut out: Vec<String> = Vec::new();
    for tag in raw {
        let tag = tag
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace('、', " ");
        let tag = tag.trim().to_string();
        if tag.is_empty() || out.contains(&tag) {
            continue;
        }
        if tag.chars().count() > MAX_TAG_CHARS {
            return Err(bad("bookmark_tags"));
        }
        out.push(tag);
    }
    if out.is_empty() || out.len() > MAX_TAGS {
        return Err(bad("bookmark_tags"));
    }
    Ok(out)
}

// ---- index ---------------------------------------------------------------

fn index_path() -> PathBuf {
    paths::agentero_data_dir().join("rss-bookmarks.sqlite")
}

fn open_index(path: &Path) -> Result<Connection, AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS rss_bookmarks (
             vault_key TEXT NOT NULL,
             server_id TEXT NOT NULL,
             article_id TEXT NOT NULL,
             note_path TEXT NOT NULL,
             article_key TEXT NOT NULL,
             archive_date TEXT NOT NULL,
             bookmarked_at TEXT NOT NULL,
             tags_json TEXT NOT NULL DEFAULT '[]',
             title TEXT NOT NULL DEFAULT '',
             feed_title TEXT NOT NULL DEFAULT '',
             url TEXT,
             content_status TEXT NOT NULL DEFAULT 'pending',
             ai_status TEXT NOT NULL DEFAULT 'pending',
             last_error TEXT,
             PRIMARY KEY (vault_key, server_id, article_id)
         );
         CREATE INDEX IF NOT EXISTS idx_rss_bookmarks_by_path
             ON rss_bookmarks(vault_key, note_path);",
    )?;
    Ok(conn)
}

fn row_to_bookmark(row: &rusqlite::Row<'_>) -> rusqlite::Result<RssBookmark> {
    let tags: String = row.get(5)?;
    Ok(RssBookmark {
        server_id: row.get(0)?,
        article_id: row.get(1)?,
        note_path: row.get(2)?,
        archive_date: row.get(3)?,
        bookmarked_at: row.get(4)?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        title: row.get(6)?,
        feed_title: row.get(7)?,
        url: row.get(8)?,
        content_status: row.get(9)?,
        ai_status: row.get(10)?,
    })
}

const SELECT: &str = "SELECT server_id, article_id, note_path, archive_date, bookmarked_at,
    tags_json, title, feed_title, url, content_status, ai_status FROM rss_bookmarks";

fn upsert(conn: &Connection, vault_key: &str, b: &RssBookmark) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO rss_bookmarks (vault_key, server_id, article_id, note_path, article_key,
             archive_date, bookmarked_at, tags_json, title, feed_title, url, content_status, ai_status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT (vault_key, server_id, article_id) DO UPDATE SET
             note_path = excluded.note_path, archive_date = excluded.archive_date,
             bookmarked_at = excluded.bookmarked_at, tags_json = excluded.tags_json,
             title = excluded.title, feed_title = excluded.feed_title, url = excluded.url",
        params![
            vault_key,
            b.server_id,
            b.article_id,
            b.note_path,
            format!("{}/{}", b.server_id, b.article_id),
            b.archive_date,
            b.bookmarked_at,
            serde_json::to_string(&b.tags)?,
            b.title,
            b.feed_title,
            b.url,
            b.content_status,
            b.ai_status,
        ],
    )?;
    Ok(())
}

fn lookup(
    conn: &Connection,
    vault_key: &str,
    server_id: &str,
    article_id: &str,
) -> Result<Option<RssBookmark>, AppError> {
    Ok(conn
        .query_row(
            &format!("{SELECT} WHERE vault_key = ?1 AND server_id = ?2 AND article_id = ?3"),
            params![vault_key, server_id, article_id],
            row_to_bookmark,
        )
        .optional()?)
}

/// Make the index match the daily files: add entries found on disk, drop rows
/// whose `RSS Key` is gone.
fn reconcile(conn: &Connection, vault: &Path, vault_key: &str) -> Result<(), AppError> {
    let dir = vault.join(md::RSS_DIR);
    let mut on_disk: HashSet<(String, String)> = HashSet::new();
    if let Ok(read) = std::fs::read_dir(&dir) {
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(date) = name.strip_suffix(".md") else {
                continue;
            };
            if NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
                continue;
            }
            let Ok(doc) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            for found in md::find_entries(&doc) {
                let Some((server_id, article_id)) = found.key.split_once('/') else {
                    continue;
                };
                on_disk.insert((server_id.to_string(), article_id.to_string()));
                let existing = lookup(conn, vault_key, server_id, article_id)?;
                let note_path = md::note_rel(date);
                if existing.as_ref().is_some_and(|b| b.note_path == note_path) {
                    continue;
                }
                upsert(
                    conn,
                    vault_key,
                    &RssBookmark {
                        server_id: server_id.to_string(),
                        article_id: article_id.to_string(),
                        note_path,
                        archive_date: date.to_string(),
                        bookmarked_at: found.bookmarked_at,
                        tags: found.tags,
                        title: found.title,
                        feed_title: found.feed_title,
                        url: found.url,
                        content_status: "partial".into(),
                        ai_status: "not_started".into(),
                    },
                )?;
            }
        }
    }
    let mut stmt =
        conn.prepare("SELECT server_id, article_id FROM rss_bookmarks WHERE vault_key = ?1")?;
    let rows = stmt
        .query_map([vault_key], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (server_id, article_id) in rows {
        if !on_disk.contains(&(server_id.clone(), article_id.clone())) {
            conn.execute(
                "DELETE FROM rss_bookmarks WHERE vault_key = ?1 AND server_id = ?2 AND article_id = ?3",
                params![vault_key, server_id, article_id],
            )?;
        }
    }
    Ok(())
}

fn index_for(vault: &Path, index: &Path) -> Result<(Connection, String), AppError> {
    let key = vault_key(vault);
    let conn = open_index(index)?;
    let once = format!("{}|{key}", index.display());
    let first = RECONCILED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(once);
    if first {
        reconcile(&conn, vault, &key)?;
    }
    Ok((conn, key))
}

// ---- operations ------------------------------------------------------------

fn digest(bytes: Option<&[u8]>) -> Option<Vec<u8>> {
    bytes.map(|b| Sha256::digest(b).to_vec())
}

fn read_opt(path: &Path) -> Result<Option<Vec<u8>>, AppError> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Edit `rel` under its file lock; `edit` returns `None` to leave the file as is.
/// The write is skipped and retried when the file changed under us.
fn edit_note<T>(
    vault: &Path,
    rel: &str,
    mut edit: impl FnMut(Option<&str>) -> Result<(Option<String>, T), AppError>,
) -> Result<T, AppError> {
    let path = vault.join(rel);
    let lock = file_lock(&path);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    for _ in 0..3 {
        let before = read_opt(&path)?;
        let text = before
            .as_deref()
            .map(|b| String::from_utf8(b.to_vec()).map_err(|_| bad("bookmark_conflict")))
            .transpose()?;
        let (next, out) = edit(text.as_deref())?;
        let Some(next) = next else { return Ok(out) };
        if digest(read_opt(&path)?.as_deref()) != digest(before.as_deref()) {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::fs::atomic_write(&path, next.as_bytes())?;
        return Ok(out);
    }
    Err(bad("bookmark_conflict"))
}

fn validate_input(input: &RssBookmarkInput) -> Result<(), AppError> {
    let ok_date = NaiveDate::parse_from_str(&input.archive_date, "%Y-%m-%d").is_ok();
    let ok_time = NaiveDateTime::parse_from_str(&input.bookmarked_at, "%Y-%m-%d %H:%M").is_ok();
    if !ok_date || !ok_time || input.article_id.trim().is_empty() || input.article_id.contains('/')
    {
        return Err(bad("bookmark_invalid"));
    }
    Ok(())
}

/// Archive an article into today's note. Returns the bookmark and whether a
/// new entry was written (`false` when it already existed in this vault).
pub async fn bookmark(
    vault_path: &str,
    input: RssBookmarkInput,
) -> Result<(RssBookmark, bool), AppError> {
    let vault = resolve_vault(vault_path)?;
    validate_input(&input)?;
    let tags = normalize_tags(&input.tags)?;
    let server_id = server_id_for(input.source).await?;
    bookmark_at(&vault, &index_path(), &server_id, &input, tags)
}

fn bookmark_at(
    vault: &Path,
    index: &Path,
    server_id: &str,
    input: &RssBookmarkInput,
    tags: Vec<String>,
) -> Result<(RssBookmark, bool), AppError> {
    let (conn, vkey) = index_for(vault, index)?;
    if let Some(existing) = lookup(&conn, &vkey, server_id, &input.article_id)? {
        let doc = std::fs::read_to_string(vault.join(&existing.note_path)).unwrap_or_default();
        if md::contains_key(&doc, &format!("{server_id}/{}", input.article_id)) {
            return Ok((existing, false));
        }
    }
    let key = format!("{server_id}/{}", input.article_id);
    let rel = md::note_rel(&input.archive_date);
    let body = input
        .body_markdown
        .as_deref()
        .filter(|b| !b.trim().is_empty())
        .unwrap_or(&input.summary_text);
    let content_status = if body.trim().is_empty() {
        "failed"
    } else {
        "partial"
    };
    let created = edit_note(vault, &rel, |doc| {
        let doc = doc
            .map(str::to_string)
            .unwrap_or_else(|| md::new_document(&input.archive_date));
        if md::contains_key(&doc, &key) {
            return Ok((None, false));
        }
        let entry = md::render_entry(&md::EntryFields {
            key: &key,
            title: &input.title,
            feed_title: &input.feed_title,
            url: input.url.as_deref(),
            published_at: input.published_at.as_deref(),
            bookmarked_at: &input.bookmarked_at,
            tags: &tags,
            content_status,
            body,
        });
        Ok((Some(md::append_entry(&doc, &entry)), true))
    })?;
    let bookmark = RssBookmark {
        server_id: server_id.to_string(),
        article_id: input.article_id.clone(),
        note_path: rel,
        archive_date: input.archive_date.clone(),
        bookmarked_at: input.bookmarked_at.clone(),
        tags,
        title: input.title.split_whitespace().collect::<Vec<_>>().join(" "),
        feed_title: input.feed_title.clone(),
        url: input.url.clone(),
        content_status: content_status.into(),
        ai_status: "not_started".into(),
    };
    upsert(&conn, &vkey, &bookmark)?;
    Ok((bookmark, created))
}

/// Bookmarks among `article_ids` for this source in the vault.
pub async fn statuses(
    vault_path: &str,
    source: FeedSource,
    article_ids: &[String],
) -> Result<Vec<RssBookmark>, AppError> {
    let vault = resolve_vault(vault_path)?;
    let server_id = server_id_for(source).await?;
    let (conn, vkey) = index_for(&vault, &index_path())?;
    let mut out = Vec::new();
    for id in article_ids {
        if let Some(b) = lookup(&conn, &vkey, &server_id, id)? {
            out.push(b);
        }
    }
    Ok(out)
}

/// Every bookmark of this source in the vault, newest first.
pub async fn list(vault_path: &str, source: FeedSource) -> Result<Vec<RssBookmark>, AppError> {
    let vault = resolve_vault(vault_path)?;
    let server_id = server_id_for(source).await?;
    let (conn, vkey) = index_for(&vault, &index_path())?;
    let mut stmt = conn.prepare(&format!(
        "{SELECT} WHERE vault_key = ?1 AND server_id = ?2 ORDER BY bookmarked_at DESC, article_id DESC"
    ))?;
    let rows = stmt
        .query_map(params![vkey, server_id], row_to_bookmark)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// OpenAI-compatible endpoint configured in Settings for RSS analysis.
#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RssAnalysisConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

const ANALYSIS_MAX_CHARS: usize = 12000;

/// Analyze one bookmarked article with the configured OpenAI-compatible
/// endpoint and store the Markdown result in its daily-note entry.
pub async fn analyze_bookmark(
    vault_path: &str,
    source: FeedSource,
    article_id: &str,
    title: &str,
    feed_title: &str,
    text: &str,
    config: &RssAnalysisConfig,
) -> Result<(), AppError> {
    let chat = crate::features::openai_chat::ChatConfig {
        base_url: config.base_url.clone(),
        api_key: config.api_key.clone(),
        model: config.model.clone(),
    };
    if !chat.is_configured() {
        return Err(bad("analysis_unconfigured"));
    }
    let vault = resolve_vault(vault_path)?;
    if article_id.trim().is_empty() {
        return Err(bad("bookmark_invalid"));
    }
    let server_id = server_id_for(source).await?;
    let (conn, vkey) = index_for(&vault, &index_path())?;
    let Some(existing) = lookup(&conn, &vkey, &server_id, article_id)? else {
        return Err(bad("bookmark_missing"));
    };
    let key = format!("{server_id}/{article_id}");
    let text: String = text.chars().take(ANALYSIS_MAX_CHARS).collect();
    let prompt = format!(
        "标题：{title}\n来源：{feed_title}\n\n{text}\n\n请用中文输出 Markdown 分析：\n1. 一段话概括核心内容；\n2. 3-5 条关键要点（列表）；\n3. 3-8 个关键词（用、分隔，放在最后一行）。不要大段复述原文。"
    );
    let analysis = crate::features::openai_chat::chat_completion(
        &chat,
        Some("你是一名阅读助手，帮助用户消化 RSS 文章。只输出 Markdown 分析结果，不要寒暄。"),
        &prompt,
    )
    .await?;
    let rel = existing.note_path.clone();
    edit_note(&vault, &rel, |doc| {
        let doc = doc.ok_or_else(|| bad("bookmark_missing"))?;
        match md::annotate_entry(doc, &key, &analysis) {
            Some(next) => Ok((Some(next), ())),
            None => Err(bad("bookmark_missing")),
        }
    })?;
    conn.execute(
        "UPDATE rss_bookmarks SET ai_status = 'done' WHERE vault_key = ?1 AND server_id = ?2 AND article_id = ?3",
        params![vkey, server_id, article_id],
    )?;
    Ok(())
}

/// Remove one article's entry from its daily note and from the index.
pub async fn remove(
    vault_path: &str,
    source: FeedSource,
    article_id: &str,
) -> Result<(), AppError> {
    let vault = resolve_vault(vault_path)?;
    let server_id = server_id_for(source).await?;
    remove_at(&vault, &index_path(), &server_id, article_id)
}

fn remove_at(
    vault: &Path,
    index: &Path,
    server_id: &str,
    article_id: &str,
) -> Result<(), AppError> {
    let (conn, vkey) = index_for(vault, index)?;
    let Some(existing) = lookup(&conn, &vkey, server_id, article_id)? else {
        return Err(bad("bookmark_missing"));
    };
    let key = format!("{server_id}/{article_id}");
    edit_note(vault, &existing.note_path, |doc| {
        Ok((doc.and_then(|d| md::remove_entry(d, &key)), ()))
    })?;
    conn.execute(
        "DELETE FROM rss_bookmarks WHERE vault_key = ?1 AND server_id = ?2 AND article_id = ?3",
        params![vkey, server_id, article_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(id: &str, date: &str) -> RssBookmarkInput {
        RssBookmarkInput {
            source: FeedSource::Cloud,
            article_id: id.into(),
            title: format!("Title {id}"),
            feed_title: "Feed".into(),
            url: Some("https://example.org".into()),
            published_at: Some("2026-11-15T00:00:00.000Z".into()),
            summary_text: "summary".into(),
            body_markdown: None,
            tags: vec!["GPR".into()],
            archive_date: date.into(),
            bookmarked_at: format!("{date} 09:35"),
        }
    }

    #[test]
    fn one_file_per_day_no_duplicates_and_rebuildable_index() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        std::fs::create_dir_all(&vault).unwrap();
        let index = dir.path().join("index.sqlite");
        let tags = vec!["GPR".to_string()];
        for id in ["a1", "a2", "a3"] {
            let (_, created) = bookmark_at(
                &vault,
                &index,
                "srv",
                &input(id, "2026-11-21"),
                tags.clone(),
            )
            .unwrap();
            assert!(created);
        }
        let files: Vec<_> = std::fs::read_dir(vault.join("notes/rss"))
            .unwrap()
            .collect();
        assert_eq!(files.len(), 1);
        // Next day, same article: no second copy, original date kept.
        let (b, created) = bookmark_at(
            &vault,
            &index,
            "srv",
            &input("a1", "2026-11-22"),
            tags.clone(),
        )
        .unwrap();
        assert!(!created);
        assert_eq!(b.note_path, "notes/rss/2026-11-21.md");
        assert!(!vault.join("notes/rss/2026-11-22.md").exists());

        // Lost index is rebuilt from the Markdown keys.
        let fresh = dir.path().join("fresh.sqlite");
        let (conn, vkey) = index_for(&vault, &fresh).unwrap();
        assert!(lookup(&conn, &vkey, "srv", "a2").unwrap().is_some());

        remove_at(&vault, &index, "srv", "a2").unwrap();
        let doc = std::fs::read_to_string(vault.join("notes/rss/2026-11-21.md")).unwrap();
        assert!(!md::contains_key(&doc, "srv/a2"));
        assert!(md::contains_key(&doc, "srv/a3"));
    }

    #[test]
    fn rejects_bad_tags_and_input() {
        assert!(normalize_tags(&[" ".into()]).is_err());
        assert!(normalize_tags(&["x".repeat(41)]).is_err());
        assert_eq!(
            normalize_tags(&[" a ".into(), "a".into(), "b".into()]).unwrap(),
            ["a", "b"]
        );
        let mut bad_input = input("a/b", "2026-11-21");
        assert!(validate_input(&bad_input).is_err());
        bad_input.article_id = "a".into();
        bad_input.archive_date = "2026-13-01".into();
        assert!(validate_input(&bad_input).is_err());
    }
}
