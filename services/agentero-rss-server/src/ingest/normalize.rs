//! Feed body → [`IncomingItem`]s via the shared Agentero parser.

use crate::storage::IncomingItem;
use agentero_core::features::paper::discovery::feeds::parse::{parse_feed_bytes, ParsedItem};

pub struct NormalizedFeed {
    pub title: String,
    pub items: Vec<IncomingItem>,
}

impl From<ParsedItem> for IncomingItem {
    fn from(p: ParsedItem) -> Self {
        Self {
            guid: p.guid,
            title: p.title,
            url: p.url,
            published_at: p.published_at,
            summary_text: p.summary_text,
            content_html: p.content_html,
            paper_url: p.paper_url,
        }
    }
}

pub fn normalize(bytes: &[u8], fallback_title: &str) -> Result<NormalizedFeed, String> {
    let parsed = parse_feed_bytes(bytes, fallback_title).map_err(|e| e.to_string())?;
    Ok(NormalizedFeed {
        title: parsed.title,
        items: parsed.items.into_iter().map(IncomingItem::from).collect(),
    })
}
