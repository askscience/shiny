//! What the assistant is working on in one conversation (`chat_agent_state`):
//! the language chosen for building plugins there, the plan it proposed, and
//! a question whose turn ended before the answer.

use serde_json::{json, Value};
use sqlx::SqlitePool;

use super::Language;
use crate::errors::AppError;
use crate::services::ask::{named_option, plain_answer, PlainAnswer};

/// Where a proposed plan stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlanState {
    /// No plan was proposed in this conversation.
    #[default]
    None,
    /// Shown to the user, not answered yet.
    Proposed,
    Approved,
    /// The user asked for changes; their remark is kept with the plan.
    Changes,
    Cancelled,
}

impl PlanState {
    pub fn as_str(self) -> &'static str {
        match self {
            PlanState::None => "none",
            PlanState::Proposed => "proposed",
            PlanState::Approved => "approved",
            PlanState::Changes => "changes",
            PlanState::Cancelled => "cancelled",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "proposed" => PlanState::Proposed,
            "approved" => PlanState::Approved,
            "changes" => PlanState::Changes,
            "cancelled" => PlanState::Cancelled,
            _ => PlanState::None,
        }
    }
}

/// A question the user's next message answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    Plan,
    Language,
}

impl Pending {
    fn as_str(self) -> &'static str {
        match self {
            Pending::Plan => "plan",
            Pending::Language => "language",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "plan" => Some(Pending::Plan),
            "language" => Some(Pending::Language),
            _ => None,
        }
    }
}

/// A plan as the assistant proposed it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub title: String,
    pub text: String,
    pub steps: Vec<String>,
}

impl Plan {
    fn to_json(&self) -> Value {
        json!({ "title": self.title, "text": self.text, "steps": self.steps })
    }

    fn from_json(raw: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(raw).ok()?;
        let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        Some(Self {
            title: text("title"),
            text: text("text"),
            steps: value
                .get("steps")
                .and_then(Value::as_array)
                .map(|steps| steps.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default(),
        })
    }
}

/// One conversation's state. A conversation without a row reads as the default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentState {
    pub language: Option<Language>,
    pub plugin: Option<String>,
    pub plan: Option<Plan>,
    pub plan_state: PlanState,
    pub plan_remark: Option<String>,
    pub pending: Option<Pending>,
}

type Row = (Option<String>, Option<String>, Option<String>, String, Option<String>, Option<String>);

pub async fn load(pool: &SqlitePool, conversation_id: &str, traveler_id: &str) -> Result<AgentState, AppError> {
    let row: Option<Row> = sqlx::query_as(
        "SELECT dev_language, dev_plugin, plan, plan_state, plan_remark, pending \
         FROM chat_agent_state WHERE conversation_id = ?1 AND traveler_id = ?2",
    )
    .bind(conversation_id)
    .bind(traveler_id)
    .fetch_optional(pool)
    .await?;
    Ok(match row {
        None => AgentState::default(),
        Some((language, plugin, plan, plan_state, plan_remark, pending)) => AgentState {
            language: language.as_deref().and_then(Language::parse),
            plugin,
            plan: plan.as_deref().and_then(Plan::from_json),
            plan_state: PlanState::parse(&plan_state),
            plan_remark,
            pending: pending.as_deref().and_then(Pending::parse),
        },
    })
}

async fn ensure_row(pool: &SqlitePool, conversation_id: &str, traveler_id: &str) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO chat_agent_state (conversation_id, traveler_id) VALUES (?1, ?2) \
         ON CONFLICT(conversation_id) DO NOTHING",
    )
    .bind(conversation_id)
    .bind(traveler_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record the language the user chose. It is chosen once: a conversation
/// that has one keeps it, and the stored language is what comes back.
pub async fn choose_language(
    pool: &SqlitePool,
    conversation_id: &str,
    traveler_id: &str,
    language: Language,
) -> Result<Language, AppError> {
    ensure_row(pool, conversation_id, traveler_id).await?;
    sqlx::query(
        "UPDATE chat_agent_state SET dev_language = COALESCE(dev_language, ?3), \
         pending = CASE WHEN pending = 'language' THEN NULL ELSE pending END, \
         updated_at = datetime('now') WHERE conversation_id = ?1 AND traveler_id = ?2",
    )
    .bind(conversation_id)
    .bind(traveler_id)
    .bind(language.as_str())
    .execute(pool)
    .await?;
    Ok(load(pool, conversation_id, traveler_id).await?.language.unwrap_or(language))
}

/// Store a plan the assistant just proposed. It replaces any earlier plan.
pub async fn propose_plan(
    pool: &SqlitePool,
    conversation_id: &str,
    traveler_id: &str,
    plan: &Plan,
) -> Result<(), AppError> {
    ensure_row(pool, conversation_id, traveler_id).await?;
    sqlx::query(
        "UPDATE chat_agent_state SET plan = ?3, plan_state = 'proposed', plan_remark = NULL, \
         pending = 'plan', updated_at = datetime('now') \
         WHERE conversation_id = ?1 AND traveler_id = ?2",
    )
    .bind(conversation_id)
    .bind(traveler_id)
    .bind(plan.to_json().to_string())
    .execute(pool)
    .await?;
    Ok(())
}

/// Record the user's answer to the plan.
pub async fn decide_plan(
    pool: &SqlitePool,
    conversation_id: &str,
    traveler_id: &str,
    decision: PlanState,
    remark: Option<&str>,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE chat_agent_state SET plan_state = ?3, plan_remark = ?4, \
         pending = CASE WHEN pending = 'plan' THEN NULL ELSE pending END, \
         updated_at = datetime('now') WHERE conversation_id = ?1 AND traveler_id = ?2",
    )
    .bind(conversation_id)
    .bind(traveler_id)
    .bind(decision.as_str())
    .bind(remark.map(str::trim).filter(|remark| !remark.is_empty()))
    .execute(pool)
    .await?;
    Ok(())
}

/// Record the plugin this conversation is working on.
pub async fn set_plugin(
    pool: &SqlitePool,
    conversation_id: &str,
    traveler_id: &str,
    plugin: &str,
) -> Result<(), AppError> {
    ensure_row(pool, conversation_id, traveler_id).await?;
    sqlx::query(
        "UPDATE chat_agent_state SET dev_plugin = ?3, updated_at = datetime('now') \
         WHERE conversation_id = ?1 AND traveler_id = ?2",
    )
    .bind(conversation_id)
    .bind(traveler_id)
    .bind(plugin)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_pending(
    pool: &SqlitePool,
    conversation_id: &str,
    traveler_id: &str,
    pending: Option<Pending>,
) -> Result<(), AppError> {
    ensure_row(pool, conversation_id, traveler_id).await?;
    sqlx::query(
        "UPDATE chat_agent_state SET pending = ?3, updated_at = datetime('now') \
         WHERE conversation_id = ?1 AND traveler_id = ?2",
    )
    .bind(conversation_id)
    .bind(traveler_id)
    .bind(pending.map(Pending::as_str))
    .execute(pool)
    .await?;
    Ok(())
}

/// Forget a conversation's state (the conversation was deleted).
pub async fn forget(pool: &SqlitePool, conversation_id: &str, traveler_id: &str) -> Result<(), AppError> {
    sqlx::query("DELETE FROM chat_agent_state WHERE conversation_id = ?1 AND traveler_id = ?2")
        .bind(conversation_id)
        .bind(traveler_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Read the user's new message as the answer to the question the last turn
/// left open, and store what it decides. Returns the state as it is now and,
/// when the message did answer, a line that tells the model so.
///
/// The reading is deliberately strict and done here, not by the model: a plan
/// is approved by a plain yes, cancelled by a plain no, and anything else is a
/// remark on it; a language is chosen only when the message names exactly one.
pub async fn settle_pending(
    pool: &SqlitePool,
    conversation_id: &str,
    traveler_id: &str,
    message: &str,
) -> Result<(AgentState, Option<String>), AppError> {
    settle_pending_on(pool, conversation_id, traveler_id, message, Language::readiness).await
}

/// [`settle_pending`], for a machine whose readiness the caller answers for.
pub async fn settle_pending_on(
    pool: &SqlitePool,
    conversation_id: &str,
    traveler_id: &str,
    message: &str,
    readiness: fn(Language) -> super::Readiness,
) -> Result<(AgentState, Option<String>), AppError> {
    let state = load(pool, conversation_id, traveler_id).await?;
    let Some(pending) = state.pending else {
        return Ok((state, None));
    };
    let note = match pending {
        Pending::Plan => {
            let (decision, remark, note) = match plain_answer(message) {
                PlainAnswer::Yes => (
                    PlanState::Approved,
                    None,
                    "The user's message approves the plan you proposed: carry it out now.".to_string(),
                ),
                PlainAnswer::No => (
                    PlanState::Cancelled,
                    None,
                    "The user's message cancels the plan you proposed: do not carry it out.".to_string(),
                ),
                PlainAnswer::Other => (
                    PlanState::Changes,
                    Some(message),
                    "The user's message is their remark on the plan you proposed: it is not approved. \
                     Revise the plan and call plan_propose again."
                        .to_string(),
                ),
            };
            decide_plan(pool, conversation_id, traveler_id, decision, remark).await?;
            Some(note)
        }
        Pending::Language => {
            let options: Vec<(&str, &str)> =
                Language::ALL.iter().map(|language| (language.as_str(), language.label())).collect();
            match named_option(message, &options).and_then(Language::parse) {
                // Named, but this machine cannot build in it: nothing is
                // chosen, or the chat would be bound to a dead end.
                Some(language) if !readiness(language).ready() => {
                    set_pending(pool, conversation_id, traveler_id, None).await?;
                    Some(format!(
                        "The user's message asks for {}, which is not ready on this machine (missing: {}). \
                         Nothing was chosen: tell them it is set up from the text chat, where the card installs it.",
                        language.label(),
                        readiness(language).missing_text().join("; ")
                    ))
                }
                Some(language) => {
                    let chosen = choose_language(pool, conversation_id, traveler_id, language).await?;
                    Some(format!(
                        "The user's message chooses {} for building plugins in this chat.",
                        chosen.label()
                    ))
                }
                None => {
                    // Not an answer: the question is dropped, as a spoken
                    // consent is, and the message is handled as usual.
                    set_pending(pool, conversation_id, traveler_id, None).await?;
                    None
                }
            }
        }
    };
    Ok((load(pool, conversation_id, traveler_id).await?, note))
}

/// The section of the prompt that tells the model what this conversation is
/// working on. Empty for a conversation with no language and no plan.
pub fn prompt_block(state: &AgentState, settled: Option<&str>) -> String {
    let mut lines: Vec<String> = Vec::new();
    if let Some(language) = state.language {
        lines.push(format!(
            "This chat builds plugins in {}. The user chose it for the whole chat: it cannot change \
             here, and only the {} commands of the plugin builder work.",
            language.label(),
            language.label()
        ));
        let readiness = language.readiness();
        if !readiness.ready() {
            lines.push(format!(
                "Not on this machine yet: {}.",
                readiness.missing_text().join("; ")
            ));
        }
    }
    if let Some(plugin) = state.plugin.as_deref().filter(|plugin| !plugin.is_empty()) {
        lines.push(format!("Plugin being worked on: {plugin}."));
    }
    if let Some(plan) = &state.plan {
        let title = if plan.title.trim().is_empty() { "untitled" } else { plan.title.trim() };
        let status = match state.plan_state {
            PlanState::None => None,
            PlanState::Proposed => Some(
                "waiting for the user's answer. Do not carry it out until they approve it.".to_string(),
            ),
            PlanState::Approved => Some("approved by the user. Follow it.".to_string()),
            PlanState::Changes => Some(format!(
                "not approved, the user asked for changes{}. Revise it and call plan_propose again.",
                state
                    .plan_remark
                    .as_deref()
                    .map(|remark| format!(": \"{}\"", remark.trim()))
                    .unwrap_or_default()
            )),
            PlanState::Cancelled => Some("cancelled by the user. Do not carry it out.".to_string()),
        };
        if let Some(status) = status {
            lines.push(format!("Plan \"{title}\": {status}"));
            if matches!(state.plan_state, PlanState::Approved | PlanState::Changes) {
                for (index, step) in plan.steps.iter().enumerate() {
                    lines.push(format!("  {}. {}", index + 1, step.trim()));
                }
            }
        }
    }
    if let Some(settled) = settled {
        lines.push(settled.to_string());
    }
    if lines.is_empty() {
        return String::new();
    }
    format!("\n## This chat's work\n{}\n", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One connection: an in-memory database lives and dies with it.
    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let mut conn = pool.acquire().await.unwrap();
        crate::db::run_migrations(&mut conn).await.unwrap();
        drop(conn);
        pool
    }

    fn plan() -> Plan {
        Plan {
            title: "Notes plugin".into(),
            text: String::new(),
            steps: vec!["Scaffold".into(), "Write the window".into()],
        }
    }

    #[tokio::test]
    async fn a_chat_keeps_the_language_chosen_first() {
        let pool = pool().await;
        assert_eq!(load(&pool, "a", "u").await.unwrap(), AgentState::default());
        assert_eq!(choose_language(&pool, "a", "u", Language::Rust).await.unwrap(), Language::Rust);
        // Asking again in the same chat does not change it.
        assert_eq!(choose_language(&pool, "a", "u", Language::Python).await.unwrap(), Language::Rust);
        // Another chat chooses on its own.
        assert_eq!(choose_language(&pool, "b", "u", Language::Python).await.unwrap(), Language::Python);
        assert_eq!(load(&pool, "a", "u").await.unwrap().language, Some(Language::Rust));
        assert_eq!(load(&pool, "b", "u").await.unwrap().language, Some(Language::Python));
        // Another account never reads it.
        assert_eq!(load(&pool, "a", "someone-else").await.unwrap(), AgentState::default());
    }

    #[tokio::test]
    async fn a_plain_yes_approves_the_plan_and_anything_else_does_not() {
        let pool = pool().await;
        propose_plan(&pool, "a", "u", &plan()).await.unwrap();
        let proposed = load(&pool, "a", "u").await.unwrap();
        assert_eq!(proposed.plan_state, PlanState::Proposed);
        assert_eq!(proposed.pending, Some(Pending::Plan));
        assert_eq!(proposed.plan, Some(plan()));

        let (state, note) = settle_pending(&pool, "a", "u", "add a search box too").await.unwrap();
        assert_eq!(state.plan_state, PlanState::Changes);
        assert_eq!(state.plan_remark.as_deref(), Some("add a search box too"));
        assert_eq!(state.pending, None);
        assert!(note.unwrap().contains("not approved"));

        // Answered once: a later "ok" about something else approves nothing.
        let (state, note) = settle_pending(&pool, "a", "u", "ok").await.unwrap();
        assert_eq!(state.plan_state, PlanState::Changes);
        assert_eq!(note, None);

        propose_plan(&pool, "a", "u", &plan()).await.unwrap();
        let (state, note) = settle_pending(&pool, "a", "u", "Sì, procedi").await.unwrap();
        assert_eq!(state.plan_state, PlanState::Approved);
        assert!(note.unwrap().contains("approves"));

        propose_plan(&pool, "a", "u", &plan()).await.unwrap();
        let (state, _) = settle_pending(&pool, "a", "u", "no").await.unwrap();
        assert_eq!(state.plan_state, PlanState::Cancelled);
    }

    #[tokio::test]
    async fn a_spoken_language_is_one_named_language() {
        let pool = pool().await;
        set_pending(&pool, "a", "u", Some(Pending::Language)).await.unwrap();
        let (state, note) = settle_pending(&pool, "a", "u", "what is the weather").await.unwrap();
        assert_eq!(state.language, None);
        assert_eq!(state.pending, None, "the question is dropped");
        assert_eq!(note, None);

        fn machine(language: Language) -> crate::services::plugin_dev::Readiness {
            match language {
                Language::Rust => Default::default(),
                Language::Python => crate::services::plugin_dev::Readiness {
                    missing: vec![crate::services::ask::AskLine::plain("Python plugin kit")],
                    fetchable: false,
                },
            }
        }
        set_pending(&pool, "a", "u", Some(Pending::Language)).await.unwrap();
        let (state, note) = settle_pending_on(&pool, "a", "u", "Python please", machine).await.unwrap();
        assert_eq!(state.language, None, "a language the machine lacks is not chosen");
        assert_eq!(state.pending, None);
        assert!(note.unwrap().contains("is not ready on this machine"));

        set_pending(&pool, "a", "u", Some(Pending::Language)).await.unwrap();
        let (state, note) = settle_pending_on(&pool, "a", "u", "in Rust", machine).await.unwrap();
        assert_eq!(state.language, Some(Language::Rust));
        assert_eq!(state.pending, None);
        assert!(note.unwrap().contains("chooses Rust"));
    }

    #[tokio::test]
    async fn a_deleted_conversation_leaves_no_state() {
        let pool = pool().await;
        choose_language(&pool, "a", "u", Language::Python).await.unwrap();
        forget(&pool, "a", "u").await.unwrap();
        assert_eq!(load(&pool, "a", "u").await.unwrap(), AgentState::default());
    }

    #[test]
    fn the_prompt_says_where_the_plan_stands() {
        assert_eq!(prompt_block(&AgentState::default(), None), "");

        let mut state = AgentState { plan: Some(plan()), plan_state: PlanState::Proposed, ..Default::default() };
        let waiting = prompt_block(&state, None);
        assert!(waiting.contains("Do not carry it out until they approve it"), "{waiting}");
        assert!(!waiting.contains("1. Scaffold"), "an unapproved plan is not a list of orders: {waiting}");

        state.plan_state = PlanState::Approved;
        let approved = prompt_block(&state, Some("The user's message approves the plan."));
        assert!(approved.contains("approved by the user"), "{approved}");
        assert!(approved.contains("1. Scaffold") && approved.contains("2. Write the window"), "{approved}");
        assert!(approved.ends_with("The user's message approves the plan.\n"), "{approved}");

        state.plan_state = PlanState::Changes;
        state.plan_remark = Some("shorter".into());
        assert!(prompt_block(&state, None).contains("\"shorter\""));
    }
}
