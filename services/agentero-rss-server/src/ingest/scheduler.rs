//! Poll due feeds: global semaphore, one request per host, in-process lease,
//! exponential backoff with jitter. State lives in SQLite, so a restart
//! resumes from `next_fetch_at` without an external queue.

use super::fetcher::{fetch, FetchError, FetchOutcome, FetchPolicy};
use super::normalize::normalize;
use crate::storage::{self, to_rfc3339, Archive, DueFeed, UpsertStats};
use agentero_core::features::paper::discovery::feeds::parse::{
    discover_feed_href, looks_like_html, resolve_href,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{watch, Semaphore};

pub const MAX_BACKOFF_SECS: i64 = 6 * 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollResult {
    Archived(UpsertStats),
    NotModified,
    Failed(String),
}

/// 1, 2, 4, 8 … minutes for the Nth consecutive failure, capped at 6 hours,
/// plus up to 10% jitter. `Retry-After` wins when it asks for longer.
pub fn backoff_secs(previous_failures: i64, retry_after: Option<u64>, jitter_seed: u64) -> i64 {
    let exp = previous_failures.clamp(0, 16) as u32;
    let base = (60i64 << exp).min(MAX_BACKOFF_SECS);
    let jitter = (jitter_seed % (base as u64 / 10 + 1)) as i64;
    let delay = (base + jitter).min(MAX_BACKOFF_SECS);
    match retry_after {
        Some(secs) => delay.max((secs as i64).min(MAX_BACKOFF_SECS)),
        None => delay,
    }
}

fn after(secs: i64) -> String {
    to_rfc3339(chrono::Utc::now() + chrono::Duration::seconds(secs))
}

fn jitter_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0)
}

async fn fetch_feed(feed: &DueFeed, policy: &FetchPolicy) -> Result<FetchOutcome, FetchError> {
    let outcome = fetch(
        &feed.url,
        feed.etag.as_deref(),
        feed.last_modified.as_deref(),
        policy,
    )
    .await?;
    let FetchOutcome::Body {
        final_url,
        content_type,
        body,
        ..
    } = &outcome
    else {
        return Ok(outcome);
    };
    let text = String::from_utf8_lossy(body);
    if !looks_like_html(content_type, &text) {
        return Ok(outcome);
    }
    // Subscribed a web page: follow its <link rel=alternate> once, unconditionally.
    let href = discover_feed_href(&text)
        .ok_or_else(|| FetchError::Blocked("page has no feed link".into()))?;
    let target = resolve_href(final_url, &href).map_err(|e| FetchError::Blocked(e.to_string()))?;
    fetch(&target, None, None, policy).await
}

/// Fetch one feed and record the outcome. Never deletes archived articles.
pub async fn poll_feed(
    archive: &Archive,
    policy: &FetchPolicy,
    feed: DueFeed,
) -> storage::StorageResult<PollResult> {
    let id = feed.id.clone();
    let attempt_at = storage::now_rfc3339();
    archive
        .run({
            let id = id.clone();
            move |c| storage::record_attempt(c, &id, &attempt_at)
        })
        .await?;
    let success_next = after(feed.refresh_interval_secs);
    let fail = |error: String, retry_after: Option<u64>| {
        let next = after(backoff_secs(feed.failure_count, retry_after, jitter_seed()));
        let id = id.clone();
        async move {
            archive
                .run({
                    let error = error.clone();
                    move |c| storage::record_failure(c, &id, &error, &next)
                })
                .await?;
            Ok(PollResult::Failed(error))
        }
    };
    match fetch_feed(&feed, policy).await {
        Ok(FetchOutcome::NotModified {
            etag,
            last_modified,
        }) => {
            archive
                .run(move |c| {
                    storage::record_not_modified(
                        c,
                        &id,
                        etag.as_deref(),
                        last_modified.as_deref(),
                        &success_next,
                    )
                })
                .await?;
            Ok(PollResult::NotModified)
        }
        Ok(FetchOutcome::Body {
            etag,
            last_modified,
            body,
            ..
        }) => {
            let parsed = match normalize(&body, "") {
                Ok(p) => p,
                Err(e) => return fail(format!("parse: {e}"), None).await,
            };
            let stats = archive
                .run(move |c| {
                    let title = Some(parsed.title.as_str()).filter(|t| !t.is_empty());
                    storage::record_success(
                        c,
                        &id,
                        title,
                        &parsed.items,
                        etag.as_deref(),
                        last_modified.as_deref(),
                        &success_next,
                    )
                })
                .await?;
            Ok(PollResult::Archived(stats))
        }
        Err(e) => {
            let retry_after = match &e {
                FetchError::Http {
                    status: 429 | 503,
                    retry_after_secs,
                } => *retry_after_secs,
                _ => None,
            };
            fail(e.to_string(), retry_after).await
        }
    }
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default()
}

pub struct Scheduler {
    pub archive: Archive,
    pub policy: Arc<FetchPolicy>,
    pub concurrency: usize,
    pub tick: Duration,
}

impl Scheduler {
    pub async fn run(self, mut shutdown: watch::Receiver<bool>) {
        let permits = Arc::new(Semaphore::new(self.concurrency.max(1)));
        let leases: Arc<Mutex<HashSet<String>>> = Arc::default();
        let hosts: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = Arc::default();
        loop {
            let now = storage::now_rfc3339();
            match self
                .archive
                .run(move |c| storage::due_feeds(c, &now, 256))
                .await
            {
                Ok(due) => {
                    for feed in due {
                        if !leases.lock().unwrap().insert(feed.id.clone()) {
                            continue;
                        }
                        let host_lock = hosts
                            .lock()
                            .unwrap()
                            .entry(host_of(&feed.url))
                            .or_default()
                            .clone();
                        let (archive, policy, permits, leases) = (
                            self.archive.clone(),
                            self.policy.clone(),
                            permits.clone(),
                            leases.clone(),
                        );
                        tokio::spawn(async move {
                            let _host = host_lock.lock().await;
                            let _permit = permits.acquire_owned().await;
                            let id = feed.id.clone();
                            match poll_feed(&archive, &policy, feed).await {
                                Ok(PollResult::Archived(s)) => log::info!(
                                    "feed {id}: +{} new, {} updated, {} unchanged",
                                    s.inserted,
                                    s.updated,
                                    s.unchanged
                                ),
                                Ok(PollResult::NotModified) => {
                                    log::debug!("feed {id}: not modified")
                                }
                                Ok(PollResult::Failed(e)) => log::warn!("feed {id}: {e}"),
                                Err(e) => log::error!("feed {id}: storage: {e}"),
                            }
                            leases.lock().unwrap().remove(&id);
                        });
                    }
                }
                Err(e) => log::error!("scheduler: {e}"),
            }
            tokio::select! {
                _ = tokio::time::sleep(self.tick) => {}
                _ = shutdown.changed() => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_caps() {
        assert_eq!(backoff_secs(0, None, 0), 60);
        assert_eq!(backoff_secs(1, None, 0), 120);
        assert_eq!(backoff_secs(4, None, 0), 960);
        assert_eq!(backoff_secs(30, None, 0), MAX_BACKOFF_SECS);
        assert!(backoff_secs(0, None, 7) <= 66);
        assert_eq!(backoff_secs(0, Some(900), 0), 900);
        assert_eq!(backoff_secs(0, Some(999_999), 0), MAX_BACKOFF_SECS);
    }
}
