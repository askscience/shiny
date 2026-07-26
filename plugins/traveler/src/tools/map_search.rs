use async_trait::async_trait;
use serde_json::Value;
use shiny_plugin_sdk::{
    errors::AppError,
    outcome::ActionOutcome,
    services::PluginCtx,
    tools::{Tool, ToolRequest, ParamHelpers},
};

pub struct MapSearch;

#[async_trait]
impl Tool for MapSearch {
    fn name(&self) -> &str { "map_search" }
    fn step_label(&self) -> &str { "Searching maps…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `map_search` — Geocode a place name via Nominatim. params: `{ q: string, limit?: number }`")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let q = req.params.require_str("q")?;
        // The full OSM implementation lives in the plugin's `services::osm` module;
        // for the demo skeleton we shell out to a simpler inline call to keep the
        // crate small. Fully implemented in PLUGINS.md §"Providing services".
        let client = reqwest::Client::new();
        let limit = req.params.param_u32("limit").unwrap_or(5);
        let url = format!(
            "https://nominatim.openstreetmap.org/search?q={}&format=json&limit={}",
            q.replace(' ', "+"), limit
        );
        let body: Value = client
            .get(&url)
            .header("User-Agent", "shiny/0.1")
            .send()
            .await?
            .json()
            .await
            .unwrap_or(Value::Array(vec![]));

        let places: Vec<Value> = body.as_array().cloned().unwrap_or_default();
        Ok(ActionOutcome::ok("map_search", Value::Array(places)))
    }
}