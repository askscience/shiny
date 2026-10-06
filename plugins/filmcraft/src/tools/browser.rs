//! Tools that drive the **window**: the editor the user is looking at.
//!
//! Both go through [`crate::relay`], so they act on the browser session and
//! every command id is the same one FilmCraft's own MCP server and
//! `filmcraft-cli` take.

use async_trait::async_trait;
use serde_json::{json, Value};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::relay;

/// Wait for the window's answer, honouring `wait_ms` within the relay's caps.
fn wait_of(params: &Value) -> std::time::Duration {
    let ms = params.param_u32("wait_ms").unwrap_or(0) as u64;
    let ms = if ms == 0 {
        relay::DEFAULT_WAIT.as_millis() as u64
    } else {
        ms
    };
    relay::MAX_WAIT.min(std::time::Duration::from_millis(ms))
}

/// `film_command` — run any engine command in the open window.
pub struct FilmCommand;

#[async_trait]
impl Tool for FilmCommand {
    fn name(&self) -> &str {
        "film_command"
    }
    fn aliases(&self) -> &[&str] {
        &["film"]
    }
    fn step_label(&self) -> &str {
        "Driving FilmCraft…"
    }
    fn doc_fragment(&self) -> Option<&str> {
        Some(
            "- `film_command` — Run a FilmCraft engine command in the open FilmCraft window \
             (the editor the user sees). params: `{ command: string, params?: object, wait_ms?: number }`. \
             `params` is the command's own parameter object; call `film_commands` first for the \
             exact ids and their parameter documentation. Common ids: `project.inspect`, \
             `sequence.inspect`, `state.inspect`, `jobs.list` (read); `file.import` \
             (`{paths:[…]}`), `sequence.addEdit`, `sequence.lift`, `sequence.extract`, \
             `timeline.move`, `sequence.renderInToOut` (edit); `effects.setParam`, \
             `lumetri.setLook`, `mixer.setStrip`, `mixer.setValue` (colour and sound); \
             `file.save`, `file.saveAs`, `file.exportMedia` (output). \
             Needs the window open; waits up to 20s by default (`wait_ms`, max 120000).",
        )
    }
    fn humanize(&self, result: &str, data: &Value) -> String {
        let command = data
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("film_command");
        match result {
            "ok" => format!("Ran {command} in the FilmCraft window"),
            _ => format!("{command} failed in the FilmCraft window"),
        }
    }

    async fn invoke(
        &self,
        _ctx: &PluginCtx,
        req: ToolRequest<'_>,
    ) -> Result<ActionOutcome, AppError> {
        let command = req.params.require_str("command")?;
        let params = req
            .params
            .get("params")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !params.is_object() {
            return Err(AppError::BadRequest("`params` must be an object".into()));
        }
        let user = req.user_id.to_string();
        let wait = wait_of(req.params);
        let result = relay::submit(
            &user,
            "engine.execute",
            json!({ "command": command, "params": params }),
            wait,
        )
        .await
        .map_err(AppError::Internal)?;

        Ok(ActionOutcome::ok(
            "film_command",
            json!({ "command": command, "result": result }),
        ))
    }
}

/// `film_commands` — what the window can do, and whether it is there.
pub struct FilmCommands;

#[async_trait]
impl Tool for FilmCommands {
    fn name(&self) -> &str {
        "film_commands"
    }
    fn step_label(&self) -> &str {
        "Listing FilmCraft commands…"
    }
    fn doc_fragment(&self) -> Option<&str> {
        Some(
            "- `film_commands` — List the engine commands available in the FilmCraft window \
             (id, label, menu path, parameter docs) and report whether the window is open. \
             params: `{}`",
        )
    }
    fn humanize(&self, result: &str, _data: &Value) -> String {
        match result {
            "ok" => "Listed the FilmCraft commands".to_string(),
            _ => "Could not list the FilmCraft commands".to_string(),
        }
    }

    async fn invoke(
        &self,
        _ctx: &PluginCtx,
        req: ToolRequest<'_>,
    ) -> Result<ActionOutcome, AppError> {
        let user = req.user_id.to_string();
        if !relay::window_online(&user) {
            // Not an error: the assistant should open the window and retry.
            return Ok(ActionOutcome::ok(
                "film_commands",
                json!({
                    "window": false,
                    "commands": [],
                    "hint": "No FilmCraft window is polling. Open it with the `show_plugin` action \
                             (name \"filmcraft\"), then call this again.",
                }),
            ));
        }
        let result = relay::submit(&user, "engine.commands", json!({}), relay::DEFAULT_WAIT)
            .await
            .map_err(AppError::Internal)?;
        Ok(ActionOutcome::ok(
            "film_commands",
            json!({ "window": true, "commands": result }),
        ))
    }
}