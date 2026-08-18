//! navigate_to — full turn-by-turn session from GPS to destination.

use async_trait::async_trait;
use serde_json::{json, Value};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::navigation::build_navigation_session;
use crate::osm::OsmClient;

pub struct NavigateTo;

#[async_trait]
impl Tool for NavigateTo {
    fn name(&self) -> &str { "navigate_to" }
    fn step_label(&self) -> &str { "Starting navigation…" }
    fn aliases(&self) -> &[&str] {
        &["navigate", "directions", "drive_to", "go_to", "start_navigation", "navigation", "start_navigator", "navigate-to"]
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let dest = data
            .pointer("/navigator/destination")
            .and_then(|v| v.as_str())
            .unwrap_or("destination");
        format!("Navigating to {dest}")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let from_lat = req
            .params
            .param_f64("from_lat")
            .or(req.ctx.lat)
            .ok_or_else(|| AppError::BadRequest("from_lat required — enable GPS".into()))?;
        let from_lon = req
            .params
            .param_f64("from_lon")
            .or(req.ctx.lon)
            .ok_or_else(|| AppError::BadRequest("from_lon required — enable GPS".into()))?;

        let session = build_navigation_session(&OsmClient::new(), from_lat, from_lon, req.params).await?;

        // Core's agent runner re-parses `data.navigator` for navigate_to —
        // keep the mirror in `data` alongside the typed navigation field.
        let data = json!({ "navigator": &session });
        Ok(ActionOutcome::ok("navigate_to", data).with_navigation(session))
    }
}
