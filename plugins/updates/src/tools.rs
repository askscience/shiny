//! Agent tools for the Updates plugin.
//!
//! Read-only tools (`updates_status`, `updates_distro`) work unprivileged.
//! Write tools (`updates_refresh`, `updates_apply`, `update_ollama`) require an
//! explicit `password` param whenever the server is not already root — the
//! password is forwarded to `sudo -S` on stdin and never stored.
//!
//! Write tools **start a background job and return immediately**. A long
//! upgrade must never occupy the shared, single-threaded plugin runtime
//! (which every plugin's tools/routes share), so completion is reported
//! through the desktop notification system and recorded by a queued waiter.

use async_trait::async_trait;
use serde_json::{json, Value};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::distro;
use crate::exec;
use crate::jobs::{self, Job};
use crate::ops;
use crate::status;

fn parse_packages(v: &Value) -> Vec<String> {
    match v.get("packages") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(s)) => s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn missing_password(password: Option<&str>) -> bool {
    !exec::is_root()
        && password
            .map(|p| p.trim().is_empty())
            .unwrap_or(true)
}

/// Outcome for a tool that has just queued a job: the caller is told it
/// started, and the desktop notification reports the result.
fn started(action: &str, job: &Job, extra: Value) -> ActionOutcome {
    let mut data = json!({ "job": job.id, "status": "started" });
    if let Some(obj) = data.as_object_mut() {
        if let Some(extra_obj) = extra.as_object() {
            for (k, v) in extra_obj {
                obj.insert(k.clone(), v.clone());
            }
        }
    }
    ActionOutcome::ok(action, data)
}

// ── updates_status ──────────────────────────────────────────────────────────

pub struct UpdatesStatusTool;

#[async_trait]
impl Tool for UpdatesStatusTool {
    fn name(&self) -> &str {
        "updates_status"
    }
    fn aliases(&self) -> &[&str] {
        &["check_updates", "list_updates", "system_updates"]
    }
    fn step_label(&self) -> &str {
        "Checking for updates…"
    }
    fn doc_fragment(&self) -> Option<&str> {
        Some(
            "- `updates_status` — Detect the distro and list pending system/package \
             and Ollama updates. params: `{ force?: boolean }`",
        )
    }
    fn humanize(&self, _result: &str, data: &Value) -> String {
        let count = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        if count == 0 {
            "Checked for updates — system is up to date".into()
        } else {
            format!("Checked for updates — {count} available")
        }
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let force = req.params.param_bool("force").unwrap_or(false);
        let snapshot = if force {
            status::collect(true)
        } else {
            status::snapshot()
        };
        let mut data = serde_json::to_value(&snapshot)?;
        let summary = if let Some(err) = &snapshot.error {
            format!("Could not check for updates: {err}")
        } else if snapshot.count == 0 {
            "System is up to date".to_string()
        } else {
            format!(
                "{} update(s) available on {}",
                snapshot.count, snapshot.distro.manager_label
            )
        };
        if let Some(obj) = data.as_object_mut() {
            obj.insert("summary".into(), json!(summary));
        }
        Ok(ActionOutcome::ok("updates_status", data))
    }
}

// ── updates_distro ──────────────────────────────────────────────────────────

pub struct UpdatesDistroTool;

#[async_trait]
impl Tool for UpdatesDistroTool {
    fn name(&self) -> &str {
        "updates_distro"
    }
    fn aliases(&self) -> &[&str] {
        &["detect_distro", "distro_info"]
    }
    fn step_label(&self) -> &str {
        "Detecting the distribution…"
    }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `updates_distro` — Report the detected Linux distribution and package manager. params: `{}`")
    }
    fn humanize(&self, _result: &str, data: &Value) -> String {
        let name = data
            .get("distro")
            .and_then(|d| d.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("Linux");
        let manager = data
            .get("distro")
            .and_then(|d| d.get("manager_label"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        format!("Detected {name} ({manager})")
    }

    async fn invoke(&self, _ctx: &PluginCtx, _req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let manager = distro::detect();
        let info = status::distro_info(manager.as_ref());
        Ok(ActionOutcome::ok(
            "updates_distro",
            json!({ "distro": info }),
        ))
    }
}

// ── updates_refresh ─────────────────────────────────────────────────────────

pub struct UpdatesRefreshTool;

#[async_trait]
impl Tool for UpdatesRefreshTool {
    fn name(&self) -> &str {
        "updates_refresh"
    }
    fn aliases(&self) -> &[&str] {
        &["refresh_updates", "update_package_lists"]
    }
    fn step_label(&self) -> &str {
        "Starting package list refresh…"
    }
    fn doc_fragment(&self) -> Option<&str> {
        Some(
            "- `updates_refresh` — Start refreshing the package manager's metadata \
             (e.g. `apt update`). Returns immediately; the user is notified when \
             it finishes. Requires the sudo password on a non-root server. \
             params: `{ password?: string }`",
        )
    }
    fn humanize(&self, _result: &str, _data: &Value) -> String {
        "Started refreshing package lists".into()
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let manager = distro::detect();
        if !manager.supported() {
            return Ok(ActionOutcome::error(
                "updates_refresh",
                "Unsupported distribution: no known package manager was detected.",
            ));
        }
        let password = req.params.param_str("password");
        if missing_password(password.as_deref()) {
            return Ok(ActionOutcome::error(
                "updates_refresh",
                "An administrator (sudo) password is required. Open the Updates window to enter it.",
            ));
        }

        let manager_id = manager.id().to_string();
        let job = jobs::start("refresh", move |job| {
            let ok = ops::refresh_job(&job, manager.as_ref(), password.as_deref());
            job.finish(ok, None);
        });
        crate::history::record_when_done(
            ctx.clone(),
            req.user_id.to_string(),
            "refresh",
            manager_id,
            "refresh".into(),
            job.clone(),
        );
        Ok(started("updates_refresh", &job, json!({})))
    }
}

// ── updates_apply ───────────────────────────────────────────────────────────

pub struct UpdatesApplyTool;

#[async_trait]
impl Tool for UpdatesApplyTool {
    fn name(&self) -> &str {
        "updates_apply"
    }
    fn aliases(&self) -> &[&str] {
        &["apply_updates", "install_updates", "update_system"]
    }
    fn step_label(&self) -> &str {
        "Starting update install…"
    }
    fn doc_fragment(&self) -> Option<&str> {
        Some(
            "- `updates_apply` — Start installing system updates (all, or a \
             selected list) and/or updating Ollama. Returns immediately; the user \
             is notified when it finishes. Requires the sudo password on a \
             non-root server. params: `{ password?: string, all?: boolean, \
             packages?: string[], include_ollama?: boolean }`",
        )
    }
    fn humanize(&self, _result: &str, data: &Value) -> String {
        let ollama = data
            .get("include_ollama")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if ollama {
            "Started installing system and Ollama updates".into()
        } else {
            "Started installing updates".into()
        }
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let manager = distro::detect();
        let include_ollama = req.params.param_bool("include_ollama").unwrap_or(false);
        let packages = parse_packages(req.params);
        let all = req
            .params
            .param_bool("all")
            .unwrap_or(packages.is_empty() && !include_ollama);

        if !manager.supported() && !include_ollama {
            return Ok(ActionOutcome::error(
                "updates_apply",
                "Unsupported distribution: no known package manager was detected.",
            ));
        }
        let password = req.params.param_str("password");
        if missing_password(password.as_deref()) {
            return Ok(ActionOutcome::error(
                "updates_apply",
                "An administrator (sudo) password is required. Open the Updates window to enter it.",
            ));
        }

        let manager_id = manager.id().to_string();
        let detail = if all {
            "all".to_string()
        } else {
            packages.join(", ")
        };
        let pkgs = packages.clone();
        let job = jobs::start("apply", move |job| {
            let mut ok = true;
            if manager.supported() && (all || !pkgs.is_empty()) {
                ok = ops::system_job(&job, manager.as_ref(), password.as_deref(), all, &pkgs);
            }
            if include_ollama {
                ok = ops::ollama_job(&job, password.as_deref()) && ok;
            }
            job.finish(ok, None);
        });
        crate::history::record_when_done(
            ctx.clone(),
            req.user_id.to_string(),
            "apply",
            manager_id,
            detail,
            job.clone(),
        );
        Ok(started("updates_apply", &job, json!({ "include_ollama": include_ollama })))
    }
}

// ── update_ollama ───────────────────────────────────────────────────────────

pub struct OllamaUpdateTool;

#[async_trait]
impl Tool for OllamaUpdateTool {
    fn name(&self) -> &str {
        "update_ollama"
    }
    fn aliases(&self) -> &[&str] {
        &["ollama_update", "upgrade_ollama"]
    }
    fn step_label(&self) -> &str {
        "Starting Ollama update…"
    }
    fn doc_fragment(&self) -> Option<&str> {
        Some(
            "- `update_ollama` — Start installing/updating Ollama with the official \
             installer. Returns immediately; the user is notified when it \
             finishes. Requires the sudo password on a non-root server. \
             params: `{ password?: string }`",
        )
    }
    fn humanize(&self, _result: &str, _data: &Value) -> String {
        "Started updating Ollama".into()
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let password = req.params.param_str("password");
        if missing_password(password.as_deref()) {
            return Ok(ActionOutcome::error(
                "update_ollama",
                "An administrator (sudo) password is required. Open the Updates window to enter it.",
            ));
        }
        let job = jobs::start("ollama", move |job| {
            let ok = ops::ollama_job(&job, password.as_deref());
            job.finish(ok, None);
        });
        crate::history::record_when_done(
            ctx.clone(),
            req.user_id.to_string(),
            "ollama",
            "ollama".into(),
            "official installer".into(),
            job.clone(),
        );
        Ok(started("update_ollama", &job, json!({})))
    }
}
