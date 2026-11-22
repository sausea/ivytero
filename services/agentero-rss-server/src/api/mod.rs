//! Read-only REST API (`/api/v1`). No endpoint mutates reader state.

mod admin;
mod articles;
mod auth;
mod feeds;

use crate::storage::{Archive, StorageError};
use axum::extract::{MatchedPath, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;

pub const API_VERSION: u32 = 1;
pub const DEFAULT_LIMIT: usize = 50;
pub const MAX_LIMIT: usize = 200;

#[derive(Clone)]
pub struct AppState {
    pub archive: Archive,
    pub read_token: Arc<String>,
    /// `None` keeps the admin UI and API switched off (404).
    pub admin_token: Option<Arc<String>>,
    /// Poll interval for feeds added from the admin UI.
    pub default_refresh_secs: i64,
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }
}

impl From<StorageError> for ApiError {
    fn from(e: StorageError) -> Self {
        log::error!("storage: {e}");
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "storage temporarily unavailable",
        )
    }
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: ErrorDetail<'a>,
}

#[derive(Serialize)]
struct ErrorDetail<'a> {
    code: &'a str,
    message: &'a str,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            error: ErrorDetail {
                code: self.code,
                message: &self.message,
            },
        };
        (self.status, Json(body)).into_response()
    }
}

pub type ApiResult<T> = Result<Json<T>, ApiError>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Capabilities {
    history_pagination: bool,
    full_web_page: bool,
    user_state: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Info {
    api_version: u32,
    server_id: String,
    /// Archive clock, comparable with `firstSeenAt` and `newSince`.
    server_time: String,
    capabilities: Capabilities,
}

async fn info(State(state): State<AppState>) -> ApiResult<Info> {
    let server_id = state.archive.run(|c| crate::storage::server_id(c)).await?;
    Ok(Json(Info {
        api_version: API_VERSION,
        server_id,
        server_time: crate::storage::now_rfc3339(),
        capabilities: Capabilities {
            history_pagination: true,
            full_web_page: false,
            user_state: false,
        },
    }))
}

/// Optional `newSince` (RFC 3339): a pure read filter, nothing is recorded.
fn new_since(q: &std::collections::HashMap<String, String>) -> Result<Option<String>, ApiError> {
    match q.get("newSince").filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(raw) => crate::storage::canonical_time(Some(raw))
            .map(Some)
            .ok_or_else(|| ApiError::bad_request("newSince must be an RFC 3339 time")),
    }
}

async fn healthz() -> &'static str {
    "ok"
}

async fn not_found() -> ApiError {
    ApiError::not_found("no such endpoint")
}

/// Route template, status and latency only: never tokens, queries or bodies.
async fn access_log(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "<unmatched>".into());
    let start = Instant::now();
    let res = next.run(req).await;
    log::info!(
        "{method} {route} {} {}ms",
        res.status().as_u16(),
        start.elapsed().as_millis()
    );
    res
}

pub fn router(state: AppState) -> Router {
    let v1 = Router::new()
        .route("/info", get(info))
        .route("/feeds", get(feeds::list))
        .route("/feeds/{feed_id}", get(feeds::get_one))
        .route("/articles", get(articles::list))
        .route("/articles/{article_id}", get(articles::get_one))
        .fallback(not_found)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_read_token,
        ));
    let mut root = Router::new()
        .route("/healthz", get(healthz))
        .nest("/api/v1", v1);
    if state.admin_token.is_some() {
        root = root.merge(admin::routes(state.clone()));
    }
    root.fallback(not_found)
        .layer(middleware::from_fn(access_log))
        .with_state(state)
}
