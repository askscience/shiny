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

/// Live approval requests + the conversations that no longer ask.
#[derive(Clone, Default)]
pub struct ApprovalRegistry {
    pending: Arc<Mutex<HashMap<String, Pending>>>,
    /// Conversations where the user chose "Allow for this chat". The durable
    /// copy lives in `chat_conversations.terminal_auto_approve`; this set makes
    /// the choice effective for the rest of the current turn immediately.
    auto_chat: Arc<RwLock<HashSet<String>>>,
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
    emit: Box<dyn Fn(ApprovalPrompt) + Send + Sync>,
}

impl ApprovalGate {
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
}
