//! Environment configuration (`RSS_*`).

use crate::ingest::fetcher::FetchPolicy;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

pub const MIN_TOKEN_CHARS: usize = 24;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub db_path: PathBuf,
    pub read_token: String,
    /// Enables `/admin` and `/api/admin` when set.
    pub admin_token: Option<String>,
    pub default_refresh_secs: i64,
    pub concurrency: usize,
    pub fetch: FetchPolicy,
}

fn var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn parsed<T: std::str::FromStr>(name: &str, default: T) -> Result<T, String> {
    match var(name) {
        Some(raw) => raw.parse().map_err(|_| format!("{name}: invalid value")),
        None => Ok(default),
    }
}

pub fn db_path_from_env() -> PathBuf {
    var("RSS_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("rss-archive.sqlite"))
}

pub fn default_refresh_from_env() -> Result<i64, String> {
    parsed("RSS_DEFAULT_REFRESH_SECS", 600)
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let read_token = var("RSS_READ_TOKEN").ok_or("RSS_READ_TOKEN is required")?;
        if read_token.chars().count() < MIN_TOKEN_CHARS {
            return Err(format!(
                "RSS_READ_TOKEN must be at least {MIN_TOKEN_CHARS} characters"
            ));
        }
        let admin_token = var("RSS_ADMIN_TOKEN");
        if let Some(admin) = &admin_token {
            if admin.chars().count() < MIN_TOKEN_CHARS {
                return Err(format!(
                    "RSS_ADMIN_TOKEN must be at least {MIN_TOKEN_CHARS} characters"
                ));
            }
            if *admin == read_token {
                return Err("RSS_ADMIN_TOKEN must differ from RSS_READ_TOKEN".into());
            }
        }
        let bind = parsed("RSS_BIND", SocketAddr::from(([127, 0, 0, 1], 8787)))?;
        let fetch = FetchPolicy {
            max_bytes: parsed("RSS_MAX_FETCH_BYTES", 2 * 1024 * 1024)?,
            timeout: Duration::from_secs(parsed("RSS_FETCH_TIMEOUT_SECS", 20)?),
            allow_private_networks: parsed("RSS_ALLOW_PRIVATE_NETWORKS", false)?,
            allow_fake_ip: parsed("RSS_ALLOW_FAKE_IP", false)?,
            ..FetchPolicy::default()
        };
        Ok(Self {
            bind,
            db_path: db_path_from_env(),
            read_token,
            admin_token,
            default_refresh_secs: default_refresh_from_env()?,
            concurrency: parsed("RSS_FETCH_CONCURRENCY", 4usize)?.max(1),
            fetch,
        })
    }
}
