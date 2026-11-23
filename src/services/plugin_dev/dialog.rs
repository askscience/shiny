//! The three actions that put a card in front of the user and wait:
//! `ask_user` (a question), `plan_propose` (a plan to approve) and
//! `plugin_dev_start` (the Python / Rust choice that opens the plugin
//! builder in a chat).
//!
//! They are core actions, run by the agent loop before the tool registry: a
//! plugin cannot claim their names, and the answer always comes from the
//! user, through [`crate::services::ask`], never from the model.

use serde_json::{json, Value};
use sqlx::SqlitePool;

use super::state::{self, Pending, Plan, PlanState};
use super::Language;
use crate::errors::AppError;
use crate::services::agent_cancel::CancelHandle;
use crate::services::agent_steps::StepText;
use crate::services::agent_tools::ActionOutcome;
use crate::services::ask::{AskContent, AskGate, AskKind, AskLine, AskOption, AskOutcome, AskReply};

pub const ASK_USER: &str = "ask_user";
pub const PLAN_PROPOSE: &str = "plan_propose";
pub const PLUGIN_DEV_START: &str = "plugin_dev_start";

/// The dialog action a model-written name means, if it is one.
pub fn canonical(action: &str) -> Option<&'static str> {
    match action.trim().to_ascii_lowercase().as_str() {
        "ask_user" | "ask" | "ask_question" | "question" | "clarify" => Some(ASK_USER),
        "plan_propose" | "propose_plan" | "plan" | "present_plan" | "plan_mode" => Some(PLAN_PROPOSE),
        "plugin_dev_start" | "plugin_dev" | "plugin_builder" | "create_plugin" | "plugin_create" => {
            Some(PLUGIN_DEV_START)
        }
        _ => None,
    }
}

/// What one dialog action needs from its turn.
pub struct Dialog<'a> {
    pub pool: &'a SqlitePool,
    pub traveler_id: &'a str,
    /// `None` only where a turn has no conversation to ask in (tests).
    pub gate: Option<&'a AskGate>,
    pub cancel: Option<&'a CancelHandle>,
    /// The turn came from a remote client (Iroh, a public URL).
    pub remote: bool,
    /// The machine plugins are built on ([`super::ThisMachine`]; tests answer
    /// for a machine of their own).
    pub machine: &'a dyn super::Machine,
}

/// The action's result, and the card as it ended when one was shown: stored
/// with the conversation, so a reopened chat shows the question and its answer.
pub struct DialogResult {
    pub outcome: ActionOutcome,
    pub record: Option<Value>,
}

/// `progress` is told when the action starts something long (installing a
/// language's tools), so the chat's status line says what is going on.
pub async fn run<P: FnMut(StepText)>(
    dialog: &Dialog<'_>,
    action: &'static str,
    params: &Value,
    progress: &mut P,
) -> Result<DialogResult, AppError> {
    match action {
        ASK_USER => ask_user(dialog, params).await,
        PLAN_PROPOSE => plan_propose(dialog, params).await,
        _ => plugin_dev_start(dialog, progress).await,
    }
}

fn outcome(action: &str, result: &str, data: Value) -> ActionOutcome {
    ActionOutcome {
        action: action.to_string(),
        result: result.to_string(),
        data,
        artifact: None,
        extra_artifacts: vec![],
        owner: None,
    }
}

fn done(action: &str, result: &str, data: Value) -> DialogResult {
    DialogResult { outcome: outcome(action, result, data), record: None }
}

/// The turn was stopped while the card was up.
fn abandoned(action: &str, content: &AskContent) -> DialogResult {
    DialogResult {
        outcome: outcome(
            action,
            "cancelled",
            json!({ "reason": "the turn ended before the user answered. Nothing was decided." }),
        ),
        record: Some(record(content, None)),
    }
}

/// This turn can neither show a card nor speak.
fn unavailable(action: &str) -> DialogResult {
    done(
        action,
        "unavailable",
        json!({
            "reason": "this turn cannot show the user a card. Ask in your reply instead, in plain \
                       words, and wait for their next message.",
        }),
    )
}

/// The card as stored: what was asked, and how it was answered.
fn record(content: &AskContent, reply: Option<&AskReply>) -> Value {
    let mut value = serde_json::to_value(content).unwrap_or_else(|_| json!({}));
    if let Value::Object(map) = &mut value {
        match reply {
            Some(reply) => {
                map.insert(
                    "answer".into(),
                    json!({ "option": reply.option, "text": reply.text }),
                );
            }
            None => {
                map.insert("abandoned".into(), json!(true));
            }
        }
    }
    value
}

/// One line of model-written text, bounded.
fn line(value: &str, max: usize) -> String {
    let joined = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() > max {
        format!("{}…", joined.chars().take(max).collect::<String>())
    } else {
        joined
    }
}

fn text_param(params: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| params.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

/// The reply's typed text, when there is some.
fn typed(reply: &AskReply) -> Option<&str> {
    reply.text.as_deref().map(str::trim).filter(|text| !text.is_empty())
}

// ── ask_user ─────────────────────────────────────────────────────────────

const MAX_OPTIONS: usize = 6;

/// The suggested answers of a question: plain strings, or `{label, detail}`.
fn question_options(params: &Value) -> Vec<AskOption> {
    let Some(items) = ["options", "choices", "answers"]
        .iter()
        .find_map(|key| params.get(*key).and_then(Value::as_array))
    else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            Value::String(label) => Some((label.as_str(), "")),
            Value::Object(map) => {
                let label = ["label", "title", "text", "name"]
                    .iter()
                    .find_map(|key| map.get(*key).and_then(Value::as_str))?;
                let detail = ["detail", "description", "note"]
                    .iter()
                    .find_map(|key| map.get(*key).and_then(Value::as_str))
                    .unwrap_or_default();
                Some((label, detail))
            }
            _ => None,
        })
        .filter(|(label, _)| !label.trim().is_empty())
        .take(MAX_OPTIONS)
        .enumerate()
        .map(|(index, (label, detail))| {
            AskOption::new((index + 1).to_string(), AskLine::plain(line(label, 80)))
                .detail(AskLine::plain(line(detail, 140)))
        })
        .collect()
}

async fn ask_user(dialog: &Dialog<'_>, params: &Value) -> Result<DialogResult, AppError> {
    let question = text_param(params, &["question", "text", "prompt", "message"]);
    if question.is_empty() {
        return Err(AppError::BadRequest("question required".into()));
    }
    let options = question_options(params);
    let mut content = AskContent::new(AskKind::Question, AskLine::plain(line(&question, 400)));
    // A question with suggested answers still takes a free one unless the
    // model closed it; one without any is nothing but its text field.
    content.allow_text =
        options.is_empty() || params.get("allow_text").and_then(Value::as_bool).unwrap_or(true);
    content.options = options;

    let Some(gate) = dialog.gate else {
        return Ok(unavailable(ASK_USER));
    };
    match gate.ask(content.clone(), dialog.cancel).await {
        AskOutcome::Answered(reply) => {
            let picked = reply
                .option
                .as_deref()
                .and_then(|id| content.options.iter().find(|option| option.id == id))
                .map(|option| option.label.text.clone());
            let answer = match (picked.as_deref(), typed(&reply)) {
                (Some(label), Some(text)) => format!("{label} — {text}"),
                (Some(label), None) => label.to_string(),
                (None, Some(text)) => text.to_string(),
                (None, None) => String::new(),
            };
            let note = if answer.is_empty() {
                "The user closed the question without answering. Go on with your best judgement, \
                 or ask in your reply."
                    .to_string()
            } else {
                format!("The user answered: \"{answer}\".")
            };
            Ok(DialogResult {
                outcome: outcome(
                    ASK_USER,
                    "ok",
                    json!({ "question": question, "answer": answer, "note": note }),
                ),
                record: Some(record(&content, Some(&reply))),
            })
        }
        AskOutcome::Abandoned => Ok(abandoned(ASK_USER, &content)),
        AskOutcome::Aloud => Ok(done(
            ASK_USER,
            "ok",
            json!({
                "question": question,
                "asked_aloud": true,
                "note": "There is no card in a spoken turn. Ask the user this question now, aloud, in \
                         one short sentence in their language, then stop: do not call a tool. Their \
                         next message is the answer.",
            }),
        )),
        AskOutcome::Unavailable => Ok(unavailable(ASK_USER)),
    }
}

// ── plan_propose ─────────────────────────────────────────────────────────

const MAX_STEPS: usize = 20;

/// A plan's steps: a list of strings (or `{step|title|text}` objects), or one
/// string with a step per line. Leading list markers are dropped: the card
/// numbers the steps itself.
fn plan_steps(params: &Value) -> Vec<String> {
    let raw: Vec<String> = match params.get("steps").or_else(|| params.get("plan")) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(step) => Some(step.clone()),
                Value::Object(map) => ["step", "title", "text", "description"]
                    .iter()
                    .find_map(|key| map.get(*key).and_then(Value::as_str))
                    .map(str::to_string),
                _ => None,
            })
            .collect(),
        Some(Value::String(text)) => text.lines().map(str::to_string).collect(),
        _ => Vec::new(),
    };
    raw.iter()
        .map(|step| line(unmarked(step), 240))
        .filter(|step| !step.is_empty())
        .take(MAX_STEPS)
        .collect()
}

/// A step without its list marker ("1. ", "2) ", "- "). "3D preview" starts
/// with a digit and is not a numbered step.
fn unmarked(step: &str) -> &str {
    let step = step.trim();
    let digits = step.chars().take_while(char::is_ascii_digit).count();
    let rest = &step[digits..];
    let after = if digits > 0 {
        rest.strip_prefix(['.', ')'])
    } else {
        rest.strip_prefix(['-', '*', '•'])
    };
    match after {
        Some(after) if after.starts_with(' ') => after.trim_start(),
        _ => step,
    }
}

/// The three answers of a plan card.
fn plan_options() -> Vec<AskOption> {
    vec![
        AskOption::new("cancel", AskLine::keyed("chat.ask.plan.cancel", "Cancel")),
        AskOption::new("changes", AskLine::keyed("chat.ask.plan.changes", "Ask for changes")),
        AskOption::new("approve", AskLine::keyed("chat.ask.plan.approve", "Approve")),
    ]
}

async fn plan_propose(dialog: &Dialog<'_>, params: &Value) -> Result<DialogResult, AppError> {
    let steps = plan_steps(params);
    if steps.is_empty() {
        return Err(AppError::BadRequest(
            "steps required: a list of the steps you intend to take, one string each".into(),
        ));
    }
    let plan = Plan {
        title: line(&text_param(params, &["title", "name", "goal"]), 120),
        text: line(&text_param(params, &["summary", "text", "description"]), 600),
        steps,
    };
    let Some(gate) = dialog.gate else {
        return Ok(unavailable(PLAN_PROPOSE));
    };
    let conversation_id = gate.conversation_id();
    // Stored before it is shown: whatever ends the turn, the user's next
    // message can still answer the plan they saw.
    state::propose_plan(dialog.pool, conversation_id, dialog.traveler_id, &plan).await?;

    let title = if plan.title.is_empty() {
        AskLine::keyed("chat.ask.plan.title", "Plan")
    } else {
        AskLine::plain(plan.title.clone())
    };
    let mut content = AskContent::new(AskKind::Plan, title);
    content.text = plan.text.clone();
    content.steps = plan.steps.clone();
    content.options = plan_options();
    content.allow_text = true;

    match gate.ask(content.clone(), dialog.cancel).await {
        AskOutcome::Answered(reply) => {
            let remark = typed(&reply);
            // Approval is the Approve button and nothing else: a remark sent
            // without it is a request for changes, even an enthusiastic one.
            let decision = match reply.option.as_deref() {
                Some("approve") => PlanState::Approved,
                Some("cancel") => PlanState::Cancelled,
                _ => PlanState::Changes,
            };
            state::decide_plan(dialog.pool, conversation_id, dialog.traveler_id, decision, remark).await?;
            let note = match (decision, remark) {
                (PlanState::Approved, Some(remark)) => {
                    format!("The user approved the plan and added: \"{remark}\". Carry it out now.")
                }
                (PlanState::Approved, None) => "The user approved the plan. Carry it out now.".to_string(),
                (PlanState::Cancelled, _) => {
                    "The user cancelled the plan. Do not carry it out; say so briefly.".to_string()
                }
                (_, Some(remark)) => format!(
                    "The user did not approve the plan and asked for changes: \"{remark}\". Revise it \
                     and call plan_propose again."
                ),
                (_, None) => "The user did not approve the plan and wants changes, without saying \
                              which. Ask them what to change."
                    .to_string(),
            };
            Ok(DialogResult {
                outcome: outcome(
                    PLAN_PROPOSE,
                    "ok",
                    json!({ "decision": decision.as_str(), "remark": remark, "note": note }),
                ),
                record: Some(record(&content, Some(&reply))),
            })
        }
        AskOutcome::Abandoned => Ok(abandoned(PLAN_PROPOSE, &content)),
        AskOutcome::Aloud => Ok(done(
            PLAN_PROPOSE,
            "ok",
            json!({
                "decision": "proposed",
                "asked_aloud": true,
                "note": "There is no card in a spoken turn. Tell the user the plan now, aloud, in two \
                         or three short sentences in their language, and ask whether to go ahead. \
                         Then stop: do not call a tool. A plain yes approves it, a plain no cancels \
                         it, anything else is taken as changes they want.",
            }),
        )),
        AskOutcome::Unavailable => Ok(unavailable(PLAN_PROPOSE)),
    }
}

// ── plugin_dev_start ─────────────────────────────────────────────────────

/// The Python / Rust card: each language with what this machine has for it.
///
/// - `ready`: everything is there;
/// - `setup`: something is missing and choosing the language installs it;
/// - `unavailable`: something is missing that cannot be installed from here,
///   so the language cannot be picked. A conversation keeps its language for
///   good and must never be bound to one that leads nowhere.
fn language_card(machine: &dyn super::Machine) -> AskContent {
    let mut content = AskContent::new(
        AskKind::Choice,
        AskLine::keyed("chat.ask.language.title", "Which language for this plugin?"),
    );
    content.options = Language::ALL
        .iter()
        .map(|language| {
            let readiness = machine.readiness(*language);
            let (state, detail) = match (language, readiness.ready()) {
                (Language::Python, true) => (
                    "ready",
                    AskLine::keyed("chat.ask.language.pythonReady", "Ready. Runs at once, nothing to compile."),
                ),
                (Language::Rust, true) => (
                    "ready",
                    AskLine::keyed(
                        "chat.ask.language.rustReady",
                        "Ready. The first build takes a few minutes.",
                    ),
                ),
                (_, false) if readiness.fetchable => (
                    "setup",
                    AskLine::keyed(
                        "chat.ask.language.needsDownload",
                        "Not on this machine yet. Choosing it installs, in a few minutes:",
                    ),
                ),
                (_, false) => (
                    "unavailable",
                    AskLine::keyed("chat.ask.language.needsInstall", "Cannot be used on this machine yet. Missing:"),
                ),
            };
            AskOption::new(language.as_str(), AskLine::plain(language.label()))
                .detail(detail)
                .state(state)
                .notes(readiness.missing)
        })
        .collect();
    content
}

fn language_data(language: Language, already: bool) -> Value {
    let lead = if already {
        format!("This chat already builds plugins in {}; the user chose it earlier.", language.label())
    } else {
        format!("The user chose {} for this chat.", language.label())
    };
    json!({
        "language": language.as_str(),
        "already": already,
        "commands": super::commands_for(Some(language)),
        "note": format!("{lead} Everything it needs is on this machine."),
    })
}

/// Nothing was chosen: the card as it ended, and why.
fn not_chosen(content: &AskContent, reply: Option<&AskReply>, reason: String) -> DialogResult {
    DialogResult {
        outcome: outcome(PLUGIN_DEV_START, "cancelled", json!({ "reason": reason })),
        record: Some(record(content, reply)),
    }
}

/// Install what the chosen language lacks. The user agreed on the card, which
/// listed it; the administrator's password, when system packages need it, is
/// asked on a card of its own and goes to the installer and nowhere else.
async fn set_up<P: FnMut(StepText)>(
    dialog: &Dialog<'_>,
    gate: &AskGate,
    language: Language,
    progress: &mut P,
) -> Result<(), String> {
    let password = if dialog.machine.needs_password(language).await {
        let mut content = AskContent::new(
            AskKind::Question,
            AskLine::keyed(
                "chat.ask.password.title",
                "Administrator password, to install the C compiler (gcc)",
            ),
        );
        content.allow_text = true;
        content.secret = true;
        match gate.ask(content, dialog.cancel).await {
            AskOutcome::Answered(reply) => match typed(&reply) {
                Some(password) => Some(password.to_string()),
                None => return Err("the user gave no administrator password. Nothing was installed.".into()),
            },
            _ => return Err("the administrator password was not given. Nothing was installed.".into()),
        }
    } else {
        None
    };
    progress(StepText::new(
        "step.installingLanguageTools",
        "Installing what the language needs… this takes a few minutes",
    ));
    dialog.machine.install(language, password.as_deref(), dialog.cancel).await
}

async fn plugin_dev_start<P: FnMut(StepText)>(
    dialog: &Dialog<'_>,
    progress: &mut P,
) -> Result<DialogResult, AppError> {
    // A plugin is code that runs on this machine: like installing one, building
    // one is for the machine's own session, never for a remote client.
    if dialog.remote {
        return Ok(done(
            PLUGIN_DEV_START,
            "refused",
            json!({
                "reason": "plugins can only be built from the machine itself, not from a remote \
                           connection. Nothing was started.",
            }),
        ));
    }
    let Some(gate) = dialog.gate else {
        return Ok(unavailable(PLUGIN_DEV_START));
    };
    let conversation_id = gate.conversation_id();
    let current = state::load(dialog.pool, conversation_id, dialog.traveler_id).await?;
    if let Some(language) = current.language {
        return Ok(done(PLUGIN_DEV_START, "ok", language_data(language, true)));
    }

    let content = language_card(dialog.machine);
    match gate.ask(content.clone(), dialog.cancel).await {
        AskOutcome::Answered(reply) => {
            let Some(language) = reply.option.as_deref().and_then(Language::parse) else {
                return Ok(not_chosen(
                    &content,
                    Some(&reply),
                    "the user closed the card without choosing a language. Nothing was started. Plugins are \
                     built in Python or in Rust only: if the user still wants one, call plugin_dev_start again so \
                     they can choose on the card; do not offer other languages."
                        .into(),
                ));
            };
            let readiness = dialog.machine.readiness(language);
            if !readiness.ready() {
                if !readiness.fetchable {
                    return Ok(not_chosen(
                        &content,
                        None,
                        format!(
                            "{} cannot be used on this machine yet (missing: {}). Nothing was chosen: tell the user.",
                            language.label(),
                            readiness.missing_text().join("; ")
                        ),
                    ));
                }
                // The language is the chat's only once its tools are there:
                // a failed installation leaves the chat free to choose again.
                if let Err(error) = set_up(dialog, gate, language, progress).await {
                    return Ok(not_chosen(
                        &content,
                        None,
                        format!(
                            "{} could not be set up: {error} Nothing was chosen: tell the user what went wrong.",
                            language.label()
                        ),
                    ));
                }
                let after = dialog.machine.readiness(language);
                if !after.ready() {
                    return Ok(not_chosen(
                        &content,
                        None,
                        format!(
                            "the installation finished but {} still lacks: {}. Nothing was chosen: tell the user.",
                            language.label(),
                            after.missing_text().join("; ")
                        ),
                    ));
                }
            }
            let chosen =
                state::choose_language(dialog.pool, conversation_id, dialog.traveler_id, language).await?;
            Ok(DialogResult {
                outcome: outcome(PLUGIN_DEV_START, "ok", language_data(chosen, false)),
                record: Some(record(&content, Some(&reply))),
            })
        }
        AskOutcome::Abandoned => Ok(abandoned(PLUGIN_DEV_START, &content)),
        AskOutcome::Aloud => {
            state::set_pending(dialog.pool, conversation_id, dialog.traveler_id, Some(Pending::Language))
                .await?;
            let spoken: Vec<String> = Language::ALL
                .iter()
                .map(|language| {
                    let readiness = dialog.machine.readiness(*language);
                    if readiness.ready() {
                        format!("{} is ready on this machine", language.label())
                    } else if readiness.fetchable {
                        format!(
                            "{} has to be installed first, which is done from the text chat (it lacks: {})",
                            language.label(),
                            readiness.missing_text().join("; ")
                        )
                    } else {
                        format!(
                            "{} cannot be used on this machine yet (it lacks: {})",
                            language.label(),
                            readiness.missing_text().join("; ")
                        )
                    }
                })
                .collect();
            Ok(done(
                PLUGIN_DEV_START,
                "ok",
                json!({
                    "asked_aloud": true,
                    "note": format!(
                        "There is no card in a spoken turn. Tell the user now, aloud, in one or two \
                         short sentences in their language, which languages the plugin can be built \
                         in on this machine, and ask which they want: {}. Then stop: do not call a \
                         tool and do not choose for them. Their next message chooses.",
                        spoken.join(". ")
                    ),
                }),
            ))
        }
        AskOutcome::Unavailable => Ok(unavailable(PLUGIN_DEV_START)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::ask::{AskPrompt, AskRegistry};
    use std::sync::{Arc, Mutex};

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

    /// A gate whose card is answered the way the chat would, and what it showed.
    fn gate(reply: Option<AskReply>, interactive: bool, voice: bool) -> (AskGate, Arc<Mutex<Vec<AskPrompt>>>) {
        let registry = AskRegistry::default();
        let shown: Arc<Mutex<Vec<AskPrompt>>> = Arc::default();
        let (answering, seen) = (registry.clone(), shown.clone());
        let gate = AskGate::new(
            registry,
            "turn".into(),
            "u".into(),
            "chat".into(),
            interactive,
            Box::new(move |prompt| {
                seen.lock().unwrap().push(prompt.clone());
                if let Some(reply) = reply.clone() {
                    answering.resolve("turn", &prompt.request_id, "u", reply);
                }
            }),
        )
        .spoken(voice);
        (gate, shown)
    }

    fn reply(option: Option<&str>, text: Option<&str>) -> Option<AskReply> {
        Some(AskReply { option: option.map(str::to_string), text: text.map(str::to_string) })
    }

    /// A machine of the test's own: Python has no kit; Rust lacks its tools
    /// until `install` has run, when `rust_ready` starts false.
    struct TestMachine {
        rust_ready: std::sync::atomic::AtomicBool,
        needs_password: bool,
        install_fails: bool,
        installs: Mutex<Vec<Option<String>>>,
    }

    impl TestMachine {
        fn new(rust_ready: bool) -> Self {
            Self {
                rust_ready: rust_ready.into(),
                needs_password: false,
                install_fails: false,
                installs: Mutex::default(),
            }
        }
    }

    #[async_trait::async_trait]
    impl crate::services::plugin_dev::Machine for TestMachine {
        fn readiness(&self, language: Language) -> crate::services::plugin_dev::Readiness {
            use std::sync::atomic::Ordering;
            match language {
                Language::Rust if self.rust_ready.load(Ordering::SeqCst) => Default::default(),
                Language::Rust => crate::services::plugin_dev::Readiness {
                    missing: vec![AskLine::plain("Rust compiler")],
                    fetchable: true,
                },
                Language::Python => crate::services::plugin_dev::Readiness {
                    missing: vec![AskLine::plain("Python plugin kit")],
                    fetchable: false,
                },
            }
        }

        async fn needs_password(&self, _language: Language) -> bool {
            self.needs_password
        }

        async fn install(
            &self,
            _language: Language,
            password: Option<&str>,
            _cancel: Option<&CancelHandle>,
        ) -> Result<(), String> {
            self.installs.lock().unwrap().push(password.map(str::to_string));
            if self.install_fails {
                return Err("the download failed.".into());
            }
            self.rust_ready.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    /// A machine with Rust ready, shared by the tests that are not about setup.
    static READY: std::sync::LazyLock<TestMachine> = std::sync::LazyLock::new(|| TestMachine::new(true));

    fn dialog<'a>(pool: &'a SqlitePool, gate: &'a AskGate) -> Dialog<'a> {
        Dialog { pool, traveler_id: "u", gate: Some(gate), cancel: None, remote: false, machine: &*READY }
    }

    /// Run an action with no one listening to its progress.
    async fn go(dialog: &Dialog<'_>, action: &'static str, params: &Value) -> Result<DialogResult, AppError> {
        super::run(dialog, action, params, &mut |_| {}).await
    }

    #[test]
    fn names_the_model_uses_resolve_to_the_three_actions() {
        assert_eq!(canonical("Ask_User"), Some(ASK_USER));
        assert_eq!(canonical("propose_plan"), Some(PLAN_PROPOSE));
        assert_eq!(canonical("create_plugin"), Some(PLUGIN_DEV_START));
        assert_eq!(canonical("plugin_activate"), None);
        assert_eq!(canonical("web_search"), None);
    }

    #[tokio::test]
    async fn a_question_returns_the_users_answer() {
        let pool = pool().await;
        let (gate, shown) = gate(reply(Some("2"), None), true, false);
        let result = go(
            &dialog(&pool, &gate),
            ASK_USER,
            &json!({ "question": "Light or dark?", "options": ["Light", { "label": "Dark", "detail": "easier at night" }] }),
        )
        .await
        .unwrap();
        assert_eq!(result.outcome.result, "ok");
        assert_eq!(result.outcome.data["answer"], "Dark");
        let shown = shown.lock().unwrap();
        assert_eq!(shown[0].content.options.len(), 2);
        assert_eq!(shown[0].content.options[1].detail.text, "easier at night");
        assert!(shown[0].content.allow_text);
        let record = result.record.unwrap();
        assert_eq!(record["kind"], "question");
        assert_eq!(record["answer"]["option"], "2");
    }

    #[tokio::test]
    async fn a_question_without_text_is_refused_and_a_spoken_one_is_asked_aloud() {
        let pool = pool().await;
        let (card, _) = gate(None, true, false);
        assert!(go(&dialog(&pool, &card), ASK_USER, &json!({})).await.is_err());

        let (spoken, shown) = gate(None, false, true);
        let result = go(&dialog(&pool, &spoken), ASK_USER, &json!({ "question": "Which city?" })).await.unwrap();
        assert_eq!(result.outcome.result, "ok");
        assert_eq!(result.outcome.data["asked_aloud"], true);
        assert!(result.record.is_none());
        assert!(shown.lock().unwrap().is_empty(), "no card in a spoken turn");

        let (silent, _) = gate(None, false, false);
        let result = go(&dialog(&pool, &silent), ASK_USER, &json!({ "question": "Which city?" })).await.unwrap();
        assert_eq!(result.outcome.result, "unavailable");
    }

    #[tokio::test]
    async fn only_the_approve_button_approves_a_plan() {
        let pool = pool().await;
        let params = json!({ "title": "Notes", "steps": ["1. Scaffold", "- Write the window", "3D preview"] });

        let (gate_changes, shown) = gate(reply(None, Some("great, go!")), true, false);
        let result = go(&dialog(&pool, &gate_changes), PLAN_PROPOSE, &params).await.unwrap();
        assert_eq!(result.outcome.data["decision"], "changes");
        assert_eq!(
            shown.lock().unwrap()[0].content.steps,
            vec!["Scaffold", "Write the window", "3D preview"]
        );
        let stored = state::load(&pool, "chat", "u").await.unwrap();
        assert_eq!(stored.plan_state, PlanState::Changes);
        assert_eq!(stored.plan_remark.as_deref(), Some("great, go!"));
        assert_eq!(stored.pending, None);

        let (gate_ok, _) = gate(reply(Some("approve"), None), true, false);
        let result = go(&dialog(&pool, &gate_ok), PLAN_PROPOSE, &params).await.unwrap();
        assert_eq!(result.outcome.data["decision"], "approved");
        assert_eq!(state::load(&pool, "chat", "u").await.unwrap().plan_state, PlanState::Approved);

        let (gate_no, _) = gate(reply(Some("cancel"), None), true, false);
        let result = go(&dialog(&pool, &gate_no), PLAN_PROPOSE, &params).await.unwrap();
        assert_eq!(result.outcome.data["decision"], "cancelled");

        let (empty, _) = gate(None, true, false);
        assert!(go(&dialog(&pool, &empty), PLAN_PROPOSE, &json!({ "title": "x" })).await.is_err());
    }

    #[tokio::test]
    async fn a_spoken_plan_waits_for_the_next_message() {
        let pool = pool().await;
        let (spoken, _) = gate(None, false, true);
        let result = go(&dialog(&pool, &spoken), PLAN_PROPOSE, &json!({ "steps": ["One", "Two"] })).await.unwrap();
        assert_eq!(result.outcome.data["decision"], "proposed");
        let stored = state::load(&pool, "chat", "u").await.unwrap();
        assert_eq!(stored.plan_state, PlanState::Proposed);
        assert_eq!(stored.pending, Some(Pending::Plan));
    }

    #[tokio::test]
    async fn the_language_is_the_users_choice_and_is_asked_once() {
        let pool = pool().await;
        let (gate_rust, shown) = gate(reply(Some("rust"), None), true, false);
        let result = go(&dialog(&pool, &gate_rust), PLUGIN_DEV_START, &json!({ "language": "python" })).await.unwrap();
        assert_eq!(result.outcome.result, "ok");
        assert_eq!(result.outcome.data["language"], "rust", "a language the model passes is ignored");
        assert_eq!(result.outcome.data["already"], false);
        {
            let shown = shown.lock().unwrap();
            let ids: Vec<&str> = shown[0].content.options.iter().map(|o| o.id.as_str()).collect();
            assert_eq!(ids, vec!["python", "rust"]);
            let states: Vec<&str> = shown[0].content.options.iter().map(|o| o.state.as_str()).collect();
            assert_eq!(states, vec!["unavailable", "ready"]);
            assert_eq!(shown[0].content.options[0].notes[0].text, "Python plugin kit");
        }

        // The same chat again: no card, the same language, even answered otherwise.
        let (gate_python, shown) = gate(reply(Some("python"), None), true, false);
        let result = go(&dialog(&pool, &gate_python), PLUGIN_DEV_START, &json!({})).await.unwrap();
        assert_eq!(result.outcome.data["language"], "rust");
        assert_eq!(result.outcome.data["already"], true);
        assert!(shown.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_language_the_machine_lacks_is_not_a_choice() {
        let pool = pool().await;
        let (card, _) = gate(reply(Some("python"), None), true, false);
        let result = go(&dialog(&pool, &card), PLUGIN_DEV_START, &json!({})).await.unwrap();
        assert_eq!(result.outcome.result, "cancelled");
        assert!(result.outcome.data["reason"].as_str().unwrap().contains("Python cannot be used on this machine yet"));
        assert_eq!(state::load(&pool, "chat", "u").await.unwrap().language, None, "the chat is not bound to it");
        assert_eq!(result.record.unwrap()["abandoned"], true);
    }

    /// The gate of a machine that needs the password: the choice card is
    /// answered with Rust, the password card with `password`.
    fn setup_gate(password: Option<&'static str>) -> (AskGate, Arc<Mutex<Vec<AskPrompt>>>) {
        let registry = AskRegistry::default();
        let shown: Arc<Mutex<Vec<AskPrompt>>> = Arc::default();
        let (answering, seen) = (registry.clone(), shown.clone());
        let gate = AskGate::new(
            registry,
            "turn".into(),
            "u".into(),
            "chat".into(),
            true,
            Box::new(move |prompt| {
                seen.lock().unwrap().push(prompt.clone());
                let reply = if prompt.content.secret {
                    AskReply { option: None, text: password.map(str::to_string) }
                } else {
                    AskReply { option: Some("rust".into()), text: None }
                };
                answering.resolve("turn", &prompt.request_id, "u", reply);
            }),
        );
        (gate, shown)
    }

    #[tokio::test]
    async fn choosing_a_language_the_machine_lacks_installs_it_first() {
        let pool = pool().await;
        let machine = TestMachine::new(false);
        let (gate, shown) = gate(reply(Some("rust"), None), true, false);
        let dialog = Dialog { machine: &machine, ..dialog(&pool, &gate) };
        let mut steps: Vec<&str> = Vec::new();
        let result = super::run(&dialog, PLUGIN_DEV_START, &json!({}), &mut |step| steps.push(step.key)).await.unwrap();
        assert_eq!(result.outcome.result, "ok", "{:?}", result.outcome.data);
        assert_eq!(result.outcome.data["language"], "rust");
        assert_eq!(*machine.installs.lock().unwrap(), vec![None], "installed once, no password needed");
        assert_eq!(steps, vec!["step.installingLanguageTools"]);
        assert_eq!(state::load(&pool, "chat", "u").await.unwrap().language, Some(Language::Rust));
        let shown = shown.lock().unwrap();
        assert_eq!(shown.len(), 1, "no password card");
        assert_eq!(shown[0].content.options[1].state, "setup");
        assert_eq!(shown[0].content.options[1].notes[0].text, "Rust compiler");
    }

    #[tokio::test]
    async fn the_administrator_password_goes_to_the_installer_and_is_not_kept() {
        let pool = pool().await;
        let machine = TestMachine { needs_password: true, ..TestMachine::new(false) };
        let (gate, shown) = setup_gate(Some("s3cret"));
        let dialog = Dialog { machine: &machine, ..dialog(&pool, &gate) };
        let result = go(&dialog, PLUGIN_DEV_START, &json!({})).await.unwrap();
        assert_eq!(result.outcome.result, "ok");
        assert_eq!(*machine.installs.lock().unwrap(), vec![Some("s3cret".to_string())]);
        {
            let shown = shown.lock().unwrap();
            assert_eq!(shown.len(), 2);
            assert!(shown[1].content.secret && shown[1].content.allow_text);
            assert_eq!(shown[1].content.title.key.as_deref(), Some("chat.ask.password.title"));
        }
        // Neither what the model reads nor what the conversation stores has it.
        let record = result.record.unwrap();
        assert_eq!(record["kind"], "choice", "only the choice card is stored");
        assert!(!record.to_string().contains("s3cret"));
        assert!(!result.outcome.data.to_string().contains("s3cret"));
    }

    #[tokio::test]
    async fn a_setup_that_fails_leaves_the_chat_free_to_choose_again() {
        let pool = pool().await;
        // No password given.
        let machine = TestMachine { needs_password: true, ..TestMachine::new(false) };
        let (no_password, _) = setup_gate(None);
        let result =
            go(&Dialog { machine: &machine, ..dialog(&pool, &no_password) }, PLUGIN_DEV_START, &json!({})).await.unwrap();
        assert_eq!(result.outcome.result, "cancelled");
        assert!(machine.installs.lock().unwrap().is_empty(), "nothing is installed without it");
        assert_eq!(state::load(&pool, "chat", "u").await.unwrap().language, None);

        // The installer fails.
        let machine = TestMachine { install_fails: true, ..TestMachine::new(false) };
        let (gate, _) = gate(reply(Some("rust"), None), true, false);
        let result = go(&Dialog { machine: &machine, ..dialog(&pool, &gate) }, PLUGIN_DEV_START, &json!({})).await.unwrap();
        assert_eq!(result.outcome.result, "cancelled");
        assert!(result.outcome.data["reason"].as_str().unwrap().contains("the download failed"));
        assert_eq!(state::load(&pool, "chat", "u").await.unwrap().language, None);
    }

    #[tokio::test]
    async fn a_closed_card_or_a_remote_client_starts_nothing() {
        let pool = pool().await;
        let (closed, _) = gate(reply(None, None), true, false);
        let result = go(&dialog(&pool, &closed), PLUGIN_DEV_START, &json!({})).await.unwrap();
        assert_eq!(result.outcome.result, "cancelled");
        assert_eq!(state::load(&pool, "chat", "u").await.unwrap().language, None);

        let (card, shown) = gate(reply(Some("rust"), None), true, false);
        let remote = Dialog { remote: true, ..dialog(&pool, &card) };
        let result = go(&remote, PLUGIN_DEV_START, &json!({})).await.unwrap();
        assert_eq!(result.outcome.result, "refused");
        assert!(shown.lock().unwrap().is_empty());
        assert_eq!(state::load(&pool, "chat", "u").await.unwrap().language, None);
    }

    #[tokio::test]
    async fn a_spoken_start_parks_the_choice() {
        let pool = pool().await;
        let (spoken, _) = gate(None, false, true);
        let result = go(&dialog(&pool, &spoken), PLUGIN_DEV_START, &json!({})).await.unwrap();
        assert_eq!(result.outcome.data["asked_aloud"], true);
        assert_eq!(state::load(&pool, "chat", "u").await.unwrap().pending, Some(Pending::Language));
    }
}
