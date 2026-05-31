use serde::{Deserialize, Serialize};
use crate::errors::AppError;

#[derive(Debug, Clone)]
pub struct SearchService {
    client: reqwest::Client,
}

impl SearchService {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("Shiny/0.1 (shiny)")
                .build()
                .unwrap(),
        }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<SearchResult>, AppError> {
        let url = format!(
            "https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1",
            urlencoding(query)
        );

        let resp = self.client.get(&url).send().await?;

        if !resp.status().is_success() {
            return Err(AppError::Internal(format!(
                "Search API error: {}",
                resp.status()
            )));
        }

        let data: DuckDuckGoResponse = resp.json().await.map_err(|e| {
            AppError::Internal(format!("Failed to parse search response: {}", e))
        })?;

        let mut results = Vec::new();

        if !data.AbstractText.is_empty() && row_within_bounds(&data.AbstractSource, &data.AbstractText) {
            results.push(SearchResult {
                title: clean_text(&data.AbstractSource),
                snippet: clean_text(&data.AbstractText),
            });
        }

        for topic in &data.RelatedTopics {
            if let Some(text) = &topic.Text {
                let title_part = text.split(" - ").next().unwrap_or("");
                if row_within_bounds(title_part, text) {
                    results.push(SearchResult {
                        title: clean_text(title_part),
                        snippet: clean_text(text),
                    });
                }
            }
            if let Some(subtopics) = &topic.Topics {
                for sub in subtopics {
                    if let Some(text) = &sub.Text {
                        let title_part = text.split(" - ").next().unwrap_or("");
                        if row_within_bounds(title_part, text) {
                            results.push(SearchResult {
                                title: clean_text(title_part),
                                snippet: clean_text(text),
                            });
                        }
                    }
                }
            }
        }

        if results.is_empty() {
            results.push(SearchResult {
                title: "No results".into(),
                snippet: format!("No information found for: {}", query),
            });
        }

        Ok(results)
    }

    /// HTML results page — richer snippets than the Instant Answer API.
    pub async fn search_html(&self, query: &str, max: usize) -> Result<Vec<SearchResult>, AppError> {
        let url = format!(
            "https://html.duckduckgo.com/html/?q={}",
            urlencoding(query)
        );

        let resp = self
            .client
            .get(&url)
            .header("Accept", "text/html")
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(AppError::Internal(format!(
                "HTML search error: {}",
                resp.status()
            )));
        }

        let html = resp.text().await.map_err(AppError::Http)?;
        Ok(parse_ddg_html(&html, max))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub title: String,
    pub snippet: String,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct DuckDuckGoResponse {
    AbstractText: String,
    AbstractSource: String,
    RelatedTopics: Vec<RelatedTopic>,
}

#[derive(Debug, Deserialize)]
struct RelatedTopic {
    Text: Option<String>,
    Topics: Option<Vec<RelatedTopic>>,
}

fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ' ' => "+".to_string(),
            _ => format!("%{:02X}", c as u8),
        })
        .collect()
}

fn decode_html_entities(s: &str) -> String {
    let mut out = s
        .replace("\\&#x27;", "'")
        .replace("\\&#39;", "'")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ");

    out = decode_numeric_entities(&out);
    collapse_ws(&out)
}

pub fn clean_text(s: &str) -> String {
    let stripped = strip_html_tags(s);
    decode_html_entities(&stripped)
}

fn decode_numeric_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '&' && i + 3 < chars.len() && chars[i + 1] == '#' {
            if chars[i + 2] == 'x' || chars[i + 2] == 'X' {
                if let Some(end) = (i + 3..chars.len()).find(|&j| chars[j] == ';') {
                    let hex: String = chars[i + 3..end].iter().collect();
                    if let Ok(code) = u32::from_str_radix(&hex, 16) {
                        if let Some(ch) = char::from_u32(code) {
                            out.push(ch);
                            i = end + 1;
                            continue;
                        }
                    }
                }
            } else if let Some(end) = (i + 2..chars.len()).find(|&j| chars[j] == ';') {
                let num: String = chars[i + 2..end].iter().collect();
                if let Ok(code) = num.parse::<u32>() {
                    if let Some(ch) = char::from_u32(code) {
                        out.push(ch);
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode_html_entities(out.trim())
}

fn parse_ddg_html(html: &str, max: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut rest = html;

    while results.len() < max {
        let Some(marker) = rest.find("class=\"result__a\"") else {
            break;
        };
        rest = &rest[marker..];
        let Some(gt) = rest.find('>') else { break };
        let after_gt = &rest[gt + 1..];
        let Some(end) = after_gt.find("</a>") else { break };
        let title = clean_text(&after_gt[..end]);

        rest = &after_gt[end..];
        let snippet = if let Some(sn_marker) = rest.find("class=\"result__snippet\"") {
            let sn = &rest[sn_marker..];
            if let Some(gt) = sn.find('>') {
                let sn_text = &sn[gt + 1..];
                if let Some(end) = sn_text.find("</a>") {
                    clean_text(&sn_text[..end])
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        if title.is_empty() {
            continue;
        }
        if !row_within_bounds(&title, &snippet) {
            continue;
        }
        let row = SearchResult {
            title,
            snippet,
        };
        if is_junk_html_row(&row) || is_aggregator_row(&row) {
            continue;
        }
        results.push(row);
    }

    results
}

fn row_within_bounds(title: &str, snippet: &str) -> bool {
    title.len() + snippet.len() <= 4000
}

/// Split long text into sequential chunks for step-by-step model calls.
pub fn text_chunks(text: &str, max_chars: usize) -> Vec<String> {
    let t = text.trim();
    if t.is_empty() {
        return vec![];
    }
    if t.chars().count() <= max_chars {
        return vec![t.to_string()];
    }

    let mut chunks = Vec::new();
    let mut start = 0usize;
    let char_indices: Vec<(usize, char)> = t.char_indices().collect();
    let total = char_indices.len();

    while start < total {
        let end = (start + max_chars).min(total);
        let byte_start = char_indices[start].0;
        let byte_end = if end >= total {
            t.len()
        } else {
            char_indices[end].0
        };
        let piece = t[byte_start..byte_end].trim();
        if !piece.is_empty() {
            chunks.push(piece.to_string());
        }
        start = end;
    }

    chunks
}

fn is_junk_html_row(r: &SearchResult) -> bool {
    is_junk_search_row(r)
}

pub fn is_junk_search_row(r: &SearchResult) -> bool {
    let title = r.title.trim().to_lowercase();
    let snippet = r.snippet.trim().to_lowercase();
    title.contains("no result")
        || snippet.contains("no information found")
        || title.is_empty()
        || title == "duckduckgo"
}

pub fn is_aggregator_row(r: &SearchResult) -> bool {
    let blob = format!("{} {}", r.title, r.snippet).to_lowercase();
    const MARKERS: &[&str] = &[
        "buy ticket",
        "book ticket",
        "book now",
        "book your",
        "vakantie",
        "boek ",
        "tickets for",
        "ticketmaster",
        "viator",
        "getyourguide",
        "tripadvisor",
        "shows on in",
        "concert tickets",
    ];
    MARKERS.iter().any(|m| blob.contains(m))
}
