//! The `Tool` trait and the parser/parameter helpers used by all tools.
//!
//! Each tool is one Rust struct implementing `Tool`. The registry maps
//! action keys to `Arc<dyn Tool>` instances; the agent runner dispatches
//! `LLM-emitted JSON action blocks to whichever tool claims that key.

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

use crate::context::AgentContext;
use crate::errors::AppError;
use crate::outcome::ActionOutcome;
use crate::services::PluginCtx;

/// What every tool receives on invocation.
pub struct ToolRequest<'a> {
    /// Authenticated user id (from the new core `users` table).
    pub user_id: &'a str,
    /// Optional profile-scoped id provided by a plugin (e.g. the traveler plugin
    /// exposes a `traveler_id` here). For backwards compat, mirrors `user_id`
    /// when no plugin converts identities.
    pub traveler_id: &'a str,
    /// Raw `params` JSON object from the LLM action block.
    pub params: &'a Value,
    /// Live agent context (lat/lon/heading/lang/model override).
    pub ctx: &'a AgentContext,
}

/// A single agent tool.
///
/// Tools are `Send + Sync` because they're stored as `Arc<dyn Tool>` in the
/// registry and invoked concurrently from request handlers.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Primary action key. Must be unique across all installed tools.
    fn name(&self) -> &str;

    /// Alternative spellings the LLM may emit. The registry registers each
    /// alias as a pointer back to this tool.
    fn aliases(&self) -> &[&str] {
        &[]
    }

    /// One-line spoken hint shown to the user while this tool runs
    /// ("Planning your trip…", "Searching the web…").
    fn step_label(&self) -> &str {
        "Working…"
    }

    /// Human-readable note appended to the agent's "completed steps" log,
    /// shown to the LLM in continuation prompts.
    fn humanize(&self, result: &str, data: &Value) -> String {
        let _ = data;
        if result == "error" {
            format!("{} failed", self.name())
        } else {
            format!("{} complete", self.name())
        }
    }

    /// Markdown snippet describing this tool to the LLM. Concatenated by the
    /// registry into the agent's system prompt. Use `None` if the tool is
    /// internal and shouldn't be advertised.
    fn doc_fragment(&self) -> Option<&str> {
        None
    }

    /// Run the tool. Implementations should be cheap; long operations should
    /// yield to the executor (the runtime is async by default).
    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError>;
}

/// Builder the plugin writes into during `Plugin::register`.
pub struct RegistryBuilder<'a> {
    pub tools: Vec<Arc<dyn Tool>>,
    pub routes: Vec<crate::routes::RouteSpec>,
    pub crons: Vec<crate::crons::CronSpec>,
    /// Markdown fragment that gets concatenated into the agent's system prompt.
    pub skills_md: String,
    /// One-line persona fragment ("a travel navigator AI").
    pub persona: String,
    /// Context lines contributed to the per-request system prompt
    /// (e.g. "Active trip: Paris Adventure").
    pub context_lines: Vec<String>,
    pub phantom: std::marker::PhantomData<&'a ()>,
}

impl<'a> RegistryBuilder<'a> {
    pub fn new() -> Self {
        Self {
            tools: Vec::new(),
            routes: Vec::new(),
            crons: Vec::new(),
            skills_md: String::new(),
            persona: String::new(),
            context_lines: Vec::new(),
            phantom: std::marker::PhantomData,
        }
    }

    pub fn tool(&mut self, t: impl Tool + 'static) -> &mut Self {
        self.tools.push(Arc::new(t));
        self
    }

    pub fn tool_arc(&mut self, t: Arc<dyn Tool>) -> &mut Self {
        self.tools.push(t);
        self
    }

    pub fn route(&mut self, r: crate::routes::RouteSpec) -> &mut Self {
        self.routes.push(r);
        self
    }

    pub fn cron(&mut self, c: crate::crons::CronSpec) -> &mut Self {
        self.crons.push(c);
        self
    }

    pub fn skills(&mut self, md: impl Into<String>) -> &mut Self {
        self.skills_md = md.into();
        self
    }

    pub fn persona(&mut self, p: impl Into<String>) -> &mut Self {
        self.persona = p.into();
        self
    }

    pub fn context_line(&mut self, line: impl Into<String>) -> &mut Self {
        self.context_lines.push(line.into());
        self
    }
}

// ---------- Parsing helpers --------------------------------------------------

/// Strip markdown ``` fences, walk the string for balanced `{...}` blocks,
/// and JSON-parse each. Returns `(action, params)` tuples — usually only one.
pub fn parse_actions(text: &str) -> Vec<(String, serde_json::Value)> {
    let fenceless = strip_code_fences(text);
    let mut out = Vec::new();
    let bytes = fenceless.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some(end) = find_json_end(&fenceless[i..]) {
                let candidate = &fenceless[i..i + end];
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(candidate) {
                    let action = v
                        .get("action")
                        .or_else(|| v.get("tool"))
                        .and_then(|a| a.as_str())
                        .unwrap_or("")
                        .to_string();
                    if !action.is_empty() {
                        let params = v
                            .get("params")
                            .cloned()
                            .unwrap_or(serde_json::Value::Object(Default::default()));
                        out.push((action, params));
                    }
                }
                i += end;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Remove the action JSON block(s) from an LLM raw response so the user only
/// hears the natural-language reply.
pub fn strip_action_blocks(text: &str) -> String {
    let mut cleaned = text.to_string();
    for (compact, pretty) in iter_json_blocks(text) {
        cleaned = cleaned.replace(&pretty, "");
        cleaned = cleaned.replace(&compact, "");
    }
    cleaned.trim().to_string()
}

fn iter_json_blocks(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some(end) = find_json_end(&text[i..]) {
                let candidate = &text[i..i + end];
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(candidate) {
                    if v.get("action").or_else(|| v.get("tool")).is_some() {
                        let compact = serde_json::to_string(&v).unwrap_or_default();
                        let pretty = serde_json::to_string_pretty(&v).unwrap_or_default();
                        out.push((compact, pretty));
                    }
                }
                i += end;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn strip_code_fences(text: &str) -> String {
    if !text.contains("```") {
        return text.to_string();
    }
    text.lines()
        .filter(|l| !l.trim_start().starts_with("```"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn find_json_end(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Normalize common action synonyms. Lives in the SDK because every registry
/// needs the same set.
pub fn normalize_action_name(action: &str) -> String {
    let trimmed = action.trim().to_lowercase();
    // Generic alias map; plugins add their own additional aliases via `Tool::aliases`.
    match trimmed.as_str() {
        "navigate" | "directions" | "drive_to" | "go_to" | "start_navigation"
        | "navigation" | "start_navigator" | "navigate-to" => "navigate_to".to_string(),
        _ => trimmed,
    }
}

// ---------- Runtime bridge adapter -----------------------------------------

/// Adapter that runs a tool's `invoke` on the plugin-owned runtime
/// (`crate::rt::bridge`). A plugin cdylib links its own Tokio copy — invoking
/// sqlx/reqwest/tokio-time while the host polls the future would abort the
/// process. Wrap every registered tool with `bridged(...)` at `register()`.
pub struct BridgedTool(pub Arc<dyn Tool>);

#[async_trait]
impl Tool for BridgedTool {
    fn name(&self) -> &str { self.0.name() }
    fn aliases(&self) -> &[&str] { self.0.aliases() }
    fn step_label(&self) -> &str { self.0.step_label() }
    fn humanize(&self, result: &str, data: &Value) -> String { self.0.humanize(result, data) }
    fn doc_fragment(&self) -> Option<&str> { self.0.doc_fragment() }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let inner = self.0.clone();
        let ctx = ctx.clone();
        let user_id = req.user_id.to_string();
        let traveler_id = req.traveler_id.to_string();
        let params = req.params.clone();
        let agent_ctx = req.ctx.clone();
        crate::rt::bridge(async move {
            let req = ToolRequest {
                user_id: &user_id,
                traveler_id: &traveler_id,
                params: &params,
                ctx: &agent_ctx,
            };
            inner.invoke(&ctx, req).await
        })
        .await
    }
}

/// Wrap a tool in the runtime-bridging adapter. Use at registration time:
/// `builder.tool_arc(bridged(tool))`.
pub fn bridged(t: Arc<dyn Tool>) -> Arc<dyn Tool> {
    Arc::new(BridgedTool(t))
}

// ---------- Parameter helpers ------------------------------------------------

pub trait ParamHelpers {
    fn param_str(&self, key: &str) -> Option<String>;
    fn param_f64(&self, key: &str) -> Option<f64>;
    fn param_u32(&self, key: &str) -> Option<u32>;
    fn param_bool(&self, key: &str) -> Option<bool>;
    fn require_str(&self, key: &str) -> Result<String, AppError>;
    fn require_f64(&self, key: &str) -> Result<f64, AppError>;
}

impl ParamHelpers for serde_json::Value {
    fn param_str(&self, key: &str) -> Option<String> {
        self.get(key).and_then(|v| v.as_str()).map(String::from)
    }

    fn param_f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(|v| v.as_f64())
    }

    fn param_u32(&self, key: &str) -> Option<u32> {
        self.get(key).and_then(|v| v.as_u64()).map(|n| n as u32)
    }

    fn param_bool(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(|v| v.as_bool())
    }

    fn require_str(&self, key: &str) -> Result<String, AppError> {
        self.param_str(key)
            .ok_or_else(|| AppError::BadRequest(format!("{key} required")))
    }

    fn require_f64(&self, key: &str) -> Result<f64, AppError> {
        self.param_f64(key)
            .ok_or_else(|| AppError::BadRequest(format!("{key} required")))
    }
}