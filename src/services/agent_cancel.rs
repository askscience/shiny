//! Cooperative cancellation for in-flight agent turns.
//!
//! A local model can take a long time to answer, and the user is allowed to
//! change their mind mid-answer — they correct themselves, ask something else,
//! or simply speak over the reply. The browser names each turn and tells the
//! core to drop it; the runner notices between tool steps and aborts an
//! in-flight model call, so the GPU is freed for the next request instead of
//! finishing an answer nobody is waiting for any more.
//!
//! A turn that has just finished is kept for a short while rather than removed,
//! so a stop that arrives *after* the answer was saved can still be recognised
//! as "late" (record the note) instead of "unknown" (ignore). That distinction
//! is what keeps the note from being written twice or never.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

/// How long a finished turn stays recognisable for a late stop.
const FINISHED_TTL: Duration = Duration::from_secs(300);

#[derive(Default)]
struct Flag {
    cancelled: AtomicBool,
    notify: Notify,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Running,
    /// Finished, and no note has been written for the late stop yet.
    Finished,
    /// Finished and already annotated — a repeat stop must do nothing.
    Annotated,
}

struct Entry {
    user_id: String,
    flag: Arc<Flag>,
    state: State,
    finished_at: Option<Instant>,
}

/// What a stop request should do.
#[derive(Debug, PartialEq, Eq)]
pub enum StopOutcome {
    /// The turn is still running and has been asked to stop. The runner will
    /// save the partial answer together with the note.
    Cancelled,
    /// The turn already finished. The caller should append the note exactly
    /// once — this is the "user stopped the reply while reading/hearing it"
    /// case.
    Annotate,
    /// Nothing to do: unknown turn id, or a stop repeated for a turn that has
    /// already been annotated.
    Ignored,
}

/// Handle owned by one agent run. Cheap to clone.
#[derive(Clone)]
pub struct CancelHandle(Arc<Flag>);

impl CancelHandle {
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }

    /// Cancel this turn directly. Used when the SSE consumer disappears: the
    /// request is gone, so the model/tools should stop instead of finishing an
    /// answer nobody can receive.
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
        self.0.notify.notify_waiters();
    }

    /// Resolves as soon as the turn is cancelled (immediately if it already is).
    /// Used as the losing branch of a `select!` around a model call.
    pub async fn cancelled(&self) {
        let notified = self.0.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if !self.is_cancelled() {
            notified.await;
        }
    }
}

/// Registry of running turns. Shared through `AppState`; clones share one map.
#[derive(Clone, Default)]
pub struct TurnRegistry {
    inner: Arc<Mutex<HashMap<String, Entry>>>,
    shutting_down: Arc<AtomicBool>,
}

impl TurnRegistry {
    /// Start tracking a turn for `user_id`. The returned handle belongs to the
    /// runner. The owner is recorded so no other account can cancel it.
    pub fn register(&self, turn_id: &str, user_id: &str) -> CancelHandle {
        let flag = Arc::new(Flag::default());
        let mut map = self.inner.lock().unwrap();
        flag.cancelled.store(self.shutting_down.load(Ordering::SeqCst), Ordering::SeqCst);
        map.retain(|_, e| match e.finished_at {
            Some(at) => at.elapsed() < FINISHED_TTL,
            None => true,
        });
        map.insert(
            turn_id.to_string(),
            Entry {
                user_id: user_id.to_string(),
                flag: flag.clone(),
                state: State::Running,
                finished_at: None,
            },
        );
        CancelHandle(flag)
    }

    /// Ask a running turn to stop (or find out that it already finished).
    /// Turns registered by another account are ignored: the id is
    /// client-supplied, so without this check any authenticated user who
    /// learned another user's turn id could abort their answer.
    pub fn cancel(&self, turn_id: &str, user_id: &str) -> StopOutcome {
        let mut map = self.inner.lock().unwrap();
        let Some(entry) = map.get_mut(turn_id) else {
            return StopOutcome::Ignored;
        };
        if entry.user_id != user_id {
            return StopOutcome::Ignored;
        }
        match entry.state {
            State::Running => {
                entry.flag.cancelled.store(true, Ordering::SeqCst);
                entry.flag.notify.notify_waiters();
                StopOutcome::Cancelled
            }
            State::Finished => {
                entry.state = State::Annotated;
                StopOutcome::Annotate
            }
            State::Annotated => StopOutcome::Ignored,
        }
    }

    /// Cancel every active turn during process shutdown; no user-facing late
    /// stop annotation is generated for a server lifecycle event.
    pub fn cancel_all(&self) {
        let map = self.inner.lock().unwrap();
        self.shutting_down.store(true, Ordering::SeqCst);
        for entry in map.values() {
            if entry.state == State::Running {
                entry.flag.cancelled.store(true, Ordering::SeqCst);
                entry.flag.notify.notify_waiters();
            }
        }
    }

    /// Mark a turn as done (called once its result has been persisted). The
    /// entry is kept briefly so a late stop is still recognised.
    pub fn finish(&self, turn_id: &str) {
        let mut map = self.inner.lock().unwrap();
        if let Some(entry) = map.get_mut(turn_id) {
            entry.state = State::Finished;
            entry.finished_at = Some(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn shutdown_wakes_active_turns_and_prevents_new_work() {
        let registry = TurnRegistry::default();
        let active = registry.register("active", "one");
        let waiter = tokio::spawn({ let active = active.clone(); async move { active.cancelled().await } });
        tokio::task::yield_now().await;
        registry.cancel_all();
        tokio::time::timeout(Duration::from_secs(1), waiter).await.unwrap().unwrap();
        assert!(active.is_cancelled());
        assert!(registry.register("late", "one").is_cancelled());
    }
}
