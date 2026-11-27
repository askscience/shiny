//! `LiteAuraBridge`: the game's only door to LiteAura.
//!
//! The game never runs a model or recognises speech: the host does, with the
//! engines and the settings the user already has. The game asks the
//! language model for the people of a case ([`Host::ask_brain`] →
//! [`BrainAnswered`]), and gets the player's chat (typed, or spoken and
//! written down by the page) and controls as calls.
//!
//! On the web the other side is `web/host.js` (`window.happywalkHost`), which
//! talks to the plugin's routes. It
//! answers by calling the exported `hw_*` functions, which only push onto a
//! queue; [`pump`] drains the queue at the start of each frame. No frame ever
//! waits for the host.

use std::collections::VecDeque;
use std::sync::Mutex;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::telemetry::Lap;

/// What the game asks the character's brain.
#[derive(Serialize, Clone, Debug, Default)]
pub struct BrainRequest {
    /// Who the model plays, when it is not the character (a suspect).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub persona: String,
    /// The conversation so far, oldest first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Turn>,
    /// What the detective says now (or, to open, what is happening).
    pub situation: String,
    /// The only actions the answer may choose from.
    pub allowed_actions: Vec<String>,
    /// The language the line must be in (the host's voice language).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub lang: String,
}

/// One turn of a conversation: who spoke (`"detective"` or `"them"`), and
/// what they said.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Turn {
    pub from: &'static str,
    pub text: String,
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

/// The player chose how the character looks.
#[derive(Message, Clone, Copy, Debug)]
pub struct LookChosen(pub crate::figure::Look);

/// Whether the wardrobe is open.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wardrobe(pub bool);

/// The page kept a clip of the last seconds (or failed to).
#[derive(Message, Clone, Copy, Debug)]
pub struct ClipSaved {
    pub ok: bool,
    pub seconds: f32,
}

/// The character was put somewhere by hand (tests, screenshots).
#[derive(Message, Clone, Copy, Debug)]
pub struct Placed {
    pub at: Vec2,
    pub facing: f32,
}

/// The player is typing in the chat: keys are letters, not controls.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Typing(pub bool);

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

    /// Keep the last seconds of play as a clip. [`ClipSaved`] follows.
    pub fn save_clip(&self) {
        js::save_clip();
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

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Debug)]
enum Inbound {
    Brain { id: u32, json: String, ms: f64 },
    Typing(bool),
    Status(String),
    Place { x: f32, z: f32, facing: f32 },
    Clip { ok: bool, seconds: f32 },
    Look(String),
    City(String),
    Detective(crate::mystery::Detective),
    Wardrobe(bool),
}

static INBOX: Mutex<VecDeque<Inbound>> = Mutex::new(VecDeque::new());

/// The game's state as the page may read it (`hw_state`).
static STATE: Mutex<String> = Mutex::new(String::new());

/// Where the traffic is, as the page may read it (`hw_traffic`).
static TRAFFIC: Mutex<String> = Mutex::new(String::new());

/// How much is being drawn, as the page may read it (`hw_render`).
static RENDER: Mutex<String> = Mutex::new(String::new());

/// Publish how much is being drawn: for measuring.
pub fn publish_render(json: String) {
    if let Ok(mut render) = RENDER.lock() {
        *render = json;
    }
}

/// The notebook of the case in play, as the page may read it (`hw_case`).
static CASE: Mutex<String> = Mutex::new(String::new());

/// Publish the notebook.
pub fn publish_case(json: String) {
    if let Ok(mut case) = CASE.lock() {
        *case = json;
    }
}

/// Publish where the traffic is: for tests and screenshots.
pub fn publish_traffic(json: String) {
    if let Ok(mut traffic) = TRAFFIC.lock() {
        *traffic = json;
    }
}

/// Publish the state the page may read.
pub fn publish_state(json: String) {
    if let Ok(mut state) = STATE.lock() {
        *state = json;
    }
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn push(message: Inbound) {
    if let Ok(mut inbox) = INBOX.lock() {
        inbox.push_back(message);
    }
}

#[cfg(target_arch = "wasm32")]
mod exports {
    use wasm_bindgen::prelude::*;

    use super::{push, Inbound};

    /// `json` is `{"action": …, "say": …}`, or empty when there is no answer.
    #[wasm_bindgen]
    pub fn hw_brain_answered(id: u32, json: &str, ms: f64) {
        push(Inbound::Brain {
            id,
            json: json.to_string(),
            ms,
        });
    }

    /// The page's joystick: forward and turn, each -1..1 (0, 0 at rest).
    #[wasm_bindgen]
    pub fn hw_steer(forward: f32, turn: f32) {
        crate::control::steer(forward, turn);
    }

    /// A tap or a click on the canvas, in CSS pixels from its top left: walk
    /// there.
    #[wasm_bindgen]
    pub fn hw_tap(x: f32, y: f32) {
        crate::control::tap(x, y);
    }

    /// Whether the player is typing in the chat.
    #[wasm_bindgen]
    pub fn hw_typing(typing: bool) {
        push(Inbound::Typing(typing));
    }

    /// A message for whoever the character is talking to (typed, or spoken
    /// and written down by the page).
    #[wasm_bindgen]
    pub fn hw_chat(text: &str) {
        push(Inbound::Detective(crate::mystery::Detective::Chat(
            text.trim().to_string(),
        )));
    }

    /// Where the character is and what it is doing, as JSON: for tests and
    /// for the page's log, never for the game itself.
    /// How much is being drawn (meshes, how many are on screen, their
    /// triangles): for measuring.
    #[wasm_bindgen]
    pub fn hw_render() -> String {
        super::RENDER
            .lock()
            .map(|render| render.clone())
            .unwrap_or_default()
    }

    #[wasm_bindgen]
    pub fn hw_traffic() -> String {
        super::TRAFFIC
            .lock()
            .map(|traffic| traffic.clone())
            .unwrap_or_default()
    }

    #[wasm_bindgen]
    pub fn hw_state() -> String {
        super::STATE
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default()
    }

    /// Put the character somewhere, facing `facing` radians: for tests and
    /// screenshots.
    #[wasm_bindgen]
    pub fn hw_place(x: f32, z: f32, facing: f32) {
        push(Inbound::Place { x, z, facing });
    }

    /// A clip was kept (or could not be): how many seconds it holds.
    #[wasm_bindgen]
    pub fn hw_clip_saved(ok: bool, seconds: f32) {
        push(Inbound::Clip { ok, seconds });
    }

    /// Dress the character: a look as JSON (missing fields keep their
    /// default, values out of range are brought in).
    #[wasm_bindgen]
    pub fn hw_look(json: &str) {
        push(Inbound::Look(json.to_string()));
    }

    /// The wardrobe opens or closes: while it is open the character stands
    /// still facing the camera and takes no orders.
    #[wasm_bindgen]
    pub fn hw_wardrobe(open: bool) {
        push(Inbound::Wardrobe(open));
    }

    /// What can be chosen, for the wardrobe to show (see `figure.rs`).
    #[wasm_bindgen]
    pub fn hw_look_options() -> String {
        crate::figure::options_json()
    }

    /// A look as JSON: the default one, or a random one from a number.
    #[wasm_bindgen]
    pub fn hw_look_sample(random: f64) -> String {
        let look = if random > 0.0 {
            crate::figure::Look::random(random as u64)
        } else {
            crate::figure::Look::default()
        };
        serde_json::to_string(&look).unwrap_or_default()
    }

    /// The player named their city: the game moves to the city that name
    /// stands for.
    #[wasm_bindgen]
    pub fn hw_city(name: &str) {
        push(Inbound::City(name.to_string()));
    }

    /// The seed a city name stands for, to start the game in it.
    #[wasm_bindgen]
    pub fn hw_city_seed(name: &str) -> u32 {
        let seed = crate::city::seed_for(name);
        (seed ^ (seed >> 32)) as u32 | 1
    }

    /// The notebook of the case in play, as JSON (see `mystery.rs`).
    #[wasm_bindgen]
    pub fn hw_case() -> String {
        super::CASE
            .lock()
            .map(|case| case.clone())
            .unwrap_or_default()
    }

    /// Walk to a suspect's door, or to the office with a negative number.
    #[wasm_bindgen]
    pub fn hw_go(target: i32) {
        push(Inbound::Detective(crate::mystery::Detective::GoTo(
            (target >= 0).then_some(target as usize),
        )));
    }

    #[wasm_bindgen]
    pub fn hw_accuse(suspect: u32) {
        push(Inbound::Detective(crate::mystery::Detective::Accuse(
            suspect as usize,
        )));
    }

    /// On to the next case, once this one is closed.
    #[wasm_bindgen]
    pub fn hw_next_case() {
        push(Inbound::Detective(crate::mystery::Detective::NextCase));
    }

    /// How far the player had come, as kept by the page.
    #[wasm_bindgen]
    pub fn hw_progress(json: &str) {
        if let Ok(progress) = serde_json::from_str(json) {
            push(Inbound::Detective(crate::mystery::Detective::Progress(
                progress,
            )));
        }
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
        pub fn log(line: &str);
        #[wasm_bindgen(js_namespace = ["window", "happywalkHost"], js_name = saveClip)]
        pub fn save_clip();
        #[wasm_bindgen(js_namespace = ["window", "performance"])]
        pub fn now() -> f64;
    }
}

/// Off the web there is no host: requests go nowhere and the clock is the
/// process's own. Enough for unit tests.
#[cfg(not(target_arch = "wasm32"))]
mod js {
    pub fn brain(_id: u32, _request_json: &str) {}
    pub fn log(_line: &str) {}
    pub fn save_clip() {}
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
            .add_message::<BrainAnswered>()
            .init_resource::<Typing>()
            .add_message::<Placed>()
            .add_message::<ClipSaved>()
            .add_message::<LookChosen>()
            .init_resource::<Wardrobe>()
            .add_systems(First, pump);
    }
}

/// Parse the brain's answer; nothing usable is `None`.
pub fn parse_answer(json: &str) -> Option<BrainAnswer> {
    let answer: BrainAnswer = serde_json::from_str(json).ok()?;
    (!answer.action.is_empty() || !answer.say.is_empty()).then_some(answer)
}

fn pump(
    mut answered: MessageWriter<BrainAnswered>,
    mut typing: ResMut<Typing>,
    mut placed: MessageWriter<Placed>,
    mut clips: MessageWriter<ClipSaved>,
    mut looks: MessageWriter<LookChosen>,
    mut cities: MessageWriter<crate::city::CityNamed>,
    mut detective: MessageWriter<crate::mystery::Detective>,
    mut wardrobe: ResMut<Wardrobe>,
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
    for message in pending {
        match message {
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
            Inbound::Typing(now) => {
                if typing.0 != now {
                    typing.0 = now;
                }
            }
            Inbound::Place { x, z, facing } => {
                placed.write(Placed {
                    at: Vec2::new(x, z),
                    facing,
                });
            }
            Inbound::Clip { ok, seconds } => {
                clips.write(ClipSaved { ok, seconds });
            }
            Inbound::Look(json) => {
                if let Ok(look) = serde_json::from_str::<crate::figure::Look>(&json) {
                    looks.write(LookChosen(look.checked()));
                }
            }
            Inbound::Detective(move_) => {
                detective.write(move_);
            }
            Inbound::City(name) => {
                cities.write(crate::city::CityNamed(name));
            }
            Inbound::Wardrobe(open) => {
                if wardrobe.0 != open {
                    wardrobe.0 = open;
                }
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
