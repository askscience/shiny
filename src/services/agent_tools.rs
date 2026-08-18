use serde_json::{json, Value};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::{Traveler, Trip};
use crate::services::artifacts::{self, Artifact};
use crate::services::web_search::SearchResult;

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
    let action_key = normalize_action_name(action);

    // Plugin registry takes priority — if a plugin claimed this action key,
    // dispatch to its `Tool::invoke`. The registry itself consults the per-user
    // activation set, so deactivated plugins refuse here.
    if state.plugins.tools().has(&action_key) {
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
        });
    }

    let traveler_active = state.plugins.is_enabled_for(&traveler.id, "traveler").await;

    let outcome = match action_key.as_str() {
        "web_search" => {
            let query = param_str(params, "query").ok_or_else(|| AppError::BadRequest("query required".into()))?;
            let results = state.search.search(&query).await?;
            let summary = if state.ollama.is_available().await {
                summarize_search_stepwise(
                    state,
                    &results,
                    &ctx.lang,
                    ctx.ollama_model.as_deref(),
                )
                .await
            } else {
                None
            };
            // Chat-only mode: without the traveler plugin there is no card
            // surface — the answer travels in `data` and spoken prose only.
            let artifact = if traveler_active {
                Some(Artifact {
                    id: Uuid::new_v4().to_string(),
                    artifact_type: "site_info".into(),
                    title: query.clone(),
                    subtitle: summary.clone(),
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
                data: json!({ "results": results, "summary": summary }),
                artifact,
                extra_artifacts: vec![],
            }
        }
        other => {
            tracing::warn!("Unknown agent action: {:?}", other);
            return Err(AppError::BadRequest(format!("Unknown action: {}", other)));
        }
    };

    Ok(outcome)
}

async fn summarize_search_stepwise(
    state: &AppState,
    results: &[SearchResult],
    lang: &str,
    model: Option<&str>,
) -> Option<String> {
    let mut lines = Vec::new();
    for row in results.iter().take(4) {
        let prompt = format!(
            "Summarize this search hit in one short sentence for language '{}': {} — {}",
            lang, row.title, row.snippet
        );
        if let Ok(line) = state.ollama.generate(&prompt, None, model).await {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                lines.push(trimmed.to_string());
            }
        }
    }
    if lines.is_empty() {
        return None;
    }
    if lines.len() == 1 {
        return Some(lines.pop()?);
    }
    let merge_prompt = format!(
        "Combine these search notes into 2-3 sentences for language '{}':\n{}",
        lang,
        lines.join("\n")
    );
    state.ollama.generate(&merge_prompt, None, model).await.ok()
}

fn param_str(params: &Value, key: &str) -> Option<String> {
    params.get(key).and_then(|v| v.as_str()).map(String::from)
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
    for (action, params) in parse_actions(text) {
        let compact = json!({ "action": action, "params": params }).to_string();
        result = result.replace(&compact, "");

        let pretty = serde_json::to_string_pretty(&json!({ "action": action, "params": params }))
            .unwrap_or_default();
        result = result.replace(&pretty, "");
    }

    result = result
        .replace("```json", "")
        .replace("```JSON", "")
        .replace("```", "");

    result
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}
