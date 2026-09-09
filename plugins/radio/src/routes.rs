//! Radio plugin REST routes — served through the plugin's `RouteSpec`s.
//! The "now playing" ICY metadata proxy the Radio window polls while a
//! station plays (browsers can't read Shoutcast/Icecast metadata directly).

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::response::IntoResponse;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, RouteHandler, user_id_from_request};
use shiny_plugin_sdk::services::PluginCtx;

const MAX_METADATA_READ: usize = 64 * 1024 + 4080;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    match tag {
        "nowplaying" => Some(nowplaying(ctx)),
        _ => None,
    }
}

fn user_id(req: &axum::extract::Request) -> Result<String, AppError> {
    user_id_from_request(req)
        .ok_or_else(|| AppError::Unauthorized("not authenticated".into()))
}

async fn take_query<T: DeserializeOwned + Send + 'static>(
    req: axum::extract::Request,
) -> Result<(T, axum::extract::Request), AppError> {
    let (mut parts, body) = req.into_parts();
    let query = axum::extract::Query::<T>::from_request_parts(&mut parts, &())
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid query: {e}")))?;
    Ok((query.0, axum::extract::Request::from_parts(parts, body)))
}

#[derive(Deserialize)]
struct NowPlayingQuery {
    url: String,
}

#[derive(Serialize)]
struct NowPlayingData {
    title: Option<String>,
    station_name: Option<String>,
}

fn nowplaying(_ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        async move {
            let _uid = user_id(&req)?;
            let (q, _) = take_query::<NowPlayingQuery>(req).await?;
            let url = q.url.trim().to_string();
            // Reject anything that isn't a public http(s) host — this endpoint
            // makes a server-side GET, so it would otherwise be an SSRF
            // vector (http://127.0.0.1, http://169.254.169.254, LAN IPs, …).
            validate_public_stream_host(&url).await?;

            let data = tokio::time::timeout(
                std::time::Duration::from_secs(8),
                fetch_icy_metadata(&url),
            )
            .await
            .unwrap_or_else(|_| Ok(NowPlayingData { title: None, station_name: None }))?;

            Ok(axum::Json(json!({ "success": true, "data": data })).into_response())
        }
    })
}

/// Require the URL to be http(s) with a host that resolves exclusively to
/// globally-routable addresses. Best-effort: a host that re-resolves to a
/// private address after this check (DNS rebinding) is out of scope.
async fn validate_public_stream_host(url: &str) -> Result<(), AppError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| AppError::BadRequest("invalid stream url".into()))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(AppError::BadRequest("url must be http(s)".into()));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::BadRequest("url must include a host".into()))?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::BadRequest("url must not include credentials".into()));
    }
    let port = parsed.port_or_known_default().unwrap_or(80);

    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| AppError::BadRequest("could not resolve stream host".into()))?
        .collect();
    if addrs.is_empty() {
        return Err(AppError::BadRequest("stream host resolved to no addresses".into()));
    }
    if addrs.iter().any(|a| !is_global_ip(a.ip())) {
        return Err(AppError::BadRequest("stream host is not a public address".into()));
    }
    Ok(())
}

/// True when the address is globally routable. Rejects loopback, private,
/// link-local, CGNAT (100.64/10), documentation, benchmarking, and reserved
/// ranges — the SSRF surface.
fn is_global_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => ipv4_is_global(v4),
        std::net::IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(mapped) => ipv4_is_global(mapped),
            None => ipv6_is_global(v6),
        },
    }
}

fn ipv4_is_global(ip: std::net::Ipv4Addr) -> bool {
    if ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_unspecified()
    {
        return false;
    }
    let o = ip.octets();
    match o {
        // 0.0.0.0/8 "this network"
        [0, ..] => false,
        // 100.64.0.0/10 shared address space (CGNAT)
        [100, b, ..] if b & 0b1100_0000 == 0b0100_0000 => false,
        // 192.0.0.0/24 IETF protocol assignments + 192.0.2.0/24 TEST-NET-1
        [192, 0, 0, _] | [192, 0, 2, _] => false,
        // 192.88.99.0/24 6to4 relay anycast (deprecated)
        [192, 88, 99, _] => false,
        // 198.18.0.0/15 benchmarking
        [198, 18 | 19, _, _] => false,
        // 198.51.100.0/24 TEST-NET-2
        [198, 51, 100, _] => false,
        // 203.0.113.0/24 TEST-NET-3
        [203, 0, 113, _] => false,
        // 240.0.0.0/4 reserved
        [240..=255, ..] => false,
        _ => true,
    }
}

fn ipv6_is_global(ip: std::net::Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    let s = ip.segments();
    // fe80::/10 link-local, fec0::/10 (deprecated site-local)
    let top10 = s[0] & 0xffc0;
    if top10 == 0xfe80 || top10 == 0xfec0 {
        return false;
    }
    // fc00::/7 unique-local
    if (s[0] & 0xfe00) == 0xfc00 {
        return false;
    }
    // 2001:db8::/32 documentation
    if s[0] == 0x2001 && s[1] == 0x0db8 {
        return false;
    }
    true
}

async fn fetch_icy_metadata(url: &str) -> Result<NowPlayingData, AppError> {
    let client = reqwest::Client::builder()
        .user_agent("Shiny/0.1 (shiny-radio)")
        .timeout(std::time::Duration::from_secs(6))
        .build()
        .unwrap_or_default();

    let resp = client
        .get(url)
        .header("Icy-MetaData", "1")
        .send()
        .await?;

    let station_name = resp
        .headers()
        .get("icy-name")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let metaint: usize = resp
        .headers()
        .get("icy-metaint")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let empty = || NowPlayingData { title: None, station_name: station_name.clone() };
    if metaint == 0 {
        return Ok(empty());
    }

    let to_read = metaint.saturating_add(4080).min(MAX_METADATA_READ);
    let mut buf: Vec<u8> = Vec::with_capacity(to_read.min(32 * 1024));
    let mut stream = resp.bytes_stream();

    use futures::StreamExt;
    while buf.len() < to_read {
        let Some(chunk) = stream.next().await else { break };
        let Ok(bytes) = chunk else { break };
        buf.extend_from_slice(&bytes);
        if let Some(title) = parse_stream_title(&buf, metaint) {
            return Ok(NowPlayingData { title: Some(title), station_name });
        }
    }
    Ok(empty())
}

fn parse_stream_title(buf: &[u8], metaint: usize) -> Option<String> {
    if buf.len() <= metaint {
        return None;
    }
    let len_byte = *buf.get(metaint)? as usize;
    let len = len_byte * 16;
    if len == 0 || buf.len() < metaint + 1 + len {
        return None;
    }
    let block = &buf[metaint + 1..metaint + 1 + len];
    let text = String::from_utf8_lossy(block);

    let key = "StreamTitle='";
    let start = text.find(key)? + key.len();
    let end = text[start..].find("';")? + start;
    let title = text[start..end].trim().to_string();
    if title.is_empty() || title == "-" {
        None
    } else {
        Some(title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(a: u8, b: u8, c: u8, d: u8) -> std::net::IpAddr {
        std::net::IpAddr::V4(std::net::Ipv4Addr::new(a, b, c, d))
    }

    #[test]
    fn public_ips_pass() {
        for ip in [
            v4(8, 8, 8, 8),          // public DNS
            v4(104, 26, 10, 78),     // public CDN
            v4(140, 82, 112, 4),     // public
            "2606:4700:4700::1111".parse().unwrap(),
        ] {
            assert!(is_global_ip(ip), "{ip} should be global");
        }
    }

    #[test]
    fn private_and_reserved_ips_fail() {
        for ip in [
            v4(127, 0, 0, 1),        // loopback
            v4(10, 0, 0, 1),         // private
            v4(172, 16, 0, 1),       // private
            v4(192, 168, 1, 1),      // private
            v4(169, 254, 169, 254),  // link-local (cloud metadata)
            v4(100, 64, 0, 1),       // CGNAT
            v4(192, 0, 2, 1),        // TEST-NET-1
            v4(198, 51, 100, 1),     // TEST-NET-2
            v4(203, 0, 113, 1),      // TEST-NET-3
            v4(0, 0, 0, 0),          // unspecified
            v4(255, 255, 255, 255),  // broadcast
            "::1".parse().unwrap(),  // v6 loopback
            "fe80::1".parse().unwrap(),   // v6 link-local
            "fc00::1".parse().unwrap(),   // v6 unique-local
            "::ffff:127.0.0.1".parse().unwrap(), // v4-mapped loopback
            "2001:db8::1".parse().unwrap(),      // documentation
        ] {
            assert!(!is_global_ip(ip), "{ip} must NOT be treated as global");
        }
    }
}
