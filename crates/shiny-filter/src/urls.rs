//! The proxy URL scheme.
//!
//! A browser talks to a proxy in two different ways and both need a way to
//! name "the real thing being fetched":
//!
//! * **Absolute-URI requests** (plain HTTP): `GET http://example.com/a.js`.
//!   The target is already the request target, so no encoding is needed.
//! * **Path requests** (an iframe, a link, a redirect — anything the browser
//!   sends as an origin-form path): `GET /p/https/example.com/a.js`.
//!
//! The in-app plugin window uses the second form as its iframe origin, and the
//! filter proxy rewrites outgoing HTML into it so subresources follow. The
//! format is kept flat and human-readable on purpose: it makes proxy logs and
//! the benchmarks legible, and it round-trips without percent-encoding the
//! target's own query string.
//!
//! ```text
//! /p/https/example.com/path/to/page?q=1   ->  https://example.com/path/to/page?q=1
//! /p/http/example.com:8080/a              ->  http://example.com:8080/a
//! ```

/// First path segment that marks a proxied target.
pub const PROXY_PREFIX: &str = "/p/";

/// Encode an absolute URL into the proxy's path form.
///
/// Returns `None` for URLs that are not absolute `http`/`https` — the caller
/// should leave those alone rather than mangle them.
pub fn encode_target(absolute: &str) -> Option<String> {
    let (scheme, rest) = absolute.split_once("://")?;
    match scheme {
        "http" | "https" => Some(format!("{PROXY_PREFIX}{scheme}/{rest}")),
        _ => None,
    }
}

/// Decode a proxy path back into the absolute URL it names.
///
/// The input may be origin-form (`/p/https/…`) or already-absolute
/// (`http://example.com/x`), because both reach the same handler.
pub fn decode_target(request_target: &str) -> Option<String> {
    if request_target.starts_with("http://") || request_target.starts_with("https://") {
        return Some(request_target.to_string());
    }
    let rest = request_target.strip_prefix(PROXY_PREFIX)?;
    let (scheme, remainder) = rest.split_once('/')?;
    match scheme {
        "http" | "https" => Some(format!("{scheme}://{remainder}")),
        _ => None,
    }
}

/// True when a path names a proxied target.
pub fn is_proxied_path(request_target: &str) -> bool {
    request_target.starts_with(PROXY_PREFIX)
}

/// The proxy-relative form of a target, for use inside rewritten documents.
///
/// A document fetched through `/p/https/example.com/` must reference
/// `/p/https/cdn.example.com/x.js` rather than `https://cdn.example.com/x.js`,
/// otherwise the browser would leave the proxy and fetch it unfiltered.
pub fn proxied_path_of(absolute: &str) -> Option<String> {
    encode_target(absolute)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_plain() {
        let encoded = encode_target("https://example.com/a/b?c=1&d=2").unwrap();
        assert_eq!(encoded, "/p/https/example.com/a/b?c=1&d=2");
        assert_eq!(
            decode_target(&encoded).unwrap(),
            "https://example.com/a/b?c=1&d=2"
        );
    }

    #[test]
    fn round_trips_with_port() {
        let encoded = encode_target("http://127.0.0.1:8080/api/plugins").unwrap();
        assert_eq!(encoded, "/p/http/127.0.0.1:8080/api/plugins");
        assert_eq!(
            decode_target(&encoded).unwrap(),
            "http://127.0.0.1:8080/api/plugins"
        );
    }

    #[test]
    fn round_trips_empty_path() {
        let encoded = encode_target("https://example.com").unwrap();
        assert_eq!(encoded, "/p/https/example.com");
        assert_eq!(decode_target(&encoded).unwrap(), "https://example.com");
    }

    #[test]
    fn absolute_targets_pass_through_decode() {
        assert_eq!(
            decode_target("http://example.com/x").unwrap(),
            "http://example.com/x"
        );
    }

    #[test]
    fn rejects_non_http_schemes() {
        assert!(encode_target("data:text/html,hi").is_none());
        assert!(encode_target("about:blank").is_none());
        assert!(encode_target("ws://example.com/socket").is_none());
        assert!(decode_target("/p/ftp/example.com").is_none());
        assert!(decode_target("/other/thing").is_none());
    }
}
