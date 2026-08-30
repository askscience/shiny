//! Minimal YouTube search scraper — parses the public search results page
//! (`ytInitialData` JSON embedded in the HTML). No API key required.

use shiny_plugin_sdk::errors::AppError;
use serde_json::Value;

#[derive(Debug, Clone, serde::Serialize)]
pub struct VideoResult {
    pub video_id: String,
    pub title: String,
    pub channel: String,
    pub duration: String,
    pub thumbnail: Option<String>,
}

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/126.0 Safari/537.36";

pub async fn search(query: &str, limit: usize) -> Result<Vec<VideoResult>, AppError> {
    let mut url = url::Url::parse("https://www.youtube.com/results")
        .map_err(|e| AppError::Internal(format!("bad url: {e}")))?;
    url.query_pairs_mut().append_pair("search_query", query);

    let client = reqwest::Client::builder()
        .user_agent(UA)
        .build()
        .map_err(|e| AppError::Internal(format!("http client: {e}")))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::Http(e))?;
    if !resp.status().is_success() {
        return Err(AppError::Internal(format!("YouTube returned {}", resp.status())));
    }
    let html = resp
        .text()
        .await
        .map_err(|e| AppError::Http(e))?;

    let json = extract_initial_data(&html).ok_or_else(|| {
        AppError::NotFound("YouTube returned no search data — try a simpler query".into())
    })?;

    let mut out = Vec::new();
    walk_for_videos(&json, &mut out);
    out.dedup_by(|a, b| a.video_id == b.video_id);
    out.truncate(limit);

    if out.is_empty() {
        return Err(AppError::NotFound("No videos matched — try a different query".into()));
    }
    Ok(out)
}

/// Find `ytInitialData = {…};` in the HTML and return the JSON object.
fn extract_initial_data(html: &str) -> Option<Value> {
    let marker = "ytInitialData";
    let start = html.find(marker)? + marker.len();
    let rest = &html[start..];
    let brace = rest.find('{')?;
    let slice = &rest[brace..];
    let end = find_json_end(slice)?;
    serde_json::from_str::<Value>(&slice[..=end]).ok()
}

fn find_json_end(s: &str) -> Option<usize> {
    let mut depth = 0;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Walk the tree for `videoRenderer` nodes (wherever YouTube nests them).
fn walk_for_videos(v: &Value, out: &mut Vec<VideoResult>) {
    match v {
        Value::Array(items) => {
            for item in items {
                walk_for_videos(item, out);
            }
        }
        Value::Object(map) => {
            if let Some(Value::Object(vr)) = map.get("videoRenderer") {
                if let Some(r) = parse_video_renderer(vr) {
                    out.push(r);
                }
            }
            for (key, val) in map {
                if key == "videoRenderer" {
                    continue;
                }
                walk_for_videos(val, out);
            }
        }
        _ => {}
    }
}

fn runs_text(v: &Value) -> String {
    v.get("runs")
        .and_then(|r| r.as_array())
        .map(|runs| {
            runs.iter()
                .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn parse_video_renderer(vr: &serde_json::Map<String, Value>) -> Option<VideoResult> {
    let video_id = vr.get("videoId")?.as_str()?.to_string();
    let title = runs_text(vr.get("title")?);
    if title.is_empty() {
        return None;
    }
    let channel = runs_text(vr.get("ownerText").unwrap_or(&Value::Null));
    let duration = vr
        .get("lengthText")
        .and_then(|l| l.get("simpleText"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    let thumbnail = vr
        .get("thumbnail")
        .and_then(|t| t.get("thumbnails"))
        .and_then(|t| t.as_array())
        .and_then(|a| a.last())
        .and_then(|t| t.get("url"))
        .and_then(|u| u.as_str())
        .map(String::from);

    Some(VideoResult {
        video_id,
        title,
        channel,
        duration,
        thumbnail,
    })
}

