use axum::extract::{Extension, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;
use crate::services::agent_tools::{
    execute_action, fetch_active_trip, parse_actions, strip_action_blocks, AgentContext,
};
use crate::services::artifacts::Artifact;
use crate::services::navigation::NavigationSession;

const MAX_ITERATIONS: usize = 4;

#[derive(Deserialize)]
pub struct AgentRequest {
    pub message: String,
    pub mode: Option<String>,
    pub lang: Option<String>,
    pub context: Option<AgentContextBody>,
}

#[derive(Deserialize)]
pub struct AgentContextBody {
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub heading: Option<f64>,
}

#[derive(Serialize)]
pub struct AgentResponse {
    pub success: bool,
    pub reply: String,
    pub mode: String,
    pub artifacts: Vec<Artifact>,
    pub actions_taken: Vec<ActionTaken>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub navigation: Option<NavigationSession>,
}

#[derive(Serialize)]
pub struct ActionTaken {
    pub action: String,
    pub result: String,
}

fn load_skill_reference() -> String {
    std::fs::read_to_string("web/skills/traveler-api-tools.md")
        .unwrap_or_else(|_| "Use JSON action blocks.".into())
}

pub async fn handle_agent(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(body): Json<AgentRequest>,
) -> Result<Json<AgentResponse>, AppError> {
    let mode = body.mode.unwrap_or_else(|| "single".into());
    let lang = body.lang.unwrap_or_else(|| "en".into());
    let ctx_body = body.context.unwrap_or(AgentContextBody {
        lat: None,
        lon: None,
        heading: None,
    });

    let ctx = AgentContext {
        lat: ctx_body.lat,
        lon: ctx_body.lon,
        heading: ctx_body.heading,
        lang: lang.clone(),
    };

    let active_trip = fetch_active_trip(&state.pool, &traveler.id).await?;
    let recent_diary = sqlx::query_as::<_, crate::models::DiaryEntry>(
        "SELECT * FROM diary_entries WHERE traveler_id = ?1 ORDER BY date DESC LIMIT 3",
    )
    .bind(&traveler.id)
    .fetch_all(&state.pool)
    .await?;

    let skill = load_skill_reference();
    let location_line = match (ctx.lat, ctx.lon) {
        (Some(lat), Some(lon)) => format!("User is at {:.5}, {:.5}", lat, lon),
        _ => "User location unknown".into(),
    };
    let trip_line = match &active_trip {
        Some(t) => format!("Active trip: {} ({})", t.name, t.id),
        None => "No active trip".into(),
    };
    let diary_line = if recent_diary.is_empty() {
        "No recent diary entries".into()
    } else {
        recent_diary
            .iter()
            .map(|e| format!("{}: {}", e.date, e.summary.as_deref().unwrap_or("")))
            .collect::<Vec<_>>()
            .join("; ")
    };

    let system = format!(
        "You are a travel navigator AI. Reply in language code '{}'. Keep spoken replies to 1-2 short sentences.\n\
         \n\
         ## Tool protocol (strict)\n\
         - To call a tool, output ONLY raw JSON on its own line — no markdown fences, no ```json blocks, no prose.\n\
         - Format: {{\"action\":\"tool_name\",\"params\":{{...}}}}\n\
         - Always include \"params\". Use {{}} when a tool has no parameters.\n\
         - Wait for tool results, then reply in natural language. Never show JSON to the user.\n\
         - Never claim a trip was created/started unless the tool succeeded.\n\
         - Wrong: ```json {{\"action\":\"list_trips\"}} ``` or {{\"action\":\"list_trips\"}}\n\
         - Right: {{\"action\":\"list_trips\",\"params\":{{}}}}\n\
         - For trip/itinerary requests use plan_trip (web research + route + narrative journey card plus nightlife/food/culture guides in the dock).\n\
         - For \"take me to\", \"navigate to\", \"directions to\", \"drive to\" → use navigate_to (starts turn-by-turn navigator; no artifact panel).\n\
         \n\
         ## Tools\n{}\n\n\
         ## Context\n{}\n{}\nDiary: {}\n\
         Mode: {} — plan_trip may save several dock guides at once; keep the spoken reply short and point users to the icons below the orb.\n\
         To modify an existing saved card (hours, tips, sections), use update_artifact with artifact_id — not show_artifact.",
        lang, skill, location_line, trip_line, diary_line, mode
    );

    let mut messages = vec![
        ("system".to_string(), system),
        ("user".to_string(), body.message.clone()),
    ];

    let mut artifacts: Vec<Artifact> = Vec::new();
    let mut actions_taken: Vec<ActionTaken> = Vec::new();
    let mut navigation: Option<NavigationSession> = None;
    let mut final_reply = String::new();

    for _ in 0..MAX_ITERATIONS {
        let response = state.ollama.chat(messages.clone()).await?;
        let actions = parse_actions(&response);

        if actions.is_empty() {
            final_reply = strip_action_blocks(&response);
            if final_reply.is_empty() {
                final_reply = response.trim().to_string();
            }
            break;
        }

        let mut results = Vec::new();
        for (action, params) in actions {
            match execute_action(&state, &traveler, &ctx, &action, &params).await {
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
                    let trip_id = active_trip.as_ref().map(|t| t.id.as_str());
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
                    results.push(json!({
                        "action": outcome.action,
                        "result": outcome.result,
                        "data": outcome.data,
                    }));
                }
                Err(e) => {
                    results.push(json!({
                        "action": action,
                        "result": "error",
                        "error": e.to_string(),
                    }));
                }
            }
        }

        messages.push(("assistant".to_string(), response));
        messages.push((
            "user".to_string(),
            format!(
                "Tool results: {}. Now reply briefly in '{}' for the user.",
                serde_json::to_string(&results).unwrap_or_default(),
                lang
            ),
        ));
    }

    if final_reply.is_empty() {
        if let Some(last) = messages.last() {
            if last.0 == "assistant" {
                final_reply = strip_action_blocks(&last.1);
            }
        }
    }
    if final_reply.is_empty() {
        final_reply = "Done.".into();
    }

    Ok(Json(AgentResponse {
        success: true,
        reply: final_reply,
        mode,
        artifacts,
        actions_taken,
        navigation,
    }))
}
