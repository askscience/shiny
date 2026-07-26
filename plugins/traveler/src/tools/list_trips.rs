use async_trait::async_trait;
use serde_json::{json, Value};
use shiny_plugin_sdk::{
    errors::AppError,
    outcome::ActionOutcome,
    services::PluginCtx,
    tools::{Tool, ToolRequest, ParamHelpers},
};

pub struct ListTrips;

#[async_trait]
impl Tool for ListTrips {
    fn name(&self) -> &str { "list_trips" }
    fn step_label(&self) -> &str { "Loading trips…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `list_trips` — List the user's trips. params: `{}`")
    }
    fn humanize(&self, _result: &str, data: &Value) -> String {
        let n = data.get("trips").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        format!("Listed {n} trip(s)")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let limit = req.params.param_u32("limit").unwrap_or(50).min(200) as i64;
        let rows = sqlx::query_as::<_, (String, String, Option<String>, String)>(
            "SELECT id, name, COALESCE(description, ''), status FROM trips \
             WHERE traveler_id = ?1 ORDER BY created_at DESC LIMIT ?2",
        )
        .bind(req.traveler_id)
        .bind(limit)
        .fetch_all(&ctx.pool)
        .await?;

        let trips: Vec<Value> = rows.into_iter().map(|(id, name, desc, status)| json!({
            "id": id, "name": name, "description": desc, "status": status
        })).collect();
        Ok(ActionOutcome::ok("list_trips", json!({ "trips": trips })))
    }
}