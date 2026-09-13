//! Classify an HTTP request into the request-type vocabulary `adblock-rust`
//! matches filter options against (`$script`, `$image`, `$xhr`, …).
//!
//! Browsers tell us most of this for free: `Sec-Fetch-Dest` is authoritative
//! when present (every current engine sends it), with `Accept` and the path
//! extension as fallbacks for non-browser and older clients. Getting this
//! wrong makes filters silently under- or over-match, so the precedence here
//! is deliberate.

use adblock::request::Request;

/// The resource kind of one proxied request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestKind {
    Document,
    Subdocument,
    Script,
    Stylesheet,
    Image,
    Media,
    Font,
    Xhr,
    Fetch,
    Websocket,
    Ping,
    Object,
    Other,
}

impl RequestKind {
    /// The string `adblock::Request::new` expects as its `request_type`.
    pub fn as_adblock_type(self) -> &'static str {
        match self {
            RequestKind::Document => "document",
            RequestKind::Subdocument => "subdocument",
            RequestKind::Script => "script",
            RequestKind::Stylesheet => "stylesheet",
            RequestKind::Image => "image",
            RequestKind::Media => "media",
            RequestKind::Font => "font",
            RequestKind::Xhr => "xhr",
            RequestKind::Fetch => "fetch",
            RequestKind::Websocket => "websocket",
            RequestKind::Ping => "ping",
            RequestKind::Object => "object",
            RequestKind::Other => "other",
        }
    }

    /// True for kinds whose *body* we may need to rewrite (HTML/CSS).
    pub fn is_rewritable(self) -> bool {
        matches!(self, RequestKind::Document | RequestKind::Subdocument)
    }
}

/// Classify from the headers and URL of an outgoing request.
///
/// `sec_fetch_dest`, `accept` and `content_type` are the raw header values when
/// present. `method` matters for `$method=` filters and for `ping`/beacon.
pub fn classify_request(
    url: &str,
    sec_fetch_dest: Option<&str>,
    accept: Option<&str>,
    content_type: Option<&str>,
    method: &str,
) -> RequestKind {
    // 1. `Sec-Fetch-Dest` is authoritative when the client sends it.
    if let Some(dest) = sec_fetch_dest {
        if let Some(kind) = from_sec_fetch_dest(dest) {
            return kind;
        }
    }

    // 2. Method-first cases that headers rarely disambiguate.
    let method_upper = method.to_ascii_uppercase();
    if content_type
        .map(|ct| ct.starts_with("text/plain") || ct.starts_with("application/x-www-form-urlencoded"))
        .unwrap_or(false)
        && method_upper == "POST"
    {
        // Beacons are usually POSTs of tiny bodies with no `Sec-Fetch-Dest`.
        // XHR/fetch is the safer default: `$ping` filters are rarer than
        // `$xhr` ones, and a false `ping` breaks real app traffic.
        // Fall through to the Accept/extension heuristics.
    }

    // 3. `Accept` tells us a great deal for XHR/fetch/document.
    if let Some(accept) = accept {
        if accept.contains("text/html") || accept.contains("application/xhtml+xml") {
            return RequestKind::Document;
        }
        if accept.contains("text/css") {
            return RequestKind::Stylesheet;
        }
        if accept.contains("image/") {
            return RequestKind::Image;
        }
        if accept.contains("video/") || accept.contains("audio/") {
            return RequestKind::Media;
        }
        if accept.contains("font/") || accept.contains("application/font") {
            return RequestKind::Font;
        }
    }

    // 4. Fall back to the path extension.
    from_extension(url).unwrap_or(RequestKind::Other)
}

fn from_sec_fetch_dest(dest: &str) -> Option<RequestKind> {
    // Values are lowercase tokens per Fetch Metadata spec, but be lenient.
    Some(match dest.trim().to_ascii_lowercase().as_str() {
        "document" => RequestKind::Document,
        "iframe" | "frame" | "nested-document" => RequestKind::Subdocument,
        "script" | "worker" | "sharedworker" | "serviceworker" => RequestKind::Script,
        "style" => RequestKind::Stylesheet,
        "image" => RequestKind::Image,
        "video" | "audio" | "track" => RequestKind::Media,
        "font" => RequestKind::Font,
        "empty" => RequestKind::Fetch,
        "object" | "embed" => RequestKind::Object,
        "manifest" | "report" | "audit" => RequestKind::Other,
        // Unknown tokens fall through to the other heuristics rather than
        // being forced into `other`.
        _ => return None,
    })
}

fn from_extension(url: &str) -> Option<RequestKind> {
    // Strip query/fragment before looking at the extension.
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url);
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    Some(match ext.as_str() {
        "html" | "htm" | "xhtml" => RequestKind::Document,
        "js" | "mjs" | "cjs" => RequestKind::Script,
        "css" => RequestKind::Stylesheet,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "svg" | "ico" | "bmp" => {
            RequestKind::Image
        }
        "mp4" | "webm" | "mkv" | "mov" | "mp3" | "ogg" | "wav" | "flac" | "m4a" | "m4v" => {
            RequestKind::Media
        }
        "woff" | "woff2" | "ttf" | "otf" | "eot" => RequestKind::Font,
        "json" | "xml" | "txt" => RequestKind::Xhr,
        _ => return None,
    })
}

/// Build the `adblock::Request` for one outbound request.
///
/// Returns `None` when the URL or source URL cannot be parsed — an
/// unparseable URL is never blocked, because failing open is the only safe
/// behaviour for a network filter.
pub fn build_adblock_request(
    url: &str,
    source_url: &str,
    kind: RequestKind,
    method: &str,
) -> Option<Request> {
    Request::new(url, source_url, kind.as_adblock_type(), method).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sec_fetch_dest_wins() {
        assert_eq!(
            classify_request(
                "https://cdn.example.com/a.bin",
                Some("script"),
                Some("text/html"),
                None,
                "GET"
            ),
            RequestKind::Script
        );
    }

    #[test]
    fn accept_html_is_a_document() {
        assert_eq!(
            classify_request(
                "https://example.com/",
                None,
                Some("text/html,application/xhtml+xml"),
                None,
                "GET"
            ),
            RequestKind::Document
        );
    }

    #[test]
    fn extension_fallback() {
        assert_eq!(
            classify_request("https://example.com/a.css", None, None, None, "GET"),
            RequestKind::Stylesheet
        );
        assert_eq!(
            classify_request("https://example.com/a.woff2", None, None, None, "GET"),
            RequestKind::Font
        );
        assert_eq!(
            classify_request("https://example.com/a", None, None, None, "GET"),
            RequestKind::Other
        );
    }

    #[test]
    fn unknown_sec_fetch_dest_falls_through() {
        assert_eq!(
            classify_request(
                "https://example.com/a.png",
                Some("something-new"),
                None,
                None,
                "GET"
            ),
            RequestKind::Image
        );
    }
}
