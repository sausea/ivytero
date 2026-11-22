//! Feed HTTP client with SSRF guards.
//!
//! Each hop (including redirects) is resolved once, every resolved address is
//! checked, and the connection is pinned to the checked address so a second
//! DNS answer cannot redirect it (DNS rebinding). Bodies are counted after
//! decompression.

use reqwest::header::{
    HeaderMap, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED, LOCATION, RETRY_AFTER,
};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;
use url::{Host, Url};

pub const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Clone)]
pub struct FetchPolicy {
    pub max_bytes: usize,
    pub timeout: Duration,
    pub connect_timeout: Duration,
    /// Only for tests / trusted intranets: permits loopback and private targets.
    pub allow_private_networks: bool,
    /// Permits only 198.18.0.0/15, which TUN proxies hand out as fake-ip DNS answers.
    pub allow_fake_ip: bool,
    pub user_agent: String,
}

impl Default for FetchPolicy {
    fn default() -> Self {
        Self {
            max_bytes: 2 * 1024 * 1024,
            timeout: Duration::from_secs(20),
            connect_timeout: Duration::from_secs(10),
            allow_private_networks: false,
            allow_fake_ip: false,
            user_agent: format!("agentero-rss-server/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

#[derive(Debug)]
pub enum FetchOutcome {
    NotModified {
        etag: Option<String>,
        last_modified: Option<String>,
    },
    Body {
        final_url: String,
        content_type: String,
        etag: Option<String>,
        last_modified: Option<String>,
        body: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    Blocked(String),
    Http {
        status: u16,
        retry_after_secs: Option<u64>,
    },
    Network(String),
    TooLarge,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Blocked(why) => write!(f, "blocked: {why}"),
            Self::Http { status, .. } => write!(f, "http {status}"),
            Self::Network(e) => write!(f, "network: {e}"),
            Self::TooLarge => f.write_str("response too large"),
        }
    }
}

pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_v4(v4);
            }
            let seg = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (seg[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (seg[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
                || (seg[0] == 0x2001 && seg[1] == 0x0db8) // documentation
                || (seg[0] == 0x0064 && seg[1] == 0xff9b) // NAT64 to arbitrary v4
                || v6 == Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0))
        }
    }
}

/// 198.18.0.0/15, also when written as an IPv4-mapped IPv6 address.
pub fn is_fake_ip(ip: IpAddr) -> bool {
    let v4 = match ip {
        IpAddr::V4(v4) => v4,
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4,
            None => return false,
        },
    };
    let o = v4.octets();
    o[0] == 198 && (o[1] & 0xfe) == 18
}

fn address_allowed(ip: IpAddr, policy: &FetchPolicy) -> bool {
    policy.allow_private_networks || is_public_ip(ip) || (policy.allow_fake_ip && is_fake_ip(ip))
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local() // includes 169.254.169.254 metadata
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.is_documentation()
        || ip.is_multicast()
        || o[0] == 0
        || (o[0] == 100 && (o[1] & 0xc0) == 64) // CGNAT 100.64/10
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0/24
        || (o[0] == 198 && (o[1] & 0xfe) == 18) // benchmarking 198.18/15
        || o[0] >= 240)
}

fn check_url(url: &Url, policy: &FetchPolicy) -> Result<(String, u16), FetchError> {
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(FetchError::Blocked(format!("scheme {}", url.scheme())));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(FetchError::Blocked("credentials in url".into()));
    }
    let host = url
        .host_str()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| FetchError::Blocked("missing host".into()))?
        .to_string();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| FetchError::Blocked("missing port".into()))?;
    if !policy.allow_private_networks && port < 1024 && port != 80 && port != 443 {
        return Err(FetchError::Blocked(format!("port {port}")));
    }
    Ok((host, port))
}

async fn resolve_checked(
    url: &Url,
    host: &str,
    port: u16,
    policy: &FetchPolicy,
) -> Result<SocketAddr, FetchError> {
    let addrs: Vec<SocketAddr> = match url.host() {
        Some(Host::Ipv4(ip)) => vec![SocketAddr::new(IpAddr::V4(ip), port)],
        Some(Host::Ipv6(ip)) => vec![SocketAddr::new(IpAddr::V6(ip), port)],
        _ => tokio::time::timeout(
            policy.connect_timeout,
            tokio::net::lookup_host((host, port)),
        )
        .await
        .map_err(|_| FetchError::Network("dns timeout".into()))?
        .map_err(|e| FetchError::Network(format!("dns: {e}")))?
        .collect(),
    };
    let first = *addrs
        .first()
        .ok_or_else(|| FetchError::Network("dns: no address".into()))?;
    if let Some(bad) = addrs.iter().find(|a| !address_allowed(a.ip(), policy)) {
        return Err(FetchError::Blocked(format!(
            "non-public address {}",
            bad.ip()
        )));
    }
    Ok(first)
}

fn header(headers: &HeaderMap, name: reqwest::header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

fn retry_after_secs(headers: &HeaderMap) -> Option<u64> {
    let raw = header(headers, RETRY_AFTER)?;
    if let Ok(secs) = raw.trim().parse::<u64>() {
        return Some(secs);
    }
    let at = chrono::DateTime::parse_from_rfc2822(raw.trim()).ok()?;
    let delta = at.with_timezone(&chrono::Utc) - chrono::Utc::now();
    Some(delta.num_seconds().max(0) as u64)
}

/// GET `url` with conditional headers, following up to [`MAX_REDIRECTS`]
/// redirects with the same checks on every hop.
pub async fn fetch(
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
    policy: &FetchPolicy,
) -> Result<FetchOutcome, FetchError> {
    let mut current =
        Url::parse(url).map_err(|e| FetchError::Blocked(format!("invalid url: {e}")))?;
    for _ in 0..=MAX_REDIRECTS {
        let (host, port) = check_url(&current, policy)?;
        let addr = resolve_checked(&current, &host, port, policy).await?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(policy.timeout)
            .connect_timeout(policy.connect_timeout)
            .user_agent(policy.user_agent.clone())
            .resolve(&host, addr)
            .no_proxy()
            .build()
            .map_err(|e| FetchError::Network(e.to_string()))?;
        let mut req = client.get(current.clone()).header(
            reqwest::header::ACCEPT,
            "application/rss+xml, application/atom+xml, application/feed+json, application/xml;q=0.9, text/xml;q=0.9, */*;q=0.5",
        );
        if let Some(tag) = etag.filter(|s| !s.is_empty()) {
            req = req.header(IF_NONE_MATCH, tag);
        }
        if let Some(lm) = last_modified.filter(|s| !s.is_empty()) {
            req = req.header(IF_MODIFIED_SINCE, lm);
        }
        let mut res = req
            .send()
            .await
            .map_err(|e| FetchError::Network(e.to_string()))?;
        let status = res.status();
        if status.is_redirection() && status.as_u16() != 304 {
            let location = header(res.headers(), LOCATION).ok_or(FetchError::Http {
                status: status.as_u16(),
                retry_after_secs: None,
            })?;
            current = current
                .join(&location)
                .map_err(|e| FetchError::Blocked(format!("bad redirect: {e}")))?;
            continue;
        }
        if status.as_u16() == 304 {
            return Ok(FetchOutcome::NotModified {
                etag: header(res.headers(), ETAG),
                last_modified: header(res.headers(), LAST_MODIFIED),
            });
        }
        if !status.is_success() {
            return Err(FetchError::Http {
                status: status.as_u16(),
                retry_after_secs: retry_after_secs(res.headers()),
            });
        }
        if res
            .content_length()
            .is_some_and(|n| n as usize > policy.max_bytes)
        {
            return Err(FetchError::TooLarge);
        }
        let content_type = header(res.headers(), reqwest::header::CONTENT_TYPE).unwrap_or_default();
        let out_etag = header(res.headers(), ETAG);
        let out_lm = header(res.headers(), LAST_MODIFIED);
        let mut body = Vec::new();
        while let Some(chunk) = res
            .chunk()
            .await
            .map_err(|e| FetchError::Network(e.to_string()))?
        {
            if body.len() + chunk.len() > policy.max_bytes {
                return Err(FetchError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        return Ok(FetchOutcome::Body {
            final_url: current.to_string(),
            content_type,
            etag: out_etag,
            last_modified: out_lm,
            body,
        });
    }
    Err(FetchError::Blocked("too many redirects".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_ip_switch_only_opens_198_18() {
        let policy = FetchPolicy {
            allow_fake_ip: true,
            ..FetchPolicy::default()
        };
        for ip in ["198.18.0.42", "198.19.255.1", "::ffff:198.18.5.128"] {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(!address_allowed(ip, &FetchPolicy::default()), "{ip}");
            assert!(address_allowed(ip, &policy), "{ip}");
        }
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "fdfe:dcba:9876::1",
        ] {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(!address_allowed(ip, &policy), "{ip}");
        }
    }

    #[test]
    fn blocks_internal_addresses() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip} must be blocked");
        }
        for ip in ["1.1.1.1", "93.184.216.34", "2606:4700:4700::1111"] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip} must be allowed");
        }
    }

    #[tokio::test]
    async fn rejects_private_target_and_credentials() {
        let policy = FetchPolicy::default();
        let err = fetch("http://127.0.0.1:8080/feed", None, None, &policy)
            .await
            .unwrap_err();
        assert!(matches!(err, FetchError::Blocked(_)));
        let err = fetch("https://user:pw@example.org/feed", None, None, &policy)
            .await
            .unwrap_err();
        assert!(matches!(err, FetchError::Blocked(_)));
        let err = fetch("file:///etc/passwd", None, None, &policy)
            .await
            .unwrap_err();
        assert!(matches!(err, FetchError::Blocked(_)));
    }
}
