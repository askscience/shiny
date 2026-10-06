//! Tools that drive the **headless** engine on server-side files.
//!
//! Use these when the browser window is not involved: inspecting a project on
//! disk, running a conform script, or starting an export the user does not
//! have to watch. Short calls only — every tool here runs on the plugin
//! runtime's serial worker, so long work is expressed as a job plus polling.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::headless;

/// Ceilings, because a tool call occupies the shared plugin worker while it waits.
const CMD_WAIT: Duration = Duration::from_secs(15);
const CMD_WAIT_MAX: Duration = Duration::from_secs(60);
const EXPORT_WAIT: Duration = Duration::from_secs(60);

fn cmd_timeout(params: &Value) -> Duration {
    let secs = params.param_u32("timeout_s").unwrap_or(0);
    let secs = if secs == 0 {
        CMD_WAIT.as_secs() as u32
    } else {
        secs
    };
    CMD_WAIT_MAX.min(Duration::from_secs(secs as u64))
}

/// `film_headless` — any engine command on the server-side session.
pub struct FilmHeadless;

#[async_trait]
impl Tool for FilmHeadless {
    fn name(&self) -> &str {
        "film_headless"
    }
    fn step_label(&self) -> &str {
        "Running a headless FilmCraft command…"
    }
    fn doc_fragment(&self) -> Option<&str> {
        Some(
            "- `film_headless` — Run a FilmCraft engine command on the plugin's own headless \
             engine, against files on this machine (not the window). params: \
             `{ command: string, params?: object, timeout_s?: number }`. Use it to open a project \
             (`file.open` with `{path}`), inspect it (`project.inspect`, `sequence.inspect`, \
             `media.import`) or script edits. This session is separate from the window: an edit \
             saved in the window is not seen until you `file.open` again. Keep it short; use \
             `film_export` for long work.",
        )
    }
    fn humanize(&self, result: &str, data: &Value) -> String {
        let command = data.get("command").and_then(Value::as_str).unwrap_or("command");
        match result {
            "ok" => format!("Ran {command} headless"),
            _ => format!("{command} failed headless"),
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
        let value = headless::execute(&command, params, cmd_timeout(req.params))
            .map_err(AppError::Internal)?;
        Ok(ActionOutcome::ok(
            "film_headless",
            json!({ "command": command, "result": value }),
        ))
    }
}

/// `film_export` — start a background render of a project on disk.
pub struct FilmExport;

#[async_trait]
impl Tool for FilmExport {
    fn name(&self) -> &str {
        "film_export"
    }
    fn step_label(&self) -> &str {
        "Starting a FilmCraft export…"
    }
    fn doc_fragment(&self) -> Option<&str> {
        Some(
            "- `film_export` — Start a background H.264/ProRes/DNxHR/PNG/… export of a `.fcproj` \
             file on this machine, using the headless engine. params: \
             `{ project: string, output: string, format?: string, preset?: string, settings?: object, timeout_s?: number }`. \
             The project is opened fresh, so always export from a saved file. Returns a job id \
             immediately; poll it with `film_export_status` and stop it with `film_export_cancel`. \
             Runs independently of the window.",
        )
    }
    fn humanize(&self, result: &str, data: &Value) -> String {
        let out = data
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or("the export");
        match result {
            "ok" => format!("Started an export of {out}"),
            _ => format!("Could not start the export of {out}"),
        }
    }

    async fn invoke(
        &self,
        _ctx: &PluginCtx,
        req: ToolRequest<'_>,
    ) -> Result<ActionOutcome, AppError> {
        let project = req.params.require_str("project")?;
        let output = req.params.require_str("output")?;
        let mut settings = req
            .params
            .get("settings")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !settings.is_object() {
            return Err(AppError::BadRequest("`settings` must be an object".into()));
        }
        if let Some(format) = req.params.param_str("format") {
            settings["format"] = json!(format);
        }
        if let Some(preset) = req.params.param_str("preset") {
            settings["preset"] = json!(preset);
        }
        settings["path"] = json!(output);

        let job = headless::start_export(&project, settings, EXPORT_WAIT).map_err(AppError::Internal)?;
        let id = job.get("job").cloned().unwrap_or(Value::Null);
        Ok(ActionOutcome::ok(
            "film_export",
            json!({
                "project": project,
                "output": output,
                "job": id,
                "started": job,
                "hint": "Poll `film_export_status` with this job id; stop it with `film_export_cancel`.",
            }),
        ))
    }
}

/// `film_export_status` — progress and results of background jobs.
pub struct FilmExportStatus;

#[async_trait]
impl Tool for FilmExportStatus {
    fn name(&self) -> &str {
        "film_export_status"
    }
    fn step_label(&self) -> &str {
        "Checking export jobs…"
    }
    fn doc_fragment(&self) -> Option<&str> {
        Some(
            "- `film_export_status` — Progress of the headless export jobs. params: \
             `{ job?: number }` — with a job id, or omit for every job. Each job reports its label, \
             progress and, when finished, the result or error.",
        )
    }
    fn humanize(&self, _result: &str, data: &Value) -> String {
        match data.get("job") {
            Some(Value::Number(_)) => format!("Checked export job {}", data["job"]),
            _ => "Checked the export jobs".to_string(),
        }
    }

    async fn invoke(
        &self,
        _ctx: &PluginCtx,
        req: ToolRequest<'_>,
    ) -> Result<ActionOutcome, AppError> {
        let jobs = headless::jobs().map_err(AppError::Internal)?;
        let data = match req.params.param_u32("job") {
            Some(id) => {
                let list = jobs.as_array().cloned().unwrap_or_default();
                let found = list
                    .into_iter()
                    .find(|j| j.get("id").and_then(Value::as_u64) == Some(id as u64));
                match found {
                    Some(job) => json!({ "job": job }),
                    None => json!({ "job": null, "hint": format!("no job {id} on the headless session") }),
                }
            }
            None => json!({ "jobs": jobs }),
        };
        Ok(ActionOutcome::ok("film_export_status", data))
    }
}

/// `film_export_cancel` — stop a background export.
pub struct FilmExportCancel;

#[async_trait]
impl Tool for FilmExportCancel {
    fn name(&self) -> &str {
        "film_export_cancel"
    }
    fn step_label(&self) -> &str {
        "Stopping an export…"
    }
    fn doc_fragment(&self) -> Option<&str> {
        Some(
            "- `film_export_cancel` — Stop a running headless export. params: `{ job: number }`",
        )
    }
    fn humanize(&self, _result: &str, data: &Value) -> String {
        format!("Stopped export job {}", data.get("job").unwrap_or(&Value::Null))
    }

    async fn invoke(
        &self,
        _ctx: &PluginCtx,
        req: ToolRequest<'_>,
    ) -> Result<ActionOutcome, AppError> {
        let id = req
            .params
            .param_u32("job")
            .ok_or_else(|| AppError::BadRequest("`job` must be a job id".into()))?;
        let value = headless::cancel(id as u64).map_err(AppError::Internal)?;
        Ok(ActionOutcome::ok("film_export_cancel", json!({ "job": id, "result": value })))
    }
}