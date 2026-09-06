//! Subscription-URL fetching (orchestrator layer responsibility per
//! architecture.md §2/§8). Fetching happens here in Rust; the fetched
//! body is handed to the core process for parsing so protocol logic stays
//! in one place. The `subscription-userinfo` response header (traffic
//! quotas / expiry) is surfaced to the UI.

use std::collections::HashMap;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_LENGTH, CONTENT_TYPE};
use serde::Serialize;

use crate::core::ImportResult;

pub const MAX_BODY_BYTES: u64 = 16 * 1024 * 1024;

/// Result of a subscription fetch + parse. When the caller asked to save
/// it (save_name), group_id references the new persisted group.
#[derive(Serialize, Clone)]
pub struct SubscribeOutcome {
    pub url: String,
    pub content_type: Option<String>,
    pub userinfo: Option<SubUserInfo>,
    pub group_id: Option<i64>,
    #[serde(flatten)]
    pub parsed: ImportResult,
}

#[derive(Serialize, Clone, Debug)]
pub struct SubUserInfo {
    pub upload: u64,
    pub download: u64,
    pub total: u64,
    pub expire: Option<u64>, // unix seconds
}

/// Build a HeaderMap from user-supplied key/value pairs. Invalid header
/// names or values abort the request.
pub fn build_headers(headers: &HashMap<String, String>) -> Result<HeaderMap, String> {
    let mut map = HeaderMap::new();
    for (key, value) in headers {
        let name = HeaderName::from_bytes(key.as_bytes())
            .map_err(|_| format!("invalid header name {key:?}"))?;
        let val = HeaderValue::from_str(value)
            .map_err(|_| format!("invalid header value for {key:?}"))?;
        map.insert(name, val);
    }
    Ok(map)
}

/// Fetch a subscription URL with the given request headers.
/// Returns (body, content-type, parsed userinfo).
pub async fn fetch_subscribe(
    client: &reqwest::Client,
    url: &str,
    headers: &HashMap<String, String>,
) -> Result<(Vec<u8>, Option<String>, Option<SubUserInfo>), String> {
    let header_map = build_headers(headers)?;
    let resp = client
        .get(url)
        .headers(header_map)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("subscription server returned HTTP {status}"));
    }
    if let Some(len) = resp
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
    {
        if let Ok(len) = len.parse::<u64>() {
            if len > MAX_BODY_BYTES {
                return Err(format!("subscription body too large ({len} bytes)"));
            }
        }
    }
    let content_type = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let userinfo = resp
        .headers()
        .get("subscription-userinfo")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_userinfo);
    let body = resp
        .bytes()
        .await
        .map_err(|e| format!("read response: {e}"))?;
    if body.len() as u64 > MAX_BODY_BYTES {
        return Err(format!(
            "subscription body too large ({} bytes)",
            body.len()
        ));
    }
    Ok((body.to_vec(), content_type, userinfo))
}

/// Parses "upload=0; download=123; total=1024; expire=1700000000".
fn parse_userinfo(value: &str) -> Option<SubUserInfo> {
    let mut info = SubUserInfo {
        upload: 0,
        download: 0,
        total: 0,
        expire: None,
    };
    for pair in value.split(';') {
        let mut it = pair.trim().splitn(2, '=');
        let (Some(key), Some(val)) = (it.next(), it.next()) else {
            continue;
        };
        let num = val.trim().parse::<u64>().ok()?;
        match key.trim() {
            "upload" => info.upload = num,
            "download" => info.download = num,
            "total" => info.total = num,
            "expire" => info.expire = Some(num),
            _ => {}
        }
    }
    Some(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn userinfo_parsing() {
        let info =
            parse_userinfo("upload=1024; download=2048; total=1073741824; expire=1780000000")
                .unwrap();
        assert_eq!(info.upload, 1024);
        assert_eq!(info.download, 2048);
        assert_eq!(info.total, 1073741824);
        assert_eq!(info.expire, Some(1780000000));
        assert!(parse_userinfo("garbage").is_some()); // lenient: all-zero row
    }

    /// Live check against a real subscription URL (needs network).
    /// Run manually: cargo test -- --ignored
    #[tokio::test]
    #[ignore = "requires network access to a live subscription"]
    async fn fetch_live_subscription() {
        let client = reqwest::Client::builder()
            .user_agent("nekos/0.1")
            .no_proxy()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap();
        let url = std::env::var("NEKOS_TEST_SUB_URL").unwrap_or_else(|_| {
            "https://6bd2f208.edge-bbe3c3f6.pages.dev/17e65fb1-5fc9-44cc-a6c6-25ecfbbcd886/sub"
                .into()
        });
        let (body, ctype, userinfo) = fetch_subscribe(&client, &url, &HashMap::new())
            .await
            .expect("fetch");
        assert!(!body.is_empty());
        assert!(ctype.is_some());
        println!(
            "url={url}\nbytes={}\ncontent_type={:?}\nuserinfo={:?}",
            body.len(),
            ctype,
            userinfo
        );
        // Body must be decodable by the core parser path: plain link text,
        // clash yaml, or an all-base64 payload (v2rayN-style).
        let text = String::from_utf8_lossy(&body);
        let all_base64 = text
            .trim()
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+/=\n\r".contains(c));
        assert!(text.contains("://") || text.contains("proxies:") || all_base64);
    }

    /// Live check against a Clash subscription that requires a specific
    /// User-Agent header (needs network). Run manually:
    /// cargo test -- --ignored
    #[tokio::test]
    #[ignore = "requires network access to a live subscription"]
    async fn fetch_live_clash_subscription_with_headers() {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap();
        let url = std::env::var("NEKOS_TEST_CLASH_URL")
            .unwrap_or_else(|_| "http://43.135.28.238/link/BRvzJbPhIM5j0ABa?clash=2".into());
        let mut headers = HashMap::new();
        let ua =
            std::env::var("NEKOS_TEST_CLASH_UA").unwrap_or_else(|_| "clash-verge/v2.5.2".into());
        headers.insert("User-Agent".into(), ua);
        let (body, _, userinfo) = fetch_subscribe(&client, &url, &headers)
            .await
            .expect("fetch");
        println!("url={url}\nbytes={}\nuserinfo={userinfo:?}", body.len());
        let text = String::from_utf8_lossy(&body);
        // The tested endpoint is a Clash YAML subscription.
        assert!(text.contains("proxies:") || text.contains("#!MANAGED-CONFIG"));
    }
}
