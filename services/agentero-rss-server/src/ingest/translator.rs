//! Background zh-CN translation of non-Chinese article titles with the free
//! web MT engines in agentero-core. The result is article data, served as
//! `titleTranslated`; nothing about readers is involved.

use crate::storage::{self, Archive};
use std::future::Future;
use std::time::Duration;
use tokio::sync::watch;

const BATCH: usize = 20;
const PAUSE: Duration = Duration::from_millis(300);
const IDLE: Duration = Duration::from_secs(30);
const BACKOFF: Duration = Duration::from_secs(300);

/// Kana or Hangul always translate; otherwise translate when letters clearly
/// outweigh Han characters, so Chinese titles with a few acronyms stay as is.
pub fn needs_translation(title: &str) -> bool {
    let (mut han, mut kana_hangul, mut letters) = (0usize, 0usize, 0usize);
    for c in title.chars() {
        match c {
            '\u{4e00}'..='\u{9fff}' | '\u{3400}'..='\u{4dbf}' => han += 1,
            '\u{3040}'..='\u{30ff}' | '\u{ac00}'..='\u{d7af}' => kana_hangul += 1,
            c if c.is_alphabetic() => letters += 1,
            _ => {}
        }
    }
    kana_hangul > 0 || letters > han * 2
}

/// One batch, newest first. Returns `(stored, failed)`.
pub async fn translate_batch<F, Fut>(
    archive: &Archive,
    translate: F,
) -> storage::StorageResult<(usize, usize)>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Option<String>>,
{
    let pending = archive
        .run(|c| storage::pending_title_translations(c, BATCH))
        .await?;
    let (mut stored, mut failed) = (0, 0);
    for (id, title) in pending {
        let translated = if needs_translation(&title) {
            let Some(out) = translate(title.clone()).await else {
                failed += 1;
                continue;
            };
            tokio::time::sleep(PAUSE).await;
            let out = out.trim().to_string();
            (!out.is_empty() && out != title.trim()).then_some(out)
        } else {
            None
        };
        archive
            .run(move |c| storage::set_title_translation(c, &id, &title, translated.as_deref()))
            .await?;
        stored += 1;
    }
    Ok((stored, failed))
}

pub struct TitleTranslator {
    pub archive: Archive,
}

impl TitleTranslator {
    pub async fn run(self, mut shutdown: watch::Receiver<bool>) {
        loop {
            let wait = match translate_batch(&self.archive, |title| async move {
                agentero_core::features::translate::free_mt_to_zh(&title).await
            })
            .await
            {
                Ok((stored, _)) if stored > 0 => Duration::ZERO,
                Ok((_, 0)) => IDLE,
                Ok((_, failed)) => {
                    log::warn!("title translation: {failed} failed, retrying later");
                    BACKOFF
                }
                Err(e) => {
                    log::error!("title translation: {e}");
                    IDLE
                }
            };
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = shutdown.changed() => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_only_chinese_titles() {
        for t in [
            "Fed holds rates",
            "日銀、利上げを決定",
            "삼성 실적 발표",
            "OpenAI 发布 GPT-6 model update",
        ] {
            assert!(needs_translation(t), "{t}");
        }
        for t in ["美联储维持利率不变", "中国 GDP 增长 5%", "2026", ""] {
            assert!(!needs_translation(t), "{t}");
        }
    }
}
