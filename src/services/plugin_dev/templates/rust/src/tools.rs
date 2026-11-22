//! The assistant's tools. One struct per tool: `name()` is what the assistant
//! calls, `invoke()` does the work and returns an `ActionOutcome` whose data
//! the assistant reads. A tool name must be unique across every plugin, so
//! keep the `@@IDENT@@_` prefix.

use async_trait::async_trait;
use serde_json::{json, Value};
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

pub struct AddItem;

#[async_trait]
impl Tool for AddItem {
    fn name(&self) -> &str {
        "@@IDENT@@_add"
    }
    fn step_label(&self) -> &str {
        "Adding…"
    }
    fn humanize(&self, result: &str, data: &Value) -> String {
        match (result, data.get("text").and_then(Value::as_str)) {
            ("ok", Some(text)) => format!("Added \"{text}\""),
            _ => "Could not add the item".into(),
        }
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let text = req.params.require_str("text")?;
        let item = crate::store::add(ctx, req.user_id, &text)?;
        Ok(ActionOutcome::ok(self.name(), item))
    }
}

pub struct ListItems;

#[async_trait]
impl Tool for ListItems {
    fn name(&self) -> &str {
        "@@IDENT@@_list"
    }
    fn step_label(&self) -> &str {
        "Reading the list…"
    }
    fn humanize(&self, _result: &str, data: &Value) -> String {
        let count = data.get("count").and_then(Value::as_u64).unwrap_or(0);
        format!("Listed {count} items")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let limit = req.params.param_u32("limit").unwrap_or(50) as i64;
        let items = crate::store::list(ctx, req.user_id, limit)?;
        Ok(ActionOutcome::ok(self.name(), json!({ "count": items.len(), "items": items })))
    }
}
