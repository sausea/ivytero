//! Read-only client for an Agentero Cloud RSS Server (`/api/v1`).
//!
//! The cloud archive only serves feeds and articles; nothing here writes back
//! to it. The bearer token stays in the Host (`rss-cloud.json`, owner-only)
//! and never crosses to the WebView. Cloud rows are mapped onto the local
//! [`FeedSub`] / [`FeedItem`] shapes without touching `feeds.sqlite`; read
//! marks live on this device in [`cloud_reads`].

use super::body::html_to_markdown;
use super::cloud_reads;
use super::{FeedItem, FeedSub};
use crate::error::AppError;
use crate::{http, paths};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use url::Url;

const TIMEOUT: Duration = Duration::from_secs(20);
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
pub const PAGE_SIZE: u32 = 50;
pub const MAX_PAGE_SIZE: u32 = 200;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum FeedSource {
    #[default]
    Local,
    Cloud,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredConfig {
    #[serde(default)]
    source: FeedSource,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    token: String,
    /// Last `serverId` seen for `base_url`; keys local bookmarks of cloud articles.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    server_id: String,
}

/// What the WebView may see: whether a token exists, never the token.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CloudFeedSettings {
    pub source: FeedSource,
    pub base_url: String,
    pub has_token: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CloudFeedInfo {
    pub api_version: u32,
    pub server_id: String,
    /// Archive clock; seeds the unread watermark so it matches `firstSeenAt`.
    #[serde(default)]
    pub server_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CloudFeedItemsPage {
    pub items: Vec<FeedItem>,
    pub next_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireFeed {
    id: String,
    url: String,
    title: String,
    #[serde(default)]
    folder: String,
    added_at: String,
    last_success_at: Option<String>,
    last_error: Option<String>,
    #[serde(default)]
    article_count: i64,
    /// Present when `newSince` was sent (servers before it ignore the param).
    #[serde(default)]
    new_count: Option<i64>,
}

#[derive(Deserialize)]
struct WireFeedList {
    items: Vec<WireFeed>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireArticle {
    id: String,
    feed_id: String,
    #[serde(default)]
    feed_title: String,
    title: String,
    #[serde(default)]
    title_translated: Option<String>,
    url: Option<String>,
    published_at: Option<String>,
    first_seen_at: String,
    #[serde(default)]
    summary_text: String,
    content_html: Option<String>,
    paper_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireArticlePage {
    items: Vec<WireArticle>,
    next_cursor: Option<String>,
}

fn config_path() -> PathBuf {
    paths::agentero_config_dir().join("rss-cloud.json")
}

fn read_config(path: &Path) -> Result<StoredConfig, AppError> {
    match std::fs::read(path) {
        Ok(raw) => Ok(serde_json::from_slice(&raw)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(StoredConfig::default()),
        Err(e) => Err(e.into()),
    }
}

fn write_config(path: &Path, config: &StoredConfig) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::fs::json_store_with(path, config, &crate::fs::AtomicOpts::OWNER_ONLY)
}

fn view(config: &StoredConfig) -> CloudFeedSettings {
    CloudFeedSettings {
        source: config.source,
        base_url: config.base_url.clone(),
        has_token: !config.token.is_empty(),
    }
}

/// `https://host[:port][/prefix]` without a trailing slash. Plain `http` is
/// only accepted for loopback so a token is never sent in clear text.
pub fn normalize_base_url(raw: &str) -> Result<String, AppError> {
    let invalid = || AppError::message("feeds.cloud_invalid_url");
    let url = Url::parse(raw.trim()).map_err(|_| invalid())?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    let scheme_ok = url.scheme() == "https" || (url.scheme() == "http" && loopback);
    if !scheme_ok
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

pub fn settings() -> Result<CloudFeedSettings, AppError> {
    Ok(view(&read_config(&config_path())?))
}

/// `token: None` keeps the saved token; `Some("")` clears it.
pub fn save_settings(
    source: FeedSource,
    base_url: &str,
    token: Option<&str>,
) -> Result<CloudFeedSettings, AppError> {
    save_settings_at(&config_path(), source, base_url, token)
}

fn save_settings_at(
    path: &Path,
    source: FeedSource,
    base_url: &str,
    token: Option<&str>,
) -> Result<CloudFeedSettings, AppError> {
    let mut config = read_config(path)?;
    config.base_url = if base_url.trim().is_empty() {
        String::new()
    } else {
        normalize_base_url(base_url)?
    };
    if let Some(token) = token {
        config.token = token.trim().to_string();
    }
    if base_url_changed(&config, path)? {
        config.server_id.clear();
    }
    if source == FeedSource::Cloud && (config.base_url.is_empty() || config.token.is_empty()) {
        return Err(AppError::message("feeds.cloud_not_configured"));
    }
    config.source = source;
    write_config(path, &config)?;
    Ok(view(&config))
}

fn base_url_changed(config: &StoredConfig, path: &Path) -> Result<bool, AppError> {
    Ok(read_config(path)?.base_url != config.base_url)
}

/// Stable `serverId` of the configured archive, cached after the first `/info`.
pub async fn server_id() -> Result<String, AppError> {
    let path = config_path();
    let config = read_config(&path)?;
    if !config.server_id.is_empty() {
        return Ok(config.server_id);
    }
    let (base_url, token) = configured()?;
    let info: CloudFeedInfo = get_json(&base_url, &token, "/info", &[]).await?;
    let mut config = read_config(&path)?;
    if config.base_url == base_url {
        config.server_id = info.server_id.clone();
        write_config(&path, &config)?;
    }
    Ok(info.server_id)
}

fn configured() -> Result<(String, String), AppError> {
    let config = read_config(&config_path())?;
    if config.base_url.is_empty() || config.token.is_empty() {
        return Err(AppError::message("feeds.cloud_not_configured"));
    }
    Ok((config.base_url, config.token))
}

async fn get_json<T: for<'de> Deserialize<'de>>(
    base_url: &str,
    token: &str,
    path: &str,
    query: &[(&str, String)],
) -> Result<T, AppError> {
    let client = http::client_builder()
        .timeout(TIMEOUT)
        .connect_timeout(Duration::from_secs(10))
        // Never forward the bearer token to another origin.
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(http::USER_AGENT)
        .build()
        .map_err(|e| AppError::message(format!("feeds.cloud_unreachable:{e}")))?;
    let mut res = client
        .get(format!("{base_url}/api/v1{path}"))
        .bearer_auth(token)
        .query(query)
        .send()
        .await
        .map_err(|e| AppError::message(format!("feeds.cloud_unreachable:{e}")))?;
    let status = res.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(AppError::message("feeds.cloud_unauthorized"));
    }
    if status.as_u16() == 404 {
        return Err(AppError::message("feeds.not_found"));
    }
    if !status.is_success() {
        return Err(AppError::message(format!(
            "feeds.cloud_http:{}",
            status.as_u16()
        )));
    }
    let mut body = Vec::new();
    while let Some(chunk) = res
        .chunk()
        .await
        .map_err(|e| AppError::message(format!("feeds.cloud_unreachable:{e}")))?
    {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(AppError::message("feeds.too_large"));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body)
        .map_err(|e| AppError::message(format!("feeds.cloud_protocol:{e}")))
}

/// Probe `/info` with the given URL and token (or the saved token when `None`).
pub async fn test_connection(
    base_url: &str,
    token: Option<&str>,
) -> Result<CloudFeedInfo, AppError> {
    let base_url = normalize_base_url(base_url)?;
    let token = match token.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => t.to_string(),
        None => read_config(&config_path())?.token,
    };
    if token.is_empty() {
        return Err(AppError::message("feeds.cloud_not_configured"));
    }
    let info: CloudFeedInfo = get_json(&base_url, &token, "/info", &[]).await?;
    if info.api_version != 1 {
        return Err(AppError::message(format!(
            "feeds.cloud_protocol:apiVersion {}",
            info.api_version
        )));
    }
    Ok(info)
}

fn to_sub(feed: WireFeed, unread_count: Option<i64>) -> FeedSub {
    FeedSub {
        id: feed.id,
        url: feed.url,
        title: feed.title,
        folder: feed.folder,
        added_at: feed.added_at,
        last_fetched_at: feed.last_success_at,
        last_error: feed.last_error,
        item_count: feed.article_count,
        unread_count,
        pinned: false,
        pinned_at: None,
    }
}

fn to_item(article: WireArticle, with_body: bool) -> FeedItem {
    let body_markdown = with_body.then(|| {
        let html = article.content_html.as_deref().unwrap_or_default();
        let md = if html.trim().is_empty() {
            article.summary_text.clone()
        } else {
            html_to_markdown(html)
        };
        md.trim().to_string()
    });
    FeedItem {
        id: article.id,
        subscription_id: article.feed_id,
        subscription_title: article.feed_title,
        title: article.title,
        title_translated: article.title_translated,
        url: article.url,
        published_at: article.published_at.or(Some(article.first_seen_at)),
        summary_text: article.summary_text,
        content_html: article.content_html,
        paper_url: article.paper_url,
        imported_at: None,
        read_at: None,
        body_markdown,
    }
}

/// This device's read watermark for the archive, seeded from its clock once.
async fn read_scope(base: &str, token: &str) -> Result<(String, String), AppError> {
    let server_id = server_id().await?;
    if let Some(since) = cloud_reads::unread_since(&cloud_reads::open()?, &server_id)? {
        snapshot(&server_id, &since)?;
        return Ok((server_id, since));
    }
    let now = archive_now(base, token).await?;
    cloud_reads::advance(&mut cloud_reads::open()?, &server_id, &now)?;
    snapshot(&server_id, &now)?;
    Ok((server_id, now))
}

/// The server's read state as first seen in this client run; later marks
/// and "mark all read" change the stored state but not this.
fn snapshot(server_id: &str, since: &str) -> Result<Arc<cloud_reads::Snapshot>, AppError> {
    static SEEN: OnceLock<Mutex<HashMap<String, Arc<cloud_reads::Snapshot>>>> = OnceLock::new();
    let mut seen = SEEN
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(snap) = seen.get(server_id) {
        return Ok(snap.clone());
    }
    let snap = Arc::new(cloud_reads::Snapshot::load(
        &cloud_reads::open()?,
        server_id,
        since,
    )?);
    seen.insert(server_id.to_string(), snap.clone());
    Ok(snap)
}

async fn archive_now(base: &str, token: &str) -> Result<String, AppError> {
    let info: CloudFeedInfo = get_json(base, token, "/info", &[]).await?;
    Ok(info
        .server_time
        .unwrap_or_else(crate::time::now_rfc3339_millis))
}

pub async fn feeds() -> Result<Vec<FeedSub>, AppError> {
    let (base, token) = configured()?;
    let (server_id, since) = read_scope(&base, &token).await?;
    let list: WireFeedList =
        get_json(&base, &token, "/feeds", &[("newSince", since.clone())]).await?;
    let new_counts = list
        .items
        .iter()
        .filter_map(|f| f.new_count.map(|n| (f.id.clone(), n)))
        .collect();
    let unread =
        cloud_reads::unread_counts(&cloud_reads::open()?, &server_id, &since, &new_counts)?;
    Ok(list
        .items
        .into_iter()
        .map(|f| {
            let n = unread.get(&f.id).copied();
            to_sub(f, n)
        })
        .collect())
}

fn with_read_state(
    server_id: &str,
    since: &str,
    articles: Vec<WireArticle>,
    with_body: bool,
) -> Result<Vec<FeedItem>, AppError> {
    let ids: Vec<&str> = articles.iter().map(|a| a.id.as_str()).collect();
    let marks = cloud_reads::marks(&cloud_reads::open()?, server_id, &ids)?;
    Ok(articles
        .into_iter()
        .map(|a| {
            let read_at = cloud_reads::read_at(since, &a.first_seen_at, marks.get(&a.id));
            FeedItem {
                read_at,
                ..to_item(a, with_body)
            }
        })
        .collect())
}

/// Pages fetched at most per call when hiding read items skips whole pages.
const MAX_UNREAD_SCAN_PAGES: usize = 10;

pub async fn items(
    feed_id: Option<&str>,
    cursor: Option<&str>,
    limit: Option<u32>,
    unread_only: bool,
) -> Result<CloudFeedItemsPage, AppError> {
    let (base, token) = configured()?;
    let (server_id, since) = read_scope(&base, &token).await?;
    let mut query = vec![(
        "limit",
        limit
            .unwrap_or(PAGE_SIZE)
            .clamp(1, MAX_PAGE_SIZE)
            .to_string(),
    )];
    if let Some(id) = feed_id.filter(|s| !s.is_empty()) {
        query.push(("feedId", id.to_string()));
    }
    let snap = snapshot(&server_id, &since)?;
    if unread_only {
        let feed = feed_id.filter(|s| !s.is_empty());
        let scan_since = cloud_reads::unread_scan_since(
            &cloud_reads::open()?,
            &server_id,
            snap.since(),
            feed,
            snap.oldest_unread(feed),
        )?;
        query.push(("newSince", scan_since));
    }
    let mut cursor = cursor.filter(|s| !s.is_empty()).map(str::to_string);
    for _ in 0..MAX_UNREAD_SCAN_PAGES {
        let mut q = query.clone();
        if let Some(c) = &cursor {
            q.push(("cursor", c.clone()));
        }
        let page: WireArticlePage = get_json(&base, &token, "/articles", &q).await?;
        let articles = if unread_only {
            let ids: Vec<&str> = page.items.iter().map(|a| a.id.as_str()).collect();
            let marks = cloud_reads::marks(&cloud_reads::open()?, &server_id, &ids)?;
            page.items
                .into_iter()
                .filter(|a| snap.listed(&since, &a.id, &a.first_seen_at, marks.get(&a.id)))
                .collect()
        } else {
            page.items
        };
        let items = with_read_state(&server_id, &since, articles, false)?;
        cursor = page.next_cursor;
        if !items.is_empty() || cursor.is_none() || !unread_only {
            return Ok(CloudFeedItemsPage {
                items,
                next_cursor: cursor,
            });
        }
    }
    Ok(CloudFeedItemsPage {
        items: Vec::new(),
        next_cursor: cursor,
    })
}

/// One article with `bodyMarkdown` filled from the archived HTML / summary.
pub async fn item(id: &str) -> Result<FeedItem, AppError> {
    let (base, token) = configured()?;
    let (server_id, since) = read_scope(&base, &token).await?;
    let article = fetch_article(&base, &token, id).await?;
    with_read_state(&server_id, &since, vec![article], true)?
        .pop()
        .ok_or_else(|| AppError::message("feeds.not_found"))
}

async fn fetch_article(base: &str, token: &str, id: &str) -> Result<WireArticle, AppError> {
    let path = format!("/articles/{}", urlencode_segment(id));
    get_json(base, token, &path, &[]).await
}

/// Record a read mark on this device; the archive only serves the article.
pub async fn set_read(id: &str, read: bool) -> Result<FeedItem, AppError> {
    let (base, token) = configured()?;
    let (server_id, since) = read_scope(&base, &token).await?;
    let article = fetch_article(&base, &token, id).await?;
    let read_at = cloud_reads::set_read(
        &cloud_reads::open()?,
        &server_id,
        &since,
        &cloud_reads::Article {
            id: &article.id,
            feed_id: &article.feed_id,
            first_seen_at: &article.first_seen_at,
        },
        read,
        &crate::time::now_rfc3339_millis(),
    )?;
    Ok(FeedItem {
        read_at,
        ..to_item(article, false)
    })
}

/// Most articles one feed sweep marks read; older unread ones stay unread.
const MAX_MARK_ALL: usize = 20_000;

/// All feeds: move the watermark to the archive clock. One feed: mark each
/// article newer than the watermark read.
pub async fn mark_all_read(feed_id: Option<&str>) -> Result<Vec<FeedSub>, AppError> {
    let (base, token) = configured()?;
    let (server_id, since) = read_scope(&base, &token).await?;
    let now = crate::time::now_rfc3339_millis();
    match feed_id.filter(|s| !s.is_empty()) {
        None => {
            let archive_now = archive_now(&base, &token).await?;
            cloud_reads::advance(&mut cloud_reads::open()?, &server_id, &archive_now)?;
        }
        Some(feed_id) => {
            let mut seen = Vec::new();
            let mut cursor: Option<String> = None;
            loop {
                let mut q = vec![
                    ("limit", MAX_PAGE_SIZE.to_string()),
                    ("feedId", feed_id.to_string()),
                    ("newSince", since.clone()),
                ];
                if let Some(c) = &cursor {
                    q.push(("cursor", c.clone()));
                }
                let page: WireArticlePage = get_json(&base, &token, "/articles", &q).await?;
                seen.extend(
                    page.items
                        .into_iter()
                        .filter(|a| a.first_seen_at > since)
                        .map(|a| (a.id, a.first_seen_at)),
                );
                cursor = page.next_cursor;
                if cursor.is_none() || seen.len() >= MAX_MARK_ALL {
                    break;
                }
            }
            cloud_reads::mark_feed_read(
                &mut cloud_reads::open()?,
                &server_id,
                &since,
                feed_id,
                &seen,
                &now,
            )?;
        }
    }
    feeds().await
}

fn urlencode_segment(raw: &str) -> String {
    raw.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
