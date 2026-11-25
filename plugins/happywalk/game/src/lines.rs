//! Stand-in lines, for when the character's brain cannot answer: the AI is
//! switched off, there is no model, or its answer is late. The character
//! never stays silent after a bump. Same voice as the brain is asked for: it
//! knows it is steered by voice, it is ironic about it, and it is never mean.

use crate::city::SolidKind;

/// What a line is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topic {
    Bump(SolidKind),
    /// The player said something that was no command.
    NotUnderstood,
}

struct Lines {
    building: &'static [&'static str],
    lamp: &'static [&'static str],
    lost: &'static [&'static str],
    /// Anything else in the street.
    thing: &'static [&'static str],
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
    lost: &[
        "Bello. E io cosa dovrei farci?",
        "Non ho capito, ma annuisco lo stesso.",
        "Parole grosse. Proviamo con avanti?",
    ],
    thing: &[
        "C'era una cosa. Adesso lo sappiamo in due.",
        "Tutta la città libera, e tu scegli questo.",
        "Io l'avevo visto. Tu no, evidentemente.",
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
    lost: &[
        "Lovely. And what do I do with that?",
        "No idea what that means, but I'm nodding.",
        "Big words. Shall we try forward?",
    ],
    thing: &[
        "There was a thing there. Now we both know.",
        "A whole city to walk in, and you choose this.",
        "I saw it coming. You, clearly, did not.",
    ],
};

fn lines(lang: &str) -> &'static Lines {
    match lang.split('-').next().unwrap_or("") {
        "it" => &ITALIAN,
        _ => &ENGLISH,
    }
}

/// The `turn`-th stand-in line about `topic`: each list is gone through in
/// order, so a line does not come back before the others were said.
pub fn stand_in(lang: &str, topic: Topic, turn: usize) -> &'static str {
    let lines = lines(lang);
    let list = match topic {
        Topic::Bump(SolidKind::Building) => lines.building,
        Topic::Bump(SolidKind::Lamp) => lines.lamp,
        Topic::NotUnderstood => lines.lost,
        Topic::Bump(_) => lines.thing,
    };
    list[turn % list.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_go_round_without_repeating() {
        let lamp = Topic::Bump(SolidKind::Lamp);
        let first = stand_in("it", lamp, 0);
        assert_ne!(first, stand_in("it", lamp, 1));
        assert_eq!(stand_in("it", lamp, ITALIAN.lamp.len()), first);
        assert_eq!(
            stand_in("de", Topic::Bump(SolidKind::Fountain), 0),
            ENGLISH.thing[0]
        );
        assert_eq!(stand_in("it", Topic::NotUnderstood, 0), ITALIAN.lost[0]);
    }
}
