//! `BrainPlugin`: what the character says, and what it makes of a sentence
//! that was no command. Both come from the language model the user chose in
//! LiteAura's settings, through the bridge.
//!
//! When something happens (a bump, a sentence it did not understand) the
//! brain describes the moment (what the player said, what happened, what
//! happened before, the lines already used) and asks for one line, and for
//! one of the actions it offers when there is something to do. The answer
//! comes back checked: an action the game offered, a short line.
//!
//! The moment does not wait. If no answer is here within [`PATIENCE`]
//! seconds, or the model is off, the character says a stand-in line
//! (`lines.rs`) and a late answer is dropped.

use std::collections::{HashMap, VecDeque};

use bevy::prelude::*;

use crate::bridge::{BrainAnswered, BrainRequest, Host, HostStatus};
use crate::city::SolidKind;
use crate::gen::PlaceKind;
use crate::hud::LastHeard;
use crate::intent::{NotUnderstood, Order};
use crate::lines::{stand_in, Topic};
use crate::nav::NoSuchPlace;
use crate::player::{Bumped, Pace, PlayerSet};
use crate::quest::{ErrandDone, Quest, ERRANDS, ROUND};
use crate::traffic::Hooted;

/// A line the character says: spoken by the host, shown in a bubble.
#[derive(Message, Clone, Debug)]
pub struct Said {
    pub text: String,
}

/// Seconds the character waits for its brain before it says a stand-in line.
pub const PATIENCE: f32 = 2.5;
/// How many past moments and lines the brain is reminded of.
const REMEMBERED: usize = 6;
/// Bumps within this many metres count as "the same spot".
const SPOT: f32 = 4.0;

/// The short memory: enough for "that is the third time you walk me into
/// this wall".
#[derive(Resource, Default)]
pub struct Memory {
    moments: VecDeque<String>,
    lines: VecDeque<String>,
    bumps_at: HashMap<(i32, i32), u32>,
    /// When the recent bumps and errands were (game seconds).
    bumped_at: VecDeque<f32>,
    errand_at: Option<f32>,
    /// What the character is out to do, in plain words.
    pub errand: String,
    /// What the player called the town.
    pub city: String,
    turns: HashMap<u8, usize>,
}

impl Memory {
    fn keep(queue: &mut VecDeque<String>, item: String) {
        queue.push_back(item);
        while queue.len() > REMEMBERED {
            queue.pop_front();
        }
    }

    /// Note a bump at `at`; returns how many times this spot was hit.
    pub fn bump(&mut self, at: Vec2) -> u32 {
        let spot = ((at.x / SPOT).round() as i32, (at.y / SPOT).round() as i32);
        let count = self.bumps_at.entry(spot).or_insert(0);
        *count += 1;
        *count
    }

    /// How the character feels, from how the last minutes went.
    pub fn mood(&self, now: f32) -> &'static str {
        let recent = self.bumped_at.iter().filter(|t| now - **t < 90.0).count();
        if self.errand_at.is_some_and(|t| now - t < 25.0) && recent < 2 {
            return "You are still glowing with pride from the errand you just did.";
        }
        match recent {
            0 | 1 => "",
            2 => "You are getting a little ruffled.",
            3 | 4 => "You are fed up, theatrically so.",
            _ => "You are at the end of your tether and say so like a soap opera.",
        }
    }

    fn next_stand_in(&mut self, lang: &str, topic: Topic) -> &'static str {
        let slot = match topic {
            Topic::Bump(SolidKind::Building) => 0,
            Topic::Bump(SolidKind::Lamp) => 1,
            Topic::NotUnderstood => 3,
            Topic::Hooted => 5,
            Topic::Errand => 6,
            Topic::NoSuchPlace => 7,
            Topic::Bump(_) => 4,
        };
        let turn = self.turns.entry(slot).or_insert(0);
        let line = stand_in(lang, topic, *turn);
        *turn += 1;
        line
    }
}

fn ordinal(n: u32) -> String {
    match n {
        2 => "second".into(),
        3 => "third".into(),
        4 => "fourth".into(),
        5 => "fifth".into(),
        n => format!("{n}th"),
    }
}

/// What was walked into, in a few concrete words: the joke is in the
/// detail.
pub fn describe(into: SolidKind, model: &str) -> String {
    let named = match model {
        m if m.starts_with("home_old") => {
            "the front door of an old townhouse with flowerpots on every balcony"
        }
        m if m.starts_with("home_town") => "the wall of a tall pastel block of flats",
        m if m.starts_with("home_garden") => "a little house smothered in climbing plants",
        "shop_bakery" => "the bakery's shop window, full of bread",
        "shop_florist" => "the florist's window",
        "shop_cafe" => "the café's window, under its striped awning",
        "shop_grocer" => "the greengrocer's window",
        "sign_board" => "the bakery's chalkboard sign on the pavement",
        "flower_buckets" => "the florist's buckets of flowers on the pavement",
        "cafe_table" => "a café table outside, with someone's coffee on it",
        "crates" => "the crates of fruit outside the greengrocer's",
        "car_small_red" => "the side of a small red car",
        "car_small_blue" => "the side of a small blue car",
        "car_small_cream" => "the side of a small cream-coloured car",
        "car_small_mint" => "the side of a small mint-green car",
        "car_taxi" => "the side of a yellow taxi",
        "car_van_cream" | "car_van_sky" => {
            "the side of a delivery van with a loaf of bread painted on it"
        }
        "car_icecream" => "the ice-cream van (closed, sadly)",
        m if m.starts_with("bike") => "a cyclist",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_string();
    }
    match into {
        SolidKind::Building => "the wall of a building",
        SolidKind::Lamp => "a lamp post",
        SolidKind::Tree => "a tree",
        SolidKind::Bench => "a bench",
        SolidKind::Planter => "a flower planter",
        SolidKind::Fountain => "the edge of the fountain",
        SolidKind::Kiosk => "the newsstand",
        SolidKind::Person => "a passer-by, who was not amused",
        SolidKind::Car => "the side of a car",
        SolidKind::Bike => "a cyclist",
    }
    .to_string()
}

fn pace(pace: Pace) -> &'static str {
    match pace {
        Pace::Slow => "slowly",
        Pace::Walk => "at a walk",
        Pace::Run => "at a run",
    }
}

/// The moment in plain words, with what came before it.
pub fn situation(memory: &Memory, last_said: &str, now: &str) -> String {
    situation_with(memory, last_said, now, "")
}

/// [`situation`], with how the character feels and what the hour is.
pub fn situation_with(memory: &Memory, last_said: &str, now: &str, feeling: &str) -> String {
    let mut out = String::new();
    if !memory.city.is_empty() {
        out.push_str(&format!("Your town is called {}.\n", memory.city));
    }
    if !memory.moments.is_empty() {
        out.push_str("Earlier, oldest first:\n");
        for moment in &memory.moments {
            out.push_str(&format!("- {moment}\n"));
        }
    }
    if !memory.lines.is_empty() {
        out.push_str("Lines you already said (say something different):\n");
        for line in &memory.lines {
            out.push_str(&format!("- {line}\n"));
        }
    }
    if !memory.errand.is_empty() {
        out.push_str(&format!("Your errand right now: {}.\n", memory.errand));
    }
    if !last_said.is_empty() {
        out.push_str(&format!("The player's last order: \"{last_said}\".\n"));
    }
    if !feeling.is_empty() {
        out.push_str(feeling);
        out.push('\n');
    }
    out.push_str(&format!("Now: {now}\nSay your one line about it."));
    out
}

/// The actions the brain may choose for a sentence that was no command.
const ACTIONS: &[(&str, Order)] = &[
    ("forward", Order::Forward),
    ("back", Order::Back),
    ("left", Order::Left),
    ("right", Order::Right),
    ("stop", Order::Stop),
    ("faster", Order::Faster),
    ("slower", Order::Slower),
    ("wave", Order::Wave),
    ("go_bakery", Order::GoTo(PlaceKind::Bakery)),
    ("go_florist", Order::GoTo(PlaceKind::Florist)),
    ("go_cafe", Order::GoTo(PlaceKind::Cafe)),
    ("go_greengrocer", Order::GoTo(PlaceKind::Grocer)),
    ("go_newsstand", Order::GoTo(PlaceKind::Newsstand)),
    ("go_park", Order::GoTo(PlaceKind::Park)),
    ("go_square", Order::GoTo(PlaceKind::Square)),
];

struct Waiting {
    id: u32,
    asked_at: f32,
    topic: Topic,
    /// What the moment was, to remember once it is answered.
    moment: String,
}

#[derive(Resource, Default)]
struct Asked(Vec<Waiting>);

pub struct BrainPlugin;

impl Plugin for BrainPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Memory>()
            .init_resource::<Asked>()
            .add_message::<Said>()
            .add_systems(Update, (notice, answer).chain().after(PlayerSet));
    }
}

/// Something happened: describe it and ask.
#[allow(clippy::too_many_arguments)]
fn notice(
    time: Res<Time>,
    mut bumps: MessageReader<Bumped>,
    mut lost: MessageReader<NotUnderstood>,
    mut hoots: MessageReader<Hooted>,
    mut errands: MessageReader<ErrandDone>,
    mut nowhere: MessageReader<NoSuchPlace>,
    quest: Res<Quest>,
    city: Res<crate::city::CityName>,
    daylight: Res<crate::sky::Daylight>,
    mut last_hoot: Local<f32>,
    last: Res<LastHeard>,
    mut memory: ResMut<Memory>,
    mut host: ResMut<Host>,
    mut asked: ResMut<Asked>,
) {
    let now = time.elapsed_secs();
    if memory.city != city.0 {
        memory.city = city.0.clone();
    }
    let errand = quest.errand().map(|e| e.task).unwrap_or_default();
    if memory.errand != errand {
        memory.errand = errand.to_string();
    }
    for done in errands.read() {
        memory.errand_at = Some(now);
        let mut moment = format!(
            "you got there and did your errand: {} ({} of {ROUND} this round)",
            ERRANDS[done.errand].task, done.done
        );
        if done.round_over {
            moment.push_str(", which finishes the round: all the errands are done");
        }
        let request = BrainRequest {
            situation: situation(&memory, &last.text, &format!("{moment}.")),
            allowed_actions: Vec::new(),
            lang: String::new(),
        };
        let id = host.ask_brain(&request);
        asked.0.push(Waiting {
            id,
            asked_at: now,
            topic: Topic::Errand,
            moment,
        });
    }
    for missing in nowhere.read() {
        let moment = format!(
            "the player sent you to {}, but there is none anywhere near",
            missing.kind.name()
        );
        let request = BrainRequest {
            situation: situation(&memory, &last.text, &format!("{moment}.")),
            allowed_actions: Vec::new(),
            lang: String::new(),
        };
        let id = host.ask_brain(&request);
        asked.0.push(Waiting {
            id,
            asked_at: now,
            topic: Topic::NoSuchPlace,
            moment,
        });
    }
    let hour = if daylight.evening > 0.5 {
        "It is evening; the street lamps are on."
    } else {
        ""
    };
    for bump in bumps.read() {
        let times = memory.bump(bump.at);
        memory.bumped_at.push_back(now);
        while memory.bumped_at.len() > 12 {
            memory.bumped_at.pop_front();
        }
        let mut moment = format!(
            "you walked {} into {}",
            pace(bump.pace),
            describe(bump.into, bump.what)
        );
        if times > 1 {
            moment.push_str(&format!(
                ", for the {} time at this very spot",
                ordinal(times)
            ));
        }
        let feeling = [memory.mood(now), hour]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        let request = BrainRequest {
            situation: situation_with(
                &memory,
                &last.text,
                &format!("{moment}, and bounced off."),
                &feeling,
            ),
            allowed_actions: Vec::new(),
            lang: String::new(),
        };
        let id = host.ask_brain(&request);
        asked.0.push(Waiting {
            id,
            asked_at: now,
            topic: Topic::Bump(bump.into),
            moment,
        });
    }
    for hoot in hoots.read() {
        // Being hooted at is funny once in a while, not every time.
        if now - *last_hoot < 25.0 && *last_hoot > 0.0 {
            continue;
        }
        *last_hoot = now.max(0.001);
        let who = match hoot.what {
            m if m.starts_with("bike") => "a cyclist had to brake and rang the bell".to_string(),
            "car_icecream" => "the ice-cream van had to brake and played its jingle".to_string(),
            "car_taxi" => "a taxi had to brake hard and hooted".to_string(),
            m if m.starts_with("car_van") => {
                "a delivery van had to brake hard and honked".to_string()
            }
            m => format!(
                "{} had to brake hard and hooted",
                describe(hoot.by, m).trim_start_matches("the side of ")
            ),
        };
        let moment = format!("standing in the road where you were told to, {who} at you");
        let request = BrainRequest {
            situation: situation(&memory, &last.text, &format!("{moment}. Nobody was hurt.")),
            allowed_actions: Vec::new(),
            lang: String::new(),
        };
        let id = host.ask_brain(&request);
        asked.0.push(Waiting {
            id,
            asked_at: now,
            topic: Topic::Hooted,
            moment,
        });
    }
    for sentence in lost.read() {
        let moment = format!(
            "the player said \"{}\", which is none of your commands",
            sentence.text
        );
        let request = BrainRequest {
            situation: situation(
                &memory,
                &sentence.text,
                &format!("{moment}. If it clearly means one of the actions, choose it; otherwise choose none and tease them."),
            ),
            allowed_actions: ACTIONS.iter().map(|(name, _)| name.to_string()).collect(),
            lang: String::new(),
        };
        let id = host.ask_brain(&request);
        asked.0.push(Waiting {
            id,
            asked_at: now,
            topic: Topic::NotUnderstood,
            moment,
        });
    }
}

/// The answers, and the stand-ins for those that did not come in time.
fn answer(
    time: Res<Time>,
    status: Res<HostStatus>,
    mut answers: MessageReader<BrainAnswered>,
    mut asked: ResMut<Asked>,
    mut memory: ResMut<Memory>,
    mut host: ResMut<Host>,
    mut said: MessageWriter<Said>,
    mut orders: MessageWriter<Order>,
) {
    let now = time.elapsed_secs();
    let mut speak = |memory: &mut Memory, host: &mut Host, moment: String, line: String| {
        Memory::keep(&mut memory.moments, moment);
        Memory::keep(&mut memory.lines, line.clone());
        host.say(&line);
        said.write(Said { text: line });
    };
    for reply in answers.read() {
        // Not waited for any more: the stand-in was already said.
        let Some(index) = asked.0.iter().position(|w| w.id == reply.id) else {
            continue;
        };
        let waiting = asked.0.remove(index);
        let line = match &reply.answer {
            Some(answer) => {
                if waiting.topic == Topic::NotUnderstood {
                    if let Some((_, order)) =
                        ACTIONS.iter().find(|(name, _)| *name == answer.action)
                    {
                        orders.write(*order);
                    }
                }
                answer.say.trim().to_string()
            }
            None => String::new(),
        };
        let line = if line.is_empty() {
            memory
                .next_stand_in(&status.lang, waiting.topic)
                .to_string()
        } else {
            line
        };
        speak(&mut memory, &mut host, waiting.moment, line);
    }
    let mut index = 0;
    while index < asked.0.len() {
        if now - asked.0[index].asked_at < PATIENCE {
            index += 1;
            continue;
        }
        let waiting = asked.0.remove(index);
        let line = memory
            .next_stand_in(&status.lang, waiting.topic)
            .to_string();
        speak(&mut memory, &mut host, waiting.moment, line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_spot_is_counted() {
        let mut memory = Memory::default();
        assert_eq!(memory.bump(Vec2::new(6.7, 4.0)), 1);
        assert_eq!(memory.bump(Vec2::new(6.6, 4.9)), 2);
        assert_eq!(memory.bump(Vec2::new(30.0, 4.0)), 1);
    }

    #[test]
    fn the_situation_tells_the_moment_and_what_came_before() {
        let mut memory = Memory::default();
        let fresh = situation(&memory, "avanti", "you walked into a wall.");
        assert!(fresh.starts_with("The player's last order: \"avanti\"."));
        assert!(fresh.contains("Now: you walked into a wall."));
        Memory::keep(&mut memory.moments, "you hit a lamp post".into());
        Memory::keep(&mut memory.lines, "Bel palo.".into());
        let later = situation(&memory, "", "you walked into a wall.");
        assert!(later.contains("- you hit a lamp post"));
        assert!(later.contains("- Bel palo."));
        assert!(!later.contains("last order"));
        memory.errand = "buy bread at the bakery".into();
        assert!(
            situation(&memory, "", "x").contains("Your errand right now: buy bread at the bakery.")
        );
    }

    #[test]
    fn a_bump_is_described_by_what_was_hit() {
        assert!(describe(SolidKind::Building, "shop_bakery").contains("bakery"));
        assert!(describe(SolidKind::Bench, "cafe_table").contains("café table"));
        assert!(describe(SolidKind::Building, "home_garden_b").contains("climbing plants"));
        assert_eq!(describe(SolidKind::Lamp, "lamp"), "a lamp post");
        assert!(describe(SolidKind::Car, "car_small_blue").contains("blue car"));
        assert!(describe(SolidKind::Car, "car_icecream").contains("ice-cream"));
    }

    #[test]
    fn the_mood_follows_the_bumps() {
        let mut memory = Memory::default();
        assert_eq!(memory.mood(100.0), "");
        memory.bumped_at.extend([60.0, 80.0, 95.0]);
        assert!(memory.mood(100.0).contains("fed up"));
        // A while later it has calmed down.
        assert_eq!(memory.mood(400.0), "");
        memory.errand_at = Some(395.0);
        assert!(memory.mood(400.0).contains("pride"));
    }

    #[test]
    fn memory_is_short() {
        let mut memory = Memory::default();
        for i in 0..20 {
            Memory::keep(&mut memory.lines, format!("line {i}"));
        }
        assert_eq!(memory.lines.len(), REMEMBERED);
        assert_eq!(memory.lines.front().unwrap(), "line 14");
    }

    #[test]
    fn stand_ins_take_turns() {
        let mut memory = Memory::default();
        let topic = Topic::Bump(SolidKind::Lamp);
        let first = memory.next_stand_in("it", topic);
        assert_ne!(first, memory.next_stand_in("it", topic));
    }
}
