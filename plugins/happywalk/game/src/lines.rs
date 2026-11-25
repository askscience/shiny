//! What the character says when it walks into something: fixed lines for
//! now (its brain writes them later). It knows it is steered by voice, it is
//! ironic about it, and it is never mean.

use bevy::prelude::*;

use crate::bridge::{Host, HostStatus};
use crate::city::SolidKind;
use crate::player::Bumped;

/// A line the character says: spoken by the host, shown in a bubble.
#[derive(Message, Clone, Debug)]
pub struct Said {
    pub text: String,
}

struct Lines {
    building: &'static [&'static str],
    lamp: &'static [&'static str],
    edge: &'static [&'static str],
}

const ITALIAN: Lines = Lines {
    building: &[
        "Ahi. Tu parli, io sbatto. Bella squadra.",
        "Hai detto avanti. Il muro non era d'accordo.",
        "Tranquillo, il naso mi serviva poco.",
        "La prossima volta dimmi anche fermati, magari.",
        "Ottimo. Ho trovato un palazzo. Con la faccia.",
        "Io eseguo, eh. Mica scelgo io.",
        "Questo palazzo era qui anche prima, sai?",
        "Va bene, guidi tu. Si vede.",
    ],
    lamp: &[
        "Un lampione. L'unica cosa illuminata, qui.",
        "Bel palo. L'ho salutato da vicino.",
        "Tra tutta la strada libera, proprio il palo.",
    ],
    edge: &[
        "Qui finisce il quartiere. E anche la tua idea.",
        "Oltre non si va. Lo dice la siepe, non io.",
        "Bella siepe. Torniamo indietro?",
    ],
};

const ENGLISH: Lines = Lines {
    building: &[
        "Ouch. You talk, I crash. What a team.",
        "You said forward. The wall disagreed.",
        "It's fine, I barely used that nose.",
        "Next time maybe say stop as well.",
        "Great. I found a building. With my face.",
        "I just follow orders, you know.",
        "This building was here before, by the way.",
        "Sure, you're driving. It shows.",
    ],
    lamp: &[
        "A lamp post. The only bright thing around here.",
        "Nice pole. I said hello up close.",
        "A whole empty street, and you pick the pole.",
    ],
    edge: &[
        "The neighbourhood ends here. So does your plan.",
        "No further. The hedge says so, not me.",
        "Lovely hedge. Shall we turn around?",
    ],
};

fn lines(lang: &str) -> &'static Lines {
    match lang.split('-').next().unwrap_or("") {
        "it" => &ITALIAN,
        _ => &ENGLISH,
    }
}

/// The `turn`-th line for a bump: each list is gone through in order, so a
/// line does not come back before the others were said.
pub fn line_for(lang: &str, into: SolidKind, turn: usize) -> &'static str {
    let lines = lines(lang);
    let list = match into {
        SolidKind::Building => lines.building,
        SolidKind::Lamp => lines.lamp,
        SolidKind::Edge => lines.edge,
    };
    list[turn % list.len()]
}

pub struct LinesPlugin;

impl Plugin for LinesPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Said>()
            .add_systems(Update, react.after(crate::player::PlayerSet));
    }
}

fn react(
    mut bumps: MessageReader<Bumped>,
    status: Res<HostStatus>,
    mut host: ResMut<Host>,
    mut said: MessageWriter<Said>,
    mut turns: Local<[usize; 3]>,
) {
    for bump in bumps.read() {
        let slot = match bump.into {
            SolidKind::Building => 0,
            SolidKind::Lamp => 1,
            SolidKind::Edge => 2,
        };
        let text = line_for(&status.lang, bump.into, turns[slot]);
        turns[slot] += 1;
        host.say(text);
        said.write(Said {
            text: text.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_go_round_without_repeating() {
        let first = line_for("it", SolidKind::Lamp, 0);
        let second = line_for("it", SolidKind::Lamp, 1);
        assert_ne!(first, second);
        assert_eq!(line_for("it", SolidKind::Lamp, ITALIAN.lamp.len()), first);
        assert_eq!(line_for("de", SolidKind::Edge, 0), ENGLISH.edge[0]);
    }
}
