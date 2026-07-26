use async_trait::async_trait;
use serde_json::{json, Value};
use shiny_plugin_sdk::{
    navigation::NavigationSession,
    errors::AppError,
    outcome::ActionOutcome,
    services::PluginCtx,
    tools::{Tool, ToolRequest, ParamHelpers},
};

pub struct NavigateTo;

#[async_trait]
impl Tool for NavigateTo {
    fn name(&self) -> &str { "navigate_to" }
    fn step_label(&self) -> &str { "Starting navigation…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `navigate_to` — Plan a route to a destination and start navigation. params: `{ destination?: string, to_lat?: number, to_lon?: number, name?: string, profile?: 'car'|'bike'|'foot' }`")
    }
    fn aliases(&self) -> &[&str] { &["navigate", "directions", "drive_to", "go_to", "start_navigation", "navigation", "start_navigator", "navigate-to"] }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let dest = data.pointer("/navigator/destination").and_then(|v| v.as_str()).unwrap_or("destination");
        format!("Navigating to {dest}")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let _ = req.params.require_str("destination").or_else(|_| {
            req.params.require_f64("to_lat")?;
            req.params.require_f64("to_lon")?;
            Ok::<String, AppError>(String::new())
        })?;

        // The full OSM/OSRM call lives in the plugin's services::osm module.
        // Skeleton returns a session with the destination coords so the UI
        // can demonstrate the navigation path in the demo.
        let dest = req.params.param_str("destination").unwrap_or_else(|| format!("{:.4}, {:.4}",
            req.params.param_f64("to_lat").unwrap_or(0.0),
            req.params.param_f64("to_lon").unwrap_or(0.0)));
        let to_lat = req.params.param_f64("to_lat").unwrap_or(0.0);
        let to_lon = req.params.param_f64("to_lon").unwrap_or(0.0);

        let nav = NavigationSession {
            destination: dest,
            to_lat, to_lon,
            geometry: vec![],
            steps: vec![],
            distance_km: 0.0,
            duration_min: 0.0,
            profile: req.params.param_str("profile").unwrap_or_else(|| "car".into()),
        };
        let data = json!({ "navigator": &nav });
        Ok(ActionOutcome::ok("navigate_to", data).with_navigation(nav))
    }
}