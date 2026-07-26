//! Runtime tool registry — maps action keys to `Arc<dyn Tool>` instances.

use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;

use shiny_plugin_sdk::context::AgentContext;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{Tool, ToolRequest, normalize_action_name, ParamHelpers};
use serde_json::Value;

use std::collections::BTreeSet;

use crate::plugins::manager::PluginManager;

/// Mutable registry of installed tools. Cheap clone via `Arc`.
#[derive(Clone, Default)]
pub struct ToolRegistry {
    /// key: normalized action key (lowercased, alias-mapped)
    tools: Arc<RwLock<HashMap<String, Arc<dyn Tool>>>>,
    /// Mapping from action key → owning plugin name, so we can check `is_enabled`.
    owner: Arc<RwLock<HashMap<String, String>>>,
    /// Reference to the plugin manager for activation lookups. None for tests.
    manager: Arc<RwLock<Option<PluginManager>>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn attach_manager(&self, m: PluginManager) {
        *self.manager.write() = Some(m);
    }

    pub fn install_owned(&self, tool: Arc<dyn Tool>, plugin_name: &str) {
        let mut map = self.tools.write();
        let mut owner = self.owner.write();
        let key = normalize_action_name(tool.name());
        owner.insert(key.clone(), plugin_name.to_string());
        map.insert(key, tool.clone());
        for alias in tool.aliases() {
            let alias_key = normalize_action_name(alias);
            owner.insert(alias_key.clone(), plugin_name.to_string());
            map.insert(alias_key, tool.clone());
        }
    }

    pub fn install(&self, tool: Arc<dyn Tool>) {
        self.install_owned(tool, "");
    }

    pub fn uninstall(&self, tool_name: &str) {
        let mut map = self.tools.write();
        let mut owner = self.owner.write();
        let key = normalize_action_name(tool_name);
        map.remove(&key);
        owner.remove(&key);
        map.retain(|_, v| v.name() != tool_name);
        owner.retain(|_, v| v != tool_name);
    }

    pub fn list(&self) -> Vec<String> {
        let map = self.tools.read();
        let mut keys: Vec<String> = map.keys().cloned().collect();
        keys.sort();
        keys
    }

    pub fn has(&self, name: &str) -> bool {
        let map = self.tools.read();
        map.contains_key(&normalize_action_name(name))
    }

    pub async fn invoke(
        &self,
        action: &str,
        ctx: &PluginCtx,
        user_id: &str,
        traveler_id: &str,
        params: &Value,
        agent_ctx: &AgentContext,
    ) -> Result<ActionOutcome, AppError> {
        let normalized = normalize_action_name(action);
        let tool = {
            let map = self.tools.read();
            map.get(&normalized)
                .cloned()
                .ok_or_else(|| AppError::BadRequest(format!("Unknown action: {}", action)))?
        };

        // Check the owning plugin's activation state for this user. Inner-built-in
        // tools (owner = "") are always enabled; they belong to core.
        let owner = {
            let owner_map = self.owner.read();
            owner_map.get(&normalized).cloned().unwrap_or_default()
        };
        if !owner.is_empty() {
            let m = self.manager.read().clone();
            if let Some(m) = m {
                if !m.is_enabled_for(user_id, &owner).await {
                    return Err(AppError::BadRequest(format!(
                        "plugin '{}' is deactivated for this user", owner
                    )));
                }
            }
        }

        let req = ToolRequest {
            user_id,
            traveler_id,
            params,
            ctx: agent_ctx,
        };
        tool.invoke(ctx, req).await
    }

    /// Markdown concat of every registered tool's `doc_fragment`. Tools owned
    /// by a plugin not in the user's active set are omitted.
    pub fn skills_markdown_for(&self, active: &BTreeSet<String>) -> String {
        let map = self.tools.read();
        let owner_map = self.owner.read();
        let mut seen = std::collections::HashSet::new();
        let mut out = String::new();
        for (key, tool) in map.iter() {
            if !seen.insert(tool.name()) {
                continue;
            }
            let owner = owner_map.get(key).cloned().unwrap_or_default();
            if !owner.is_empty() && !active.contains(&owner) {
                continue;
            }
            if let Some(doc) = tool.doc_fragment() {
                out.push_str(doc);
                out.push('\n');
            }
        }
        out
    }

    /// Look up the step label for an action key (or generic fallback).
    pub fn step_label(&self, action: &str) -> String {
        let map = self.tools.read();
        if let Some(t) = map.get(&normalize_action_name(action)) {
            t.step_label().to_string()
        } else {
            "Working…".to_string()
        }
    }

    /// Build the human-readable "completed" note for a result.
    pub fn humanize(&self, action: &str, result: &str, data: &Value) -> String {
        if result == "error" {
            let msg = data
                .get("error")
                .or_else(|| data.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return format!("{action} failed: {msg}");
        }
        let map = self.tools.read();
        if let Some(t) = map.get(&normalize_action_name(action)) {
            t.humanize(result, data)
        } else {
            format!("{action} complete")
        }
    }
}

// Re-export helpers for internal use against `serde_json::Value`.
pub use shiny_plugin_sdk::tools::ParamHelpers as _SdkParamHelpers;