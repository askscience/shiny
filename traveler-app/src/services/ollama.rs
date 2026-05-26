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
            .await?;

        if !resp.status().is_success() {
            return Err(AppError::Internal(format!(
                "Ollama API error: {}",
                resp.status()
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
            .await?;

        if !resp.status().is_success() {
            return Err(AppError::Internal(format!(
                "Ollama generate error: {}",
                resp.status()
            )));
        }

        let data: GenerateResponse = resp.json().await.map_err(|e| {
            AppError::Internal(format!("Failed to parse Ollama generate response: {}", e))
        })?;

        Ok(data.response)
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
