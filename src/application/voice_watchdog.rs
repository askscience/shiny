//! Voice sidecar supervision.
//!
//! The speech daemons deliberately outlive the server, but nothing owned them:
//! they were spawned once at core startup (and once per login by
//! `scripts/shiny-session`) and a killed one — session teardown, a crash, the
//! OOM killer — stayed dead until the next core restart. That is exactly how
//! TTS and STT go silent for days while the server keeps running: nothing
//! fails loudly, `/api/voice/status` just reports `unavailable` and every reply
//! toasts "Voice playback unavailable".
//!
//! This task is the missing owner. It probes each enabled engine through its
//! own client — the same probe the UI uses — and re-runs the launcher when one
//! has been silent for [`MISSES_BEFORE_RESTART`] consecutive checks. The
//! debounce matters: a sidecar binds its port long before it answers (Whisper
//! loads CTranslate2, Supertonic loads ONNX weights), and restarting that
//! window would race the launcher's own port probe.
//!
//! Attempts back off from [`FIRST_RETRY`] to [`MAX_RETRY`], so a genuinely
//! broken install (no interpreter, no model, no network) does not fork a
//! launcher every probe — and every attempt is logged, because a silent sidecar
//! used to be invisible in `data/shiny.log`.

use std::time::{Duration, Instant};

use crate::api::AppState;
use crate::application::sidecars::{
    spawn_qwen_tts_sidecar, spawn_supertonic_sidecar, spawn_whisper_sidecar,
};

/// How often every enabled engine is probed.
const POLL_INTERVAL: Duration = Duration::from_secs(15);
/// Consecutive failed probes before an engine counts as down.
const MISSES_BEFORE_RESTART: u32 = 3;
/// Delay before the second restart attempt; doubles per attempt up to
/// [`MAX_RETRY`].
const FIRST_RETRY: Duration = Duration::from_secs(30);
const MAX_RETRY: Duration = Duration::from_secs(300);

/// Liveness bookkeeping for one engine.
struct Engine {
    /// Name as it should read in a log line.
    name: &'static str,
    misses: u32,
    restarts: u32,
    next_attempt: Option<Instant>,
}

impl Engine {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            misses: 0,
            restarts: 0,
            next_attempt: None,
        }
    }

    /// Feed one probe result. Returns true when the launcher should run now.
    fn observe(&mut self, up: bool, now: Instant) -> bool {
        if up {
            if self.restarts > 0 {
                tracing::info!(
                    "{} sidecar is answering again after {} restart attempt(s)",
                    self.name,
                    self.restarts
                );
            }
            self.misses = 0;
            self.restarts = 0;
            self.next_attempt = None;
            return false;
        }

        self.misses += 1;
        if self.misses < MISSES_BEFORE_RESTART {
            return false;
        }
        if self.next_attempt.is_some_and(|at| now < at) {
            return false;
        }
        self.next_attempt = Some(now + retry_delay(self.restarts));
        self.restarts += 1;
        true
    }
}

/// 30 s, 60 s, 120 s, 240 s, then 300 s for every further attempt.
fn retry_delay(attempts_so_far: u32) -> Duration {
    FIRST_RETRY
        .saturating_mul(1 << attempts_so_far.min(4))
        .min(MAX_RETRY)
}

/// Supervise the voice sidecars for as long as the server lives.
pub fn start(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run(state))
}

async fn run(state: AppState) {
    let mut supertonic = Engine::new("Supertonic");
    let mut whisper = Engine::new("faster-whisper");
    let mut qwen = Engine::new("Qwen3-TTS");

    let mut tick = tokio::time::interval(POLL_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tick.tick().await;
        let now = Instant::now();

        // A disabled engine stays disabled: `AUTO_START_*` is the user's own
        // statement about whether this machine runs it at all.
        if state.config.auto_start_supertonic
            && supertonic.observe(state.supertonic.is_available().await, now)
        {
            tracing::warn!(
                "Supertonic TTS is not answering at {}; starting it (attempt {})",
                state.config.supertonic_url,
                supertonic.restarts
            );
            spawn_supertonic_sidecar(&state.config);
        }

        if state.config.auto_start_whisper && whisper.observe(state.whisper.health().await.is_some(), now)
        {
            tracing::warn!(
                "faster-whisper STT is not answering at {}; starting it (attempt {})",
                state.config.whisper_url,
                whisper.restarts
            );
            spawn_whisper_sidecar(&state.config);
        }

        if state.config.auto_start_qwen_tts && qwen.observe(state.qwen_tts.health().await.is_some(), now) {
            tracing::warn!(
                "Qwen3-TTS is not answering at {}; starting it (attempt {})",
                state.config.qwen_tts_url,
                qwen.restarts
            );
            spawn_qwen_tts_sidecar(&state.config);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A single failed probe is a loading sidecar, not a dead one.
    #[test]
    fn one_miss_is_not_a_restart() {
        let mut engine = Engine::new("test");
        let now = Instant::now();
        assert!(!engine.observe(false, now));
        assert!(!engine.observe(false, now));
        assert!(engine.observe(false, now), "the third miss restarts");
    }

    /// A probe that recovers before the debounce expires must not restart.
    #[test]
    fn a_recovery_resets_the_counter() {
        let mut engine = Engine::new("test");
        let now = Instant::now();
        engine.observe(false, now);
        engine.observe(false, now);
        engine.observe(true, now);
        assert!(!engine.observe(false, now));
        assert!(!engine.observe(false, now));
    }

    /// Attempts are spaced out, so a broken install cannot fork a launcher on
    /// every probe.
    #[test]
    fn restart_attempts_back_off() {
        let mut engine = Engine::new("test");
        let start = Instant::now();
        let mut now = start;
        let mut attempts = Vec::new();
        for _ in 0..30 {
            if engine.observe(false, now) {
                attempts.push(now.duration_since(start));
            }
            now += POLL_INTERVAL;
        }
        // Ten minutes of a sidecar that never comes back: four launcher runs,
        // not one every probe.
        assert_eq!(attempts.len(), 4, "backoff must not allow an attempt per probe");
        assert_eq!(
            attempts[0],
            Duration::from_secs(30),
            "the first restart waits out the debounce"
        );
        assert_eq!(attempts[1], Duration::from_secs(60), "then one backoff later");
        assert_eq!(attempts[2], Duration::from_secs(120));
        assert_eq!(attempts[3], Duration::from_secs(240));
    }

    /// The cap holds however long an engine stays down.
    #[test]
    fn retry_delay_is_capped() {
        assert_eq!(retry_delay(0), Duration::from_secs(30));
        assert_eq!(retry_delay(1), Duration::from_secs(60));
        assert_eq!(retry_delay(4), Duration::from_secs(300));
        assert_eq!(retry_delay(40), Duration::from_secs(300));
    }
}
