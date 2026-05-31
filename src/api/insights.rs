use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::api::AppState;
use crate::errors::AppError;
use crate::services::insights;

#[derive(Deserialize)]
pub struct ContextInsightsParams {
    pub destination: String,
    pub lat: f64,
    pub lon: f64,
    pub ollama_model: Option<String>,
}

#[derive(Serialize)]
pub struct InsightCardsResponse {
    pub success: bool,
    pub destination: String,
    pub data: Vec<insights::InsightCard>,
}

/// GET /api/insights/context?destination=Milan&lat=45.46&lon=9.19
pub async fn context(
    State(state): State<AppState>,
    Query(params): Query<ContextInsightsParams>,
) -> Result<Json<InsightCardsResponse>, AppError> {
    let destination = params.destination.trim();
    if destination.is_empty() {
        return Err(AppError::BadRequest("destination required".into()));
    }

    let model = params
        .ollama_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let cards = insights::build_context_cards(
        state.osm.client(),
        &state.search,
        &state.ollama,
        destination,
        params.lat,
        params.lon,
        model,
    )
    .await?;

    Ok(Json(InsightCardsResponse {
        success: true,
        destination: destination.to_string(),
        data: cards,
    }))
}
