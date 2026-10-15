//! Provider-aware AI model listing. Returns the models for whichever provider
//! the current user has selected in Assistant settings (Ollama or an
//! OpenAI-compatible endpoint), so the settings page can populate the right
//! model picker.

use axum::extract::{Extension, State};
use axum::Json;
use serde::Serialize;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;

#[derive(Serialize)]
pub struct AiModelsResponse {
    pub success: bool,
    pub data: AiModelsData,
}

#[derive(Serialize)]
pub struct AiModelsData {
    pub provider: String,
    pub models: Vec<String>,
    pub default: String,
    pub available: bool,
}

#[derive(Serialize)]
pub struct AiThinkingResponse {
    pub success: bool,
    pub data: AiThinkingData,
}

#[derive(Serialize)]
pub struct AiThinkingData {
    pub provider: String,
    /// The model this answer refers to: the user's choice when set, else the
    /// provider default.
    pub model: String,
    /// True when the model can emit a separate reasoning trace (`think`).
    pub supported: bool,
}

pub async fn list_models(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
) -> Result<Json<AiModelsResponse>, AppError> {
    let ai = state.resolve_ai(&traveler.id).await;
    let provider = if ai.client.is_openai() {
        "openai"
    } else {
        "ollama"
    };
    let default = ai.client.default_model().to_string();
    let available = ai.client.is_available().await;
    let models = if available {
        ai.client.list_models().await.unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok(Json(AiModelsResponse {
        success: true,
        data: AiModelsData {
            provider: provider.to_string(),
            models,
            default,
            available,
        },
    }))
}

/// Whether the resolved provider + model can emit a reasoning trace. Gates the
/// brain toggle in the chat and the voice-bar menu; the agent re-checks the
/// same capability before every turn, so the UI is never the only guard.
pub async fn thinking(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
) -> Result<Json<AiThinkingResponse>, AppError> {
    let ai = state.resolve_ai(&traveler.id).await;
    let provider = if ai.client.is_openai() {
        "openai"
    } else {
        "ollama"
    };
    let model = ai
        .model
        .clone()
        .unwrap_or_else(|| ai.client.default_model().to_string());
    let supported = ai.client.thinking_support(ai.model.as_deref()).await.supported;

    Ok(Json(AiThinkingResponse {
        success: true,
        data: AiThinkingData {
            provider: provider.to_string(),
            model,
            supported,
        },
    }))
}
