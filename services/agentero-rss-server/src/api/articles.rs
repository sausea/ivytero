use super::{ApiError, ApiResult, AppState, DEFAULT_LIMIT, MAX_LIMIT};
use crate::storage::{self, ArticleRow};
use axum::extract::{Path, Query, State};
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Serialize;
use std::collections::HashMap;

const MAX_CURSOR_CHARS: usize = 256;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticlePage {
    items: Vec<ArticleRow>,
    next_cursor: Option<String>,
}

/// Opaque keyset cursor over `(sort_at, id)` of the last row returned.
pub fn encode_cursor(sort_at: &str, id: &str) -> String {
    URL_SAFE_NO_PAD.encode(format!("{sort_at}\n{id}"))
}

pub fn decode_cursor(raw: &str) -> Result<(String, String), ApiError> {
    let invalid = || ApiError::bad_request("invalid cursor");
    if raw.is_empty() || raw.len() > MAX_CURSOR_CHARS {
        return Err(invalid());
    }
    let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| invalid())?;
    let text = String::from_utf8(bytes).map_err(|_| invalid())?;
    let (sort_at, id) = text.split_once('\n').ok_or_else(invalid)?;
    if storage::canonical_time(Some(sort_at)).as_deref() != Some(sort_at)
        || id.is_empty()
        || id.len() > 64
    {
        return Err(invalid());
    }
    Ok((sort_at.to_string(), id.to_string()))
}

pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<ArticlePage> {
    let limit = match q.get("limit").filter(|s| !s.is_empty()) {
        None => DEFAULT_LIMIT,
        Some(raw) => match raw.parse::<usize>() {
            Ok(n) if (1..=MAX_LIMIT).contains(&n) => n,
            _ => {
                return Err(ApiError::bad_request(format!(
                    "limit must be 1..={MAX_LIMIT}"
                )))
            }
        },
    };
    let cursor = q
        .get("cursor")
        .filter(|s| !s.is_empty())
        .map(|s| decode_cursor(s))
        .transpose()?;
    let feed_id = q.get("feedId").filter(|s| !s.is_empty()).cloned();
    let since = super::new_since(&q)?;
    let (items, has_more) = state
        .archive
        .run(move |c| {
            let after = cursor.as_ref().map(|(s, i)| (s.as_str(), i.as_str()));
            storage::list_articles(c, feed_id.as_deref(), after, since.as_deref(), limit, false)
        })
        .await?;
    let next_cursor = if has_more {
        items
            .last()
            .map(|last| encode_cursor(&last.sort_at, &last.id))
    } else {
        None
    };
    Ok(Json(ArticlePage { items, next_cursor }))
}

pub async fn get_one(
    State(state): State<AppState>,
    Path(article_id): Path<String>,
) -> ApiResult<ArticleRow> {
    state
        .archive
        .run(move |c| storage::get_article(c, &article_id))
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("article not found"))
}
