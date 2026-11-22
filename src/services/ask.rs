//! Questions the assistant asks the user in the middle of a turn.
//!
//! Three shapes share one round trip, the same one a terminal command's
//! allow/deny uses ([`super::approval`]): the turn emits an `ask` event,
//! parks on a one-shot channel, and `POST /api/agent/answer` resolves it.
//!
//! - **question**: free text, with optional suggested answers;
//! - **plan**: the steps the assistant intends to take, approved, sent back
//!   with a remark, or cancelled;
//! - **choice**: one of a fixed set of options, each with its own state (the
//!   plugin builder's Python / Rust card).
//!
//! A turn that cannot show a card (a spoken turn with the chat closed) does
//! not park: the caller has the assistant ask aloud and reads the user's next
//! message as the answer ([`plain_answer`], [`named_option`]).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::oneshot;

use crate::services::agent_cancel::CancelHandle;

/// What kind of card the chat draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AskKind {
    Question,
    Plan,
    Choice,
}

impl AskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AskKind::Question => "question",
            AskKind::Plan => "plan",
            AskKind::Choice => "choice",
        }
    }
}

/// A line the core wrote: its English, and the catalogue key the web client
/// translates it with. A line the model wrote has no key and is shown as is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AskLine {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

impl AskLine {
    pub fn plain(text: impl Into<String>) -> Self {
        Self { text: text.into(), key: None }
    }

    pub fn keyed(key: impl Into<String>, text: impl Into<String>) -> Self {
        Self { text: text.into(), key: Some(key.into()) }
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// One answer the user can pick.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AskOption {
    pub id: String,
    pub label: AskLine,
    /// One line under the label ("Ready. Runs at once.").
    #[serde(skip_serializing_if = "AskLine::is_empty")]
    pub detail: AskLine,
    /// What picking this option still needs, item by item (what is missing
    /// on this machine). Empty when nothing is.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<AskLine>,
    /// `ready`; `setup` when picking the option installs what [`Self::notes`]
    /// lists; `unavailable` when it lists what cannot be installed from here,
    /// and the card does not let the option be picked.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub state: String,
}

impl AskOption {
    pub fn new(id: impl Into<String>, label: AskLine) -> Self {
        Self { id: id.into(), label, ..Self::default() }
    }

    pub fn detail(mut self, detail: AskLine) -> Self {
        self.detail = detail;
        self
    }

    pub fn state(mut self, state: impl Into<String>) -> Self {
        self.state = state.into();
        self
    }

    pub fn notes(mut self, notes: Vec<AskLine>) -> Self {
        self.notes = notes;
        self
    }
}

/// What is asked.
#[derive(Debug, Clone, Serialize)]
pub struct AskContent {
    pub kind: AskKind,
    pub title: AskLine,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// A plan's steps, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<AskOption>,
    /// The card has a text field (a question's free answer, a plan's remark).
    pub allow_text: bool,
    /// The text is a secret (a password): the field hides it, and the card is
    /// neither stored with the conversation nor shown with its answer.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub secret: bool,
}

impl AskContent {
    pub fn new(kind: AskKind, title: AskLine) -> Self {
        Self {
            kind,
            title,
            text: String::new(),
            steps: Vec::new(),
            options: Vec::new(),
            allow_text: false,
            secret: false,
        }
    }
}

/// The event handed to the transport: the content plus the id to answer.
#[derive(Debug, Clone, Serialize)]
pub struct AskPrompt {
    pub request_id: String,
    #[serde(flatten)]
    pub content: AskContent,
}

/// The user's answer to one card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AskReply {
    /// Id of the option picked (`approve`, `changes`, `cancel` for a plan).
    pub option: Option<String>,
    /// Text typed in the card.
    pub text: Option<String>,
}

/// How a parked question ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskOutcome {
    Answered(AskReply),
    /// The turn was stopped, or its stream closed, before an answer came.
    Abandoned,
    /// This turn cannot show a card: ask aloud (a spoken turn) and read the
    /// next message as the answer.
    Aloud,
    /// This turn can neither show a card nor speak.
    Unavailable,
}

struct Pending {
    turn_id: String,
    traveler_id: String,
    sender: oneshot::Sender<AskReply>,
}

/// Live questions, by request id.
#[derive(Clone, Default)]
pub struct AskRegistry {
    pending: Arc<Mutex<HashMap<String, Pending>>>,
}

/// Removes the request when the waiting future ends, whatever ended it.
struct PendingGuard<'a> {
    registry: &'a AskRegistry,
    request_id: String,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        self.registry.discard(&self.request_id);
    }
}

impl AskRegistry {
    fn request(&self, turn_id: &str, traveler_id: &str) -> (String, oneshot::Receiver<AskReply>) {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (sender, receiver) = oneshot::channel();
        self.lock().insert(
            request_id.clone(),
            Pending {
                turn_id: turn_id.to_string(),
                traveler_id: traveler_id.to_string(),
                sender,
            },
        );
        (request_id, receiver)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Pending>> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Hand the user's answer to the turn waiting for it. False when there is
    /// no such question: already answered, its turn ended, or it belongs to
    /// another account or turn.
    pub fn resolve(&self, turn_id: &str, request_id: &str, traveler_id: &str, reply: AskReply) -> bool {
        let mut pending = self.lock();
        match pending.get(request_id) {
            Some(entry) if entry.turn_id == turn_id && entry.traveler_id == traveler_id => {}
            _ => return false,
        }
        match pending.remove(request_id) {
            Some(entry) => entry.sender.send(reply).is_ok(),
            None => false,
        }
    }

    pub fn discard(&self, request_id: &str) {
        self.lock().remove(request_id);
    }
}

/// One turn's handle to the question round trip.
pub struct AskGate {
    registry: AskRegistry,
    turn_id: String,
    traveler_id: String,
    conversation_id: String,
    /// The transport can show a card mid-turn (the text chat's stream).
    interactive: bool,
    /// A spoken turn: without a card, the assistant asks aloud.
    voice: bool,
    emit: Box<dyn Fn(AskPrompt) + Send + Sync>,
}

impl AskGate {
    pub fn new(
        registry: AskRegistry,
        turn_id: String,
        traveler_id: String,
        conversation_id: String,
        interactive: bool,
        emit: Box<dyn Fn(AskPrompt) + Send + Sync>,
    ) -> Self {
        Self { registry, turn_id, traveler_id, conversation_id, interactive, voice: false, emit }
    }

    /// Mark the turn as spoken (speech in, speech out).
    pub fn spoken(mut self, voice: bool) -> Self {
        self.voice = voice;
        self
    }

    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    /// Whether [`Self::ask`] shows a card (and waits) on this turn.
    pub fn shows_card(&self) -> bool {
        self.interactive
    }

    /// Show the card and wait for the answer.
    pub async fn ask(&self, content: AskContent, cancel: Option<&CancelHandle>) -> AskOutcome {
        if !self.interactive {
            return if self.voice { AskOutcome::Aloud } else { AskOutcome::Unavailable };
        }
        let (request_id, receiver) = self.registry.request(&self.turn_id, &self.traveler_id);
        let _pending = PendingGuard { registry: &self.registry, request_id: request_id.clone() };
        (self.emit)(AskPrompt { request_id, content });

        let reply = match cancel {
            Some(handle) => tokio::select! {
                biased;
                _ = handle.cancelled() => None,
                answer = receiver => answer.ok(),
            },
            None => receiver.await.ok(),
        };
        match reply {
            Some(reply) => AskOutcome::Answered(reply),
            None => AskOutcome::Abandoned,
        }
    }
}

/// A short message read as agreement or refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlainAnswer {
    Yes,
    No,
    /// Neither: a remark, a new request, anything longer than a few words.
    Other,
}

/// Read a message as a plain yes or no, with the same strict rule a spoken
/// consent uses: "yes but add a search box" is a remark, not an approval.
pub fn plain_answer(message: &str) -> PlainAnswer {
    match crate::services::approval::spoken_answer(message) {
        crate::services::approval::SpokenAnswer::Yes => PlainAnswer::Yes,
        crate::services::approval::SpokenAnswer::No => PlainAnswer::No,
        crate::services::approval::SpokenAnswer::Other => PlainAnswer::Other,
    }
}

/// The one option a short message names, by id or label ("rust", "in Python
/// please"). `None` when it names none of them or more than one, or runs
/// longer than a short answer: a guess must never stand in for a choice.
pub fn named_option<'a>(message: &str, options: &'a [(&'a str, &'a str)]) -> Option<&'a str> {
    let words: Vec<String> = message
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    if words.is_empty() || words.len() > 8 {
        return None;
    }
    let mut found: Option<&str> = None;
    for (id, label) in options {
        let hit = words.iter().any(|w| w == &id.to_lowercase() || w == &label.to_lowercase());
        if hit {
            if found.is_some() {
                return None;
            }
            found = Some(id);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(registry: &AskRegistry, interactive: bool, voice: bool, answer: Option<AskReply>) -> AskGate {
        let answering = registry.clone();
        AskGate::new(
            registry.clone(),
            "turn".into(),
            "user".into(),
            "chat".into(),
            interactive,
            Box::new(move |prompt| {
                if let Some(reply) = answer.clone() {
                    assert!(answering.resolve("turn", &prompt.request_id, "user", reply));
                }
            }),
        )
        .spoken(voice)
    }

    fn question() -> AskContent {
        AskContent::new(AskKind::Question, AskLine::plain("Which colour?"))
    }

    #[tokio::test]
    async fn the_answer_reaches_the_waiting_turn() {
        let registry = AskRegistry::default();
        let reply = AskReply { option: Some("blue".into()), text: None };
        let outcome = gate(&registry, true, false, Some(reply.clone())).ask(question(), None).await;
        assert_eq!(outcome, AskOutcome::Answered(reply));
        assert!(registry.lock().is_empty(), "an answered question leaves nothing behind");
    }

    #[tokio::test]
    async fn another_account_or_turn_cannot_answer() {
        let registry = AskRegistry::default();
        let (request_id, _receiver) = registry.request("turn", "user");
        assert!(!registry.resolve("turn", &request_id, "someone-else", AskReply::default()));
        assert!(!registry.resolve("other-turn", &request_id, "user", AskReply::default()));
        assert!(!registry.resolve("turn", "unknown", "user", AskReply::default()));
        assert!(registry.resolve("turn", &request_id, "user", AskReply::default()));
        assert!(!registry.resolve("turn", &request_id, "user", AskReply::default()), "answered once");
    }

    #[tokio::test]
    async fn a_stopped_turn_abandons_its_question() {
        let registry = AskRegistry::default();
        let turns = crate::services::agent_cancel::TurnRegistry::default();
        let handle = turns.register("turn", "user");
        handle.cancel();
        let outcome = gate(&registry, true, false, None).ask(question(), Some(&handle)).await;
        assert_eq!(outcome, AskOutcome::Abandoned);
        assert!(registry.lock().is_empty());
    }

    #[tokio::test]
    async fn a_turn_without_a_card_asks_aloud_or_not_at_all() {
        let registry = AskRegistry::default();
        assert_eq!(gate(&registry, false, true, None).ask(question(), None).await, AskOutcome::Aloud);
        assert_eq!(gate(&registry, false, false, None).ask(question(), None).await, AskOutcome::Unavailable);
        // A spoken turn with the chat open still gets the card.
        let reply = AskReply { option: None, text: Some("green".into()) };
        assert_eq!(
            gate(&registry, true, true, Some(reply.clone())).ask(question(), None).await,
            AskOutcome::Answered(reply)
        );
    }

    #[test]
    fn a_plain_answer_is_short_and_unambiguous() {
        assert_eq!(plain_answer("Sì, procedi"), PlainAnswer::Yes);
        assert_eq!(plain_answer("ok"), PlainAnswer::Yes);
        assert_eq!(plain_answer("no, annulla"), PlainAnswer::No);
        assert_eq!(plain_answer("yes but add a search box to the window first"), PlainAnswer::Other);
        assert_eq!(plain_answer("add a search box"), PlainAnswer::Other);
    }

    #[test]
    fn a_named_option_is_exactly_one_of_them() {
        let options = [("python", "Python"), ("rust", "Rust")];
        assert_eq!(named_option("Rust", &options), Some("rust"));
        assert_eq!(named_option("in python, please", &options), Some("python"));
        assert_eq!(named_option("facciamolo in Rust.", &options), Some("rust"));
        assert_eq!(named_option("python or rust?", &options), None, "both named");
        assert_eq!(named_option("the fast one", &options), None);
        assert_eq!(named_option("rusty", &options), None, "whole words only");
        assert_eq!(
            named_option("I was thinking that maybe we could try it in rust this time", &options),
            None,
            "too long to be an answer"
        );
    }
}
