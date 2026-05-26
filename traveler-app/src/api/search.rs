use axum::extract::{State, Extension};
use axum::Json;
use serde::{Serialize, Deserialize};

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;
use crate::services::web_search::SearchResult;

#[derive(Deserialize)]
pub struct SearchBody {
    pub query: String,
}

#[derive(Serialize)]
pub struct SearchResponse {
    pub success: bool,
    pub data: Vec<SearchResult>,
    pub summary: Option<String>,
}

pub async fn search_web(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
    Json(body): Json<SearchBody>,
) -> Result<Json<SearchResponse>, AppError> {
    let results = state.search.search(&body.query).await?;

    let summary = if state.ollama.is_available().await {
        let results_text: String = results
            .iter()
            .map(|r| format!("- {}: {}", r.title, r.snippet))
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "Summarize these search results about '{}' in 2-3 sentences:\n\n{}",
            body.query, results_text
        );

        match state.ollama.generate(&prompt, None).await {
            Ok(s) => Some(s),
            Err(_) => None,
        }
    } else {
        None
    };

    Ok(Json(SearchResponse {
        success: true,
        data: results,
        summary,
    }))
}
