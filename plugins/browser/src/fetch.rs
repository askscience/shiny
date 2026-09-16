//! Fetching a filtered document as text, for the AI tools.
//!
//! The request goes through the same proxy the window renders through, which
//! matters for two reasons: the page the model reads is the page the user
//! would see (ads and trackers already removed), and the shell-out happens
//! through the one code path that is already tested.

use std::time::Duration;

use shiny_plugin_sdk::errors::AppError;

/// What a browser sends when it is asking for a document.
///
/// The proxy classifies a request by `Sec-Fetch-Dest`, then `Accept`, then the
/// URL's extension. A tool fetch has none of those, so without this header a
/// page read is `other`: it is not counted as a document and a mislabelled
/// response is not recognised as HTML.
pub const ACCEPT_HTML: &str =
    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";

/// User-Agent for the related-news fetch.
///
/// Deliberately a normal browser string rather than `browser/<version>`: search
/// engines serve an empty shell (or a challenge page) to unknown library
/// agents, and the news shelf is built by parsing their HTML.
pub const NEWS_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

/// Fetch `url` through the plugin's own filter proxy and return its text.
pub async fn text(url: &str) -> Result<String, AppError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(25))
        .user_agent(concat!("browser/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| AppError::Internal(format!("http client: {e}")))?;

    let response = client
        .get(url)
        // A real browser asks for a document. Without this the proxy has no
        // `Sec-Fetch-Dest` and no `Accept` to classify by, so a page read is
        // counted as `other` (no document metrics) and its HTML is not sniffed
        // when the origin mislabels the response.
        .header(reqwest::header::ACCEPT, ACCEPT_HTML)
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

    Ok(html_to_text(&body))
}

/// Reduce an HTML document to readable text.
///
/// This is deliberately not a parser. It drops the elements whose contents are
/// never readable (`script`, `style`, `noscript`, `svg`, `head`) and then
/// strips the remaining tags, which is enough for a model to answer questions
/// about a page and stays honest about being an approximation.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 3);
    let lower = html.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0usize;
    let mut skip_until: Option<usize> = None;

    while i < bytes.len() {
        if let Some(end) = skip_until {
            if i < end {
                i += 1;
                continue;
            }
            skip_until = None;
        }

        if bytes[i] != b'<' {
            // Copy one UTF-8 character at a time from the *original* string.
            let ch = html[i..].chars().next().unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }

        // A tag: find its end.
        let Some(rel_end) = html[i..].find('>') else {
            break;
        };
        let end = i + rel_end + 1;
        let tag = &lower[i..end];

        // Block-level boundaries become newlines so words do not run together.
        if tag.starts_with("<br")
            || tag.starts_with("</p")
            || tag.starts_with("</div")
            || tag.starts_with("</li")
            || tag.starts_with("</h1")
            || tag.starts_with("</h2")
            || tag.starts_with("</h3")
            || tag.starts_with("</tr")
        {
            out.push('\n');
        }

        // Skip the bodies of elements whose text is not content.
        for name in ["script", "style", "noscript", "svg", "head", "template"] {
            if tag.starts_with(&format!("<{name}")) && !tag.starts_with(&format!("<{name}/")) {
                let close = format!("</{name}");
                if let Some(close_rel) = lower[end..].find(&close) {
                    skip_until = Some(end + close_rel);
                } else {
                    // Unclosed: drop the rest of the document.
                    skip_until = Some(bytes.len());
                }
                break;
            }
        }

        i = end;
    }

    collapse_whitespace(&out)
}

/// Collapse runs of whitespace, keeping paragraph breaks.
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_lines = 0usize;
    for line in text.lines() {
        let trimmed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if trimmed.is_empty() {
            blank_lines += 1;
            if blank_lines > 1 {
                continue;
            }
            out.push('\n');
        } else {
            blank_lines = 0;
            out.push_str(&trimmed);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tags_and_scripts() {
        let html = r#"<html><head><title>T</title><style>body{}</style></head>
<body><h1>Hello</h1><script>var x = "<b>not text</b>";</script><p>World &amp; co</p></body></html>"#;
        let text = html_to_text(html);
        assert!(text.contains("Hello"), "got {text:?}");
        assert!(text.contains("World &amp; co"), "got {text:?}");
        assert!(!text.contains("var x"), "script body leaked: {text:?}");
        assert!(!text.contains("body{}"), "style body leaked: {text:?}");
        assert!(!text.contains("<h1>"), "tags not stripped: {text:?}");
    }

    #[test]
    fn keeps_paragraph_breaks() {
        let text = html_to_text("<p>one</p><p>two</p>");
        assert_eq!(text, "one\ntwo");
    }

    #[test]
    fn handles_unicode_without_corruption() {
        let text = html_to_text("<p>città — perché 日本語</p>");
        assert!(text.contains("città"), "got {text:?}");
        assert!(text.contains("日本語"), "got {text:?}");
    }

    #[test]
    fn unclosed_document_does_not_panic() {
        let text = html_to_text("<html><body><p>hello");
        assert!(text.contains("hello"));
    }
}
