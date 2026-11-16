//! Per-command approval for the assistant's terminal tool.
//!
//! The agent loop is a long-lived server-side task, so consent is a round trip:
//! before `terminal_exec` runs, the turn emits an `approval` SSE event and
//! parks on a one-shot channel. `POST /api/agent/approve` resolves that waiting
//! request with **Allow** or **Deny**; **Allow for this chat** also remembers
//! the choice on the conversation, so later commands in the same chat run
//! without asking.
//!
//! The chat's own safety guard ([`super::terminal_guard`]) runs before this:
//! a command that could destroy the system never reaches the user as a
//! question, and no conversation-level consent can unblock it.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use parking_lot::RwLock;
use tokio::sync::oneshot;

use crate::services::agent_cancel::CancelHandle;

/// The user's answer to one prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
}

/// What the runner may do with the command.
pub enum Authorization {
    Allow,
    /// A spoken turn: there is no card to show, so the assistant asks aloud
    /// and the user's next spoken answer decides (see [`VoiceAsk`]).
    AskAloud,
    /// The user answered "Deny".
    Denied(&'static str),
    /// This turn cannot ask the user (no interactive transport), so the
    /// command is refused rather than run unapproved.
    Unavailable(&'static str),
}

/// Prompt handed to the transport (the SSE writer) for the chat UI.
#[derive(Debug, Clone)]
pub struct ApprovalPrompt {
    pub request_id: String,
    pub action: String,
    pub command: String,
}

struct Pending {
    turn_id: String,
    traveler_id: String,
    conversation_id: String,
    sender: oneshot::Sender<Decision>,
}

/// Removes the request when the waiting future ends — answered, cancelled or
/// dropped with its turn — so no stale prompt is left in the registry.
struct PendingGuard<'a> {
    registry: &'a ApprovalRegistry,
    request_id: String,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        self.registry.discard(&self.request_id);
    }
}

/// A gated call waiting for the user's spoken yes or no.
///
/// A voice turn cannot show the allow/deny card, so the call is parked here
/// with its exact parameters and the assistant asks aloud. If the very next
/// spoken message in that conversation is a plain yes, that stored call — not
/// a new one the model writes — is what runs. Anything else drops it. Typed
/// messages never answer it: consent for a spoken request is spoken.
#[derive(Debug, Clone)]
pub struct VoiceAsk {
    pub traveler_id: String,
    /// The action name the model used, and its parameters as they would run.
    pub action: String,
    pub params: serde_json::Value,
    asked_at: std::time::Instant,
}

/// How long a spoken question stays answerable.
const VOICE_ASK_TTL: std::time::Duration = std::time::Duration::from_secs(120);

/// What a spoken answer to a parked question means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpokenAnswer {
    Yes,
    No,
    /// Not an answer: the question is dropped and the message handled as usual.
    Other,
}

/// Read a short spoken reply as yes or no. Deliberately strict: the whole
/// utterance has to be an agreement (at most a few words, starting with one),
/// so "yes but send it to Anna instead" is not taken as consent to the
/// original call.
pub fn spoken_answer(message: &str) -> SpokenAnswer {
    const YES: &[&str] = &[
        "yes", "yeah", "yep", "ok", "okay", "sure", "confirm", "confirmed", "go ahead", "do it",
        "please do", "si", "sì", "certo", "va bene", "procedi", "conferma", "confermo", "fallo",
        "d'accordo", "oui", "d'accord", "vas-y", "ja", "klar", "mach das", "sí", "claro", "vale",
        "adelante", "hazlo", "sim", "pode", "da", "tak", "evet", "да", "давай", "はい", "是", "好",
        "네", "نعم",
    ];
    const NO: &[&str] = &[
        "no", "nope", "don't", "do not", "cancel", "stop", "annulla", "non farlo", "lascia stare",
        "non", "annule", "nein", "abbrechen", "nao", "não", "cancela", "nie", "hayır", "нет",
        "いいえ", "不", "아니", "لا",
    ];
    let cleaned: String = message
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if matches!(c, '.' | ',' | '!' | '?' | ';' | ':' | '。' | '、') { ' ' } else { c })
        .collect();
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    if words.is_empty() || words.len() > 4 {
        return SpokenAnswer::Other;
    }
    let text = words.join(" ");
    let starts = |list: &[&str]| {
        list.iter().any(|word| {
            text == *word || text.strip_prefix(word).is_some_and(|rest| rest.starts_with(' '))
        })
    };
    // "no" first: "no, don't" must never be read through a later "ok".
    if starts(NO) {
        SpokenAnswer::No
    } else if starts(YES) {
        SpokenAnswer::Yes
    } else {
        SpokenAnswer::Other
    }
}

/// Live approval requests + the conversations that no longer ask.
#[derive(Clone, Default)]
pub struct ApprovalRegistry {
    pending: Arc<Mutex<HashMap<String, Pending>>>,
    /// Conversations where the user chose "Allow for this chat". The durable
    /// copy lives in `chat_conversations.terminal_auto_approve`; this set makes
    /// the choice effective for the rest of the current turn immediately.
    auto_chat: Arc<RwLock<HashSet<String>>>,
    /// One parked spoken question per conversation.
    voice_asks: Arc<Mutex<HashMap<String, VoiceAsk>>>,
}

impl ApprovalRegistry {
    pub fn set_auto_chat(&self, conversation_id: &str, on: bool) {
        let mut auto = self.auto_chat.write();
        if on {
            auto.insert(conversation_id.to_string());
        } else {
            auto.remove(conversation_id);
        }
    }

    pub fn is_auto_chat(&self, conversation_id: &str) -> bool {
        self.auto_chat.read().contains(conversation_id)
    }

    /// Park a gated call until the user answers aloud. A newer question in the
    /// same conversation replaces an older one.
    pub fn park_voice_ask(
        &self,
        conversation_id: &str,
        traveler_id: &str,
        action: &str,
        params: serde_json::Value,
    ) {
        let mut asks = self.voice_asks.lock().unwrap_or_else(|e| e.into_inner());
        asks.retain(|_, ask| ask.asked_at.elapsed() < VOICE_ASK_TTL);
        asks.insert(
            conversation_id.to_string(),
            VoiceAsk {
                traveler_id: traveler_id.to_string(),
                action: action.to_string(),
                params,
                asked_at: std::time::Instant::now(),
            },
        );
    }

    /// Take the question the next spoken message answers, if one is parked,
    /// still fresh and asked of this same account. Taking removes it: whatever
    /// the answer turns out to be, the question is answered once.
    pub fn take_voice_ask(&self, conversation_id: &str, traveler_id: &str) -> Option<VoiceAsk> {
        let ask = self
            .voice_asks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(conversation_id)?;
        (ask.traveler_id == traveler_id && ask.asked_at.elapsed() < VOICE_ASK_TTL).then_some(ask)
    }

    fn request(
        &self,
        turn_id: &str,
        traveler_id: &str,
        conversation_id: &str,
    ) -> (String, oneshot::Receiver<Decision>) {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().unwrap().insert(
            request_id.clone(),
            Pending {
                turn_id: turn_id.to_string(),
                traveler_id: traveler_id.to_string(),
                conversation_id: conversation_id.to_string(),
                sender,
            },
        );
        (request_id, receiver)
    }

    /// Resolve the request the UI answered. Returns the conversation it
    /// belonged to when the request was real and owned by this traveler, so
    /// the caller can persist "Allow for this chat".
    pub fn resolve(
        &self,
        turn_id: &str,
        request_id: &str,
        traveler_id: &str,
        allow: bool,
        remember: bool,
    ) -> Option<String> {
        let mut pending = self.pending.lock().unwrap();
        let entry = pending.get(request_id)?;
        if entry.turn_id != turn_id || entry.traveler_id != traveler_id {
            return None;
        }
        let entry = pending.remove(request_id)?;
        let conversation_id = entry.conversation_id;
        if allow && remember {
            self.set_auto_chat(&conversation_id, true);
        }
        let _ = entry.sender.send(if allow { Decision::Allow } else { Decision::Deny });
        Some(conversation_id)
    }

    /// Drop a request the runner no longer waits for (turn stopped, stream
    /// closed). A late UI answer then resolves nothing.
    pub fn discard(&self, request_id: &str) {
        self.pending.lock().unwrap().remove(request_id);
    }
}

/// One turn's handle to the approval round trip.
pub struct ApprovalGate {
    registry: ApprovalRegistry,
    turn_id: String,
    traveler_id: String,
    conversation_id: String,
    /// False on transports that cannot ask (the one-shot JSON endpoint, a voice
    /// turn before the text chat is open): the command is then refused rather
    /// than run unapproved.
    interactive: bool,
    /// A spoken turn: ask aloud instead of refusing when no card can be shown.
    voice: bool,
    emit: Box<dyn Fn(ApprovalPrompt) + Send + Sync>,
}

impl ApprovalGate {
    /// Mark the turn as spoken (speech in, speech out).
    pub fn spoken(mut self, voice: bool) -> Self {
        self.voice = voice;
        self
    }

    /// Park this call for the user's spoken answer (after [`Authorization::AskAloud`]).
    pub fn park_voice_ask(&self, action: &str, params: serde_json::Value) {
        self.registry
            .park_voice_ask(&self.conversation_id, &self.traveler_id, action, params);
    }

    pub fn new(
        registry: ApprovalRegistry,
        turn_id: String,
        traveler_id: String,
        conversation_id: String,
        interactive: bool,
        emit: Box<dyn Fn(ApprovalPrompt) + Send + Sync>,
    ) -> Self {
        Self {
            registry,
            turn_id,
            traveler_id,
            conversation_id,
            interactive,
            voice: false,
            emit,
        }
    }

    /// Ask before running `command`, unless this chat already allows commands.
    pub async fn authorize(
        &self,
        action: &str,
        command: &str,
        cancel: Option<&CancelHandle>,
    ) -> Authorization {
        if self.registry.is_auto_chat(&self.conversation_id) {
            return Authorization::Allow;
        }
        if !self.interactive && self.voice {
            return Authorization::AskAloud;
        }
        if !self.interactive {
            return Authorization::Unavailable(
                "this command needs the user's approval and this turn cannot ask for it. \
                 Nothing was run — ask the user to repeat the request in the text chat.",
            );
        }

        let (request_id, receiver) = self.registry.request(
            &self.turn_id,
            &self.traveler_id,
            &self.conversation_id,
        );
        let _pending = PendingGuard {
            registry: &self.registry,
            request_id: request_id.clone(),
        };
        (self.emit)(ApprovalPrompt {
            request_id,
            action: action.to_string(),
            command: command.to_string(),
        });

        let decision = match cancel {
            Some(handle) => tokio::select! {
                biased;
                _ = handle.cancelled() => Decision::Deny,
                answer = receiver => answer.unwrap_or(Decision::Deny),
            },
            None => receiver.await.unwrap_or(Decision::Deny),
        };

        match decision {
            Decision::Allow => Authorization::Allow,
            Decision::Deny => Authorization::Denied(
                "the user denied this command. Nothing was run — do not retry it; offer a safer \
                 alternative only if it genuinely helps.",
            ),
        }
    }

    /// Whether [`Self::authorize`] would actually pop a prompt for this turn.
    pub fn will_ask(&self) -> bool {
        self.interactive && !self.registry.is_auto_chat(&self.conversation_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Emit handler that answers every prompt the way the chat would.
    fn answer_with(
        registry: &ApprovalRegistry,
        allow: bool,
        remember: bool,
    ) -> Box<dyn Fn(ApprovalPrompt) + Send + Sync> {
        let registry = registry.clone();
        Box::new(move |prompt| {
            let registry = registry.clone();
            tokio::spawn(async move {
                registry.resolve("turn-1", &prompt.request_id, "user-1", allow, remember);
            });
        })
    }

    #[tokio::test]
    async fn allow_resolves_the_waiting_gate() {
        let registry = ApprovalRegistry::default();
        let gate = ApprovalGate::new(
            registry.clone(),
            "turn-1".into(),
            "user-1".into(),
            "chat-1".into(),
            true,
            answer_with(&registry, true, false),
        );
        assert!(matches!(
            gate.authorize("terminal_exec", "ls", None).await,
            Authorization::Allow
        ));
    }

    #[tokio::test]
    async fn deny_blocks_the_command() {
        let registry = ApprovalRegistry::default();
        let gate = ApprovalGate::new(
            registry.clone(),
            "turn-1".into(),
            "user-1".into(),
            "chat-1".into(),
            true,
            answer_with(&registry, false, false),
        );
        assert!(matches!(
            gate.authorize("terminal_exec", "ls", None).await,
            Authorization::Denied(_)
        ));
    }

    #[tokio::test]
    async fn allow_for_this_chat_skips_later_prompts() {
        let registry = ApprovalRegistry::default();
        let gate = ApprovalGate::new(
            registry.clone(),
            "turn-1".into(),
            "user-1".into(),
            "chat-1".into(),
            true,
            answer_with(&registry, true, true),
        );
        assert!(matches!(
            gate.authorize("terminal_exec", "ls", None).await,
            Authorization::Allow
        ));
        assert!(registry.is_auto_chat("chat-1"));
        assert!(matches!(
            gate.authorize("terminal_exec", "pwd", None).await,
            Authorization::Allow
        ));
    }

    #[tokio::test]
    async fn a_non_interactive_turn_refuses_instead_of_running() {
        let registry = ApprovalRegistry::default();
        let gate = ApprovalGate::new(
            registry,
            "turn-1".into(),
            "user-1".into(),
            "chat-1".into(),
            false,
            Box::new(|_| {}),
        );
        assert!(matches!(
            gate.authorize("terminal_exec", "ls", None).await,
            Authorization::Unavailable(_)
        ));
    }

    #[tokio::test]
    async fn another_traveler_cannot_answer_the_prompt() {
        let registry = ApprovalRegistry::default();
        let (request_id, mut receiver) = registry.request("turn-1", "user-1", "chat-1");
        assert!(registry
            .resolve("turn-1", &request_id, "user-2", true, false)
            .is_none());
        assert!(receiver.try_recv().is_err());
        assert!(registry
            .resolve("turn-1", &request_id, "user-1", true, false)
            .is_some());
        assert_eq!(receiver.try_recv(), Ok(Decision::Allow));
    }

    #[test]
    fn spoken_answers_are_read_strictly() {
        for yes in ["Yes.", "sì", "Si, procedi", "ok", "Va bene!", "go ahead", "oui"] {
            assert_eq!(spoken_answer(yes), SpokenAnswer::Yes, "{yes}");
        }
        for no in ["No", "no, don't", "annulla", "No ok", "nein"] {
            assert_eq!(spoken_answer(no), SpokenAnswer::No, "{no}");
        }
        for other in ["", "yes but send it to Anna instead please", "what time is it", "note", "sicuro?"] {
            assert_eq!(spoken_answer(other), SpokenAnswer::Other, "{other}");
        }
    }

    #[tokio::test]
    async fn a_spoken_turn_asks_aloud_and_the_question_is_answered_once() {
        let registry = ApprovalRegistry::default();
        let gate = ApprovalGate::new(
            registry.clone(),
            "turn-1".into(),
            "user-1".into(),
            "chat-1".into(),
            false,
            Box::new(|_| {}),
        )
        .spoken(true);
        assert!(matches!(
            gate.authorize("mail_send", "mail_send: to a@b.c", None).await,
            Authorization::AskAloud
        ));
        gate.park_voice_ask("mail_send", serde_json::json!({ "to": ["a@b.c"] }));
        // Another account cannot answer it, and asking consumed it.
        assert!(registry.take_voice_ask("chat-1", "user-2").is_none());
        assert!(registry.take_voice_ask("chat-1", "user-1").is_none());
        gate.park_voice_ask("mail_send", serde_json::json!({ "to": ["a@b.c"] }));
        let ask = registry.take_voice_ask("chat-1", "user-1").expect("parked");
        assert_eq!(ask.action, "mail_send");
        assert!(registry.take_voice_ask("chat-1", "user-1").is_none());
    }
}
