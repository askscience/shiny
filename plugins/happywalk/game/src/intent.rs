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

use crate::bridge::{now_ms, Heard, Host, HostStatus, Wardrobe};
use crate::gen::PlaceKind;
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
    /// Greet whoever is near.
    Wave,
    /// Walk to the nearest place of a kind.
    GoTo(PlaceKind),
    /// Keep the last seconds of play.
    Clip,
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
        ("saluta", Order::Wave),
        ("ciao", Order::Wave),
        ("buongiorno", Order::Wave),
        ("clip", Order::Clip),
        ("salva", Order::Clip),
        ("registra", Order::Clip),
        ("forno", Order::GoTo(PlaceKind::Bakery)),
        ("panetteria", Order::GoTo(PlaceKind::Bakery)),
        ("pane", Order::GoTo(PlaceKind::Bakery)),
        ("fiorista", Order::GoTo(PlaceKind::Florist)),
        ("fiori", Order::GoTo(PlaceKind::Florist)),
        ("bar", Order::GoTo(PlaceKind::Cafe)),
        ("caffe", Order::GoTo(PlaceKind::Cafe)),
        ("fruttivendolo", Order::GoTo(PlaceKind::Grocer)),
        ("frutta", Order::GoTo(PlaceKind::Grocer)),
        ("edicola", Order::GoTo(PlaceKind::Newsstand)),
        ("giornale", Order::GoTo(PlaceKind::Newsstand)),
        ("parco", Order::GoTo(PlaceKind::Park)),
        ("piazza", Order::GoTo(PlaceKind::Square)),
    ],
    phrases: &[
        "più veloce",
        "più piano",
        "a sinistra",
        "a destra",
        "gira",
        "caffè",
        "al",
        "alla",
        "in",
        "dal",
    ],
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
        ("wave", Order::Wave),
        ("hello", Order::Wave),
        ("hi", Order::Wave),
        ("clip", Order::Clip),
        ("save", Order::Clip),
        ("bakery", Order::GoTo(PlaceKind::Bakery)),
        ("bread", Order::GoTo(PlaceKind::Bakery)),
        ("florist", Order::GoTo(PlaceKind::Florist)),
        ("flowers", Order::GoTo(PlaceKind::Florist)),
        ("cafe", Order::GoTo(PlaceKind::Cafe)),
        ("coffee", Order::GoTo(PlaceKind::Cafe)),
        ("grocer", Order::GoTo(PlaceKind::Grocer)),
        ("greengrocer", Order::GoTo(PlaceKind::Grocer)),
        ("fruit", Order::GoTo(PlaceKind::Grocer)),
        ("newsstand", Order::GoTo(PlaceKind::Newsstand)),
        ("newspaper", Order::GoTo(PlaceKind::Newsstand)),
        ("park", Order::GoTo(PlaceKind::Park)),
        ("square", Order::GoTo(PlaceKind::Square)),
    ],
    phrases: &[
        "turn left",
        "turn right",
        "slow down",
        "go back",
        "café",
        "to the",
    ],
};

const SPANISH: Vocabulary = Vocabulary {
    lang: "es",
    words: &[
        ("adelante", Order::Forward),
        ("avanza", Order::Forward),
        ("camina", Order::Forward),
        ("atras", Order::Back),
        ("izquierda", Order::Left),
        ("derecha", Order::Right),
        ("para", Order::Stop),
        ("alto", Order::Stop),
        ("quieto", Order::Stop),
        ("rapido", Order::Faster),
        ("corre", Order::Faster),
        ("despacio", Order::Slower),
        ("lento", Order::Slower),
        ("saluda", Order::Wave),
        ("hola", Order::Wave),
        ("clip", Order::Clip),
        ("guarda", Order::Clip),
        ("panaderia", Order::GoTo(PlaceKind::Bakery)),
        ("pan", Order::GoTo(PlaceKind::Bakery)),
        ("floristeria", Order::GoTo(PlaceKind::Florist)),
        ("flores", Order::GoTo(PlaceKind::Florist)),
        ("cafe", Order::GoTo(PlaceKind::Cafe)),
        ("fruteria", Order::GoTo(PlaceKind::Grocer)),
        ("fruta", Order::GoTo(PlaceKind::Grocer)),
        ("quiosco", Order::GoTo(PlaceKind::Newsstand)),
        ("periodico", Order::GoTo(PlaceKind::Newsstand)),
        ("parque", Order::GoTo(PlaceKind::Park)),
        ("plaza", Order::GoTo(PlaceKind::Square)),
    ],
    phrases: &[
        "más rápido",
        "más despacio",
        "a la izquierda",
        "a la derecha",
        "atrás",
        "café",
        "panadería",
        "frutería",
        "floristería",
        "periódico",
        "gira",
    ],
};

const FRENCH: Vocabulary = Vocabulary {
    lang: "fr",
    words: &[
        ("avance", Order::Forward),
        ("marche", Order::Forward),
        ("arriere", Order::Back),
        ("recule", Order::Back),
        ("gauche", Order::Left),
        ("droite", Order::Right),
        ("stop", Order::Stop),
        ("arrete", Order::Stop),
        ("halte", Order::Stop),
        ("vite", Order::Faster),
        ("cours", Order::Faster),
        ("doucement", Order::Slower),
        ("lentement", Order::Slower),
        ("salue", Order::Wave),
        ("bonjour", Order::Wave),
        ("salut", Order::Wave),
        ("clip", Order::Clip),
        ("enregistre", Order::Clip),
        ("boulangerie", Order::GoTo(PlaceKind::Bakery)),
        ("pain", Order::GoTo(PlaceKind::Bakery)),
        ("fleuriste", Order::GoTo(PlaceKind::Florist)),
        ("fleurs", Order::GoTo(PlaceKind::Florist)),
        ("cafe", Order::GoTo(PlaceKind::Cafe)),
        ("primeur", Order::GoTo(PlaceKind::Grocer)),
        ("fruits", Order::GoTo(PlaceKind::Grocer)),
        ("kiosque", Order::GoTo(PlaceKind::Newsstand)),
        ("journal", Order::GoTo(PlaceKind::Newsstand)),
        ("parc", Order::GoTo(PlaceKind::Park)),
        ("place", Order::GoTo(PlaceKind::Square)),
    ],
    phrases: &[
        "plus vite",
        "plus doucement",
        "à gauche",
        "à droite",
        "arrière",
        "arrête",
        "café",
        "tourne",
        "en avant",
    ],
};

const GERMAN: Vocabulary = Vocabulary {
    lang: "de",
    words: &[
        ("vorwarts", Order::Forward),
        ("los", Order::Forward),
        ("geh", Order::Forward),
        ("zuruck", Order::Back),
        ("links", Order::Left),
        ("rechts", Order::Right),
        ("stopp", Order::Stop),
        ("halt", Order::Stop),
        ("schneller", Order::Faster),
        ("lauf", Order::Faster),
        ("langsamer", Order::Slower),
        ("langsam", Order::Slower),
        ("winke", Order::Wave),
        ("hallo", Order::Wave),
        ("clip", Order::Clip),
        ("speichern", Order::Clip),
        ("backerei", Order::GoTo(PlaceKind::Bakery)),
        ("brot", Order::GoTo(PlaceKind::Bakery)),
        ("blumenladen", Order::GoTo(PlaceKind::Florist)),
        ("blumen", Order::GoTo(PlaceKind::Florist)),
        ("cafe", Order::GoTo(PlaceKind::Cafe)),
        ("kaffee", Order::GoTo(PlaceKind::Cafe)),
        ("obstladen", Order::GoTo(PlaceKind::Grocer)),
        ("obst", Order::GoTo(PlaceKind::Grocer)),
        ("kiosk", Order::GoTo(PlaceKind::Newsstand)),
        ("zeitung", Order::GoTo(PlaceKind::Newsstand)),
        ("park", Order::GoTo(PlaceKind::Park)),
        ("platz", Order::GoTo(PlaceKind::Square)),
    ],
    phrases: &[
        "vorwärts",
        "zurück",
        "nach links",
        "nach rechts",
        "bäckerei",
        "café",
    ],
};

/// The vocabulary for a language code; English when the game has none.
pub fn vocabulary(lang: &str) -> &'static Vocabulary {
    match lang.split('-').next().unwrap_or("") {
        "it" => &ITALIAN,
        "es" => &SPANISH,
        "fr" => &FRENCH,
        "de" => &GERMAN,
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
            'ù' | 'ú' | 'û' | 'ü' => Some('u'),
            'â' | 'ä' => Some('a'),
            'ê' | 'ë' => Some('e'),
            'î' | 'ï' => Some('i'),
            'ô' | 'ö' => Some('o'),
            'ñ' => Some('n'),
            'ç' => Some('c'),
            'ß' => Some('s'),
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
    wardrobe: Res<Wardrobe>,
    mut utterance: ResMut<Utterance>,
    mut commands: MessageWriter<Order>,
    mut lost: MessageWriter<NotUnderstood>,
    mut laps: MessageWriter<Lap>,
) {
    if wardrobe.0 {
        // Being dressed: what is said is not for the character.
        heard.clear();
        return;
    }
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
        assert_eq!(it.commands("buonasera a tutti"), []);
        assert_eq!(it.commands("saluta"), [Wave]);
    }

    #[test]
    fn a_place_is_somewhere_to_go() {
        let it = vocabulary("it");
        assert_eq!(
            it.commands("vai al forno"),
            [Forward, GoTo(PlaceKind::Bakery)]
        );
        assert_eq!(
            it.commands("andiamo a prendere un caffè in piazza"),
            [GoTo(PlaceKind::Cafe), GoTo(PlaceKind::Square)]
        );
        assert_eq!(
            vocabulary("en").commands("to the park please"),
            [GoTo(PlaceKind::Park)]
        );
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
    fn other_languages_have_the_whole_set_of_orders() {
        for lang in ["it", "en", "es", "fr", "de"] {
            let vocabulary = vocabulary(lang);
            assert_eq!(vocabulary.lang, lang);
            let has = |order: Order| vocabulary.words.iter().any(|(_, o)| *o == order);
            for order in [Forward, Back, Left, Right, Stop, Faster, Slower, Wave, Clip] {
                assert!(has(order), "{lang}: no word for {order:?}");
            }
            for kind in PlaceKind::ALL {
                assert!(has(GoTo(kind)), "{lang}: no word for {kind:?}");
            }
            // Words are stored the way they are matched: plain, lower case.
            for (word, _) in vocabulary.words {
                assert_eq!(plain(word), *word, "{lang}");
            }
        }
        assert_eq!(
            vocabulary("es").commands("más rápido a la panadería"),
            [Faster, GoTo(PlaceKind::Bakery)]
        );
        assert_eq!(vocabulary("fr").commands("arrête"), [Stop]);
        assert_eq!(
            vocabulary("de").commands("zurück zur Bäckerei".to_lowercase().as_str()),
            [Back, GoTo(PlaceKind::Bakery)]
        );
    }

    #[test]
    fn unknown_languages_use_english() {
        assert_eq!(vocabulary("sv").lang, "en");
    }
}
