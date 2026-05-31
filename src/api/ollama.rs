use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::api::AppState;
use crate::errors::AppError;

#[derive(Serialize)]
pub struct OllamaModelsResponse {
    pub success: bool,
    pub data: OllamaModelsData,
}

#[derive(Serialize)]
pub struct OllamaModelsData {
    pub models: Vec<String>,
    pub default: String,
    pub available: bool,
}

pub async fn list_models(State(state): State<AppState>) -> Result<Json<OllamaModelsResponse>, AppError> {
    let default = state.ollama.default_model().to_string();
    let available = state.ollama.is_available().await;
    let models = if available {
        state.ollama.list_models().await.unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok(Json(OllamaModelsResponse {
        success: true,
        data: OllamaModelsData {
            models,
            default,
            available,
        },
    }))
}
