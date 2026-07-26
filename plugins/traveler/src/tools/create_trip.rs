use async_trait::async_trait;
use serde_json::json;
use shiny_plugin_sdk::{
    context::AgentContext,
    errors::AppError,
    outcome::ActionOutcome,
    services::PluginCtx,
    tools::{Tool, ToolRequest, ParamHelpers},
};
use uuid::Uuid;

pub struct CreateTrip;

#[async_trait]
impl Tool for CreateTrip {
    fn name(&self) -> &str { "create_trip" }
    fn step_label(&self) -> &str { "Creating trip…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `create_trip` — Create a new trip. params: `{ name: string, description?: string }`")
    }
    fn aliases(&self) -> &[&str] { &["new_trip"] }

    fn humanize(&self, result: &str, data: &serde_json::Value) -> String {
        if result == "error" {
            return data.get("error").and_then(|v| v.as_str())
                .map(|m| format!("create_trip failed: {m}"))
                .unwrap_or_else(|| "create_trip failed".into());
        }
        let name = data.pointer("/trip/name").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() { "create_trip complete".into() } else { format!("create_trip: {name}") }
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let name = req.params.require_str("name")?;
        let description = req.params.get("description").and_then(|v| v.as_str()).map(String::from);

        let id = Uuid::new_v4().to_string();
        let status = "planned";
        sqlx::query(
            "INSERT INTO trips (id, traveler_id, name, description, status, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
        )
        .bind(&id)
        .bind(req.traveler_id)
        .bind(&name)
        .bind(&description)
        .bind(status)
        .execute(&ctx.pool)
        .await?;

        let data = json!({
            "trip": { "id": id, "name": name, "description": description, "status": status }
        });
        Ok(ActionOutcome::ok("create_trip", data))
    }
}

// Suppress unused warning for AgentContext import alias.
#[allow(dead_code)]
fn _force_agent_ctx_use(_c: &AgentContext) {}