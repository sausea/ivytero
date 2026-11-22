//! Minimal OpenAI-compatible chat-completions client shared by features that
//! call a user-configured BYOK endpoint (RSS analysis, vocabulary lookup).

use crate::error::AppError;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ChatConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl ChatConfig {
    pub fn is_configured(&self) -> bool {
        !self.base_url.trim().is_empty()
            && !self.api_key.trim().is_empty()
            && !self.model.trim().is_empty()
    }
}

/// One non-streaming chat completion. `system` is optional; `user` is required.
pub async fn chat_completion(
    config: &ChatConfig,
    system: Option<&str>,
    user: &str,
) -> Result<String, AppError> {
    if !config.is_configured() {
        return Err(AppError::message("openai_chat: not configured"));
    }
    let base = config.base_url.trim().trim_end_matches('/');
    let url = if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    };
    let mut messages = Vec::new();
    if let Some(sys) = system.map(str::trim).filter(|s| !s.is_empty()) {
        messages.push(json!({"role": "system", "content": sys}));
    }
    messages.push(json!({"role": "user", "content": user}));
    let body = json!({
        "model": config.model.trim(),
        "messages": messages,
        "stream": false,
        "temperature": 0.3,
    });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| AppError::message(format!("openai_chat client: {e}")))?;
    let resp = client
        .post(&url)
        .bearer_auth(config.api_key.trim())
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::message(format!("openai_chat request: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::message(format!("openai_chat read: {e}")))?;
    if !status.is_success() {
        let snippet: String = text.chars().take(300).collect();
        return Err(AppError::message(format!(
            "openai_chat http {}: {snippet}",
            status.as_u16()
        )));
    }
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| AppError::message(format!("openai_chat parse: {e}")))?;
    let content = v
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::message("openai_chat: empty content"))?;
    Ok(content.to_string())
}
