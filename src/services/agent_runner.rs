//! Shared agent execution loop with step callbacks.

use serde_json::{Value, json};

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;
use crate::services::agent_steps::{
    build_continuation_messages, build_planning_messages, describe_tool_step,
    messages_char_count, step_label_for_action,
};
use crate::services::agent_tools::{
    execute_action, parse_actions, strip_action_blocks, AgentContext,
};
use crate::services::artifacts::Artifact;
use crate::services::navigation::NavigationSession;

const MAX_TOOL_STEPS: usize = 10;

pub struct AgentRunInput {
    pub message: String,
    pub mode: String,
    pub lang: String,
    pub ai_name: String,
    pub system: String,
    /// Compact plugin catalog line for continuation prompts (e.g.
    /// "traveler: Trip tracking…; hello: Demo…"). Empty when no plugins.
    pub plugins_hint: String,
    pub ctx: AgentContext,
}

pub struct ActionTaken {
    pub action: String,
    pub result: String,
}

pub struct AgentRunResult {
    pub success: bool,
    pub reply: String,
    pub mode: String,
    /// Response artifacts as tagged JSON values (include the `plugin` key so
    /// the UI can group surfaces by plugin without a refetch).
    pub artifacts: Vec<Value>,
    pub actions_taken: Vec<ActionTaken>,
    pub navigation: Option<NavigationSession>,
    /// Plugin the AI chose to surface (via the show_plugin tool).
    pub focus_plugin: Option<String>,
    pub steps: Vec<String>,
}

pub async fn run_agent<F>(
    state: &AppState,
    traveler: &Traveler,
    trip_id: Option<&str>,
    input: AgentRunInput,
    mut on_step: F,
) -> Result<AgentRunResult, AppError>
where
    F: FnMut(&str),
{
    let mut artifacts: Vec<Value> = Vec::new();
    let mut actions_taken: Vec<ActionTaken> = Vec::new();
    let mut navigation: Option<NavigationSession> = None;
    let mut focus_plugin: Option<String> = None;
    let mut produced_owners: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut completed_steps: Vec<String> = Vec::new();
    let mut final_reply = String::new();

    on_step("Thinking…");

    for iteration in 0..MAX_TOOL_STEPS {
        let messages = if completed_steps.is_empty() && iteration == 0 {
            build_planning_messages(&input.system, &input.message)
        } else {
            build_continuation_messages(
                &input.ai_name,
                &input.lang,
                &input.mode,
                &input.message,
                &completed_steps,
                &input.plugins_hint,
            )
        };

        let size = messages_char_count(&messages);
        tracing::debug!("Agent Ollama call #{iteration}: ~{size} chars");
        if size > 200_000 {
            tracing::warn!("Agent prompt very large ({size} chars), continuing with slim context");
        }

        let response = state
            .ollama
            .chat(messages, input.ctx.ollama_model.as_deref())
            .await?;
        let actions = parse_actions(&response);

        if actions.is_empty() {
            // The model replied in plain language — that IS the final reply
            // (tool results already reached it via the [Done] step notes).
            final_reply = strip_action_blocks(&response);
            if final_reply.is_empty() {
                final_reply = response.trim().to_string();
            }
            break;
        }

        let (action, params) = actions
            .into_iter()
            .next()
            .ok_or_else(|| AppError::Internal("Empty tool action list".into()))?;

        on_step(step_label_for_action(&action));

        match execute_action(state, traveler, &input.ctx, &action, &params).await {
            Ok(outcome) => {
                actions_taken.push(ActionTaken {
                    action: outcome.action.clone(),
                    result: outcome.result.clone(),
                });

                if outcome.action == "navigate_to" && outcome.result == "ok" {
                    if let Ok(nav) = serde_json::from_value::<NavigationSession>(
                        outcome.data.get("navigator").cloned().unwrap_or(json!({})),
                    ) {
                        navigation = Some(nav);
                    }
                }

                if outcome.action == "show_plugin" && outcome.result == "ok" {
                    if let Some(p) = outcome.data.get("plugin").and_then(|v| v.as_str()) {
                        focus_plugin = Some(p.to_string());
                    }
                }

                let mut produced: Vec<Artifact> = Vec::new();
                if let Some(art) = outcome.artifact {
                    produced.push(art);
                }
                produced.extend(outcome.extra_artifacts);
                for art in produced {
                    if let Err(e) = crate::services::artifacts::save_artifact(
                        &state.pool,
                        &traveler.id,
                        trip_id,
                        &art,
                        outcome.owner.as_deref(),
                    )
                    .await
                    {
                        tracing::warn!("Failed to autosave artifact: {}", e);
                    }
                    // Deterministic surface tracking: the plugin that produced
                    // cards gets surfaced even if the model skips show_plugin.
                    if let Some(owner) = &outcome.owner {
                        if owner != "core" {
                            produced_owners.insert(owner.clone());
                        }
                    }
                    // Tag the response payload so the UI can group by plugin
                    // without refetching the summary list.
                    let mut v = serde_json::to_value(&art).unwrap_or_else(|_| json!({}));
                    if let (Some(owner), Value::Object(map)) = (&outcome.owner, &mut v) {
                        map.insert("plugin".into(), json!(owner));
                    }
                    artifacts.push(v);
                }

                let note = describe_tool_step(&outcome.action, &outcome.result, &outcome.data);
                completed_steps.push(note.clone());
                on_step(&note);
            }
            Err(e) => {
                actions_taken.push(ActionTaken {
                    action: action.clone(),
                    result: "error".into(),
                });
                let note = describe_tool_step(&action, "error", &json!({ "error": e.to_string() }));
                completed_steps.push(note.clone());
                on_step(&note);
            }
        }
    }

    if final_reply.is_empty() {
        final_reply = "Done.".into();
    }

    // Fallback surface: cards came from exactly one plugin — show its window
    // even when the model didn't call show_plugin explicitly.
    if focus_plugin.is_none() && produced_owners.len() == 1 {
        focus_plugin = produced_owners.into_iter().next();
    }

    Ok(AgentRunResult {
        success: true,
        reply: final_reply,
        mode: input.mode,
        artifacts,
        actions_taken,
        navigation,
        focus_plugin,
        steps: completed_steps,
    })
}
