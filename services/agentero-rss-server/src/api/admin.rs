//! Collection management for the server operator: `/admin` (static UI) and
//! `/api/admin` (bearer `RSS_ADMIN_TOKEN`). Mounted only when that token is
//! set; it edits feeds, never reader state.

use super::{articles, auth, info, ApiError, ApiResult, AppState};
use crate::opml;
use crate::storage::{self, FeedRow};
use agentero_core::features::paper::discovery::feeds::parse::normalize_feed_url;
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

const INDEX_HTML: &str = include_str!("admin_ui/index.html");
const APP_JS: &str = include_str!("admin_ui/app.js");
const APP_CSS: &str = include_str!("admin_ui/app.css");
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; \
    img-src 'self' https: data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

fn asset(body: &'static str, content_type: &'static str) -> Response {
    let mut res = body.into_response();
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

async fn index() -> Response {
    asset(INDEX_HTML, "text/html; charset=utf-8")
}
async fn app_js() -> Response {
    asset(APP_JS, "text/javascript; charset=utf-8")
}
async fn app_css() -> Response {
    asset(APP_CSS, "text/css; charset=utf-8")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddFeed {
    url: String,
    title: Option<String>,
    folder: Option<String>,
    interval_secs: Option<i64>,
}

enum AddOutcome {
    Added(Box<FeedRow>),
    Duplicate,
}

fn add_one(
    conn: &rusqlite::Connection,
    url: &str,
    title: Option<&str>,
    folder: &str,
    interval: i64,
) -> storage::StorageResult<AddOutcome> {
    let taken: i64 = conn.query_row("SELECT COUNT(*) FROM feeds WHERE url = ?1", [url], |r| {
        r.get(0)
    })?;
    if taken > 0 {
        return Ok(AddOutcome::Duplicate);
    }
    // Untitled feeds keep the url until the first fetch supplies the channel title.
    let title = title
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or(url);
    let feed = storage::add_feed(conn, url, title, interval)?;
    if folder.is_empty() {
        return Ok(AddOutcome::Added(Box::new(feed)));
    }
    storage::set_folder(conn, &feed.id, folder)?;
    let feed = storage::get_feed(conn, &feed.id)?
        .ok_or_else(|| storage::StorageError("feed vanished after insert".into()))?;
    Ok(AddOutcome::Added(Box::new(feed)))
}

fn folder_arg(raw: Option<&str>) -> Result<String, ApiError> {
    storage::normalize_folder(raw.unwrap_or("")).map_err(|e| ApiError::bad_request(e.0))
}

fn interval_or_default(state: &AppState, secs: Option<i64>) -> Result<i64, ApiError> {
    let secs = secs.unwrap_or(state.default_refresh_secs);
    storage::check_interval(secs).map_err(|e| ApiError::bad_request(e.0))
}

async fn add_feed(
    State(state): State<AppState>,
    Json(body): Json<AddFeed>,
) -> Result<(StatusCode, Json<FeedRow>), ApiError> {
    let url = normalize_feed_url(&body.url)
        .map_err(|_| ApiError::bad_request("not an http(s) feed URL"))?;
    let interval = interval_or_default(&state, body.interval_secs)?;
    let folder = folder_arg(body.folder.as_deref())?;
    let outcome = state
        .archive
        .run(move |c| add_one(c, &url, body.title.as_deref(), &folder, interval))
        .await?;
    match outcome {
        AddOutcome::Added(feed) => Ok((StatusCode::CREATED, Json(*feed))),
        AddOutcome::Duplicate => Err(ApiError::new(
            StatusCode::CONFLICT,
            "duplicate",
            "feed already exists",
        )),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PatchFeed {
    enabled: Option<bool>,
    interval_secs: Option<i64>,
    title: Option<String>,
    folder: Option<String>,
}

async fn patch_feed(
    State(state): State<AppState>,
    Path(feed_id): Path<String>,
    Json(body): Json<PatchFeed>,
) -> ApiResult<FeedRow> {
    if let Some(secs) = body.interval_secs {
        storage::check_interval(secs).map_err(|e| ApiError::bad_request(e.0))?;
    }
    if body.title.as_deref().is_some_and(|t| t.trim().is_empty()) {
        return Err(ApiError::bad_request("title must not be empty"));
    }
    let folder = body
        .folder
        .as_deref()
        .map(|f| folder_arg(Some(f)))
        .transpose()?;
    state
        .archive
        .run(move |c| {
            if storage::get_feed(c, &feed_id)?.is_none() {
                return Ok(None);
            }
            if let Some(enabled) = body.enabled {
                storage::set_enabled(c, &feed_id, enabled)?;
            }
            if let Some(secs) = body.interval_secs {
                storage::set_interval(c, &feed_id, secs)?;
            }
            if let Some(title) = &body.title {
                storage::set_title(c, &feed_id, title)?;
            }
            if let Some(folder) = &folder {
                storage::set_folder(c, &feed_id, folder)?;
            }
            storage::get_feed(c, &feed_id)
        })
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("feed not found"))
}

async fn refresh_feed(
    State(state): State<AppState>,
    Path(feed_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if state
        .archive
        .run(move |c| storage::schedule_now(c, &feed_id))
        .await?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("feed not found"))
    }
}

const MAX_BATCH: usize = 10_000;

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum BatchAction {
    Refresh,
    Enable,
    Disable,
    Move,
}

#[derive(Deserialize)]
struct Batch {
    ids: Vec<String>,
    action: BatchAction,
    folder: Option<String>,
}

#[derive(Serialize)]
struct BatchReport {
    updated: usize,
}

async fn batch(State(state): State<AppState>, Json(body): Json<Batch>) -> ApiResult<BatchReport> {
    if body.ids.len() > MAX_BATCH {
        return Err(ApiError::bad_request(format!(
            "at most {MAX_BATCH} feeds per batch"
        )));
    }
    let folder = match body.action {
        BatchAction::Move => Some(folder_arg(body.folder.as_deref())?),
        _ => None,
    };
    let updated = state
        .archive
        .run(move |c| {
            let tx = c.transaction()?;
            let mut updated = 0;
            for id in &body.ids {
                let changed = match body.action {
                    BatchAction::Refresh => storage::schedule_now(&tx, id)?,
                    BatchAction::Enable => storage::set_enabled(&tx, id, true)?,
                    BatchAction::Disable => storage::set_enabled(&tx, id, false)?,
                    BatchAction::Move => {
                        storage::set_folder(&tx, id, folder.as_deref().unwrap_or(""))?
                    }
                };
                updated += usize::from(changed);
            }
            tx.commit()?;
            Ok(updated)
        })
        .await?;
    Ok(Json(BatchReport { updated }))
}

#[derive(Serialize)]
struct Skipped {
    url: String,
    reason: &'static str,
}

#[derive(Serialize)]
struct ImportReport {
    added: Vec<FeedRow>,
    skipped: Vec<Skipped>,
}

async fn import_opml(State(state): State<AppState>, body: String) -> ApiResult<ImportReport> {
    let entries = opml::parse(&body).map_err(ApiError::bad_request)?;
    let interval = state.default_refresh_secs;
    let report = state
        .archive
        .run(move |c| {
            let mut report = ImportReport {
                added: Vec::new(),
                skipped: Vec::new(),
            };
            for entry in entries {
                let Ok(url) = normalize_feed_url(&entry.url) else {
                    report.skipped.push(Skipped {
                        url: entry.url,
                        reason: "invalid_url",
                    });
                    continue;
                };
                let folder = storage::normalize_folder(&entry.folder)?;
                match add_one(c, &url, entry.title.as_deref(), &folder, interval)? {
                    AddOutcome::Added(feed) => report.added.push(*feed),
                    AddOutcome::Duplicate => report.skipped.push(Skipped {
                        url,
                        reason: "duplicate",
                    }),
                }
            }
            Ok(report)
        })
        .await?;
    Ok(Json(report))
}

async fn export_opml(State(state): State<AppState>) -> Result<Response, ApiError> {
    let feeds = state.archive.run(|c| storage::list_feeds(c, false)).await?;
    let body = opml::export(&feeds, &storage::now_rfc3339());
    Ok((
        [
            (header::CONTENT_TYPE, "text/x-opml; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"agentero-rss.opml\"",
            ),
        ],
        body,
    )
        .into_response())
}

pub fn routes(state: AppState) -> Router<AppState> {
    let api = Router::new()
        .route("/info", get(info))
        .route("/feeds", get(super::feeds::list).post(add_feed))
        .route("/feeds/batch", post(batch))
        .route("/feeds/{feed_id}", axum::routing::patch(patch_feed))
        .route("/feeds/{feed_id}/refresh", post(refresh_feed))
        .route("/articles", get(articles::list))
        .route("/articles/{article_id}", get(articles::get_one))
        .route("/opml", get(export_opml).post(import_opml))
        .fallback(super::not_found)
        .layer(middleware::from_fn_with_state(
            state,
            auth::require_admin_token,
        ));
    Router::new()
        .route("/admin", get(index))
        .route("/admin/", get(index))
        .route("/admin/app.js", get(app_js))
        .route("/admin/app.css", get(app_css))
        .nest("/api/admin", api)
}
