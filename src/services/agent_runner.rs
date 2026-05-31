//! Shared agent execution loop with step callbacks.

use serde_json::json;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;
use crate::services::agent_steps::{
    build_continuation_messages, build_final_reply_messages, build_planning_messages,
    describe_tool_step, messages_char_count, step_label_for_action,
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
    pub artifacts: Vec<Artifact>,
    pub actions_taken: Vec<ActionTaken>,
    pub navigation: Option<NavigationSession>,
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
    let mut artifacts: Vec<Artifact> = Vec::new();
    let mut actions_taken: Vec<ActionTaken> = Vec::new();
    let mut navigation: Option<NavigationSession> = None;
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
                &input.message,
                &completed_steps,
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
            if completed_steps.is_empty() {
                final_reply = strip_action_blocks(&response);
                if final_reply.is_empty() {
                    final_reply = response.trim().to_string();
                }
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
                    )
                    .await
                    {
                        tracing::warn!("Failed to autosave artifact: {}", e);
                    }
                    artifacts.push(art);
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

    if final_reply.is_empty() && !completed_steps.is_empty() {
        on_step("Preparing reply…");
        let messages = build_final_reply_messages(
            &input.ai_name,
            &input.lang,
            &input.message,
            &completed_steps,
        );
        final_reply = state
            .ollama
            .chat(messages, input.ctx.ollama_model.as_deref())
            .await?
            .trim()
            .to_string();
    }

    if final_reply.is_empty() {
        final_reply = "Done.".into();
    }

    Ok(AgentRunResult {
        success: true,
        reply: final_reply,
        mode: input.mode,
        artifacts,
        actions_taken,
        navigation,
        steps: completed_steps,
    })
}
