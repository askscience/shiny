//! `IntentPlugin`: from recognised text to what the player meant.
//!
//! The known commands ("forward", "left", "stop"…) become an [`Order`] at
//! once, with no model in between, and from **partial** results: the
//! character moves while the player is still finishing the sentence. A final
//! result in which no command was found is reported as [`NotUnderstood`]
//! (the character's brain gets those).
//!
//! The vocabulary is the game's own, per language; the same words are given
//! to the recogniser as its grammar, so it only ever proposes these.

use bevy::prelude::*;

use crate::bridge::{now_ms, Heard, Host, HostStatus};
use crate::telemetry::Lap;

/// What the character can be told to do.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Forward,
    Back,
    Left,
    Right,
    Stop,
    Faster,
    Slower,
}

/// A whole sentence in which there was no command.
#[derive(Message, Clone, Debug)]
pub struct NotUnderstood {
    pub text: String,
}

/// The words of one language. Words are matched one by one, lower case,
/// without accents; the words around them are skipped ("più veloce" is
/// "veloce").
pub struct Vocabulary {
    pub lang: &'static str,
    words: &'static [(&'static str, Order)],
    /// Extra phrases worth telling the recogniser (accents as spoken).
    phrases: &'static [&'static str],
}

const ITALIAN: Vocabulary = Vocabulary {
    lang: "it",
    words: &[
        ("avanti", Order::Forward),
        ("vai", Order::Forward),
        ("cammina", Order::Forward),
        ("indietro", Order::Back),
        ("sinistra", Order::Left),
        ("destra", Order::Right),
        ("fermati", Order::Stop),
        ("fermo", Order::Stop),
        ("ferma", Order::Stop),
        ("stop", Order::Stop),
        ("alt", Order::Stop),
        ("veloce", Order::Faster),
        ("corri", Order::Faster),
        ("piano", Order::Slower),
        ("lento", Order::Slower),
        ("rallenta", Order::Slower),
    ],
    phrases: &["più veloce", "più piano", "a sinistra", "a destra", "gira"],
};

const ENGLISH: Vocabulary = Vocabulary {
    lang: "en",
    words: &[
        ("forward", Order::Forward),
        ("go", Order::Forward),
        ("walk", Order::Forward),
        ("back", Order::Back),
        ("backward", Order::Back),
        ("left", Order::Left),
        ("right", Order::Right),
        ("stop", Order::Stop),
        ("halt", Order::Stop),
        ("faster", Order::Faster),
        ("run", Order::Faster),
        ("slower", Order::Slower),
        ("slow", Order::Slower),
    ],
    phrases: &["turn left", "turn right", "slow down", "go back"],
};

/// The vocabulary for a language code; English when the game has none.
pub fn vocabulary(lang: &str) -> &'static Vocabulary {
    match lang.split('-').next().unwrap_or("") {
        "it" => &ITALIAN,
        _ => &ENGLISH,
    }
}

fn plain(word: &str) -> String {
    word.chars()
        .filter_map(|c| match c {
            'à' | 'á' => Some('a'),
            'è' | 'é' => Some('e'),
            'ì' | 'í' => Some('i'),
            'ò' | 'ó' => Some('o'),
            'ù' | 'ú' => Some('u'),
            c if c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .collect()
}

impl Vocabulary {
    /// What to tell the recogniser to expect.
    pub fn grammar(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = self.words.iter().map(|(word, _)| *word).collect();
        out.extend(self.phrases);
        out
    }

    /// The commands in `text`, in the order they were said. Every other word
    /// is skipped, so "vai a sinistra" is a walk and a turn, and "gira a
    /// destra, per favore" is a turn.
    pub fn commands(&self, text: &str) -> Vec<Order> {
        text.split_whitespace()
            .map(plain)
            .filter_map(|word| self.words.iter().find(|(w, _)| *w == word).map(|(_, c)| *c))
            .collect()
    }
}

/// How many commands of the sentence being spoken were already carried out,
/// so a growing partial result only adds the new ones.
#[derive(Resource, Default)]
struct Utterance {
    done: usize,
}

impl Utterance {
    /// The commands of `text` not carried out yet.
    fn fresh(
        &mut self,
        vocabulary: &Vocabulary,
        text: &str,
        is_final: bool,
    ) -> (Vec<Order>, usize) {
        let all = vocabulary.commands(text);
        let total = all.len();
        let fresh = all.into_iter().skip(self.done).collect();
        // A partial result can be revised to fewer commands; what was done
        // stays done, and the count follows the sentence as it stands.
        self.done = if is_final { 0 } else { total };
        (fresh, total)
    }
}

pub struct IntentPlugin;

impl Plugin for IntentPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Utterance>()
            .add_message::<Order>()
            .add_message::<NotUnderstood>()
            .add_systems(PreUpdate, (teach_recogniser, understand).chain());
    }
}

/// Give the recogniser the vocabulary of the language it listens in.
fn teach_recogniser(status: Res<HostStatus>, host: Res<Host>, mut taught: Local<String>) {
    if status.lang.is_empty() || *taught == status.lang {
        return;
    }
    *taught = status.lang.clone();
    host.set_grammar(&vocabulary(&status.lang).grammar());
}

fn understand(
    mut heard: MessageReader<Heard>,
    status: Res<HostStatus>,
    mut utterance: ResMut<Utterance>,
    mut commands: MessageWriter<Order>,
    mut lost: MessageWriter<NotUnderstood>,
    mut laps: MessageWriter<Lap>,
) {
    let vocabulary = vocabulary(&status.lang);
    for h in heard.read() {
        let (fresh, total) = utterance.fresh(vocabulary, &h.text, h.is_final);
        if h.is_final && total == 0 && !h.text.is_empty() {
            lost.write(NotUnderstood {
                text: h.text.clone(),
            });
        }
        if fresh.is_empty() {
            continue;
        }
        // The command acts in this very frame: this is the whole way from the
        // word to the movement.
        let now = now_ms();
        let about = format!("{:?} from \"{}\"", fresh, h.text);
        if h.speech_end_ms > 0.0 {
            laps.write(Lap {
                link: "speech→move",
                ms: (now - h.speech_end_ms).max(0.0),
                about: about.clone(),
            });
        }
        laps.write(Lap {
            link: "text→move",
            ms: (now - h.heard_at_ms).max(0.0),
            about,
        });
        for command in fresh {
            commands.write(command);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Order::*;

    #[test]
    fn italian_commands() {
        let it = vocabulary("it");
        assert_eq!(it.commands("avanti"), [Forward]);
        assert_eq!(it.commands("più veloce"), [Faster]);
        assert_eq!(it.commands("piu piano"), [Slower]);
        assert_eq!(it.commands("gira a sinistra poi fermati"), [Left, Stop]);
        assert_eq!(it.commands("buongiorno"), []);
    }

    #[test]
    fn the_words_around_a_command_are_skipped() {
        let it = vocabulary("it");
        assert_eq!(it.commands("vai a destra"), [Forward, Right]);
        let en = vocabulary("en-US");
        assert_eq!(en.commands("turn left please"), [Left]);
        assert_eq!(en.commands("slow down"), [Slower]);
    }

    #[test]
    fn a_growing_sentence_only_adds_its_new_commands() {
        let it = vocabulary("it");
        let mut utterance = Utterance::default();
        assert_eq!(utterance.fresh(it, "avanti", false).0, [Forward]);
        assert_eq!(utterance.fresh(it, "avanti", false).0, []);
        assert_eq!(utterance.fresh(it, "avanti sinistra", false).0, [Left]);
        // The final result repeats the sentence: nothing is done twice.
        assert_eq!(utterance.fresh(it, "avanti sinistra", true).0, []);
        // The next sentence starts from nothing.
        assert_eq!(utterance.fresh(it, "fermati", true).0, [Stop]);
        // A sentence that ends with nothing understood still ends.
        assert_eq!(utterance.fresh(it, "destra", false).0, [Right]);
        assert_eq!(utterance.fresh(it, "", true).0, []);
        assert_eq!(utterance.fresh(it, "vai", false).0, [Forward]);
        // A revision to fewer commands does nothing; growing again adds.
        assert_eq!(utterance.fresh(it, "vai sinistra", false).0, [Left]);
        assert_eq!(utterance.fresh(it, "vai", false).0, []);
        assert_eq!(utterance.fresh(it, "vai destra", false).0, [Right]);
    }

    #[test]
    fn the_grammar_holds_every_word() {
        let grammar = vocabulary("it").grammar();
        for word in ["avanti", "fermati", "più veloce"] {
            assert!(grammar.contains(&word), "{word}");
        }
    }

    #[test]
    fn unknown_languages_use_english() {
        assert_eq!(vocabulary("sv").lang, "en");
    }
}
