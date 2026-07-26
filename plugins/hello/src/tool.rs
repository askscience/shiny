use async_trait::async_trait;
use serde_json::{json, Value};
use shiny_plugin_sdk::{
    errors::AppError,
    outcome::ActionOutcome,
    services::PluginCtx,
    tools::{Tool, ToolRequest, ParamHelpers},
};

pub struct HelloTool;

#[async_trait]
impl Tool for HelloTool {
    fn name(&self) -> &str { "hello" }
    fn step_label(&self) -> &str { "Saying hello…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `hello` — Say hello. params: `{ name?: string }`")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let who = data.get("who").and_then(|v| v.as_str()).unwrap_or("world");
        format!("Said hello to {who}")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let name = req.params.param_str("name").unwrap_or_else(|| "world".into());
        let reply = format!("Hello, {name}!");
        Ok(ActionOutcome::ok("hello", json!({ "who": name, "reply": reply })))
    }
}