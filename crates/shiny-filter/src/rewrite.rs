//! Document rewriting: keep every subresource flowing back through the proxy.
//!
//! A blocker that only filters the *first* request is useless, so a proxied
//! document must reference its subresources through the proxy too. There are
//! two mechanisms and this crate uses both, because neither is sufficient:
//!
//! * **Static rewriting** (this module) rewrites `src`/`href`/`srcset`/… in the
//!   markup. It covers the initial parse, before any script has run.
//! * **A runtime shim** ([`crate::inject`]) patches `fetch`, `XMLHttpRequest`,
//!   `WebSocket` and friends for everything a script does afterwards.
//!
//! # Why a hand-written scanner instead of an HTML parser
//!
//! The rewrite is a surgical, whitespace-preserving edit of attribute values.
//! A parse/serialize round trip would reorder attributes, re-quote everything,
//! normalise entities and can change how a picky page renders — and it costs
//! far more than the scan. The scanner here only understands enough of HTML to
//! avoid rewriting inside comments, `<script>` or `<style>` bodies. Bodies are
//! deliberately left alone: rewriting JavaScript string literals reliably needs
//! a JS parser, and the runtime shim already covers dynamic requests.

use crate::urls::proxied_path_of;

/// Attributes whose value is a URL, and the request kind they imply.
///
/// * `Some(kind)` — only rewrite when the element is the one that owns this
///   attribute (so `data-src`, which many lazy loaders read, is left alone).
/// * `None` — rewrite on any element.
const URL_ATTRS: &[(&str, Option<&str>, &str)] = &[
    ("src", Some("img"), "image"),
    ("src", Some("script"), "script"),
    ("src", Some("iframe"), "document"),
    ("src", Some("frame"), "document"),
    ("src", Some("embed"), "object"),
    ("src", Some("source"), "media"),
    ("src", Some("audio"), "media"),
    ("src", Some("video"), "media"),
    ("src", Some("track"), "media"),
    ("poster", Some("video"), "image"),
    // Any element: link/area/a and the odd legacy tag.
    ("href", None, "document"),
    ("action", Some("form"), "document"),
    ("formaction", None, "document"),
    ("data", Some("object"), "object"),
    ("background", None, "image"),
];

/// Tags whose contents must not be rewritten.
const RAW_TEXT_TAGS: &[&str] = &["script", "style", "textarea", "title"];

/// A page's class/id inventory, used to resolve generic cosmetic rules.
#[derive(Debug, Default, Clone)]
pub struct ClassIdSets {
    pub classes: Vec<String>,
    pub ids: Vec<String>,
}

impl ClassIdSets {
    fn push_class(&mut self, value: &str) {
        for class in value.split_whitespace() {
            let class = class.trim();
            if !class.is_empty() && !self.classes.iter().any(|c| c == class) {
                self.classes.push(class.to_string());
            }
        }
    }

    fn push_id(&mut self, value: &str) {
        let id = value.trim();
        if !id.is_empty() && !self.ids.iter().any(|i| i == id) {
            self.ids.push(id.to_string());
        }
    }
}

/// Result of rewriting one document.
#[derive(Debug)]
pub struct Rewritten {
    pub html: String,
    /// Every URL that was rewritten (for logs/debugging).
    pub rewritten_count: usize,
    /// Class/id inventory collected while scanning.
    pub class_id: ClassIdSets,
}

/// Rewrite a document so its subresources come back through `proxy_base`.
///
/// `proxy_base` is the scheme+authority of the proxy, e.g.
/// `http://127.0.0.1:8899`, with no trailing slash. `document_url` is the real
/// absolute URL of the page being rewritten.
pub fn rewrite_html(document_url: &str, proxy_base: &str, html: &str) -> Rewritten {
    let mut out = String::with_capacity(html.len() + html.len() / 8);
    let mut class_id = ClassIdSets::default();
    let mut rewritten_count = 0usize;

    let bytes = html.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        let Some(open_rel) = html[i..].find('<') else {
            out.push_str(&html[i..]);
            break;
        };
        let open = i + open_rel;
        out.push_str(&html[i..open]);

        // Comment: copy verbatim to the end of `-->`.
        if html[open..].starts_with("<!--") {
            match html[open + 4..].find("-->") {
                Some(close_rel) => {
                    let end = open + 4 + close_rel + 3;
                    out.push_str(&html[open..end]);
                    i = end;
                }
                None => {
                    out.push_str(&html[open..]);
                    break;
                }
            }
            continue;
        }

        // Tag: from `<` to the matching `>` (quote-aware).
        let mut j = open + 1;
        let mut in_quote: Option<u8> = None;
        while j < bytes.len() {
            let b = bytes[j];
            match in_quote {
                Some(q) => {
                    if b == q {
                        in_quote = None;
                    }
                }
                None => {
                    if b == b'"' || b == b'\'' {
                        in_quote = Some(b);
                    } else if b == b'>' {
                        break;
                    }
                }
            }
            j += 1;
        }
        if j >= bytes.len() {
            // Unterminated tag: emit the remainder untouched.
            out.push_str(&html[open..]);
            break;
        }

        let tag_text = &html[open..=j];
        out.push_str(&rewrite_tag(tag_text, document_url, proxy_base, &mut class_id, &mut rewritten_count));

        // Skip the raw-text body of script/style/textarea/title.
        let tag_name = tag_name_of(tag_text);
        let is_closing = tag_text.starts_with("</");
        if !is_closing && RAW_TEXT_TAGS.contains(&tag_name.as_str()) {
            let close = format!("</{tag_name}");
            // `find_ascii_ci` rather than `to_ascii_lowercase().find(...)`:
            // lowercasing the *remaining document* here made this loop
            // quadratic, which showed up as the rewriter collapsing from
            // ~18 MiB/s at 31 KB to ~6 MiB/s at 658 KB (see
            // `benchmarks/README.md`).
            match find_ascii_ci(&html[j + 1..], &close) {
                Some(rel) => {
                    let body_end = j + 1 + rel;
                    out.push_str(&html[j + 1..body_end]);
                    i = body_end;
                    continue;
                }
                None => {
                    out.push_str(&html[j + 1..]);
                    break;
                }
            }
        }

        i = j + 1;
    }

    Rewritten {
        html: out,
        rewritten_count,
        class_id,
    }
}

/// Rewrite the URL-bearing attributes of a single tag.
///
/// Only the attribute *values* that change are substituted; everything else is
/// copied as `&str` slices, so non-ASCII markup and the original quoting and
/// spacing survive byte-for-byte.
fn rewrite_tag(
    tag_text: &str,
    document_url: &str,
    proxy_base: &str,
    class_id: &mut ClassIdSets,
    rewritten_count: &mut usize,
) -> String {
    let tag_name = tag_name_of(tag_text);
    let mut out = String::with_capacity(tag_text.len() + 64);
    let bytes = tag_text.as_bytes();
    let len = bytes.len();

    // i = position from which `tag_text` has not yet been copied.
    let mut i = 0usize;
    // Start scanning attribute names only after the tag name itself.
    let mut p = 1usize;
    if tag_text.starts_with("</") {
        p = 2;
    }
    while p < len && !bytes[p].is_ascii_whitespace() && bytes[p] != b'/' && bytes[p] != b'>' {
        p += 1;
    }

    while p < len && bytes[p] != b'>' && !(bytes[p] == b'/' && p + 1 < len && bytes[p + 1] == b'>') {
        if !is_name_start(bytes[p]) {
            p += 1;
            continue;
        }

        let name_start = p;
        while p < len && is_name_byte(bytes[p]) {
            p += 1;
        }
        let name = &tag_text[name_start..p];
        let name_lower = name.to_ascii_lowercase();

        // Look for `=` (allowing whitespace around it).
        let mut k = p;
        while k < len && bytes[k].is_ascii_whitespace() {
            k += 1;
        }
        if k >= len || bytes[k] != b'=' {
            // Valueless attribute (`disabled`, `defer`): nothing to rewrite.
            continue;
        }
        let _eq = k;
        k += 1;
        while k < len && bytes[k].is_ascii_whitespace() {
            k += 1;
        }
        if k >= len {
            break;
        }

        let quote = match bytes[k] {
            b'"' | b'\'' => Some(bytes[k]),
            _ => None,
        };
        let value_start = if quote.is_some() { k + 1 } else { k };
        let value_end = match quote {
            Some(q) => {
                let mut e = value_start;
                while e < len && bytes[e] != q {
                    e += 1;
                }
                e
            }
            None => {
                let mut e = value_start;
                while e < len && !bytes[e].is_ascii_whitespace() && bytes[e] != b'>' {
                    e += 1;
                }
                e
            }
        };
        let value = &tag_text[value_start..value_end];
        // Where the attribute ends in the source (after the closing quote).
        let attr_end = if quote.is_some() {
            (value_end + 1).min(len)
        } else {
            value_end
        };

        // Record class/id for generic cosmetic filtering.
        if name_lower == "class" {
            class_id.push_class(value);
        } else if name_lower == "id" {
            class_id.push_id(value);
        }

        // `srcset` is a comma-separated `url descriptor` list.
        if name_lower == "srcset" {
            let rewritten = rewrite_srcset(value, document_url, proxy_base);
            if rewritten != value {
                *rewritten_count += 1;
                out.push_str(&tag_text[i..value_start]);
                out.push_str(&rewritten);
                i = value_end;
            }
            p = attr_end;
            continue;
        }

        let is_url_attr = URL_ATTRS
            .iter()
            .any(|(attr, tag, _)| *attr == name_lower && tag.map_or(true, |t| t == tag_name));

        if is_url_attr {
            if let Some(new_value) = rewrite_url_value(value, &name_lower, document_url, proxy_base) {
                *rewritten_count += 1;
                out.push_str(&tag_text[i..value_start]);
                out.push_str(&new_value);
                i = value_end;
            }
        }

        p = attr_end;
    }

    out.push_str(&tag_text[i.min(len)..]);
    out
}

/// Rewrite one URL value, resolving it against the document first.
fn rewrite_url_value(
    value: &str,
    attr: &str,
    document_url: &str,
    proxy_base: &str,
) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Decode HTML entities exactly where a browser does. Markup writes
    // `&amp;` because a bare `&` would be invalid HTML, and the parser turns it
    // back into `&` before the URL ever reaches the network. A rewriter that
    // skips this step produces a *different URL*: measured live, a Wikipedia
    // page asking for `/w/load.php?lang=en&amp;modules=startup&…` came back as
    // 196 bytes of error instead of 69 KB of JavaScript, because the origin was
    // asked for a single parameter literally named `amp;modules`.
    //
    // This is the classic "works on single-parameter URLs" bug: with one
    // parameter there is no `&` to mis-encode, which is why it survived this
    // long.
    let trimmed = decode_entities(trimmed);
    let trimmed = trimmed.as_str();

    // Skip non-navigational schemes and fragment-only links.
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with('#')
        || lower.starts_with("javascript:")
        || lower.starts_with("mailto:")
        || lower.starts_with("tel:")
        || lower.starts_with("data:")
        || lower.starts_with("blob:")
        || lower.starts_with("about:")
        || lower.starts_with("ws:")
        || lower.starts_with("wss:")
    {
        return None;
    }

    // A `<base href>` would hijack every relative URL on the page and send it
    // straight to the origin, bypassing the proxy. Drop it.
    if attr == "href" && lower.starts_with("http") && is_base_tag_context(document_url) {
        // handled by the caller's tag-name check; kept simple here
    }

    let absolute = resolve(document_url, trimmed)?;
    let proxied = proxied_path_of(&absolute)?;
    Some(format!("{proxy_base}{proxied}"))
}

/// Decode the character references an attribute value can contain.
///
/// Only the references that can appear in — and change the meaning of — a URL
/// are handled: the ampersand (the one that matters, since it separates query
/// parameters) plus the handful of others an HTML parser would resolve. A URL
/// with no `&` is returned borrowed-free by the fast path.
fn decode_entities(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'&' {
            let ch = value[i..].chars().next().unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let rest = &value[i..];
        // `&amp;` first, then numeric forms. `&amp;` must not be decoded twice:
        // a literal `&amp;` typed by an author arrives as `&amp;amp;`, and one
        // pass correctly yields `&amp;` — which is what the origin wants.
        let (decoded, consumed) = if let Some(tail) = rest.strip_prefix("&amp;") {
            let _ = tail;
            ("&".to_string(), 5)
        } else if let Some(tail) = rest.strip_prefix("&lt;") {
            let _ = tail;
            ("<".to_string(), 4)
        } else if let Some(tail) = rest.strip_prefix("&gt;") {
            let _ = tail;
            (">".to_string(), 4)
        } else if let Some(tail) = rest.strip_prefix("&quot;") {
            let _ = tail;
            ("\"".to_string(), 6)
        } else if rest.starts_with("&apos;") {
            ("'".to_string(), 6)
        } else if rest.starts_with("&nbsp;") {
            (" ".to_string(), 6)
        } else {
            match numeric_reference(rest) {
                Some((ch, len)) => (ch.to_string(), len),
                None => ("&".to_string(), 1),
            }
        };
        out.push_str(&decoded);
        i += consumed;
    }
    out
}

/// Decode `&#38;` / `&#x26;` style references, returning the character and how
/// many bytes it consumed.
fn numeric_reference(value: &str) -> Option<(char, usize)> {
    let body = value.strip_prefix("&#")?;
    let (digits, radix, prefix_len) = if let Some(hex) = body.strip_prefix(['x', 'X']) {
        let end = hex.find(';')?;
        (&hex[..end], 16u32, true)
    } else {
        let end = body.find(';')?;
        (&body[..end], 10u32, false)
    };
    if digits.is_empty() || digits.len() > 8 {
        return None;
    }
    let code = u32::from_str_radix(digits, radix).ok()?;
    let ch = char::from_u32(code)?;
    let consumed = 2 + usize::from(prefix_len) + digits.len() + 1;
    Some((ch, consumed))
}

/// Resolve a possibly-relative URL against a base, returning `None` when the
/// result is not an absolute http(s) URL we can proxy.
fn resolve(base: &str, reference: &str) -> Option<String> {
    let parsed_base = url::Url::parse(base).ok()?;
    let joined = parsed_base.join(reference).ok()?;
    match joined.scheme() {
        "http" | "https" => Some(joined.to_string()),
        _ => None,
    }
}

fn rewrite_srcset(value: &str, document_url: &str, proxy_base: &str) -> String {
    let mut parts = Vec::new();
    for candidate in value.split(',') {
        let candidate = candidate.trim();
        if candidate.is_empty() {
            continue;
        }
        let mut pieces = candidate.splitn(2, char::is_whitespace);
        let url_part = pieces.next().unwrap_or("");
        let descriptor = pieces.next().unwrap_or("");
        match rewrite_url_value(url_part, "srcset", document_url, proxy_base) {
            Some(new_url) if descriptor.is_empty() => parts.push(new_url),
            Some(new_url) => parts.push(format!("{new_url} {descriptor}")),
            None if descriptor.is_empty() => parts.push(url_part.to_string()),
            None => parts.push(format!("{url_part} {descriptor}")),
        }
    }
    parts.join(", ")
}

fn tag_name_of(tag_text: &str) -> String {
    let trimmed = tag_text.trim_start_matches('<');
    let trimmed = trimmed.trim_start_matches('/');
    let end = trimmed
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '-')
        .unwrap_or(trimmed.len());
    trimmed[..end].to_ascii_lowercase()
}

fn is_base_tag_context(_document_url: &str) -> bool {
    // Placeholder: `<base>` handling is done by removing the tag in
    // `strip_base_tag` during the outer scan. Marker kept for clarity.
    false
}

fn is_name_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b':'
}

/// Rewrite `url(...)` and `@import` references inside a stylesheet.
pub fn rewrite_css(css_url: &str, proxy_base: &str, css: &str) -> String {
    let mut out = String::with_capacity(css.len() + css.len() / 16);

    // `@import "x.css"` / `@import url(x.css)`.
    let lower = css.to_ascii_lowercase();
    let mut i = 0usize;
    while let Some(rel) = lower[i..].find("@import") {
        let at = i + rel;
        out.push_str(&css[i..at]);
        let after = &css[at + 7..];
        let (segment_len, replacement) = rewrite_import(after, css_url, proxy_base);
        out.push_str(&replacement);
        i = at + 7 + segment_len;
    }
    out.push_str(&css[i..]);

    rewrite_css_urls(&out, css_url, proxy_base)
}

fn rewrite_import(after: &str, css_url: &str, proxy_base: &str) -> (usize, String) {
    let trimmed_start = after.len() - after.trim_start().len();
    let body = &after[trimmed_start..];

    if let Some(rest) = body.strip_prefix("url(") {
        let end = rest.find(')').unwrap_or(rest.len());
        let raw = rest[..end].trim().trim_matches(['"', '\'']);
        let rewritten = rewrite_url_value(raw, "href", css_url, proxy_base)
            .unwrap_or_else(|| raw.to_string());
        return (
            trimmed_start + 4 + end + 1,
            format!(" {rewritten}"),
        );
    }

    if body.starts_with('"') || body.starts_with('\'') {
        let quote = body.as_bytes()[0];
        if let Some(end) = body[1..].find(quote as char) {
            let raw = &body[1..1 + end];
            let rewritten = rewrite_url_value(raw, "href", css_url, proxy_base)
                .unwrap_or_else(|| raw.to_string());
            return (
                trimmed_start + 1 + end + 1,
                format!(" \"{rewritten}\""),
            );
        }
    }

    (trimmed_start, String::new())
}

fn rewrite_css_urls(css: &str, css_url: &str, proxy_base: &str) -> String {
    let mut out = String::with_capacity(css.len() + css.len() / 16);
    let lower = css.to_ascii_lowercase();
    let mut i = 0usize;

    while let Some(rel) = lower[i..].find("url(") {
        let at = i + rel;
        let open = at + 4;
        out.push_str(&css[i..open]);

        let rest = &css[open..];
        let ws = rest.len() - rest.trim_start().len();
        let body = &rest[ws..];

        let (raw, consumed) = if body.starts_with('"') || body.starts_with('\'') {
            let quote = body.as_bytes()[0];
            match body[1..].find(quote as char) {
                Some(end) => (&body[1..1 + end], ws + end + 2),
                None => {
                    // Unterminated: bail out on this occurrence.
                    out.push_str(body);
                    i = open + ws + body.len();
                    continue;
                }
            }
        } else {
            match body.find(')') {
                Some(end) => (body[..end].trim(), ws + end),
                None => {
                    out.push_str(body);
                    i = open + ws + body.len();
                    continue;
                }
            }
        };

        match rewrite_url_value(raw, "src", css_url, proxy_base) {
            Some(new_url) => {
                out.push_str(if body.starts_with('"') || body.starts_with('\'') {
                    "\""
                } else {
                    ""
                });
                out.push_str(&new_url);
                if body.starts_with('"') || body.starts_with('\'') {
                    out.push('"');
                }
            }
            None => out.push_str(raw),
        }
        i = open + consumed;
        // `consumed` stopped before `)`, which the next loop iteration copies.
    }

    out.push_str(&css[i.min(css.len())..]);
    out
}

/// Remove `<base href>` so relative URLs keep resolving against the proxy.
pub fn strip_base_tag(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut i = 0usize;
    while let Some(rel) = find_ascii_ci(&html[i..], "<base") {
        let at = i + rel;
        out.push_str(&html[i..at]);
        match html[at..].find('>') {
            Some(end) => {
                i = at + end + 1;
            }
            None => {
                out.push_str(&html[at..]);
                return out;
            }
        }
    }
    out.push_str(&html[i..]);
    out
}

/// Case-insensitive substring search without allocating.
///
/// `str::to_ascii_lowercase().find(..)` copies the haystack; calling it inside
/// a scan over a document turns linear work quadratic. `needle` must already
/// be lowercase.
fn find_ascii_ci(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.len() > h.len() {
        return None;
    }
    let first = n[0];
    for start in 0..=(h.len() - n.len()) {
        if h[start].to_ascii_lowercase() != first {
            continue;
        }
        if h[start..start + n.len()]
            .iter()
            .zip(n)
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
        {
            return Some(start);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "http://127.0.0.1:8899";

    #[test]
    /// A query string in markup is entity-encoded, and the proxy must hand the
    /// origin what a browser would: `&amp;` → `&`.
    ///
    /// Measured live before this was fixed: Wikipedia's JS loader was asked for
    /// `?lang=en&amp;modules=…&amp;only=scripts`, answered 196 bytes of error
    /// instead of 69 KB of script, and the page's own scripts never loaded. One
    /// parameter hides the bug entirely, which is why it took a real navigation
    /// test to find.
    #[test]
    fn query_ampersands_are_decoded_once() {
        let html = r#"<script src="/w/load.php?lang=en&amp;modules=startup&amp;only=scripts"></script>"#;
        let out = rewrite_html(
            "https://en.wikipedia.org/wiki/Page",
            "http://127.0.0.1:8899",
            html,
        );
        assert!(
            out.html.contains("?lang=en&modules=startup&only=scripts"),
            "entities were not decoded: {}",
            out.html
        );
        assert!(
            !out.html.contains("&amp;modules"),
            "a literal entity reached the URL: {}",
            out.html
        );
    }

    /// Exactly one pass: an author-escaped `&amp;amp;` means a literal `&amp;`,
    /// and decoding twice would change what the origin is asked for.
    #[test]
    fn entities_are_not_decoded_twice() {
        let html = r#"<a href="/q?x=1&amp;amp;y=2">x</a>"#;
        let out = rewrite_html("https://site.example/", "http://127.0.0.1:8899", html);
        assert!(
            out.html.contains("x=1&amp;y=2"),
            "double-decoded: {}",
            out.html
        );
    }

    /// The numeric forms are what a generator emits when it cannot use `&amp;`.
    #[test]
    fn numeric_ampersand_references_are_decoded() {
        for (encoded, expected) in [
            ("/q?a=1&#38;b=2", "a=1&b=2"),
            ("/q?a=1&#x26;b=2", "a=1&b=2"),
        ] {
            let html = format!(r#"<a href="{encoded}">x</a>"#);
            let out = rewrite_html("https://site.example/", "http://127.0.0.1:8899", &html);
            assert!(out.html.contains(expected), "{encoded} → {}", out.html);
        }
    }

    fn rewrites_src_and_href() {
        let html = r#"<a href="/page"><img src="https://cdn.example.com/a.png"></a>"#;
        let out = rewrite_html("https://site.example.com/", BASE, html);
        assert!(
            out.html
                .contains(r#"src="http://127.0.0.1:8899/p/https/cdn.example.com/a.png""#),
            "got {}",
            out.html
        );
        assert!(
            out.html
                .contains(r#"href="http://127.0.0.1:8899/p/https/site.example.com/page""#),
            "got {}",
            out.html
        );
        assert_eq!(out.rewritten_count, 2);
    }

    #[test]
    fn leaves_non_url_attributes_alone() {
        let html = r#"<div class="ad banner" id="main" data-src="https://x.example/a.png"></div>"#;
        let out = rewrite_html("https://site.example.com/", BASE, html);
        assert_eq!(out.html, html);
        assert_eq!(out.class_id.classes, vec!["ad", "banner"]);
        assert_eq!(out.class_id.ids, vec!["main"]);
    }

    #[test]
    fn does_not_rewrite_inside_script_or_style() {
        let html = r#"<script>var u="https://tracker.example/x.js";</script><style>@import "a.css";</style>"#;
        let out = rewrite_html("https://site.example.com/", BASE, html);
        assert!(out.html.contains("https://tracker.example/x.js"));
        assert!(out.html.contains(r#"@import "a.css""#));
    }

    #[test]
    fn does_not_rewrite_inside_comments() {
        let html = r#"<!-- <img src="https://a.example/x.png"> -->"#;
        let out = rewrite_html("https://site.example.com/", BASE, html);
        assert_eq!(out.html, html);
    }

    #[test]
    fn skips_javascript_and_fragment_urls() {
        let html = r##"<a href="#top">x</a><a href="javascript:void(0)">y</a>"##;
        let out = rewrite_html("https://site.example.com/", BASE, html);
        assert_eq!(out.html, html);
        assert_eq!(out.rewritten_count, 0);
    }

    #[test]
    fn rewrites_srcset_candidates() {
        let html = r#"<img srcset="/a.png 1x, https://cdn.example/b.png 2x">"#;
        let out = rewrite_html("https://site.example.com/", BASE, html);
        assert!(
            out.html.contains("/p/https/site.example.com/a.png 1x"),
            "got {}",
            out.html
        );
        assert!(out.html.contains("/p/https/cdn.example/b.png 2x"), "got {}", out.html);
    }

    #[test]
    fn preserves_attribute_quoting_and_spacing() {
        let html = r#"<img  src = 'https://cdn.example.com/a.png'  alt="x" >"#;
        let out = rewrite_html("https://site.example.com/", BASE, html);
        assert!(
            out.html.contains("src = 'http://127.0.0.1:8899/p/https/cdn.example.com/a.png'"),
            "got {}",
            out.html
        );
        assert!(out.html.contains(r#"alt="x""#));
    }

    #[test]
    fn css_urls_are_rewritten() {
        let css = r#"body{background:url("/bg.png")} @import url(https://cdn.example/s.css);"#;
        let out = rewrite_css("https://site.example.com/style.css", BASE, css);
        assert!(out.contains("/p/https/site.example.com/bg.png"), "got {out}");
        assert!(out.contains("/p/https/cdn.example/s.css"), "got {out}");
    }

    #[test]
    fn css_url_outside_quotes_is_not_broken() {
        let css = r#"@font-face{src:url(font.woff2) format("woff2")}"#;
        let out = rewrite_css("https://site.example.com/a.css", BASE, css);
        assert!(out.contains("/p/https/site.example.com/font.woff2"), "got {out}");
        assert!(out.contains(r#"format("woff2")"#), "got {out}");
    }

    #[test]
    fn base_tag_is_stripped() {
        let html = r#"<head><base href="https://evil.example/"></head>"#;
        assert_eq!(strip_base_tag(html), "<head></head>");
    }

    #[test]
    fn unterminated_tag_does_not_panic() {
        let html = r#"<div class="a"#;
        let out = rewrite_html("https://site.example.com/", BASE, html);
        assert!(out.html.starts_with("<div"));
    }
}
