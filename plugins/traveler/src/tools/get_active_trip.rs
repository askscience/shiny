use async_trait::async_trait;
use serde_json::{json, Value};
use shiny_plugin_sdk::{
    errors::AppError,
    outcome::ActionOutcome,
    services::PluginCtx,
    tools::{Tool, ToolRequest},
};

pub struct GetActiveTrip;

#[async_trait]
impl Tool for GetActiveTrip {
    fn name(&self) -> &str { "get_active_trip" }
    fn step_label(&self) -> &str { "Checking active trip…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `get_active_trip` — Return the active (running) trip. params: `{}`")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        if data.get("trip").is_none() { "No active trip".into() } else { "Active trip loaded".into() }
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let row = sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT id, name, COALESCE(description, '') FROM trips \
             WHERE traveler_id = ?1 AND status = 'active' ORDER BY start_time DESC LIMIT 1",
        )
        .bind(req.traveler_id)
        .fetch_optional(&ctx.pool)
        .await?;

        let data = match row {
            Some((id, name, desc)) => json!({ "trip": { "id": id, "name": name, "description": desc } }),
            None => json!({}),
        };
        Ok(ActionOutcome::ok("get_active_trip", data))
    }
}