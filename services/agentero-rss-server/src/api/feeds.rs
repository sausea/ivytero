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
    let since = super::new_since(&q)?;
    let items = state
        .archive
        .run(move |c| {
            let mut feeds = storage::list_feeds(c, enabled_only)?;
            if let Some(since) = since {
                let counts = storage::new_counts(c, &since)?;
                for f in &mut feeds {
                    f.new_count = Some(counts.get(&f.id).copied().unwrap_or(0));
                }
            }
            Ok(feeds)
        })
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
