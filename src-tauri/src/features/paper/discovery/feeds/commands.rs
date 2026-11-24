//! Tauri commands for plaza feed subscriptions.

use super::{
    add_and_fetch, items as list_items, list, mark_imported, refresh, remove, rename, resolve_body,
    set_pinned, FeedItem, FeedItemsPage, FeedList, FeedRefreshResult, FeedSub,
};
use crate::core::error::{map_err, ApiResult};
use serde::{Deserialize, Serialize};

#[tauri::command]
#[specta::specta]
pub async fn feeds_list() -> ApiResult<FeedList> {
    match list() {
        Ok(data) => ApiResult::ok(data),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FeedsAddArgs {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn feeds_add(args: FeedsAddArgs) -> ApiResult<FeedSub> {
    match add_and_fetch(args.url, args.title).await {
        Ok(data) => ApiResult::ok(data),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FeedsIdArgs {
    pub id: String,
}

#[tauri::command]
#[specta::specta]
pub async fn feeds_remove(args: FeedsIdArgs) -> ApiResult<()> {
    match remove(&args.id) {
        Ok(()) => ApiResult::ok(()),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FeedsRenameArgs {
    pub id: String,
    pub title: String,
}

#[tauri::command]
#[specta::specta]
pub async fn feeds_rename(args: FeedsRenameArgs) -> ApiResult<FeedSub> {
    match rename(&args.id, &args.title) {
        Ok(data) => ApiResult::ok(data),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FeedsRefreshArgs {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub stale_only: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn feeds_refresh(args: FeedsRefreshArgs) -> ApiResult<FeedRefreshResult> {
    match refresh(args.id, args.stale_only).await {
        Ok(data) => ApiResult::ok(data),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FeedsItemsArgs {
    #[serde(default)]
    pub subscription_id: Option<String>,
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub before_published_at: Option<String>,
    #[serde(default)]
    pub before_id: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn feeds_items(args: FeedsItemsArgs) -> ApiResult<FeedItemsPage> {
    let filter = args.filter.as_deref().unwrap_or("all");
    let limit = args.limit.unwrap_or(100).min(200);
    match list_items(
        args.subscription_id.as_deref(),
        filter,
        limit,
        args.before_published_at.as_deref(),
        args.before_id.as_deref(),
    ) {
        Ok(data) => ApiResult::ok(data),
        Err(e) => map_err(e),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn feeds_mark_imported(args: FeedsIdArgs) -> ApiResult<FeedItem> {
    match mark_imported(&args.id) {
        Ok(data) => ApiResult::ok(data),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FeedsSetPinnedArgs {
    pub id: String,
    pub pinned: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn feeds_set_pinned(args: FeedsSetPinnedArgs) -> ApiResult<FeedSub> {
    match set_pinned(&args.id, args.pinned) {
        Ok(data) => ApiResult::ok(data),
        Err(e) => map_err(e),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn feeds_resolve_body(args: FeedsIdArgs) -> ApiResult<FeedItem> {
    match resolve_body(&args.id).await {
        Ok(data) => ApiResult::ok(data),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FeedsSetReadArgs {
    pub id: String,
    pub read: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn feeds_set_read(args: FeedsSetReadArgs) -> ApiResult<FeedItem> {
    match super::set_read(&args.id, args.read) {
        Ok(data) => ApiResult::ok(data),
        Err(e) => map_err(e),
    }
}

use super::cloud::{self, CloudFeedInfo, CloudFeedItemsPage, CloudFeedSettings, FeedSource};

/// Cloud RSS source settings. The token is reported as `hasToken` only.
#[tauri::command]
#[specta::specta]
pub async fn feeds_cloud_settings_get() -> ApiResult<CloudFeedSettings> {
    match cloud::settings() {
        Ok(data) => ApiResult::ok(data),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FeedsCloudSettingsSetArgs {
    pub source: FeedSource,
    pub base_url: String,
    /// `null` keeps the saved token; `""` clears it.
    #[serde(default)]
    pub token: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn feeds_cloud_settings_set(
    args: FeedsCloudSettingsSetArgs,
) -> ApiResult<CloudFeedSettings> {
    match cloud::save_settings(args.source, &args.base_url, args.token.as_deref()) {
        Ok(data) => ApiResult::ok(data),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FeedsCloudTestArgs {
    pub base_url: String,
    /// `null` tests with the saved token.
    #[serde(default)]
    pub token: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn feeds_cloud_test(args: FeedsCloudTestArgs) -> ApiResult<CloudFeedInfo> {
    match cloud::test_connection(&args.base_url, args.token.as_deref()).await {
        Ok(data) => ApiResult::ok(data),
        Err(e) => map_err(e),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn feeds_cloud_list() -> ApiResult<FeedList> {
    match cloud::feeds().await {
        Ok(subscriptions) => ApiResult::ok(FeedList { subscriptions }),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FeedsCloudItemsArgs {
    #[serde(default)]
    pub feed_id: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    /// Skip articles this device had read before this client run started.
    #[serde(default)]
    pub unread_only: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn feeds_cloud_items(args: FeedsCloudItemsArgs) -> ApiResult<CloudFeedItemsPage> {
    match cloud::items(
        args.feed_id.as_deref(),
        args.cursor.as_deref(),
        args.limit,
        args.unread_only,
    )
    .await
    {
        Ok(data) => ApiResult::ok(data),
        Err(e) => map_err(e),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn feeds_cloud_item(args: FeedsIdArgs) -> ApiResult<FeedItem> {
    match cloud::item(&args.id).await {
        Ok(data) => ApiResult::ok(data),
        Err(e) => map_err(e),
    }
}

/// Read marks for cloud articles stay on this device; the archive is not written.
#[tauri::command]
#[specta::specta]
pub async fn feeds_cloud_set_read(args: FeedsSetReadArgs) -> ApiResult<FeedItem> {
    match cloud::set_read(&args.id, args.read).await {
        Ok(data) => ApiResult::ok(data),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FeedsMarkAllReadArgs {
    /// One subscription, or every subscription when absent.
    #[serde(default)]
    pub id: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn feeds_cloud_mark_all_read(args: FeedsMarkAllReadArgs) -> ApiResult<FeedList> {
    match cloud::mark_all_read(args.id.as_deref()).await {
        Ok(subscriptions) => ApiResult::ok(FeedList { subscriptions }),
        Err(e) => map_err(e),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn feeds_mark_all_read(args: FeedsMarkAllReadArgs) -> ApiResult<FeedList> {
    match super::mark_all_read(args.id.as_deref()) {
        Ok(subscriptions) => ApiResult::ok(FeedList { subscriptions }),
        Err(e) => map_err(e),
    }
}

use agentero_core::features::rss_archive::{self, RssBookmark, RssBookmarkInput};

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RssBookmarkResult {
    pub bookmark: RssBookmark,
    pub created: bool,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RssDailyBookmarkArgs {
    pub vault_path: String,
    pub input: RssBookmarkInput,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RssDailyStatusesArgs {
    pub vault_path: String,
    pub source: FeedSource,
    pub article_ids: Vec<String>,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RssDailyListArgs {
    pub vault_path: String,
    pub source: FeedSource,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RssDailyRemoveArgs {
    pub vault_path: String,
    pub source: FeedSource,
    pub article_id: String,
}

#[tauri::command]
#[specta::specta]
pub async fn rss_daily_bookmark(args: RssDailyBookmarkArgs) -> ApiResult<RssBookmarkResult> {
    match rss_archive::bookmark(&args.vault_path, args.input).await {
        Ok((bookmark, created)) => ApiResult::ok(RssBookmarkResult { bookmark, created }),
        Err(e) => map_err(e),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn rss_daily_statuses(args: RssDailyStatusesArgs) -> ApiResult<Vec<RssBookmark>> {
    match rss_archive::statuses(&args.vault_path, args.source, &args.article_ids).await {
        Ok(data) => ApiResult::ok(data),
        Err(e) => map_err(e),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn rss_daily_list(args: RssDailyListArgs) -> ApiResult<Vec<RssBookmark>> {
    match rss_archive::list(&args.vault_path, args.source).await {
        Ok(data) => ApiResult::ok(data),
        Err(e) => map_err(e),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn rss_daily_remove(args: RssDailyRemoveArgs) -> ApiResult<()> {
    match rss_archive::remove(&args.vault_path, args.source, &args.article_id).await {
        Ok(()) => ApiResult::ok(()),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RssDailyAnalyzeArgs {
    pub vault_path: String,
    pub source: FeedSource,
    pub article_id: String,
    pub title: String,
    pub feed_title: String,
    pub text: String,
    pub config: agentero_core::features::rss_archive::RssAnalysisConfig,
}

#[tauri::command]
#[specta::specta]
pub async fn rss_daily_analyze(args: RssDailyAnalyzeArgs) -> ApiResult<()> {
    match rss_archive::analyze_bookmark(
        &args.vault_path,
        args.source,
        &args.article_id,
        &args.title,
        &args.feed_title,
        &args.text,
        &args.config,
    )
    .await
    {
        Ok(()) => ApiResult::ok(()),
        Err(e) => map_err(e),
    }
}
