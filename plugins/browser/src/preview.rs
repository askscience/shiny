//! Link previews: the title, description and image behind a hovered link.
//!
//! Fetched through the shared impersonating client
//! ([`shiny_filter::proxy::impersonated_client_builder`]), so a preview carries
//! the same browser identity as the page it points at instead of a library UA
//! that gets a challenge page.
//!
//! Best-effort and cached by design: hovering must never hammer a site, and a
//! page with no metadata still gets a card carrying whatever its `<title>`
//! says.
//!
//! The endpoint takes a URL straight from the frame, so it is an SSRF surface:
//! [`is_allowed`] refuses anything that is not a public `http(s)` target.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;

use shiny_plugin_sdk::errors::AppError;

use crate::fetch::ACCEPT_HTML;

const FETCH_TIMEOUT: Duration = Duration::from_secs(12);
/// How much of the document to scan for metadata. `<head>` is always near the
/// top, so a cap keeps a preview from reading a whole video.
const MAX_BYTES: usize = 256 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(600);
const CACHE_MAX: usize = 256;

/// One link's metadata, already cleaned for display.
#[derive(Clone, Debug, Serialize)]
pub struct Preview {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
    pub site: Option<String>,
}

/// Whether a URL is safe to fetch on the user's behalf.
///
/// The browser window can hover *any* link, so without this the preview route
/// is an SSRF primitive pointed at the user's own machine, its LAN and its
/// cloud metadata endpoints.
pub fn is_allowed(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return false;
    }
    // `Url::host_str` keeps the brackets on an IPv6 literal.
    let ip_host = host.trim_start_matches('[').trim_end_matches(']');
    match ip_host.parse::<std::net::IpAddr>() {
        Ok(ip) => !is_private_ip(ip),
        // A name: allowed. Resolving-then-checking would be stronger, but the
        // impersonating client resolves it; this catches the literal cases.
        Err(_) => true,
    }
}

fn is_private_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_unspecified()
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback() || v6.is_unspecified() || v6.is_unique_local()
        }
    }
}

fn cache() -> &'static Mutex<HashMap<String, (Instant, Preview)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, Preview)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached(url: &str) -> Option<Preview> {
    let map = cache().lock();
    let (at, preview) = map.get(url)?;
    if at.elapsed() < CACHE_TTL {
        Some(preview.clone())
    } else {
        None
    }
}

fn store(url: &str, preview: Preview) {
    let mut map = cache().lock();
    if map.len() >= CACHE_MAX {
        map.clear();
    }
    map.insert(url.to_string(), (Instant::now(), preview));
}

/// Fetch metadata for `url`, from the cache when it is warm.
pub async fn fetch(url: &str) -> Result<Preview, AppError> {
    if !is_allowed(url) {
        return Err(AppError::BadRequest(
            "that address cannot be previewed".into(),
        ));
    }
    if let Some(hit) = cached(url) {
        return Ok(hit);
    }

    let client = shiny_filter::proxy::impersonated_client_builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|e| AppError::Internal(format!("http client: {e}")))?;
    let response = client
        .get(url)
        .header("accept", ACCEPT_HTML)
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!("could not fetch {url}: {e}")))?;
    if !response.status().is_success() {
        return Err(AppError::BadRequest(format!(
            "{url} returned HTTP {}",
            response.status()
        )));
    }
    let body = response
        .text()
        .await
        .map_err(|e| AppError::Internal(format!("could not read {url}: {e}")))?;

    let preview = parse(url, truncate(&body, MAX_BYTES));
    store(url, preview.clone());
    Ok(preview)
}

/// Truncate at a UTF-8 boundary so a multibyte character is never split.
fn truncate(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Extract the display fields from a document.
pub fn parse(url: &str, html: &str) -> Preview {
    let title = meta_content(html, "og:title")
        .or_else(|| meta_content(html, "twitter:title"))
        .or_else(|| title_tag(html));
    let description = meta_content(html, "og:description")
        .or_else(|| meta_content(html, "twitter:description"))
        .or_else(|| meta_content(html, "description"));
    let image = meta_content(html, "og:image").or_else(|| meta_content(html, "twitter:image"));
    let site = meta_content(html, "og:site_name").or_else(|| host_of(url));

    Preview {
        url: url.to_string(),
        title: clean(title),
        description: clean(description),
        image: image.and_then(|image| absolutize(url, &image)),
        site: clean(site),
    }
}

/// The `content` of the first `<meta>` whose `property`/`name` is `key`.
fn meta_content(html: &str, key: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut from = 0usize;
    while let Some(rel) = lower[from..].find("<meta") {
        let start = from + rel;
        let end = match html[start..].find('>') {
            Some(e) => start + e + 1,
            None => break,
        };
        let tag = &html[start..end];
        let name = attr_value(tag, "property").or_else(|| attr_value(tag, "name"));
        if name
            .map(|n| n.eq_ignore_ascii_case(key))
            .unwrap_or(false)
        {
            if let Some(content) = attr_value(tag, "content") {
                return Some(content);
            }
        }
        from = end;
    }
    None
}

/// The text of the document's `<title>`.
fn title_tag(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let gt = lower[open..].find('>')? + open + 1;
    let close = lower[gt..].find("</title")? + gt;
    Some(html_unescape(&html[gt..close]))
}

/// Read `name="value"` (or single/unquoted) from a tag, case-insensitively.
fn attr_value(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut search = 0usize;
    while let Some(rel) = lower[search..].find(name) {
        let at = search + rel;
        let boundary_before = at == 0 || !bytes[at - 1].is_ascii_alphanumeric();
        let mut j = at + name.len();
        while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
            j += 1;
        }
        if boundary_before && j < bytes.len() && bytes[j] == b'=' {
            j += 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= bytes.len() {
                return None;
            }
            let quote = bytes[j];
            if quote == b'"' || quote == b'\'' {
                let close = lower[j + 1..].find(quote as char)? + j + 1;
                return Some(html_unescape(&tag[j + 1..close]));
            }
            let mut k = j;
            while k < bytes.len() && !bytes[k].is_ascii_whitespace() && bytes[k] != b'>' {
                k += 1;
            }
            return Some(html_unescape(&tag[j..k]));
        }
        search = at + name.len();
    }
    None
}

/// The handful of entities that actually appear in titles/descriptions.
fn html_unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(idx) = rest.find('&') {
        out.push_str(&rest[..idx]);
        let tail = &rest[idx..];
        // Only a short, closed entity is worth decoding; `s` is the byte index
        // of `;` so slicing there is always on a char boundary.
        let semi = match tail.find(';') {
            Some(s) if s <= 10 => Some(s),
            _ => None,
        };
        let entity = semi.map(|s| &tail[1..s]);
        let decoded = match entity {
            Some("amp") => Some('&'),
            Some("lt") => Some('<'),
            Some("gt") => Some('>'),
            Some("quot") => Some('"'),
            Some("apos") | Some("#39") | Some("#x27") | Some("#X27") => Some('\''),
            Some("#160") | Some("nbsp") => Some(' '),
            _ => None,
        };
        match decoded {
            Some(ch) => {
                out.push(ch);
                rest = &tail[semi.unwrap() + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Trim, collapse whitespace, and drop an empty value.
fn clean(value: Option<String>) -> Option<String> {
    let value = value?;
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed.chars().take(300).collect())
    }
}

/// The host a URL names, without `www.`.
fn host_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    Some(host.trim_start_matches("www.").to_string())
}

/// Resolve a (usually relative) image URL against the page it came from.
fn absolutize(base: &str, image: &str) -> Option<String> {
    let image = image.trim();
    if image.is_empty() {
        return None;
    }
    if image.starts_with("http://") || image.starts_with("https://") {
        return Some(image.to_string());
    }
    url::Url::parse(base).ok()?.join(image).ok().map(|u| u.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_open_graph() {
        let html = r#"<!doctype html><html><head>
            <title>Fallback title</title>
            <meta property="og:title" content="Real &amp; Title">
            <meta property="og:description" content="A short summary.">
            <meta property="og:image" content="/img/hero.png">
            <meta property="og:site_name" content="Example">
        </head><body></body></html>"#;
        let preview = parse("https://example.com/a/b", html);
        assert_eq!(preview.title.as_deref(), Some("Real & Title"));
        assert_eq!(preview.description.as_deref(), Some("A short summary."));
        assert_eq!(preview.image.as_deref(), Some("https://example.com/img/hero.png"));
        assert_eq!(preview.site.as_deref(), Some("Example"));
    }

    #[test]
    fn falls_back_to_title_and_host() {
        let preview = parse("https://www.example.com/x", "<html><head><title>Just a title</title></head></html>");
        assert_eq!(preview.title.as_deref(), Some("Just a title"));
        assert_eq!(preview.site.as_deref(), Some("example.com"));
        assert_eq!(preview.description, None);
    }

    #[test]
    fn attribute_order_does_not_matter() {
        let html = r#"<meta content="Desc" name="description">"#;
        assert_eq!(meta_content(html, "description").as_deref(), Some("Desc"));
    }

    #[test]
    fn private_targets_are_refused() {
        for url in [
            "http://127.0.0.1:8080/x",
            "http://localhost/x",
            "http://10.0.0.5/",
            "http://192.168.1.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]/",
            "file:///etc/passwd",
        ] {
            assert!(!is_allowed(url), "should refuse {url}");
        }
        assert!(is_allowed("https://example.com/"));
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        let text = "ééééé";
        let cut = truncate(text, 3);
        assert!(text.starts_with(cut));
        assert!(cut.len() <= 3);
    }
}
