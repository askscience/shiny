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
use crate::services::agent_steps::{correction_block, voice_style_block, LONG_FORM_BLOCK};
use crate::services::agent_tools::{fetch_active_trip, AgentContext};
use crate::services::navigation::NavigationSession;

#[derive(Deserialize)]
pub struct AgentRequest {
    pub message: String,
    pub mode: Option<String>,
    pub lang: Option<String>,
    pub ai_name: Option<String>,
    pub ollama_model: Option<String>,
    /// Ollama's reasoning switch for this turn. `true` asks a thinking model
    /// for its reasoning trace (streamed as `thinking` events), `false` turns
    /// it off, omitted leaves the model's own default. Ignored by OpenAI, and
    /// downgraded to the model default when the model cannot think.
    pub think: Option<bool>,
    pub stream: Option<bool>,
    pub context: Option<AgentContextBody>,
    /// Client-side desktop layout (workspaces + which window is where), so the
    /// model can reorganize without creating empty workspaces.
    pub desktop: Option<DesktopState>,
    /// Conversation thread id — keeps the chat history so the model remembers
    /// earlier turns. Omit to start a new conversation.
    pub conversation_id: Option<String>,
    /// True when the user spoke (STT) and the answer will be read aloud (TTS):
    /// the reply must be conversational spoken prose, not lists/markdown.
    #[serde(default)]
    pub voice: bool,
    /// True when the user interrupted the previous reply to correct it: the
    /// message is applied to the request being answered, not read as a new one.
    /// Absent in every client that predates the conversation window.
    #[serde(default)]
    pub correction: bool,
    /// Client-generated id for this turn. Lets the user stop it mid-answer
    /// (`POST /api/agent/stop`) and lets the core tell "stopped" apart from
    /// "finished" when it saves the turn.
    pub turn_id: Option<String>,
    /// True when this client can show the terminal allow/deny prompt mid-turn
    /// (the text chat's SSE transport does). A turn that cannot ask never runs
    /// an unapproved terminal command.
    #[serde(default)]
    pub approve_tools: bool,
}

#[derive(Deserialize)]
pub struct AgentApproveRequest {
    pub turn_id: String,
    /// Id of the `approval` SSE event being answered.
    pub request_id: String,
    /// `allow`, `deny` or `allow_chat` (allow every command for this chat).
    pub decision: String,
}

#[derive(Deserialize)]
pub struct AgentStopRequest {
    pub turn_id: String,
    /// Conversation the turn belongs to, so a stop that arrives after the
    /// answer was saved can still leave the invisible "user stopped this"
    /// note on the right thread.
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
    /// False when the client is on a vertical, phone-like screen and has
    /// switched the workspace system off. Missing (older clients) means on.
    #[serde(default)]
    pub workspaces_enabled: Option<bool>,
    /// Plugins whose windows the user has explicitly focused. Non-empty means
    /// this turn runs in focus mode: the skills of these plugins — and only
    /// theirs — enter the prompt. Missing (older clients) means "none focused",
    /// which is the normal, unscoped mode.
    #[serde(default)]
    pub focused: Vec<String>,
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
    /// The model's reasoning trace for the turn (empty for non-thinking
    /// models/turns). Also streamed live as `thinking` events.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub thinking: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub navigation: Option<NavigationSession>,
    /// Plugin the AI chose to surface in a window (via show_plugin).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus_plugin: Option<String>,
    /// File the AI asked to open in a plugin window (a generated image).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_file: Option<serde_json::Value>,
    /// Conversation thread id for the chat history (continue this chat).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    /// True when the user stopped this turn before it finished.
    #[serde(default)]
    pub interrupted: bool,
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
    /// A status line: `message` is the English text, `key`/`params` the
    /// catalogue entry the web client translates (absent when the line has no
    /// catalogue entry, e.g. a plugin tool's plain-English label). `action` is
    /// the tool the line belongs to — the client picks the tool's icon from it
    /// (Settings → Appearance → voice bar: icons instead of the line).
    Step {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        key: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        params: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        action: Option<String>,
    },
    /// Reasoning trace of a thinking model, one block per tool-loop iteration.
    Thinking { message: String },
    /// The turn is parked on a terminal command until the user answers. The
    /// client shows the command with allow / deny / allow-for-this-chat and
    /// posts the choice to `/api/agent/approve`.
    Approval {
        request_id: String,
        action: String,
        command: String,
    },
    /// A tool's artifact, saved and streamed while the turn is still running.
    /// The UI ingests it immediately, so a plugin window opened by this card
    /// exists for the rest of the turn (the Browser's AI tab control depends
    /// on it).
    Artifact { data: serde_json::Value },
    Done { data: AgentResponse },
    Error { message: String },
}

struct CachedCoreSkill {
    modified: Option<std::time::SystemTime>,
    text: String,
}

/// Read the core skill once per file revision. The file is edited live in the
/// no-build UI, so the cache is keyed by mtime rather than baked at compile
/// time: an edit is still picked up on the next turn without a restart.
fn load_core_skill() -> String {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Option<CachedCoreSkill>>> =
        std::sync::OnceLock::new();
    const PATH: &str = "web/skills/core-assistant.md";

    let modified = std::fs::metadata(PATH).and_then(|m| m.modified()).ok();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.as_ref() {
            if cached.modified == modified {
                return cached.text.clone();
            }
        }
    }

    let text = std::fs::read_to_string(PATH)
        .unwrap_or_else(|_| "Use JSON action blocks.".into());
    if let Ok(mut guard) = cache.lock() {
        *guard = Some(CachedCoreSkill {
            modified,
            text: text.clone(),
        });
    }
    text
}

fn first_name(full: &str) -> String {
    full.split_whitespace()
        .next()
        .unwrap_or(full)
        .to_string()
}

/// Human-readable language name for the system prompt — a bare two-letter code
/// ("it") is easy for a model to ignore, a name ("Italian") is not.
fn language_name(code: &str) -> String {
    match code.to_lowercase().as_str() {
        "ar" => "Arabic",
        "bg" => "Bulgarian",
        "cs" => "Czech",
        "da" => "Danish",
        "de" => "German",
        "el" => "Greek",
        "en" => "English",
        "es" => "Spanish",
        "et" => "Estonian",
        "fi" => "Finnish",
        "fr" => "French",
        "ga" => "Irish",
        "hi" => "Hindi",
        "hr" => "Croatian",
        "hu" => "Hungarian",
        "id" => "Indonesian",
        "it" => "Italian",
        "ja" => "Japanese",
        "ko" => "Korean",
        "lt" => "Lithuanian",
        "lv" => "Latvian",
        "mt" => "Maltese",
        "nl" => "Dutch",
        "pl" => "Polish",
        "pt" => "Portuguese",
        "ro" => "Romanian",
        "ru" => "Russian",
        "sk" => "Slovak",
        "sl" => "Slovenian",
        "sv" => "Swedish",
        "tr" => "Turkish",
        "uk" => "Ukrainian",
        "vi" => "Vietnamese",
        "zh" => "Chinese",
        other => other,
    }
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
    if d.workspaces_enabled == Some(false) {
        // A vertical, phone-like screen: one column, no workspaces at all.
        let windows = d
            .workspaces
            .iter()
            .flat_map(|ws| ws.windows.iter())
            .cloned()
            .collect::<Vec<_>>();
        let list = if windows.is_empty() {
            "none".to_string()
        } else {
            windows.join(", ")
        };
        return format!(
            "\n## Desktop (current layout)\n\
             This screen is vertical, so the desktop is a SINGLE column of windows ({list}).\n\
             There are no workspaces here: workspace_create, workspace_remove, workspace_switch \n\
             and workspace_move do nothing. Do not call them, and never tell the user you \n\
             created, removed or switched a workspace. Use desktop_fullscreen or show_plugin \n\
             to change what is on screen.\n"
        );
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

/// "## Focus mode" block — present only while the user has focused plugin
/// windows. Focus mode is enforced in `agent_tools::execute_action`, so this
/// block explains the rule the model is already under rather than asking for it.
fn focus_mode_block(focused: &std::collections::BTreeSet<String>) -> String {
    let list = focused.iter().cloned().collect::<Vec<_>>().join(", ");
    format!(
        "\n## Focus mode (ON)\n\
         The user has focused: {list}. Only web search and the {list} tools run this turn: \
         another plugin's tools, and the desktop actions (show_plugin, desktop_focus, \
         workspace_*) are refused before they run.\n\
         You cannot change the focus; only the user can.\n\
         When the request needs another plugin: do NOT call a tool, not even web search — do \
         not substitute a search for the plugin you cannot use. Answer in ONE short sentence \
         asking the user to focus that plugin (the eye button in its window, or right-clicking \
         its icon in the top bar), then wait for them.\n\
         Suggest a focus change only when it genuinely helps the work at hand — never \
         routinely, and at most once per answer.\n"
    )
}

/// The one-line focus state at the top of `## Context`, so the model never has
/// to infer whether focus mode is on from the presence of a section.
fn focus_state_line(focused: &std::collections::BTreeSet<String>) -> String {
    if focused.is_empty() {
        "Focus mode: OFF — no plugin window is focused, so every active plugin is available.\n"
            .to_string()
    } else {
        let list = focused.iter().cloned().collect::<Vec<_>>().join(", ");
        format!(
            "Focus mode: ON — the user has focused: {list}. Only web search and the {list} \
             tools can run this turn; you cannot change this yourself.\n"
        )
    }
}

/// "## Craft apps (open now)" — the prompt's view of the Craft apps whose MCP
/// server is up this turn, split by what the tools act on. **Live** apps (their
/// page exposes the vendor's control channel, so the CLI was started with
/// `--connect`) change the window in front of the user. **Headless** apps drive
/// an engine in a separate process: the model must save/export and hand over a
/// path — unsaved work dies with that process. Both lists come from the same app
/// ids, so an app added later in the suite lands in whichever mode it supports
/// without any change here.
fn craft_apps_block(running: &[String], live: &[String]) -> String {
    if running.is_empty() {
        return String::new();
    }
    let display = |id: &String| {
        crate::plugins::store::CraftApp::by_id(id)
            .map(|app| app.name.to_string())
            .unwrap_or_else(|| id.clone())
    };
    let names: Vec<String> = running.iter().map(display).collect();
    let tools: Vec<String> = running.iter().map(|id| format!("{id}_*")).collect();
    let live_set: std::collections::BTreeSet<&String> = live.iter().collect();
    let live_names: Vec<String> = running
        .iter()
        .filter(|id| live_set.contains(id))
        .map(display)
        .collect();
    let headless_names: Vec<String> = running
        .iter()
        .filter(|id| !live_set.contains(id))
        .map(display)
        .collect();

    let mut block = format!(
        "\n## Craft apps (open now)\n\
         {} — their windows are open, so their own app tools ({}) are available this turn. \
         Drive them with those tools, never their command-line through the terminal.\n",
        names.join(", "),
        tools.join(", "),
    );
    if !live_names.is_empty() {
        block.push_str(&format!(
            "{}: live — the tools act on the open window itself: what you draw or change appears \
             there as you make it. No export is needed unless the user wants a file.\n",
            live_names.join(", "),
        ));
    }
    if !headless_names.is_empty() {
        block.push_str(&format!(
            "{}: headless — the tools drive the app's engine in a separate process, not the window, \
             and that process ends with this session, so unsaved work is lost. Save or export the \
             document (in the app's own format, under the user's home — that is the app's write \
             root) before you answer, tell the user the path, and only then say it is done.\n",
            headless_names.join(", "),
        ));
    }
    block
}

/// The windows the client reported open this turn, across every workspace, kept
/// to the session's active plugins. This is the set the Craft MCP host follows
/// (see `plugins::craft_mcp`): a plain name set, so whatever is not an open
/// Craft app is dropped when the servers start.
fn open_windows(
    desktop: Option<&DesktopState>,
    installed: &std::collections::BTreeSet<String>,
) -> std::collections::BTreeSet<String> {
    desktop
        .map(|d| {
            d.workspaces
                .iter()
                .flat_map(|ws| ws.windows.iter())
                .filter(|name| installed.contains(*name))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

struct PreparedAgent {
    input: AgentRunInput,
    trip_id: Option<String>,
    conversation_id: String,
    /// Ollama model the client explicitly asked for on this request (from the
    /// `ollama_model` body field). Honored over the persisted preference.
    requested_model: Option<String>,
    /// Client-generated id for this turn, used for stop/cancel.
    turn_id: Option<String>,
    /// The client can render the terminal allow/deny prompt mid-turn.
    can_approve: bool,
}

async fn prepare_agent(
    state: &AppState,
    traveler: &Traveler,
    body: AgentRequest,
) -> Result<PreparedAgent, AppError> {
    let mode = body.mode.unwrap_or_else(|| "single".into());
    let lang = body.lang.unwrap_or_else(|| "en".into());
    let voice = body.voice;
    let correction = body.correction;
    let ctx_body = body.context.unwrap_or(AgentContextBody {
        lat: None,
        lon: None,
        heading: None,
    });

    let requested_model: Option<String> = body
        .ollama_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);

    let mut ctx = AgentContext {
        lat: ctx_body.lat,
        lon: ctx_body.lon,
        heading: ctx_body.heading,
        lang: lang.clone(),
        ollama_model: requested_model.clone(),
        workspaces_enabled: body
            .desktop
            .as_ref()
            .and_then(|d| d.workspaces_enabled)
            .unwrap_or(true),
        // Filled in below, once the active set is known: the focus set is the
        // focused windows intersected with it.
        focused: Vec::new(),
    };

    // Per-user active plugin set FOR THIS SESSION — drives which plugins'
    // skills / persona / context lines enter the system prompt. Fresh mode
    // (`session.remember` off) yields the explicitly-enabled plugins, so a
    // fresh sign-in sees only what the user has turned on.
    let installed: std::collections::BTreeSet<String> =
        state.plugins.session_active_set(&traveler.id).await;

    // Focus mode: the windows the user explicitly focused. It narrows this turn
    // to those plugins' skills, so it only counts for plugins that are active —
    // a stale focus entry must never widen or break the prompt.
    let focused: std::collections::BTreeSet<String> = body
        .desktop
        .as_ref()
        .map(|d| {
            d.focused
                .iter()
                .filter(|n| installed.contains(*n))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let focus_mode = !focused.is_empty();

    // Focus mode *enforces* the scope, it does not just describe it: the plugin
    // registry refuses any other plugin's tools while this is non-empty, so the
    // prompt's promise and what the model can actually call are the same thing.
    ctx.focused = focused.iter().cloned().collect();

    // Windows the user has open right now, across every workspace: a Craft app
    // among them keeps its MCP server up, so the AI drives the app the user is
    // working in without entering focus mode.
    let open = open_windows(body.desktop.as_ref(), &installed);

    // Craft-app AI control rides those two sets, and it is enforced here in
    // code: every open app's MCP server is started (and the closed ones stopped)
    // before the tool list is built, and while the user is in focus mode only
    // the focused apps' servers stay up — so this turn advertises and can call
    // exactly the apps the user has in front of them, one or several. No prompt
    // wording is involved.
    let craft_home = state
        .os_identity_for(traveler)
        .map(|identity| identity.home)
        .or_else(|| std::env::var("HOME").ok())
        .unwrap_or_else(|| ".".to_string());
    state
        .craft_mcp
        .sync(
            &state.plugins,
            &state.craft_control,
            std::path::Path::new(&state.config.plugins_dir),
            &craft_home,
            &open,
            &focused,
        )
        .await;

    let traveler_active = installed.contains("traveler");
    let active_trip = if traveler_active {
        fetch_active_trip(&state.pool, &traveler.id).await?
    } else {
        None
    };
    let recent_diary = if traveler_active {
        sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT date, summary FROM diary_entries WHERE traveler_id = ?1 ORDER BY date DESC LIMIT 3",
        )
        .bind(&traveler.id)
        .fetch_all(&state.pool)
        .await?
    } else {
        Vec::new()
    };

    // Skill markdown = the core assistant reference (always-on tools only)
    // PLUS whatever skills the user's active plugins advertise. Plugin-owned
    // domains (e.g. traveler) document their own tools via their skills.
    //
    // In focus mode the scope shrinks to the focused plugins: the AI gets what
    // the work in front of the user needs and nothing else.
    let core_skill = load_core_skill();
    let skill_scope = if focus_mode { &focused } else { &installed };
    let plugin_skill = state.plugins.skills_markdown_for(skill_scope);
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

    let workspaces_enabled = ctx.workspaces_enabled;
    let plugin_windows_block = if active_lines.is_empty() {
        String::new()
    } else if workspaces_enabled {
        format!(
            "\n## Plugin windows\n\
             Each active plugin has its own window, tiled Hyprland-style and grouped on \
             numbered workspaces (desktops). Active plugins:\n{}\n\
             Call {{\"action\":\"show_plugin\",\"params\":{{\"name\":\"<plugin>\"}}}} only when the \
             user asks to open, show or focus that plugin's window — do not call it just \
             because you used the plugin's tools. Use desktop_fullscreen to make it full screen, \
             and workspace_create / workspace_remove / workspace_switch / workspace_move to \
             manage desktops. Focus mode is separate: while a window is focused only web \
             search and that plugin's tools run, and only the user changes it.\n",
            active_lines.join("\n")
        )
    } else {
        format!(
            "\n## Plugin windows\n\
             Each active plugin has its own window. Active plugins:\n{}\n\
             Call {{\"action\":\"show_plugin\",\"params\":{{\"name\":\"<plugin>\"}}}} only when the \
             user asks to open, show or focus that plugin's window — do not call it just \
             because you used the plugin's tools. Use desktop_fullscreen to make it full screen. \
             This screen is vertical, so there is ONE column and no workspaces: the \
             workspace_create / workspace_remove / workspace_switch / workspace_move tools do \
             nothing here — never call them, and never tell the user you did.\n",
            active_lines.join("\n")
        )
    };

    let plugin_catalog_block = if catalog.is_empty() {
        "\n## Plugins\nNo plugins installed.".to_string()
    } else {
        // In focus mode the catalog must not promise tools the registry will
        // refuse: say plainly that only the focused plugins' tools work.
        let active_label = if focus_mode {
            "Active plugins (only the focused ones' tools work this turn):"
        } else {
            "Active plugins (tools available):"
        };
        format!(
            "\n## Plugins\n\
             {active_label}\n{}\n\
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

    // The Craft apps whose MCP server is up this turn, and which of them act on
    // the open window (live) rather than on a headless engine. Their tools are in
    // the "## Tools" docs like any other plugin's; this block says what they act
    // on, so the model never promises a change it cannot make.
    let running_craft = state.craft_mcp.running_apps().await;
    let live_craft = state.craft_mcp.live_apps().await;
    let craft_block = craft_apps_block(&running_craft, &live_craft);

    let desktop_block = desktop_state_block(body.desktop.as_ref());
    // Focus mode is a property of this turn, not of the desktop layout: it rides
    // with the focused windows the client just reported. The state line is
    // always present — ON or OFF — so the model never has to infer it.
    let focus_line = focus_state_line(&focused);
    let focus_block = if focus_mode { focus_mode_block(&focused) } else { String::new() };

    // Conversation memory: resolve (or start) the thread and surface its recent
    // turns so the model remembers what was said earlier.
    let conversation_id = crate::services::chat_memory::resolve_conversation(
        &state.pool,
        &traveler.id,
        body.conversation_id.as_deref(),
    )
    .await?;
    let history = crate::services::chat_memory::recent_history(&state.pool, &conversation_id, 40).await?;
    // "Allow for this chat" is a property of the conversation, not of a single
    // turn: load it before the loop so no prompt is shown for a chat that
    // already consented.
    let auto_approve: i64 = sqlx::query_scalar(
        "SELECT terminal_auto_approve FROM chat_conversations WHERE id = ?1 AND traveler_id = ?2",
    )
    .bind(&conversation_id)
    .bind(&traveler.id)
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or(0);
    if auto_approve != 0 {
        state.approvals.set_auto_chat(&conversation_id, true);
    }
    let history_block = if history.is_empty() {
        String::new()
    } else {
        let lines: Vec<String> = history
            .iter()
            .map(|(role, content)| format!("{role}: {}", content.trim()))
            .collect();
        format!("\n## Conversation history (remember earlier turns)\n{}\n", lines.join("\n"))
    };

    // How the reply is delivered shapes how it must be written. Voice: the
    // answer is spoken, so it has to be conversational prose with no markdown
    // or lists. The long-form rules apply to both: write the document in full,
    // then confirm it in the chat in one or two sentences.
    let style_lead = if voice {
        "You are talking with the user by voice: answer conversationally, in plain spoken prose."
    } else {
        "Answer completely and helpfully — be concise for simple questions, but give detail, steps, or lists whenever the answer needs them."
    };
    let voice_style = voice_style_block(voice);
    // The user stopped the last answer to amend the request: say so, or "no,
    // intendo Milano" arrives as a question about nothing.
    let correction_note = correction_block(correction);

    let system = format!(
        "You are {ai_name}, {persona}. Always reply in {lang_name}. {style_lead}\n\
         The user may wake you by saying \"hey {ai_lower}\".\n\
         Address the user as {user_first} when it feels natural.\n\
         {voice_style}{correction_note}{long_form}\n\
         ## Tool protocol (strict)\n\
         - Call exactly ONE tool per turn.\n\
         - Output ONLY raw JSON on its own line — no markdown fences.\n\
         - Format: {{\"action\":\"tool_name\",\"params\":{{...}}}}\n\
         - Always include \"params\". Use {{}} when a tool has no parameters.\n\
         \n\
         ## Tools\n{skill}\n\n\
         ## Context\n{focus_line}User name: {user_first}\n{location_line}\n{trip_line}\nDiary: {diary_line}{context_block}\n\
         Mode: {mode} — answer fully and clearly.{history_block}{plugin_windows_block}{craft_block}{focus_block}{plugin_catalog_block}{desktop_block}",
        ai_name = ai_name,
        persona = persona,
        lang_name = language_name(&lang),
        style_lead = style_lead,
        ai_lower = ai_name.to_lowercase(),
        user_first = user_first,
        voice_style = voice_style,
        long_form = LONG_FORM_BLOCK,
        skill = skill,
        focus_line = focus_line,
        location_line = location_line,
        trip_line = trip_line,
        diary_line = diary_line,
        context_block = context_block,
        mode = mode,
        history_block = history_block,
        plugin_windows_block = plugin_windows_block,
        craft_block = craft_block,
        focus_block = focus_block,
        plugin_catalog_block = plugin_catalog_block,
        desktop_block = desktop_block,
    );

    Ok(PreparedAgent {
        trip_id: active_trip.as_ref().map(|t| t.id.clone()),
        conversation_id,
        requested_model,
        turn_id: body.turn_id.filter(|id| !id.trim().is_empty()),
        can_approve: body.approve_tools,
        input: AgentRunInput {
            message: body.message,
            mode,
            lang,
            ai_name,
            system,
            plugins_hint,
            history,
            voice,
            correction,
            think: body.think,
            ctx,
            craft_open: open,
            craft_home,
            // Attached by the transport handler: it needs the SSE sink.
            approval: None,
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
        thinking: result.thinking,
        navigation: result.navigation,
        focus_plugin: result.focus_plugin,
        open_file: result.open_file,
        conversation_id: None,
        interrupted: result.interrupted,
    }
}

pub async fn handle_agent(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(body): Json<AgentRequest>,
) -> Result<Json<AgentResponse>, AppError> {
    let mut prepared = prepare_agent(&state, &traveler, body).await?;
    let ai = state.resolve_ai(&traveler.id).await;
    let trip_id = prepared.trip_id.clone();
    let conversation_id = prepared.conversation_id.clone();
    let user_message = prepared.input.message.clone();
    let turn_id = prepared.turn_id.clone();
    // The one-shot JSON transport cannot pop a prompt mid-turn: attach a
    // non-interactive gate so a command is refused instead of running
    // unapproved (an "allow for this chat" conversation still runs).
    prepared.input.approval = Some(crate::services::approval::ApprovalGate::new(
        state.approvals.clone(),
        turn_id.clone().unwrap_or_default(),
        traveler.id.clone(),
        conversation_id.clone(),
        false,
        Box::new(|_| {}),
    ));
    // Honor the model the client asked for on this request (Ollama only); the
    // OpenAI client bakes its configured model in.
    let model: Option<String> = if ai.client.is_openai() {
        None
    } else {
        prepared.requested_model.clone().or_else(|| ai.model.clone())
    };

    let cancel = turn_id
        .as_deref()
        .map(|id| state.agent_turns.register(id, &traveler.id));

    let result = run_agent(
        &state,
        &traveler,
        trip_id.as_deref(),
        &ai.client,
        model.as_deref(),
        prepared.input,
        cancel.clone(),
        |_, _| {},
        |_| {},
        |_| {},
    )
    .await?;

    let stop_note = finish_turn(
        &state,
        &ai.client,
        &traveler.id,
        &conversation_id,
        &user_message,
        &result,
        cancel.as_ref(),
        turn_id.as_deref(),
    )
    .await;

    let mut resp = to_response(result);
    resp.conversation_id = Some(conversation_id);
    resp.interrupted = stop_note;
    Ok(Json(resp))
}

/// Persist a finished turn — with the invisible "the user stopped this" note
/// when it was stopped — and retire its cancellation entry.
///
/// Returns whether the turn is recorded as interrupted. The note is written
/// exactly once, whichever moment the stop arrives in: the runner handles the
/// mid-run case (and the stop landing while the answer is being saved), and
/// `/api/agent/stop` handles the turn that had already finished.
#[allow(clippy::too_many_arguments)]
async fn finish_turn(
    state: &AppState,
    ai: &crate::services::ai::AiClient,
    traveler_id: &str,
    conversation_id: &str,
    user_message: &str,
    result: &AgentRunResult,
    cancel: Option<&crate::services::agent_cancel::CancelHandle>,
    turn_id: Option<&str>,
) -> bool {
    use crate::services::chat_memory;

    let stopped = result.interrupted || cancel.is_some_and(|c| c.is_cancelled());
    let note = stopped.then_some(chat_memory::INTERRUPTED_NOTE);
    let thinking = (!result.thinking.trim().is_empty()).then_some(result.thinking.as_str());

    let local_title = match chat_memory::save_turn_with_note(
        &state.pool,
        traveler_id,
        conversation_id,
        user_message,
        &result.reply,
        note,
        thinking,
        &result.events,
    )
    .await
    {
        Ok(title) => title,
        Err(e) => {
            tracing::warn!("Failed to save agent turn: {e}");
            None
        }
    };

    // The local title above is already visible. Upgrade it with the model's
    // three-word title in the background, after Done, never on the reply path.
    if let Some(local_title) = local_title {
        let pool = state.pool.clone();
        let ai = ai.clone();
        let traveler_id = traveler_id.to_string();
        let conversation_id = conversation_id.to_string();
        let user_message = user_message.to_string();
        tokio::spawn(async move {
            if let Err(e) = chat_memory::upgrade_title(
                &pool,
                &ai,
                &traveler_id,
                &conversation_id,
                &user_message,
                &local_title,
            )
            .await
            {
                tracing::debug!("conversation title upgrade failed: {e}");
            }
        });
    }

    if let Some(id) = turn_id {
        state.agent_turns.finish(id);
    }
    stopped
}

/// Stop an in-flight turn, or annotate one that already finished.
pub async fn handle_agent_stop(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(body): Json<AgentStopRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::services::agent_cancel::StopOutcome;

    let outcome = state.agent_turns.cancel(&body.turn_id, &traveler.id);
    let annotate = outcome == StopOutcome::Annotate;

    // The turn already finished: its answer was generated and saved, and the
    // user stopped it while reading or listening. Record that, so the next
    // request knows the reply was not taken in full. A stop for an unknown —
    // or already annotated — turn changes nothing.
    if annotate {
        if let Some(conversation_id) = body.conversation_id.as_deref() {
            let _ = crate::services::chat_memory::append_note(
                &state.pool,
                &traveler.id,
                conversation_id,
                crate::services::chat_memory::INTERRUPTED_NOTE,
            )
            .await;
        }
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "cancelled": outcome == StopOutcome::Cancelled,
            "annotated": annotate,
        },
    })))
}

/// Answer the terminal command the turn is waiting on.
///
/// `allow_chat` also persists the consent on the conversation
/// (`terminal_auto_approve`), so every later command in that same chat runs
/// without a prompt — other chats still ask. The hard-coded safety guard runs
/// before the prompt, so it can never be answered into allowing a destructive
/// command.
pub async fn handle_agent_approve(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(body): Json<AgentApproveRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (allow, remember) = match body.decision.as_str() {
        "allow" => (true, false),
        "deny" => (false, false),
        "allow_chat" => (true, true),
        other => {
            return Err(AppError::BadRequest(format!(
                "unknown decision '{other}' — use allow, deny or allow_chat"
            )))
        }
    };

    let resolved = state.approvals.resolve(
        &body.turn_id,
        &body.request_id,
        &traveler.id,
        allow,
        remember,
    );

    let Some(conversation_id) = resolved else {
        // The turn already ended, the request was answered twice, or it belongs
        // to another account. Nothing to approve.
        return Ok(Json(serde_json::json!({
            "success": false,
            "data": { "allowed": false, "reason": "no pending approval request" },
        })));
    };

    if remember && allow {
        sqlx::query(
            "UPDATE chat_conversations SET terminal_auto_approve = 1 \
             WHERE id = ?1 AND traveler_id = ?2",
        )
        .bind(&conversation_id)
        .bind(&traveler.id)
        .execute(&state.pool)
        .await
        .map_err(AppError::Database)?;
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "data": { "allowed": allow, "remembered": remember && allow },
    })))
}

pub async fn handle_agent_stream(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(body): Json<AgentRequest>,
) -> Result<Sse<impl stream::Stream<Item = Result<Event, Infallible>>>, AppError> {
    let prepared = prepare_agent(&state, &traveler, body).await?;
    let ai = state.resolve_ai(&traveler.id).await;
    // Honor the model the client asked for on this request (Ollama only).
    let model: Option<String> = if ai.client.is_openai() {
        None
    } else {
        prepared.requested_model.clone().or_else(|| ai.model.clone())
    };
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    let state = state.clone();
    let traveler = traveler.clone();
    let ai = ai.clone();
    let trip_id = prepared.trip_id.clone();
    let can_approve = prepared.can_approve;
    let mut input = prepared.input;
    let conversation_id = prepared.conversation_id.clone();
    let user_message = input.message.clone();
    let turn_id = prepared.turn_id.clone();
    let cancel = turn_id
        .as_deref()
        .map(|id| state.agent_turns.register(id, &traveler.id));
    let disconnect_cancel = cancel.clone();
    // Detect a closed SSE consumer immediately, not only when the next event
    // happens to be sent: a long model call should be abandoned as soon as the
    // receiver disappears. The watcher's sender clone must be dropped when the
    // turn ends — see the end of the spawned task.
    let disconnect_watcher = disconnect_cancel.clone().map(|handle| {
        let closed_tx = tx.clone();
        tokio::spawn(async move {
            closed_tx.closed().await;
            handle.cancel();
        })
    });

    tokio::spawn(async move {
        let emit: std::sync::Arc<dyn Fn(AgentStreamEvent) + Send + Sync> =
            std::sync::Arc::new(move |event: AgentStreamEvent| {
                if let Ok(json) = serde_json::to_string(&event) {
                    if tx.send(json).is_err() {
                        // The SSE consumer is gone (tab closed, request aborted).
                        // Stop the model and the tool loop instead of finishing an
                        // answer nobody can receive.
                        if let Some(handle) = disconnect_cancel.as_ref() {
                            handle.cancel();
                        }
                    }
                }
            });
        let emit_step = emit.clone();
        let emit_thinking = emit.clone();
        let emit_approval = emit.clone();
        let emit_artifact = emit.clone();

        // The chat's allow/deny prompt is a mid-turn round trip: the gate emits
        // an `approval` event over this same stream and parks until
        // `/api/agent/approve` resolves it.
        input.approval = Some(crate::services::approval::ApprovalGate::new(
            state.approvals.clone(),
            turn_id.clone().unwrap_or_default(),
            traveler.id.clone(),
            conversation_id.clone(),
            can_approve,
            Box::new(move |prompt| {
                emit_approval(AgentStreamEvent::Approval {
                    request_id: prompt.request_id,
                    action: prompt.action,
                    command: prompt.command,
                })
            }),
        ));

        match run_agent(
            &state,
            &traveler,
            trip_id.as_deref(),
            &ai.client,
            model.as_deref(),
            input,
            cancel.clone(),
            |step, action| emit_step(AgentStreamEvent::Step {
                message: step.text.clone(),
                key: step.key.clone(),
                params: (!step.params.is_empty()).then(|| serde_json::Value::Object(step.params.clone())),
                action: action.map(str::to_string),
            }),
            |msg| emit_thinking(AgentStreamEvent::Thinking {
                message: msg.to_string(),
            }),
            |data| emit_artifact(AgentStreamEvent::Artifact { data }),
        )
        .await
        {
            Ok(result) => {
                let interrupted = finish_turn(
                    &state,
                    &ai.client,
                    &traveler.id,
                    &conversation_id,
                    &user_message,
                    &result,
                    cancel.as_ref(),
                    turn_id.as_deref(),
                )
                .await;
                let mut data = to_response(result);
                data.conversation_id = Some(conversation_id);
                data.interrupted = interrupted;
                emit(AgentStreamEvent::Done { data });
            }
            Err(e) => {
                if let Some(id) = turn_id.as_deref() {
                    state.agent_turns.finish(id);
                }
                emit(AgentStreamEvent::Error {
                    message: crate::i18n::translate_current(&e.to_string()),
                })
            }
        }

        // `closed()` can only fire while the receiver is alive, and the
        // receiver (the SSE body) ends only once every sender is gone. The
        // watcher holds one, so a finished turn must drop it here — otherwise
        // the response never ends and the browser keeps the connection
        // occupied for good.
        if let Some(watcher) = disconnect_watcher {
            watcher.abort();
            let _ = watcher.await;
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

#[cfg(test)]
mod tests {
    use super::{
        craft_apps_block, focus_mode_block, focus_state_line, open_windows, DesktopState,
        WorkspaceSnapshot,
    };
    use std::collections::BTreeSet;

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    /// The model used to answer "focus mode is off" while a window was focused:
    /// the state has to be stated in every turn, ON and OFF alike.
    #[test]
    fn focus_state_line_always_states_the_mode() {
        let off = focus_state_line(&BTreeSet::new());
        assert!(off.starts_with("Focus mode: OFF"), "{off}");

        let on = focus_state_line(&set(&["files"]));
        assert!(on.starts_with("Focus mode: ON"), "{on}");
        assert!(on.contains("files"), "{on}");
        assert!(on.contains("web search"), "names the hard limit: {on}");
    }

    /// The block describes the rule the dispatch layer already enforces, and the
    /// model is told it can only SUGGEST a focus change.
    #[test]
    fn focus_block_states_the_hard_scope_and_the_suggestion_only_rule() {
        let block = focus_mode_block(&set(&["files", "word"]));
        assert!(block.contains("## Focus mode (ON)"), "{block}");
        assert!(block.contains("files, word"), "{block}");
        assert!(block.contains("Only web search"), "{block}");
        assert!(block.contains("You cannot change the focus"), "{block}");
        assert!(block.contains("do NOT call a tool"), "{block}");
        // No tool for the model to reach for: focus is the user's call.
        assert!(!block.contains("plugin_focus"), "{block}");
        assert!(!block.contains("plugin_unfocus"), "{block}");
        // Never nag: suggestions are gated on being useful.
        assert!(block.contains("genuinely helps"), "{block}");
    }

    /// The open windows are what the Craft MCP host follows: every workspace
    /// counts (an app left on another workspace is still open), a plugin that is
    /// not active this session is not a window the turn can drive, and a client
    /// that sends no desktop opens nothing.
    #[test]
    fn open_windows_span_every_workspace_and_stay_inside_the_active_set() {
        let active = set(&["photocraft", "wordcraft"]);
        let desktop = DesktopState {
            active: Some(2),
            workspaces: vec![
                WorkspaceSnapshot {
                    index: 1,
                    windows: vec!["photocraft".into(), "calc".into()],
                },
                WorkspaceSnapshot {
                    index: 2,
                    windows: vec!["wordcraft".into()],
                },
            ],
            workspaces_enabled: Some(true),
            focused: vec!["wordcraft".into()],
        };
        assert_eq!(
            open_windows(Some(&desktop), &active),
            set(&["photocraft", "wordcraft"])
        );
        assert!(
            open_windows(None, &active).is_empty(),
            "no desktop in the request means no window to drive"
        );
    }

    /// The prompt says where a Craft app's tools act: live on the window when the
    /// page exposes the control channel, or on a headless engine whose unsaved
    /// work dies with the process.
    #[test]
    fn craft_block_tells_live_from_headless() {
        let running = vec!["vectorcraft".to_string()];
        let headless = craft_apps_block(&running, &[]);
        assert!(headless.contains("VectorCraft"), "{headless}");
        assert!(headless.contains("vectorcraft_*"), "{headless}");
        assert!(headless.contains("headless"), "{headless}");
        assert!(headless.contains("not the window"), "{headless}");
        assert!(headless.contains("Save or export"), "{headless}");
        assert!(
            headless.contains("write root"),
            "the model has to know where saving is allowed: {headless}"
        );
        assert!(
            headless.contains("is lost"),
            "the model must know the process is ephemeral: {headless}"
        );

        let live = craft_apps_block(&running, &running);
        assert!(live.contains("live —"), "{live}");
        assert!(live.contains("open window"), "{live}");
        assert!(
            !live.contains("Save or export"),
            "a live app needs no export to be seen: {live}"
        );
        assert!(craft_apps_block(&[], &[]).is_empty());
    }
}
