//! Core AI services exposed to plugins.
//!
//! These clients live in the SDK so both the binary and plugins link against
//! one canonical implementation; the binary constructs them once at startup
//! and hands clones (via `PluginCtx`) to each plugin's `register()`.

use std::sync::Arc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use crate::errors::AppError;
use crate::manifest::Manifest;

// ---------- PluginCtx --------------------------------------------------------

/// Snapshot of the live core config that's useful to plugins. Lives in the
/// SDK so plugin authors don't have to depend on the binary crate. Plugins
/// only read this — they don't reconfigure the running core.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSnapshot {
    pub server_host: String,
    pub server_port: u16,
    pub database_url: String,
    pub ollama_url: String,
    pub ollama_model: String,
    pub supertonic_url: String,
    pub supertonic_voice: String,
    pub web_dir: String,
    pub vosk_models_dir: String,
    pub auto_start_supertonic: bool,
    pub log_level: String,
    pub plugins_dir: String,
    pub admin_token: Option<String>,
}

/// The handle a plugin receives at `on_load` and at every tool invocation.
/// Cheap to clone (every field is an `Arc` or shared-handle internally).
///
/// NOTE: nothing that binds to a runtime is shared across the dlopen
/// boundary. A `SqlitePool` carries live libsqlite3 objects (each side
/// links its own libsqlite3 — cross-free segfaults), and a pre-built
/// reqwest client captures the host's reactor (cross-reactor panics).
/// Every plugin therefore opens its OWN pool and HTTP clients, lazily,
/// on its own runtime — via the async accessors below.
#[derive(Clone)]
pub struct PluginCtx {
    pool: Arc<tokio::sync::OnceCell<SqlitePool>>,
    ollama: Arc<tokio::sync::OnceCell<OllamaClient>>,
    search: Arc<tokio::sync::OnceCell<SearchService>>,
    supertonic: Arc<tokio::sync::OnceCell<SupertonicClient>>,
    pub config: ConfigSnapshot,
    pub manifest: Manifest,
}

impl PluginCtx {
    pub fn new(config: ConfigSnapshot, manifest: Manifest) -> Arc<Self> {
        Arc::new(Self {
            pool: Arc::new(tokio::sync::OnceCell::new()),
            ollama: Arc::new(tokio::sync::OnceCell::new()),
            search: Arc::new(tokio::sync::OnceCell::new()),
            supertonic: Arc::new(tokio::sync::OnceCell::new()),
            config,
            manifest,
        })
    }

    /// Clone with a different manifest (the host loader uses this to build
    /// the per-plugin ctx). All lazy cells start fresh, so each plugin
    /// opens its own connections on first use.
    pub fn with_manifest(&self, manifest: Manifest) -> Arc<Self> {
        Self::new(self.config.clone(), manifest)
    }

    /// The plugin's own SQLite pool — opened lazily on first use, inside the
    /// plugin's process and runtime. All DB work in plugin code must go
    /// through this accessor, never through a pool passed from the host.
    pub async fn pool(&self) -> &SqlitePool {
        self.pool
            .get_or_init(|| async {
                SqlitePool::connect(&self.config.database_url)
                    .await
                    .expect("plugin failed to open SQLite database")
            })
            .await
    }

    /// Plugin-owned Ollama client, built from the config snapshot.
    pub async fn ollama(&self) -> &OllamaClient {
        self.ollama
            .get_or_init(|| async {
                OllamaClient::new(self.config.ollama_url.clone(), self.config.ollama_model.clone())
            })
            .await
    }

    /// Plugin-owned web search client.
    pub async fn search(&self) -> &SearchService {
        self.search
            .get_or_init(|| async { SearchService::new() })
            .await
    }

    /// Plugin-owned Supertonic TTS client, built from the config snapshot.
    pub async fn supertonic(&self) -> &SupertonicClient {
        self.supertonic
            .get_or_init(|| async {
                SupertonicClient::new(
                    self.config.supertonic_url.clone(),
                    self.config.supertonic_voice.clone(),
                )
            })
            .await
    }
}

// ---------- Ollama -----------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    message: ChatMessageResponse,
}

#[derive(Debug, Deserialize)]
struct ChatMessageResponse {
    content: String,
}

#[derive(Debug, Serialize)]
struct GenerateRequest {
    model: String,
    prompt: String,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct GenerateResponse {
    response: String,
}

#[derive(Clone)]
pub struct OllamaClient {
    client: reqwest::Client,
    base_url: String,
    model: String,
}

impl OllamaClient {
    pub fn new(base_url: String, model: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
            model,
        }
    }

    pub async fn chat(
        &self,
        messages: Vec<(String, String)>,
        model: Option<&str>,
    ) -> Result<String, AppError> {
        let msgs: Vec<ChatMessage> = messages
            .into_iter()
            .map(|(role, content)| ChatMessage { role, content })
            .collect();

        let body = ChatRequest {
            model: self.resolve_model(model).to_string(),
            messages: msgs,
            stream: false,
        };

        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| self.map_request_error("chat", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let detail = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(format!(
                "Ollama chat failed ({}){}",
                status,
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {}", detail.trim())
                }
            )));
        }

        let data: ChatResponse = resp.json().await.map_err(|e| {
            AppError::Internal(format!("Failed to parse Ollama response: {}", e))
        })?;

        Ok(data.message.content)
    }

    pub async fn generate(
        &self,
        prompt: &str,
        system: Option<&str>,
        model: Option<&str>,
    ) -> Result<String, AppError> {
        let full_prompt = match system {
            Some(sys) => format!("{}\n\n{}", sys, prompt),
            None => prompt.to_string(),
        };

        let body = GenerateRequest {
            model: self.resolve_model(model).to_string(),
            prompt: full_prompt,
            stream: false,
        };

        let resp = self
            .client
            .post(format!("{}/api/generate", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| self.map_request_error("generate", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let detail = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(format!(
                "Ollama generate failed ({}){}",
                status,
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {}", detail.trim())
                }
            )));
        }

        let data: GenerateResponse = resp.json().await.map_err(|e| {
            AppError::Internal(format!("Failed to parse Ollama generate response: {}", e))
        })?;

        Ok(data.response)
    }

    fn map_request_error(&self, op: &str, err: reqwest::Error) -> AppError {
        if err.is_connect() {
            AppError::Internal(format!(
                "AI unavailable — cannot reach Ollama at {} for {}. Start Ollama or set OLLAMA_URL.",
                self.base_url, op
            ))
        } else if err.is_timeout() {
            AppError::Internal(format!(
                "AI request timed out during {}. Try again or use a smaller model.",
                op
            ))
        } else {
            AppError::Http(err)
        }
    }

    pub fn default_model(&self) -> &str {
        &self.model
    }

    fn resolve_model<'a>(&'a self, override_model: Option<&'a str>) -> &'a str {
        override_model
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .unwrap_or(&self.model)
    }

    pub async fn list_models(&self) -> Result<Vec<String>, AppError> {
        let resp = self
            .client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
            .map_err(|e| self.map_request_error("list models", e))?;

        if !resp.status().is_success() {
            return Err(AppError::Internal(format!(
                "Ollama list models failed ({})",
                resp.status()
            )));
        }

        #[derive(Deserialize)]
        struct TagsResponse {
            models: Vec<TagModel>,
        }

        #[derive(Deserialize)]
        struct TagModel {
            name: String,
        }

        let data: TagsResponse = resp.json().await.map_err(|e| {
            AppError::Internal(format!("Failed to parse Ollama models: {}", e))
        })?;

        let mut names: Vec<String> = data.models.into_iter().map(|m| m.name).collect();
        names.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        Ok(names)
    }

    pub async fn is_available(&self) -> bool {
        self.client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

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

    pub fn client(&self) -> &reqwest::Client {
        &self.client
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

fn is_junk_html_row(r: &SearchResult) -> bool {
    is_junk_search_row(r)
}

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

// ---------- Supertonic TTS ---------------------------------------------------

#[derive(Clone)]
pub struct SupertonicClient {
    client: reqwest::Client,
    base_url: String,
    default_voice: String,
}

impl SupertonicClient {
    pub fn new(base_url: String, default_voice: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
            default_voice,
        }
    }

    pub async fn is_available(&self) -> bool {
        self.client
            .get(format!("{}/docs", self.base_url))
            .send()
            .await
            .map(|r| r.status().is_success() || r.status().as_u16() == 404)
            .unwrap_or(false)
            || self
                .client
                .get(&self.base_url)
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false)
    }

    pub async fn synthesize(
        &self,
        text: &str,
        lang: &str,
        voice: Option<&str>,
    ) -> Result<Vec<u8>, AppError> {
        let voice = voice.unwrap_or(&self.default_voice);
        let body = serde_json::json!({
            "model": "supertonic-3",
            "input": text,
            "voice": voice,
            "response_format": "wav",
            "lang": lang,
        });

        let resp = self
            .client
            .post(format!("{}/v1/audio/speech", self.base_url))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(format!(
                "Supertonic TTS error {}: {}",
                status, err
            )));
        }

        let bytes = resp.bytes().await?.to_vec();
        Ok(bytes)
    }
}