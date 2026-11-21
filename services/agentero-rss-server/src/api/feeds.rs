use super::{ApiError, ApiResult, AppState};
use crate::storage::{self, FeedRow};
use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize)]
pub struct FeedList {
    items: Vec<FeedRow>,
}

pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<FeedList> {
    let enabled_only = match q.get("enabled").map(String::as_str) {
        None | Some("false") => false,
        Some("true") => true,
        Some(_) => return Err(ApiError::bad_request("enabled must be true or false")),
    };
    let items = state
        .archive
        .run(move |c| storage::list_feeds(c, enabled_only))
        .await?;
    Ok(Json(FeedList { items }))
}

pub async fn get_one(
    State(state): State<AppState>,
    Path(feed_id): Path<String>,
) -> ApiResult<FeedRow> {
    state
        .archive
        .run(move |c| storage::get_feed(c, &feed_id))
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("feed not found"))
}
