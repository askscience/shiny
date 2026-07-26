use async_trait::async_trait;
use serde_json::{json, Value};
use shiny_plugin_sdk::{
    artifacts::{Artifact, PlanDay},
    errors::AppError,
    outcome::ActionOutcome,
    services::PluginCtx,
    tools::{Tool, ToolRequest, ParamHelpers},
};

pub struct PlanTrip;

#[async_trait]
impl Tool for PlanTrip {
    fn name(&self) -> &str { "plan_trip" }
    fn step_label(&self) -> &str { "Planning your trip…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `plan_trip` — Build a multi-day trip plan with themed guides. params: `{ destination: string, days?: number, profile?: 'car'|'bike'|'foot' }`")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let dest = data.pointer("/destination/name").and_then(|v| v.as_str()).unwrap_or("destination");
        let guides = data.get("guides_created").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Planned trip to {dest} — {guides} guides")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let destination = req.params.require_str("destination")?;
        let days = req.params.param_u32("days").unwrap_or(3).max(1).min(14);

        // Skeleton artifact — in the full plugin this dispatches the multi-day
        // overview story + themed guide generation via Ollama (see PLUGINS.md).
        let mut day_items = Vec::new();
        for d in 1..=days {
            day_items.push(PlanDay { day: d, title: format!("Day {d} — explore {destination}"), items: vec![] });
        }
        let artifact = Artifact {
            id: uuid::Uuid::new_v4().to_string(),
            artifact_type: "travel_plan".into(),
            title: format!("Trip to {destination}"),
            subtitle: Some(format!("{days}-day plan")),
            coordinates: None,
            sections: vec![],
            actions: vec![],
            days: day_items,
            route: None,
            geometry: vec![],
            narrative: Some(format!("A {days}-day itinerary to {destination}.")),
            theme: Some("overview".into()),
            destination: Some(destination.clone()),
        };
        let data = json!({
            "destination": { "name": destination },
            "guides_created": 0,
        });
        Ok(ActionOutcome::ok("plan_trip", data).with_artifact(artifact))
    }
}