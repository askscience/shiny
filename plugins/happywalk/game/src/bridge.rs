//! `LiteAuraBridge`: the game's only door to LiteAura.
//!
//! The game never recognises speech, runs a model or synthesises a voice: the
//! host does, with the engines and the settings the user already has. The
//! contract is three flows, all asynchronous:
//!
//! * **in — recognised text**: [`Heard`], partial results included, so a
//!   command can act before the player has finished the sentence;
//! * **out — a request to the language model**, **in — its structured
//!   answer**: [`Host::ask_brain`] → [`BrainAnswered`];
//! * **out — text to speak**: [`Host::say`] → [`Spoke`].
//!
//! On the web the other side is `web/host.js` (`window.happywalkHost`), which
//! runs the recogniser in a worker and talks to the plugin's routes. It
//! answers by calling the exported `hw_*` functions, which only push onto a
//! queue; [`pump`] drains the queue at the start of each frame. No frame ever
//! waits for the host.

use std::collections::VecDeque;
use std::sync::Mutex;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::telemetry::Lap;

/// Text the recogniser understood.
#[derive(Message, Clone, Debug)]
pub struct Heard {
    pub text: String,
    /// `false` for a partial result: the phrase may still grow or change.
    pub is_final: bool,
    /// When the recogniser produced it.
    pub heard_at_ms: f64,
    /// When the player's voice last went quiet before that (0 when the
    /// result arrived while they were still speaking).
    pub speech_end_ms: f64,
}

/// What the game asks the character's brain.
#[derive(Serialize, Clone, Debug, Default)]
pub struct BrainRequest {
    /// What just happened, in plain words ("the player said X, then Y").
    pub situation: String,
    /// The only actions the answer may choose from.
    pub allowed_actions: Vec<String>,
    /// The language the line must be in (the host's voice language).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub lang: String,
}

/// The brain's answer: one allowed action and a line to say. The plugin
/// validates the action against the request before it gets here.
#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
pub struct BrainAnswer {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub say: String,
}

#[derive(Message, Clone, Debug)]
pub struct BrainAnswered {
    pub id: u32,
    /// `None` when the model was off, late or answered nonsense: the caller
    /// falls back to a line of its own.
    pub answer: Option<BrainAnswer>,
}

#[derive(Message, Clone, Debug)]
pub struct Spoke {
    pub id: u32,
    pub ok: bool,
}

/// What the page reports about itself.
#[derive(Resource, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct HostStatus {
    /// `"starting"`, `"loading-model"`, `"listening"`, `"muted"`, `"error"`.
    #[serde(default)]
    pub voice: String,
    #[serde(default)]
    pub lang: String,
    #[serde(default)]
    pub detail: String,
    /// Whether the player is speaking right now.
    #[serde(default)]
    pub speaking: bool,
}

/// Requests to the host.
#[derive(Resource, Default)]
pub struct Host {
    next_id: u32,
}

impl Host {
    fn id(&mut self) -> u32 {
        self.next_id = self.next_id.wrapping_add(1);
        self.next_id
    }

    /// Ask the character's brain. The answer comes as [`BrainAnswered`].
    pub fn ask_brain(&mut self, request: &BrainRequest) -> u32 {
        let id = self.id();
        let json = serde_json::to_string(request).unwrap_or_default();
        js::brain(id, &json);
        id
    }

    /// Speak a line with the host's voice. [`Spoke`] follows when it starts.
    pub fn say(&mut self, text: &str) -> u32 {
        let id = self.id();
        js::say(id, text);
        id
    }

    /// The words the recogniser should expect (a restricted vocabulary is
    /// faster and far more accurate than free dictation). Empty: free speech.
    pub fn set_grammar(&self, words: &[&str]) {
        js::set_grammar(&serde_json::to_string(words).unwrap_or_default());
    }
}

/// A line for the page's log (the same place the page logs its own side).
pub fn host_log(line: &str) {
    js::log(line);
}

/// The page's clock, in milliseconds.
pub fn now_ms() -> f64 {
    js::now()
}

// ── The queue the host fills ─────────────────────────────────────────────

#[derive(Debug)]
enum Inbound {
    Heard(Heard),
    Brain { id: u32, json: String, ms: f64 },
    Spoke { id: u32, ok: bool, ms: f64 },
    Status(String),
}

static INBOX: Mutex<VecDeque<Inbound>> = Mutex::new(VecDeque::new());

fn push(message: Inbound) {
    if let Ok(mut inbox) = INBOX.lock() {
        inbox.push_back(message);
    }
}

/// Recognised text, from the host. Tests and the keyboard use it too.
pub fn deliver_heard(text: &str, is_final: bool, heard_at_ms: f64, speech_end_ms: f64) {
    push(Inbound::Heard(Heard {
        text: text.trim().to_lowercase(),
        is_final,
        heard_at_ms,
        speech_end_ms,
    }));
}

#[cfg(target_arch = "wasm32")]
mod exports {
    use wasm_bindgen::prelude::*;

    use super::{push, Inbound};

    #[wasm_bindgen]
    pub fn hw_heard(text: &str, is_final: bool, heard_at_ms: f64, speech_end_ms: f64) {
        super::deliver_heard(text, is_final, heard_at_ms, speech_end_ms);
    }

    /// `json` is `{"action": …, "say": …}`, or empty when there is no answer.
    #[wasm_bindgen]
    pub fn hw_brain_answered(id: u32, json: &str, ms: f64) {
        push(Inbound::Brain {
            id,
            json: json.to_string(),
            ms,
        });
    }

    #[wasm_bindgen]
    pub fn hw_spoke(id: u32, ok: bool, ms: f64) {
        push(Inbound::Spoke { id, ok, ms });
    }

    #[wasm_bindgen]
    pub fn hw_status(json: &str) {
        push(Inbound::Status(json.to_string()));
    }
}

#[cfg(target_arch = "wasm32")]
mod js {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = ["window", "happywalkHost"])]
        pub fn brain(id: u32, request_json: &str);
        #[wasm_bindgen(js_namespace = ["window", "happywalkHost"])]
        pub fn say(id: u32, text: &str);
        #[wasm_bindgen(js_namespace = ["window", "happywalkHost"], js_name = setGrammar)]
        pub fn set_grammar(words_json: &str);
        #[wasm_bindgen(js_namespace = ["window", "happywalkHost"])]
        pub fn log(line: &str);
        #[wasm_bindgen(js_namespace = ["window", "performance"])]
        pub fn now() -> f64;
    }
}

/// Off the web there is no host: requests go nowhere and the clock is the
/// process's own. Enough for unit tests.
#[cfg(not(target_arch = "wasm32"))]
mod js {
    pub fn brain(_id: u32, _request_json: &str) {}
    pub fn say(_id: u32, _text: &str) {}
    pub fn set_grammar(_words_json: &str) {}
    pub fn log(_line: &str) {}
    pub fn now() -> f64 {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_secs_f64()
            * 1000.0
    }
}

// ── The plugin ───────────────────────────────────────────────────────────

pub struct LiteAuraBridgePlugin;

impl Plugin for LiteAuraBridgePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Host>()
            .init_resource::<HostStatus>()
            .add_message::<Heard>()
            .add_message::<BrainAnswered>()
            .add_message::<Spoke>()
            .add_systems(First, pump);
    }
}

/// Parse the brain's answer; nothing usable is `None`.
pub fn parse_answer(json: &str) -> Option<BrainAnswer> {
    let answer: BrainAnswer = serde_json::from_str(json).ok()?;
    (!answer.action.is_empty() || !answer.say.is_empty()).then_some(answer)
}

fn pump(
    mut heard: MessageWriter<Heard>,
    mut answered: MessageWriter<BrainAnswered>,
    mut spoke: MessageWriter<Spoke>,
    mut laps: MessageWriter<Lap>,
    mut status: ResMut<HostStatus>,
) {
    let pending: Vec<Inbound> = match INBOX.lock() {
        Ok(mut inbox) => inbox.drain(..).collect(),
        Err(_) => return,
    };
    if pending.is_empty() {
        return;
    }
    let now = now_ms();
    for message in pending {
        match message {
            Inbound::Heard(h) => {
                if h.text.is_empty() {
                    continue;
                }
                let kind = if h.is_final { "final" } else { "partial" };
                let about = format!("\"{}\", {kind}", h.text);
                if h.speech_end_ms > 0.0 && h.heard_at_ms >= h.speech_end_ms {
                    laps.write(Lap {
                        link: "speech→text",
                        ms: h.heard_at_ms - h.speech_end_ms,
                        about: about.clone(),
                    });
                }
                laps.write(Lap {
                    link: "text→game",
                    ms: (now - h.heard_at_ms).max(0.0),
                    about,
                });
                heard.write(h);
            }
            Inbound::Brain { id, json, ms } => {
                let answer = parse_answer(&json);
                let about = match &answer {
                    Some(a) => format!("#{id} {} \"{}\"", a.action, a.say),
                    None => format!("#{id} no answer"),
                };
                laps.write(Lap {
                    link: "brain",
                    ms,
                    about,
                });
                answered.write(BrainAnswered { id, answer });
            }
            Inbound::Spoke { id, ok, ms } => {
                laps.write(Lap {
                    link: "voice",
                    ms,
                    about: format!("#{id} {}", if ok { "speaking" } else { "failed" }),
                });
                spoke.write(Spoke { id, ok });
            }
            Inbound::Status(json) => {
                if let Ok(next) = serde_json::from_str::<HostStatus>(&json) {
                    if *status != next {
                        if status.voice != next.voice {
                            info!("voice: {} {} {}", next.voice, next.lang, next.detail);
                        }
                        *status = next;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_needs_an_action_or_a_line() {
        assert_eq!(
            parse_answer(r#"{"action":"stop","say":"Ahi."}"#),
            Some(BrainAnswer {
                action: "stop".into(),
                say: "Ahi.".into()
            })
        );
        assert_eq!(parse_answer(r#"{"say":"Ahi."}"#).unwrap().action, "");
        assert_eq!(parse_answer("{}"), None);
        assert_eq!(parse_answer(""), None);
        assert_eq!(parse_answer("not json"), None);
    }
}
