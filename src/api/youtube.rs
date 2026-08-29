//! YouTube helpers: in-tile search for the YouTube window.
//!
//! The full YouTube site refuses iframing, so the window's own search bar
//! needs a backend. This route proxies to the registered `youtube_search`
//! plugin tool — the same interim pattern as `/api/radio/nowplaying` until
//! plugin HTTP routes land (roadmap #2).

use axum::extract::{Extension, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;
use shiny_plugin_sdk::context::AgentContext;

#[derive(Deserialize)]
pub struct SearchQuery {
    q: String,
}

/// GET /api/youtube/search?q=…
pub async fn search(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let q = q.q.trim().to_string();
    if q.is_empty() {
        return Err(AppError::BadRequest("q required".into()));
    }

    let agent_ctx = AgentContext {
        lat: None,
        lon: None,
        heading: None,
        lang: String::new(),
        ollama_model: None,
    };
    let plugin_ctx = state.plugin_ctx();
    let outcome = state
        .plugins
        .tools()
        .invoke(
            "youtube_search",
            &plugin_ctx,
            &traveler.id,
            &traveler.id,
            &json!({ "query": q, "limit": 12 }),
            &agent_ctx,
        )
        .await?;

    Ok(Json(json!({ "success": true, "data": outcome.data })))
}
