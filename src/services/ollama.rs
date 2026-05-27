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
    message: ChatMessageResponse,
    done: bool,
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
    done: bool,
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

    pub async fn chat(&self, messages: Vec<(String, String)>) -> Result<String, AppError> {
        let msgs: Vec<ChatMessage> = messages
            .into_iter()
            .map(|(role, content)| ChatMessage { role, content })
            .collect();

        let body = ChatRequest {
            model: self.model.clone(),
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

    pub async fn generate(&self, prompt: &str, system: Option<&str>) -> Result<String, AppError> {
        let full_prompt = match system {
            Some(sys) => format!("{}\n\n{}", sys, prompt),
            None => prompt.to_string(),
        };

        let body = GenerateRequest {
            model: self.model.clone(),
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

    pub async fn is_available(&self) -> bool {
        self.client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}
