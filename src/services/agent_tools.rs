use serde_json::{json, Value};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::{Traveler, Trip};
use crate::services::artifacts::{self, Artifact};

use shiny_plugin_sdk::outcome::ActionOutcome as SdkActionOutcome;

// `AgentContext` now lives in the SDK so plugins can receive the same type.
// Re-export keeps existing call sites (`crate::services::agent_tools::AgentContext`)
// in the binary working.
pub use shiny_plugin_sdk::context::AgentContext;

#[derive(Debug, Clone)]
pub struct ActionOutcome {
    pub action: String,
    pub result: String,
    pub data: Value,
    pub artifact: Option<Artifact>,
    pub extra_artifacts: Vec<Artifact>,
    /// Plugin that produced this outcome ("" / None = core). Used to tag
    /// saved artifacts so the UI can group surfaces by plugin.
    pub owner: Option<String>,
}

/// Dispatch an LLM-emitted action. Core is a simple AI assistant: the only
/// built-in tool is `web_search`. Everything else (trips, GPS, maps, diary,
/// navigation, artifact cards) is contributed by plugins — the traveler
/// plugin owns that domain.
pub async fn execute_action(
    state: &AppState,
    traveler: &Traveler,
    ctx: &AgentContext,
    action: &str,
    params: &Value,
) -> Result<ActionOutcome, AppError> {
    let mut action_key = normalize_action_name(action);

    // The model occasionally namespaces tool names ("word.doc_write",
    // "radio.radio_stop"): when the raw key isn't registered, retry with the
    // segment after the first dot — it usually matches a real tool name.
    if !state.plugins.tools().has(&action_key) {
        if let Some((_, rest)) = action_key.split_once('.') {
            let candidate = normalize_action_name(rest);
            if state.plugins.tools().has(&candidate) {
                action_key = candidate;
            }
        }
    }

    // Plugin registry takes priority — if a plugin claimed this action key,
    // dispatch to its `Tool::invoke`. The registry itself consults the per-user
    // activation set, so deactivated plugins refuse here.
    if state.plugins.tools().has(&action_key) {
        let owner = state.plugins.tools().owner_of(&action_key);
        let plugin_ctx = state.plugin_ctx();
        let outcome = state
            .plugins
            .tools()
            .invoke(&action_key, &plugin_ctx, &traveler.id, &traveler.id, params, ctx)
            .await?;
        return Ok(ActionOutcome {
            action: outcome.action,
            result: outcome.result,
            data: outcome.data,
            artifact: outcome.artifact,
            extra_artifacts: outcome.extra_artifacts,
            owner: if owner.is_empty() { None } else { Some(owner) },
        });
    }

    // No plugin claimed the verb. If it belongs to the traveler domain, refuse
    // with a clear message (plugin uninstalled, or deactivated for this user).
    let is_traveler_verb = matches!(action_key.as_str(),
        "create_trip" | "list_trips" | "get_trip" | "get_active_trip" | "start_trip"
        | "end_trip" | "trip_stats" | "submit_location" | "list_locations"
        | "trip_route" | "map_search" | "map_reverse" | "map_route"
        | "navigate_to" | "map_poi" | "list_diary" | "get_diary"
        | "search_diary" | "generate_diary" | "plan_trip"
        | "show_artifact" | "update_artifact"
    );
    if is_traveler_verb {
        return Ok(ActionOutcome {
            action: action_key.clone(),
            result: "error".into(),
            data: json!({ "error": "plugin 'traveler' is deactivated for this user" }),
            artifact: None,
            extra_artifacts: vec![],
            owner: None,
        });
    }

    let traveler_active = state
        .plugins
        .session_active_plugin_enabled(&traveler.id, "traveler")
        .await;

    let outcome = match action_key.as_str() {
        // Surface a plugin's window (tile or full screen, per user preference).
        // The frontend receives `focus_plugin` in the agent response.
        "show_plugin" => {
            let name = param_str(params, "name")
                .or_else(|| param_str(params, "plugin"))
                .ok_or_else(|| AppError::BadRequest("name required".into()))?;
            let installed = state.plugins.list().iter().any(|m| m.name == name);
            if installed && state.plugins.session_active_plugin_enabled(&traveler.id, &name).await {
                ActionOutcome {
                    action: action_key.clone(),
                    result: "ok".into(),
                    data: json!({ "plugin": name }),
                    artifact: None,
                    extra_artifacts: vec![],
                    owner: None,
                }
            } else if !installed {
                ActionOutcome {
                    action: action_key.clone(),
                    result: "error".into(),
                    data: json!({ "error": format!("plugin '{name}' is not installed") }),
                    artifact: None,
                    extra_artifacts: vec![],
                    owner: None,
                }
            } else {
                ActionOutcome {
                    action: action_key.clone(),
                    result: "error".into(),
                    data: json!({ "error": format!("plugin '{name}' is inactive — activate it with plugin_activate first") }),
                    artifact: None,
                    extra_artifacts: vec![],
                    owner: None,
                }
            }
        }
        // Activate a plugin for this user so its tools become available.
        "plugin_activate" => {
            let name = param_str(params, "name")
                .or_else(|| param_str(params, "plugin"))
                .ok_or_else(|| AppError::BadRequest("name required".into()))?;
            let installed = state.plugins.list().iter().any(|m| m.name == name);
            if !installed {
                ActionOutcome {
                    action: action_key.clone(),
                    result: "error".into(),
                    data: json!({ "error": format!("plugin '{name}' is not installed") }),
                    artifact: None,
                    extra_artifacts: vec![],
                    owner: None,
                }
            } else if fresh_session(state, &traveler.id).await {
                ActionOutcome {
                    action: action_key.clone(),
                    result: "error".into(),
                    data: json!({ "error": "Plugins are off this session — turn on 'Remember workspace' in Settings to use plugins" }),
                    artifact: None,
                    extra_artifacts: vec![],
                    owner: None,
                }
            } else {
                let already = state.plugins.is_enabled_for(&traveler.id, &name).await;
                state.plugins.set_enabled_for(&traveler.id, &name, true).await?;
                ActionOutcome {
                    action: action_key.clone(),
                    result: "ok".into(),
                    data: json!({ "plugin": name, "enabled": true, "already": already }),
                    artifact: None,
                    extra_artifacts: vec![],
                    owner: None,
                }
            }
        }
        // Deactivate a plugin for this user — its tools stop working.
        "plugin_deactivate" => {
            let name = param_str(params, "name")
                .or_else(|| param_str(params, "plugin"))
                .ok_or_else(|| AppError::BadRequest("name required".into()))?;
            let installed = state.plugins.list().iter().any(|m| m.name == name);
            if !installed {
                ActionOutcome {
                    action: action_key.clone(),
                    result: "error".into(),
                    data: json!({ "error": format!("plugin '{name}' is not installed") }),
                    artifact: None,
                    extra_artifacts: vec![],
                    owner: None,
                }
            } else if fresh_session(state, &traveler.id).await {
                ActionOutcome {
                    action: action_key.clone(),
                    result: "error".into(),
                    data: json!({ "error": "Plugins are off this session — turn on 'Remember workspace' in Settings to use plugins" }),
                    artifact: None,
                    extra_artifacts: vec![],
                    owner: None,
                }
            } else {
                let already = !state.plugins.is_enabled_for(&traveler.id, &name).await;
                state.plugins.set_enabled_for(&traveler.id, &name, false).await?;
                ActionOutcome {
                    action: action_key.clone(),
                    result: "ok".into(),
                    data: json!({ "plugin": name, "enabled": false, "already": already }),
                    artifact: None,
                    extra_artifacts: vec![],
                    owner: None,
                }
            }
        }
        // Every installed plugin with its activation status — lets the model
        // re-check the catalog mid-conversation (e.g. after activating).
        "list_plugins" => {
            let plugins: Vec<Value> = {
                let mut out = Vec::new();
                for m in state.plugins.list() {
                    let active = state
                        .plugins
                        .session_active_plugin_enabled(&traveler.id, &m.name)
                        .await;
                    out.push(json!({
                        "name": m.name,
                        "active": active,
                        "description": m.description.clone().or_else(|| m.summary.clone()).unwrap_or_default(),
                    }));
                }
                out
            };
            ActionOutcome {
                action: action_key.clone(),
                result: "ok".into(),
                data: json!({ "plugins": plugins }),
                artifact: None,
                extra_artifacts: vec![],
                owner: None,
            }
        }
        // ── Desktop control (Hyprland-style window/workspace management) ──
        // These are relay tools: the desktop state lives in the browser per
        // traveler. The server validates the plugin name and passes the intent
        // back through `actions_taken` so the frontend desktop manager applies
        // it — the same path `show_plugin` uses for `focus_plugin`.
        "desktop_fullscreen" => {
            let name = param_str(params, "name")
                .or_else(|| param_str(params, "plugin"))
                .ok_or_else(|| AppError::BadRequest("name required".into()))?;
            if let Some(err) = active_plugin_error(state, &traveler.id, &name, &action_key).await {
                return Ok(err);
            }
            let on = params.get("on").and_then(|v| v.as_bool()).unwrap_or(true);
            ActionOutcome {
                action: action_key.clone(),
                result: "ok".into(),
                data: json!({ "plugin": name, "fullscreen": on }),
                artifact: None,
                extra_artifacts: vec![],
                owner: None,
            }
        }
        "desktop_focus" => {
            let name = param_str(params, "name")
                .or_else(|| param_str(params, "plugin"))
                .ok_or_else(|| AppError::BadRequest("name required".into()))?;
            if let Some(err) = active_plugin_error(state, &traveler.id, &name, &action_key).await {
                return Ok(err);
            }
            ActionOutcome {
                action: action_key.clone(),
                result: "ok".into(),
                data: json!({ "plugin": name }),
                artifact: None,
                extra_artifacts: vec![],
                owner: None,
            }
        }
        "workspace_create" => ActionOutcome {
            action: action_key.clone(),
            result: "ok".into(),
            data: json!({ "created": true }),
            artifact: None,
            extra_artifacts: vec![],
            owner: None,
        },
        "workspace_remove" => ActionOutcome {
            action: action_key.clone(),
            result: "ok".into(),
            data: json!({ "removed": true }),
            artifact: None,
            extra_artifacts: vec![],
            owner: None,
        },
        "workspace_switch" => {
            let to = params
                .get("to")
                .map(|v| workspace_target(v, "next"))
                .unwrap_or_else(|| "next".to_string());
            ActionOutcome {
                action: action_key.clone(),
                result: "ok".into(),
                data: json!({ "workspace": to }),
                artifact: None,
                extra_artifacts: vec![],
                owner: None,
            }
        }
        "workspace_move" => {
            let name = param_str(params, "name")
                .or_else(|| param_str(params, "plugin"))
                .ok_or_else(|| AppError::BadRequest("name required".into()))?;
            if let Some(err) = active_plugin_error(state, &traveler.id, &name, &action_key).await {
                return Ok(err);
            }
            // `to` accepts a 1-based number, a numeric string, or "new" to
            // spin up a fresh workspace. Missing/unknown targets default to
            // "new" so "move X to a new workspace" works in a single call.
            let to = params
                .get("to")
                .map(|v| workspace_target(v, "new"))
                .unwrap_or_else(|| "new".to_string());
            ActionOutcome {
                action: action_key.clone(),
                result: "ok".into(),
                data: json!({ "plugin": name, "workspace": to }),
                artifact: None,
                extra_artifacts: vec![],
                owner: None,
            }
        }
        "web_search" => {
            let query = param_str(params, "query").ok_or_else(|| AppError::BadRequest("query required".into()))?;
            // The Instant Answer API is empty for most queries — use the HTML
            // results endpoint first and fall back to it for instant answers.
            let mut results = state.search.search_html(&query, 5).await?;
            if results.is_empty() {
                results = state.search.search(&query).await?;
            }
            // Chat-only mode: without the traveler plugin there is no card
            // surface — the answer travels in `data` and spoken prose only.
            let artifact = if traveler_active {
                Some(Artifact {
                    id: Uuid::new_v4().to_string(),
                    artifact_type: "site_info".into(),
                    title: query.clone(),
                    subtitle: results.first().map(|r| r.snippet.clone()),
                    coordinates: None,
                    sections: results.iter().take(4).map(|r| artifacts::ArtifactSection {
                        label: r.title.clone(),
                        value: r.snippet.clone(),
                    }).collect(),
                    actions: vec![],
                    days: vec![],
                    route: None,
                    geometry: vec![],
                    narrative: None,
                    theme: None,
                    destination: None,
                })
            } else {
                None
            };
            ActionOutcome {
                action: action_key.clone(),
                result: "ok".into(),
                data: json!({ "results": results }),
                artifact,
                extra_artifacts: vec![],
                owner: Some("core".into()),
            }
        }
        other => {
            tracing::warn!("Unknown agent action: {:?}", other);
            return Err(AppError::BadRequest(format!("Unknown action: {}", other)));
        }
    };

    Ok(outcome)
}

fn param_str(params: &Value, key: &str) -> Option<String> {
    params.get(key).and_then(|v| v.as_str()).map(String::from)
}

/// Normalize a workspace target: 1-based number or numeric string → 0-based
/// index string; "new" passes through; anything else falls back to `default`.
fn workspace_target(v: &Value, default: &str) -> String {
    if let Some(n) = v.as_u64() {
        return n.saturating_sub(1).to_string();
    }
    if let Some(s) = v.as_str() {
        if s == "new" {
            return "new".to_string();
        }
        if let Ok(n) = s.parse::<u64>() {
            return n.saturating_sub(1).to_string();
        }
        return s.to_string();
    }
    default.to_string()
}

/// Error outcome for a desktop-control tool whose plugin isn't installed or
/// isn't active for this user. Returns `None` when the plugin is usable.
async fn active_plugin_error(
    state: &AppState,
    traveler_id: &str,
    name: &str,
    action: &str,
) -> Option<ActionOutcome> {
    let installed = state.plugins.list().iter().any(|m| m.name == name);
    if !installed {
        return Some(ActionOutcome {
            action: action.into(),
            result: "error".into(),
            data: json!({ "error": format!("plugin '{name}' is not installed") }),
            artifact: None,
            extra_artifacts: vec![],
            owner: None,
        });
    }
    if !state.plugins.session_active_plugin_enabled(traveler_id, name).await {
        return Some(ActionOutcome {
            action: action.into(),
            result: "error".into(),
            data: json!({ "error": format!("plugin '{name}' is inactive — activate it with plugin_activate first") }),
            artifact: None,
            extra_artifacts: vec![],
            owner: None,
        });
    }
    None
}

/// True when the user is in fresh mode (`session.remember` off) — plugins are
/// disabled for the session and cannot be managed from the agent.
async fn fresh_session(state: &AppState, traveler_id: &str) -> bool {
    !state.plugins.session_remember(traveler_id).await
}

pub async fn fetch_active_trip(pool: &SqlitePool, traveler_id: &str) -> Result<Option<Trip>, AppError> {
    Ok(sqlx::query_as::<_, Trip>(
        "SELECT * FROM trips WHERE traveler_id = ?1 AND status = 'active' LIMIT 1",
    )
    .bind(traveler_id)
    .fetch_optional(pool)
    .await?)
}

fn normalize_action_name(raw: &str) -> String {
    let a = raw.trim().trim_matches('"').to_lowercase();
    match a.as_str() {
        "navigate" | "start_navigation" | "start_navigator" | "navigation"
        | "directions" | "drive_to" | "navigate-to" | "go_to" => "navigate_to".into(),
        "activate_plugin" | "enable_plugin" => "plugin_activate".into(),
        "deactivate_plugin" | "disable_plugin" => "plugin_deactivate".into(),
        "plugins" | "list_available_plugins" | "available_plugins" => "list_plugins".into(),
        // Desktop control — the model phrases these loosely; normalize them so
        // "move the radio to a new workspace" never fails as an unknown action.
        "move_to_workspace" | "move_window" | "move_plugin"
        | "move_window_to_workspace" | "move_plugin_to_workspace" => "workspace_move".into(),
        "new_workspace" | "add_workspace" | "create_workspace" => "workspace_create".into(),
        "delete_workspace" | "close_workspace" | "remove_workspace" => "workspace_remove".into(),
        "switch_workspace" | "goto_workspace" | "go_to_workspace" => "workspace_switch".into(),
        "fullscreen" | "make_fullscreen" | "fullscreen_plugin"
        | "toggle_fullscreen" => "desktop_fullscreen".into(),
        "focus_window" | "focus_plugin" | "switch_focus" => "desktop_focus".into(),
        _ => a,
    }
}

pub fn parse_actions(text: &str) -> Vec<(String, Value)> {
    let normalized = text
        .replace("```json", "")
        .replace("```JSON", "")
        .replace("```", "");

    let mut actions = Vec::new();
    let mut search_from = 0;
    while let Some(start) = normalized[search_from..].find('{') {
        let abs_start = search_from + start;
        if let Some(end) = find_json_end(&normalized[abs_start..]) {
            let slice = &normalized[abs_start..abs_start + end + 1];
            if let Ok(v) = serde_json::from_str::<Value>(slice) {
                let action = v
                    .get("action")
                    .and_then(|a| a.as_str())
                    .or_else(|| v.get("tool").and_then(|a| a.as_str()));
                if let Some(action) = action {
                    let params = v
                        .get("params")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    actions.push((action.to_string(), params));
                }
            }
            search_from = abs_start + end + 1;
        } else {
            break;
        }
    }
    actions
}

fn find_json_end(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn strip_action_blocks(text: &str) -> String {
    let mut result = text.to_string();

    // Remove well-formed action blocks (compact + pretty JSON) exactly.
    for (action, params) in parse_actions(text) {
        let compact = json!({ "action": action, "params": params }).to_string();
        result = result.replace(&compact, "");

        let pretty = serde_json::to_string_pretty(&json!({ "action": action, "params": params }))
            .unwrap_or_default();
        result = result.replace(&pretty, "");
    }

    // The model sometimes emits MALFORMED JSON (a stray `\"` inside a cell
    // formula, an unquoted key, …) that serde_json rejects, so `parse_actions`
    // misses it and the raw tool call would leak into the visible reply. Strip
    // any remaining brace-balanced `{…}` block regardless of whether it parses.
    result = strip_balanced_braces(&result);

    result = result
        .replace("```json", "")
        .replace("```JSON", "")
        .replace("```", "");

    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Remove every `{…}` region (nesting aware) from `s`, keeping everything else.
fn strip_balanced_braces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for c in s.chars() {
        if c == '{' {
            depth += 1;
            continue;
        }
        if c == '}' && depth > 0 {
            depth -= 1;
            continue;
        }
        if depth > 0 {
            continue;
        }
        out.push(c);
    }
    out
}
