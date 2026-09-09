//! OpenAI-compatible chat client.
//!
//! Speaks the standard `/v1/chat/completions` and `/v1/models` protocol, so it
//! works against OpenAI, OpenRouter, Groq, LM Studio, vLLM, Ollama's OpenAI
//! compatibility layer, and any other server exposing that surface.

use serde::{Deserialize, Serialize};

use crate::errors::AppError;

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
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessageResponse,
}

#[derive(Debug, Deserialize)]
struct ChatMessageResponse {
    content: String,
}

#[derive(Clone)]
pub struct OpenAiClient {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

impl OpenAiClient {
    pub fn new(base_url: String, api_key: String, model: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: normalize_base_url(&base_url),
            api_key,
            model,
        }
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn request(&self, method: &str, path: &str) -> reqwest::RequestBuilder {
        let url = self.endpoint(path);
        let builder = match method {
            "GET" => self.client.get(&url),
            "POST" => self.client.post(&url),
            _ => self.client.get(&url),
        };
        if self.api_key.trim().is_empty() {
            builder
        } else {
            builder.header("Authorization", format!("Bearer {}", self.api_key.trim()))
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
            .request("POST", "/chat/completions")
            .json(&body)
            .send()
            .await
            .map_err(|e| self.map_request_error("chat", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let detail = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(format!(
                "OpenAI-compatible chat failed ({}){}",
                status,
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {}", detail.trim())
                }
            )));
        }

        let data: ChatResponse = resp.json().await.map_err(|e| {
            AppError::Internal(format!(
                "Failed to parse OpenAI-compatible response: {}",
                e
            ))
        })?;

        let content = data
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default();
        Ok(content)
    }

    pub async fn generate(
        &self,
        prompt: &str,
        system: Option<&str>,
        model: Option<&str>,
    ) -> Result<String, AppError> {
        let mut messages = Vec::new();
        if let Some(sys) = system {
            messages.push(("system".to_string(), sys.to_string()));
        }
        messages.push(("user".to_string(), prompt.to_string()));
        self.chat(messages, model).await
    }

    fn map_request_error(&self, op: &str, err: reqwest::Error) -> AppError {
        if err.is_connect() {
            AppError::Internal(format!(
                "AI unavailable — cannot reach OpenAI-compatible endpoint at {} for {}.",
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
            .request("GET", "/models")
            .send()
            .await
            .map_err(|e| self.map_request_error("list models", e))?;

        if !resp.status().is_success() {
            return Err(AppError::Internal(format!(
                "OpenAI-compatible list models failed ({})",
                resp.status()
            )));
        }

        #[derive(Deserialize)]
        struct ModelsResponse {
            data: Vec<ModelData>,
        }

        #[derive(Deserialize)]
        struct ModelData {
            id: String,
        }

        let data: ModelsResponse = resp.json().await.map_err(|e| {
            AppError::Internal(format!(
                "Failed to parse OpenAI-compatible models: {}",
                e
            ))
        })?;

        let mut names: Vec<String> = data.data.into_iter().map(|m| m.id).collect();
        names.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        Ok(names)
    }

    pub async fn is_available(&self) -> bool {
        self.request("GET", "/models")
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

/// Accept both "https://api.openai.com" and "https://api.openai.com/v1".
fn normalize_base_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{}/v1", trimmed)
    }
}
