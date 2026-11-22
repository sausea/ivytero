//! Agentero Cloud RSS Server: archive feeds 24/7 and serve them read-only.
//!
//! The archive holds collection config and feed content only. Reader state
//! (stars, read marks, tags, AI summaries, vault paths) never reaches it.

pub mod api;
pub mod config;
pub mod ingest;
pub mod opml;
pub mod storage;
