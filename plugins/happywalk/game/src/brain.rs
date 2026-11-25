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
use crate::hud::LastHeard;
use crate::intent::{NotUnderstood, Order};
use crate::lines::{stand_in, Topic};
use crate::player::{Bumped, Pace, Player, PlayerSet};

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

    fn next_stand_in(&mut self, lang: &str, topic: Topic) -> &'static str {
        let slot = match topic {
            Topic::Bump(SolidKind::Building) => 0,
            Topic::Bump(SolidKind::Lamp) => 1,
            Topic::NotUnderstood => 3,
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

fn thing(into: SolidKind) -> &'static str {
    match into {
        SolidKind::Building => "the wall of a building",
        SolidKind::Lamp => "a lamp post",
        SolidKind::Tree => "a tree",
        SolidKind::Bench => "a bench",
        SolidKind::Planter => "a flower planter",
        SolidKind::Fountain => "a fountain",
        SolidKind::Kiosk => "the newsstand",
    }
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
    let mut out = String::new();
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
    if !last_said.is_empty() {
        out.push_str(&format!("The player's last words: \"{last_said}\".\n"));
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
    players: Query<&Transform, With<Player>>,
    last: Res<LastHeard>,
    mut memory: ResMut<Memory>,
    mut host: ResMut<Host>,
    mut asked: ResMut<Asked>,
) {
    let now = time.elapsed_secs();
    for bump in bumps.read() {
        let at = players
            .single()
            .map(|t| Vec2::new(t.translation.x, t.translation.z))
            .unwrap_or_default();
        let times = memory.bump(at);
        let mut moment = format!(
            "doing as told, you walked {} straight into {}",
            pace(bump.pace),
            thing(bump.into)
        );
        if times > 1 {
            moment.push_str(&format!(
                ", for the {} time at this very spot",
                ordinal(times)
            ));
        }
        let request = BrainRequest {
            situation: situation(
                &memory,
                &last.text,
                &format!("{moment}, and stopped with a jolt."),
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
        assert!(fresh.starts_with("The player's last words: \"avanti\"."));
        assert!(fresh.contains("Now: you walked into a wall."));
        Memory::keep(&mut memory.moments, "you hit a lamp post".into());
        Memory::keep(&mut memory.lines, "Bel palo.".into());
        let later = situation(&memory, "", "you walked into a wall.");
        assert!(later.contains("- you hit a lamp post"));
        assert!(later.contains("- Bel palo."));
        assert!(!later.contains("last words"));
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
