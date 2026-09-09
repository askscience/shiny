//! Provider-agnostic AI client.
//!
//! The assistant can be backed either by a local Ollama server (the default)
//! or by a per-user OpenAI-compatible endpoint configured in Assistant
//! settings. Every chat/generate call site goes through `AiClient` so the
//! choice of provider is transparent to the rest of the core.

use std::collections::HashMap;

use sqlx::SqlitePool;

use crate::errors::AppError;
use crate::services::ollama::OllamaClient;
use crate::services::openai::OpenAiClient;

/// Provider preference keys (stored in `user_preferences`).
pub const PREF_PROVIDER: &str = "ai.provider";
pub const PREF_OPENAI_BASE_URL: &str = "ai.openai_base_url";
pub const PREF_OPENAI_API_KEY: &str = "ai.openai_api_key";
pub const PREF_OPENAI_MODEL: &str = "ai.openai_model";
pub const PREF_OLLAMA_MODEL: &str = "ai.ollama_model";

#[derive(Clone)]
pub enum AiClient {
    Ollama(OllamaClient),
    OpenAi(OpenAiClient),
}

impl AiClient {
    pub async fn chat(
        &self,
        messages: Vec<(String, String)>,
        model: Option<&str>,
    ) -> Result<String, AppError> {
        match self {
            AiClient::Ollama(c) => c.chat(messages, model).await,
            AiClient::OpenAi(c) => c.chat(messages, model).await,
        }
    }

    pub async fn generate(
        &self,
        prompt: &str,
        system: Option<&str>,
        model: Option<&str>,
    ) -> Result<String, AppError> {
        match self {
            AiClient::Ollama(c) => c.generate(prompt, system, model).await,
            AiClient::OpenAi(c) => c.generate(prompt, system, model).await,
        }
    }

    pub fn default_model(&self) -> &str {
        match self {
            AiClient::Ollama(c) => c.default_model(),
            AiClient::OpenAi(c) => c.default_model(),
        }
    }

    pub async fn list_models(&self) -> Result<Vec<String>, AppError> {
        match self {
            AiClient::Ollama(c) => c.list_models().await,
            AiClient::OpenAi(c) => c.list_models().await,
        }
    }

    pub async fn is_available(&self) -> bool {
        match self {
            AiClient::Ollama(c) => c.is_available().await,
            AiClient::OpenAi(c) => c.is_available().await,
        }
    }

    pub fn is_openai(&self) -> bool {
        matches!(self, AiClient::OpenAi(_))
    }
}

/// A resolved AI target for one request: the client plus the per-user model
/// override. The override is only meaningful for Ollama (whose default model is
/// server-wide); the OpenAI client bakes its configured model in directly.
#[derive(Clone)]
pub struct ResolvedAi {
    pub client: AiClient,
    pub model: Option<String>,
}

/// Read the current user's assistant provider preferences and build the right
/// client. Defaults to the shared Ollama client when no OpenAI endpoint is
/// configured (or its base URL/model are blank).
pub async fn resolve_for_user(
    pool: &SqlitePool,
    ollama: &OllamaClient,
    traveler_id: &str,
) -> ResolvedAi {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM user_preferences WHERE user_id = ?1 AND key IN (?, ?, ?, ?, ?)",
    )
    .bind(traveler_id)
    .bind(PREF_PROVIDER)
    .bind(PREF_OPENAI_BASE_URL)
    .bind(PREF_OPENAI_API_KEY)
    .bind(PREF_OPENAI_MODEL)
    .bind(PREF_OLLAMA_MODEL)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let prefs: HashMap<&str, String> = rows
        .iter()
        .map(|(k, v)| (k.as_str(), v.clone()))
        .collect();

    if prefs.get(PREF_PROVIDER).map(|s| s.as_str()) == Some("openai") {
        let base = prefs
            .get(PREF_OPENAI_BASE_URL)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        let model = prefs
            .get(PREF_OPENAI_MODEL)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        if let (Some(base), Some(model)) = (base, model) {
            let key = prefs
                .get(PREF_OPENAI_API_KEY)
                .cloned()
                .unwrap_or_default();
            return ResolvedAi {
                client: AiClient::OpenAi(OpenAiClient::new(
                    base.to_string(),
                    key,
                    model.to_string(),
                )),
                model: None,
            };
        }
    }

    let model = prefs
        .get(PREF_OLLAMA_MODEL)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    ResolvedAi {
        client: AiClient::Ollama(ollama.clone()),
        model,
    }
}
