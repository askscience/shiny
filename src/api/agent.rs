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
    /// Client-side desktop layout (workspaces + which window is where), so the
    /// model can reorganize without creating empty workspaces.
    pub desktop: Option<DesktopState>,
    /// Conversation thread id — keeps the chat history so the model remembers
    /// earlier turns. Omit to start a new conversation.
    pub conversation_id: Option<String>,
}

#[derive(Deserialize)]
pub struct AgentContextBody {
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub heading: Option<f64>,
}

#[derive(Deserialize)]
pub struct DesktopState {
    pub active: Option<u32>,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceSnapshot>,
}

#[derive(Deserialize)]
pub struct WorkspaceSnapshot {
    pub index: u32,
    #[serde(default)]
    pub windows: Vec<String>,
}

#[derive(Serialize)]
pub struct AgentResponse {
    pub success: bool,
    pub reply: String,
    pub mode: String,
    /// Tagged artifact payloads (include the owning `plugin` key).
    pub artifacts: Vec<serde_json::Value>,
    pub actions_taken: Vec<ActionTaken>,
    pub steps: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub navigation: Option<NavigationSession>,
    /// Plugin the AI chose to surface in a window (via show_plugin).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus_plugin: Option<String>,
    /// Conversation thread id for the chat history (continue this chat).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

#[derive(Serialize)]
pub struct ActionTaken {
    pub action: String,
    pub result: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
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

/// "## Desktop" block describing the current workspace layout, so the model
/// knows where each window lives and can reorganize without creating empty
/// workspaces.
fn desktop_state_block(desktop: Option<&DesktopState>) -> String {
    let Some(d) = desktop else {
        return String::new();
    };
    if d.workspaces.is_empty() {
        return String::new();
    }
    let mut lines = Vec::new();
    for ws in &d.workspaces {
        let label = if ws.windows.is_empty() {
            "empty".to_string()
        } else {
            ws.windows.join(", ")
        };
        lines.push(format!("  workspace {}: {label}", ws.index));
    }
    let active = d.active.map(|a| a.to_string()).unwrap_or_else(|| "1".into());
    format!(
        "\n## Desktop (current layout)\n\
         Windows are grouped on numbered workspaces; the active workspace is {active}.\n\
         {}\n\
         To reorganize, move windows with workspace_move to an existing workspace number or \"new\". \
         Prefer moving into existing workspaces over creating empty ones.\n",
        lines.join("\n")
    )
}

struct PreparedAgent {
    input: AgentRunInput,
    trip_id: Option<String>,
    conversation_id: String,
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

    // Full plugin catalog — active AND inactive. The model reads every
    // description so it can activate an inactive plugin (plugin_activate)
    // when the request needs it, and knows which are currently usable.
    let catalog: Vec<(String, String, bool)> = state
        .plugins
        .list()
        .into_iter()
        .map(|m| {
            let active = installed.contains(&m.name);
            let desc = m
                .description
                .clone()
                .or_else(|| m.summary.clone())
                .unwrap_or_default();
            (m.name, desc, active)
        })
        .collect();

    let active_lines: Vec<String> = catalog
        .iter()
        .filter(|(_, _, a)| *a)
        .map(|(n, d, _)| format!("- {}: {}", n, d))
        .collect();
    let inactive_lines: Vec<String> = catalog
        .iter()
        .filter(|(_, _, a)| !*a)
        .map(|(n, d, _)| format!("- {}: {}", n, d))
        .collect();

    let plugin_windows_block = if active_lines.is_empty() {
        String::new()
    } else {
        format!(
            "\n## Plugin windows\n\
             Each active plugin has its own window, tiled Hyprland-style and grouped on \
             numbered workspaces (desktops). Active plugins:\n{}\n\
             When the request clearly belongs to a plugin's domain, call \
             {{\"action\":\"show_plugin\",\"params\":{{\"name\":\"<plugin>\"}}}} after using its tools — \
             this focuses and opens its window. Use desktop_fullscreen to make it full screen, \
             and workspace_create / workspace_remove / workspace_switch / workspace_move to \
             manage desktops.\n",
            active_lines.join("\n")
        )
    };

    let plugin_catalog_block = if catalog.is_empty() {
        "\n## Plugins\nNo plugins installed.".to_string()
    } else {
        format!(
            "\n## Plugins\n\
             Active plugins (tools available):\n{}\n\
             Available (inactive) plugins — activate with \
             {{\"action\":\"plugin_activate\",\"params\":{{\"name\":\"<plugin>\"}}}} if the request \
             needs one; deactivate with plugin_deactivate when the user asks to turn one off:\n{}",
            if active_lines.is_empty() { "- none".to_string() } else { active_lines.join("\n") },
            if inactive_lines.is_empty() { "- none".to_string() } else { inactive_lines.join("\n") },
        )
    };

    // Compact hint for continuation turns: every installed plugin with its
    // status so the model can still activate/deactivate mid-conversation.
    let plugins_hint = catalog
        .iter()
        .map(|(n, d, a)| {
            if *a {
                format!("{n}: {d}")
            } else {
                format!("{n}: {d} (inactive)")
            }
        })
        .collect::<Vec<_>>()
        .join("; ");

    let desktop_block = desktop_state_block(body.desktop.as_ref());

    // Conversation memory: resolve (or start) the thread and surface its recent
    // turns so the model remembers what was said earlier.
    let conversation_id = crate::services::chat_memory::resolve_conversation(
        &state.pool,
        &traveler.id,
        body.conversation_id.as_deref(),
    )
    .await?;
    let history = crate::services::chat_memory::recent_history(&state.pool, &conversation_id, 16).await?;
    let history_block = if history.is_empty() {
        String::new()
    } else {
        let lines: Vec<String> = history
            .iter()
            .map(|(role, content)| {
                let trimmed = content.trim().chars().take(500).collect::<String>();
                format!("{role}: {trimmed}")
            })
            .collect();
        format!("\n## Conversation history (remember earlier turns)\n{}\n", lines.join("\n"))
    };

    let system = format!(
        "You are {ai_name}, {persona}. Reply in language code '{lang}'. Answer completely and helpfully — be concise for simple questions, but give detail, steps, or lists whenever the answer needs them.\n\
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
         Mode: {mode} — answer fully and clearly.{history_block}{plugin_windows_block}{plugin_catalog_block}{desktop_block}",
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
        history_block = history_block,
        plugin_windows_block = plugin_windows_block,
        plugin_catalog_block = plugin_catalog_block,
        desktop_block = desktop_block,
    );

    Ok(PreparedAgent {
        trip_id: active_trip.as_ref().map(|t| t.id.clone()),
        conversation_id,
        input: AgentRunInput {
            message: body.message,
            mode,
            lang,
            ai_name,
            system,
            plugins_hint,
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
                data: a.data,
            })
            .collect(),
        steps: result.steps,
        navigation: result.navigation,
        focus_plugin: result.focus_plugin,
        conversation_id: None,
    }
}

pub async fn handle_agent(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(body): Json<AgentRequest>,
) -> Result<Json<AgentResponse>, AppError> {
    let prepared = prepare_agent(&state, &traveler, body).await?;
    let trip_id = prepared.trip_id.clone();
    let conversation_id = prepared.conversation_id.clone();
    let user_message = prepared.input.message.clone();

    let result = run_agent(
        &state,
        &traveler,
        trip_id.as_deref(),
        prepared.input,
        |_| {},
    )
    .await?;

    let _ = crate::services::chat_memory::save_turn(
        &state.pool,
        &state.ollama,
        &traveler.id,
        &conversation_id,
        &user_message,
        &result.reply,
    )
    .await;

    let mut resp = to_response(result);
    resp.conversation_id = Some(conversation_id);
    Ok(Json(resp))
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
    let conversation_id = prepared.conversation_id.clone();
    let user_message = input.message.clone();

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
            Ok(result) => {
                let _ = crate::services::chat_memory::save_turn(
                    &state.pool,
                    &state.ollama,
                    &traveler.id,
                    &conversation_id,
                    &user_message,
                    &result.reply,
                )
                .await;
                let mut data = to_response(result);
                data.conversation_id = Some(conversation_id);
                emit(AgentStreamEvent::Done { data });
            }
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
