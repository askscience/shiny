use axum::extract::{Extension, State};
use axum::response::{IntoResponse, Sse};
use axum::response::sse::{Event, KeepAlive};
use axum::Json;
use futures::stream;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use tokio_stream::StreamExt as _;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;
use crate::services::agent_runner::{run_agent, AgentRunInput, AgentRunResult};
use crate::services::agent_tools::{fetch_active_trip, AgentContext};
use crate::services::artifacts::Artifact;
use crate::services::navigation::NavigationSession;

#[derive(Deserialize)]
pub struct AgentRequest {
    pub message: String,
    pub mode: Option<String>,
    pub lang: Option<String>,
    pub ai_name: Option<String>,
    pub ollama_model: Option<String>,
    pub stream: Option<bool>,
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
    pub steps: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub navigation: Option<NavigationSession>,
}

#[derive(Serialize)]
pub struct ActionTaken {
    pub action: String,
    pub result: String,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum AgentStreamEvent {
    Step { message: String },
    Done { data: AgentResponse },
    Error { message: String },
}

fn load_core_skill() -> String {
    std::fs::read_to_string("web/skills/core-assistant.md")
        .unwrap_or_else(|_| "Use JSON action blocks.".into())
}

fn first_name(full: &str) -> String {
    full.split_whitespace()
        .next()
        .unwrap_or(full)
        .to_string()
}

struct PreparedAgent {
    input: AgentRunInput,
    trip_id: Option<String>,
}

async fn prepare_agent(
    state: &AppState,
    traveler: &Traveler,
    body: AgentRequest,
) -> Result<PreparedAgent, AppError> {
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
        ollama_model: body
            .ollama_model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from),
    };

    let active_trip = if state.plugins.is_enabled_for(&traveler.id, "traveler").await {
        fetch_active_trip(&state.pool, &traveler.id).await?
    } else {
        None
    };
    let recent_diary = if state.plugins.is_enabled_for(&traveler.id, "traveler").await {
        sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT date, summary FROM diary_entries WHERE traveler_id = ?1 ORDER BY date DESC LIMIT 3",
        )
        .bind(&traveler.id)
        .fetch_all(&state.pool)
        .await?
    } else {
        Vec::new()
    };

    // Per-user active plugin set — drives which plugins' skills / persona /
    // context lines enter the system prompt.
    let active_set = state.plugins.disabled_for(&traveler.id).await;
    let installed: std::collections::BTreeSet<String> = state
        .plugins
        .list()
        .into_iter()
        .map(|m| m.name)
        .filter(|n| !active_set.contains(n))
        .collect();

    // Skill markdown = the core assistant reference (always-on tools only)
    // PLUS whatever skills the user's active plugins advertise. Plugin-owned
    // domains (e.g. traveler) document their own tools via their skills.
    let core_skill = load_core_skill();
    let plugin_skill = state.plugins.skills_markdown_for(&installed);
    let skill = if plugin_skill.trim().is_empty() {
        core_skill
    } else {
        format!("{core_skill}\n\n---\n\n{plugin_skill}")
    };

    // Persona concat for the active set; fallback to a neutral helpful
    // assistant persona when no plugin is active.
    let plugin_persona = state.plugins.persona_concat_for(&installed);
    let persona = if plugin_persona.trim().is_empty() {
        "a helpful AI assistant".to_string()
    } else {
        plugin_persona
    };

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
            .map(|(date, summary)| format!("{}: {}", date, summary.as_deref().unwrap_or("")))
            .collect::<Vec<_>>()
            .join("; ")
    };

    let ai_name = body
        .ai_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Shiny")
        .to_string();
    let user_first = first_name(&traveler.name);

    let context_lines = state.plugins.context_lines_for(&installed);
    let context_block = if context_lines.is_empty() {
        String::new()
    } else {
        format!("\n{}\n", context_lines.join("\n"))
    };

    let system = format!(
        "You are {ai_name}, {persona}. Reply in language code '{lang}'. Keep spoken replies to 1-2 short sentences.\n\
         The user may wake you by saying \"hey {ai_lower}\".\n\
         Address the user as {user_first} when it feels natural.\n\
         \n\
         ## Tool protocol (strict)\n\
         - Call exactly ONE tool per turn.\n\
         - Output ONLY raw JSON on its own line — no markdown fences.\n\
         - Format: {{\"action\":\"tool_name\",\"params\":{{...}}}}\n\
         - Always include \"params\". Use {{}} when a tool has no parameters.\n\
         \n\
         ## Tools\n{skill}\n\n\
         ## Context\nUser name: {user_first}\n{location_line}\n{trip_line}\nDiary: {diary_line}{context_block}\n\
         Mode: {mode} — keep the spoken reply short.",
        ai_name = ai_name,
        persona = persona,
        lang = lang,
        ai_lower = ai_name.to_lowercase(),
        user_first = user_first,
        skill = skill,
        location_line = location_line,
        trip_line = trip_line,
        diary_line = diary_line,
        context_block = context_block,
        mode = mode,
    );

    Ok(PreparedAgent {
        trip_id: active_trip.as_ref().map(|t| t.id.clone()),
        input: AgentRunInput {
            message: body.message,
            mode,
            lang,
            ai_name,
            system,
            ctx,
        },
    })
}

fn to_response(result: AgentRunResult) -> AgentResponse {
    AgentResponse {
        success: result.success,
        reply: result.reply,
        mode: result.mode,
        artifacts: result.artifacts,
        actions_taken: result
            .actions_taken
            .into_iter()
            .map(|a| ActionTaken {
                action: a.action,
                result: a.result,
            })
            .collect(),
        steps: result.steps,
        navigation: result.navigation,
    }
}

pub async fn handle_agent(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(body): Json<AgentRequest>,
) -> Result<Json<AgentResponse>, AppError> {
    let prepared = prepare_agent(&state, &traveler, body).await?;
    let trip_id = prepared.trip_id.clone();

    let result = run_agent(
        &state,
        &traveler,
        trip_id.as_deref(),
        prepared.input,
        |_| {},
    )
    .await?;

    Ok(Json(to_response(result)))
}

pub async fn handle_agent_stream(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(body): Json<AgentRequest>,
) -> Result<Sse<impl stream::Stream<Item = Result<Event, Infallible>>>, AppError> {
    let prepared = prepare_agent(&state, &traveler, body).await?;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    let state = state.clone();
    let traveler = traveler.clone();
    let trip_id = prepared.trip_id.clone();
    let input = prepared.input;

    tokio::spawn(async move {
        let emit = |event: AgentStreamEvent| {
            if let Ok(json) = serde_json::to_string(&event) {
                let _ = tx.send(json);
            }
        };

        match run_agent(
            &state,
            &traveler,
            trip_id.as_deref(),
            input,
            |msg| emit(AgentStreamEvent::Step {
                message: msg.to_string(),
            }),
        )
        .await
        {
            Ok(result) => emit(AgentStreamEvent::Done {
                data: to_response(result),
            }),
            Err(e) => emit(AgentStreamEvent::Error {
                message: e.to_string(),
            }),
        }
    });

    let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx)
        .map(|payload| Ok(Event::default().data(payload)));

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

pub async fn handle_agent_dispatch(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(body): Json<AgentRequest>,
) -> Result<axum::response::Response, AppError> {
    if body.stream.unwrap_or(false) {
        let sse = handle_agent_stream(State(state), Extension(traveler), Json(body)).await?;
        Ok(sse.into_response())
    } else {
        let json = handle_agent(State(state), Extension(traveler), Json(body)).await?;
        Ok(json.into_response())
    }
}
