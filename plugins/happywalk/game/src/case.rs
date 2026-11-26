//! The mystery generator: a case from a city, a number and a level.
//!
//! A case is a small absurd crime in the player's town (all the bread gone
//! overnight, the fountain painted blue, forty ducks in the café) with a
//! handful of suspects, one of whom did it. It is written entirely by this
//! module, from a seed, before anyone plays it: who did it, why, what each
//! suspect looks like, knows, hides and claims, and which clues point where.
//! The language model only acts it out; it can neither change the culprit nor
//! invent a clue that matters.
//!
//! **Every case can be solved by reasoning.** Three kinds of clue exist:
//!
//! - a **trace** at the scene (flour, blue paint, feathers...): whoever did
//!   it carries it, and a suspect's traces are learned by talking to them;
//! - a **sighting** by a witness: something about the culprit's looks
//!   (glasses, a beard, a hat, tall, dressed in red...), which the player can
//!   check by looking at the suspects;
//! - an **alibi**: where a suspect says they were and with whom; asking that
//!   person confirms an innocent and gives the culprit away.
//!
//! The generator only keeps a case in which the culprit fits every clue and
//! each innocent is ruled out by at least one ([`Case::is_fair`]); tests check
//! it over thousands of cases.
//!
//! **Levels** make cases longer: more suspects, innocents who carry the same
//! trace (so the trace alone is not enough), sightings that rule out fewer at
//! a time, and alibis to check from level 3 on.

use bevy::math::IVec2;

use crate::figure::Look;
use crate::gen::PlaceKind;

/// The dice of one case.
struct Dice(u64);

impl Dice {
    fn new(seed: u64, number: u32, level: u32, attempt: u32) -> Self {
        let mut dice = Dice(seed ^ 0xC0FF_EE00_0000_0000);
        for part in [number as u64, level as u64, attempt as u64] {
            dice.0 ^= part.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            dice.next();
        }
        dice
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn pick<'a, T>(&mut self, list: &'a [T]) -> &'a T {
        &list[self.below(list.len())]
    }

    /// `count` different indices below `n`.
    fn several(&mut self, n: usize, count: usize) -> Vec<usize> {
        let mut all: Vec<usize> = (0..n).collect();
        for i in 0..all.len() {
            let j = i + self.below(all.len() - i);
            all.swap(i, j);
        }
        all.truncate(count.min(n));
        all
    }
}

/// What a culprit leaves behind, and carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Trace {
    Flour,
    BluePaint,
    Feathers,
    Soil,
    Ink,
    Wool,
    Sawdust,
    Glitter,
}

impl Trace {
    pub const ALL: [Trace; 8] = [
        Trace::Flour,
        Trace::BluePaint,
        Trace::Feathers,
        Trace::Soil,
        Trace::Ink,
        Trace::Wool,
        Trace::Sawdust,
        Trace::Glitter,
    ];

    /// As found at the scene.
    pub fn at_scene(self) -> &'static str {
        match self {
            Trace::Flour => "floury footprints",
            Trace::BluePaint => "drops of blue paint",
            Trace::Feathers => "a trail of feathers",
            Trace::Soil => "clods of fresh soil",
            Trace::Ink => "inky fingerprints",
            Trace::Wool => "loose threads of pink wool",
            Trace::Sawdust => "a little pile of sawdust",
            Trace::Glitter => "a lot of glitter",
        }
    }

    /// As carried by someone.
    pub fn on_someone(self) -> &'static str {
        match self {
            Trace::Flour => "flour on their sleeves",
            Trace::BluePaint => "blue paint on their hands",
            Trace::Feathers => "a feather stuck to their clothes",
            Trace::Soil => "soil on their shoes",
            Trace::Ink => "ink on their fingers",
            Trace::Wool => "pink wool threads on their clothes",
            Trace::Sawdust => "sawdust in their hair",
            Trace::Glitter => "glitter all over",
        }
    }
}

/// A crime: what happened, where, and what the culprit left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crime {
    pub what: &'static str,
    pub at: PlaceKind,
    pub trace: Trace,
}

/// The crimes a case can be about. To add one: add a row.
pub const CRIMES: &[Crime] = &[
    Crime {
        what: "all the bread in the bakery vanished overnight",
        at: PlaceKind::Bakery,
        trace: Trace::Flour,
    },
    Crime {
        what: "the fountain in the square was painted bright blue",
        at: PlaceKind::Square,
        trace: Trace::BluePaint,
    },
    Crime {
        what: "forty ducks were found sitting in the café, ordering nothing",
        at: PlaceKind::Cafe,
        trace: Trace::Feathers,
    },
    Crime {
        what: "every flower at the florist's was swapped for broccoli",
        at: PlaceKind::Florist,
        trace: Trace::Soil,
    },
    Crime {
        what: "all the newspapers at the newsstand now print only recipes",
        at: PlaceKind::Newsstand,
        trace: Trace::Ink,
    },
    Crime {
        what: "someone knitted a scarf for every tree in the park",
        at: PlaceKind::Park,
        trace: Trace::Wool,
    },
    Crime {
        what: "the park's benches were all turned to face the wall",
        at: PlaceKind::Park,
        trace: Trace::Sawdust,
    },
    Crime {
        what: "the greengrocer's watermelons were stacked into a pyramid on the roof",
        at: PlaceKind::Grocer,
        trace: Trace::Soil,
    },
    Crime {
        what: "the bakery's cakes were all decorated with tiny moustaches",
        at: PlaceKind::Bakery,
        trace: Trace::Flour,
    },
    Crime {
        what: "the square's pigeons were found wearing tiny hats",
        at: PlaceKind::Square,
        trace: Trace::Feathers,
    },
    Crime {
        what: "the café's chairs were glued to the ceiling",
        at: PlaceKind::Cafe,
        trace: Trace::Sawdust,
    },
    Crime {
        what: "the newsstand was covered in glitter from top to bottom",
        at: PlaceKind::Newsstand,
        trace: Trace::Glitter,
    },
];

/// Why someone would do such a thing.
pub const MOTIVES: &[&str] = &[
    "lost a bet with their cousin",
    "wanted to be on the evening news, just once",
    "calls it art, and is very serious about it",
    "was taking revenge on the pigeons, for reasons",
    "has a secret love for ducks",
    "was bored on a Tuesday",
    "wanted to make the mayor laugh",
    "read about it in a dream",
    "was practising for a talent show",
    "thought it would be a lovely surprise",
];

/// What people in town do.
pub const ROLES: &[(&str, Option<Trace>)] = &[
    ("baker", Some(Trace::Flour)),
    ("house painter", Some(Trace::BluePaint)),
    ("pigeon keeper", Some(Trace::Feathers)),
    ("gardener", Some(Trace::Soil)),
    ("newspaper seller", Some(Trace::Ink)),
    ("knitting champion", Some(Trace::Wool)),
    ("carpenter", Some(Trace::Sawdust)),
    ("party planner", Some(Trace::Glitter)),
    ("taxi driver", None),
    ("retired teacher", None),
    ("street sweeper", None),
    ("mime artist", None),
    ("letter carrier", None),
    ("ice-cream seller", None),
    ("opera singer", None),
    ("dog walker", None),
];

pub const NAMES: &[&str] = &[
    "Ada", "Bruno", "Carla", "Dino", "Elsa", "Fausto", "Gina", "Ugo", "Irma", "Leo", "Mara",
    "Nino", "Olga", "Pia", "Rino", "Sara", "Tito", "Vera", "Zeno", "Bice", "Ciro", "Delia", "Ezio",
    "Flora", "Gigi", "Ines", "Lino", "Nora", "Otto", "Rita",
];

pub const MANNERS: &[&str] = &[
    "touchy and dramatic",
    "chatty and nosy",
    "grumpy but fair",
    "dreamy, always somewhere else",
    "very polite and very nervous",
    "a terrible liar, even about the weather",
    "proud and a little pompous",
    "cheerful, laughs at everything",
    "suspicious of everyone",
    "sleepy, yawns between words",
];

/// Little innocent secrets: why an innocent might seem shifty.
pub const SECRETS: &[&str] = &[
    "secretly takes dance lessons",
    "talks to their plants every morning",
    "ate a whole cake alone last night",
    "is writing a romance novel about a lamp post",
    "pretends to read the newspaper but only looks at the pictures",
    "has never learned to ride a bike",
    "hides from their neighbour's cat",
    "sings in the shower so loud the street hears",
];

/// Something about a person that shows: what a witness can have seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Feature {
    Glasses,
    NoGlasses,
    Beard,
    NoBeard,
    Hat,
    NoHat,
    Tall,
    Short,
    DarkHair,
    RedHair,
    FairHair,
    GreyHair,
    ColourfulHair,
    ShavedHead,
    Dressed(&'static str),
}

impl Feature {
    /// As a witness would put it.
    pub fn words(self) -> String {
        match self {
            Feature::Glasses => "wore glasses".into(),
            Feature::NoGlasses => "had no glasses".into(),
            Feature::Beard => "had a beard or a moustache".into(),
            Feature::NoBeard => "had no beard".into(),
            Feature::Hat => "wore a hat".into(),
            Feature::NoHat => "had no hat".into(),
            Feature::Tall => "was tall".into(),
            Feature::Short => "was short".into(),
            Feature::DarkHair => "had dark hair".into(),
            Feature::RedHair => "had red hair".into(),
            Feature::FairHair => "had fair hair".into(),
            Feature::GreyHair => "had grey or white hair".into(),
            Feature::ColourfulHair => "had brightly coloured hair".into(),
            Feature::ShavedHead => "had a shaved head".into(),
            Feature::Dressed(colour) => format!("was dressed in {colour}"),
        }
    }
}

/// What a look shows, in a witness's terms.
pub fn features(look: &Look) -> Vec<Feature> {
    let mut out = vec![
        if look.glasses > 0 {
            Feature::Glasses
        } else {
            Feature::NoGlasses
        },
        if look.beard > 0 {
            Feature::Beard
        } else {
            Feature::NoBeard
        },
        if look.hat > 0 {
            Feature::Hat
        } else {
            Feature::NoHat
        },
    ];
    match look.height {
        0 => out.push(Feature::Short),
        2 => out.push(Feature::Tall),
        _ => {}
    }
    if look.hair == 0 {
        out.push(Feature::ShavedHead);
    } else if look.hat == 0 || look.hair >= 3 {
        // Hair shows unless a hat hides short hair.
        out.push(match look.hair_colour {
            0..=2 => Feature::DarkHair,
            3 => Feature::RedHair,
            4 => Feature::FairHair,
            5 | 6 => Feature::GreyHair,
            _ => Feature::ColourfulHair,
        });
    }
    let colour = match look.top_colour {
        0 => "orange",
        1 => "pink",
        2 => "red",
        3 | 4 => "blue",
        5 | 6 => "green",
        7 => "purple",
        8 => "cream",
        9 => "yellow",
        10 => "white",
        _ => "grey",
    };
    out.push(Feature::Dressed(colour));
    out
}

/// Where someone says they were at the time, and who can say so.
#[derive(Clone, Debug, PartialEq)]
pub struct Alibi {
    pub place: PlaceKind,
    /// The suspect (an index into the case's suspects) who was there too.
    pub with: usize,
    /// Whether it is so.
    pub true_: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suspect {
    pub name: &'static str,
    pub role: &'static str,
    pub manner: &'static str,
    pub secret: &'static str,
    pub look: Look,
    /// Where to find them.
    pub hangs_out: PlaceKind,
    /// The traces they carry (found out by talking to them).
    pub traces: Vec<Trace>,
    pub alibi: Option<Alibi>,
}

/// A clue, and who or what gives it.
#[derive(Clone, Debug, PartialEq)]
pub enum Clue {
    /// Found at the scene: the culprit carries this.
    Trace(Trace),
    /// A witness saw the culprit; `by` is the suspect who saw it, or `None`
    /// for the person who reported the case.
    Sighting { feature: Feature, by: Option<usize> },
    /// Asking `by` about `of`'s alibi.
    AlibiCheck { of: usize, by: usize },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Case {
    pub number: u32,
    pub level: u32,
    pub crime: Crime,
    pub motive: &'static str,
    /// Who reports the case to the character: a name and a manner.
    pub client: (&'static str, &'static str),
    pub suspects: Vec<Suspect>,
    pub culprit: usize,
    pub clues: Vec<Clue>,
}

/// How big a case is at a level.
pub fn size(level: u32) -> (usize, bool, bool) {
    let suspects = (2 + level as usize).min(6);
    // Innocents carrying the same trace from level 2, alibis from level 3.
    (suspects, level >= 2, level >= 3)
}

impl Case {
    /// Does `suspect` fit every clue?
    pub fn fits(&self, suspect: usize) -> bool {
        self.clues.iter().all(|clue| !self.rules_out(clue, suspect))
    }

    /// Does this clue rule `suspect` out?
    pub fn rules_out(&self, clue: &Clue, suspect: usize) -> bool {
        let who = &self.suspects[suspect];
        match clue {
            Clue::Trace(trace) => !who.traces.contains(trace),
            Clue::Sighting { feature, .. } => !features(&who.look).contains(feature),
            Clue::AlibiCheck { of, .. } => {
                *of == suspect && who.alibi.as_ref().is_some_and(|a| a.true_)
            }
        }
    }

    /// The case can be solved: the culprit fits every clue, and nobody else
    /// does.
    pub fn is_fair(&self) -> bool {
        self.fits(self.culprit)
            && (0..self.suspects.len())
                .filter(|s| *s != self.culprit)
                .all(|s| !self.fits(s))
    }
}

/// Generate case `number` of a city at a `level` (from 1).
pub fn case(seed: u64, number: u32, level: u32) -> Case {
    let level = level.max(1);
    // A few tries: a draw where two suspects look the same in every way a
    // witness could tell is thrown away.
    for attempt in 0..64 {
        if let Some(case) = attempt_case(seed, number, level, attempt) {
            return case;
        }
    }
    // Practically unreachable; the smallest case is always possible.
    attempt_case(seed, number, 1, 999).expect("a level-1 case")
}

fn attempt_case(seed: u64, number: u32, level: u32, attempt: u32) -> Option<Case> {
    let mut dice = Dice::new(seed, number, level, attempt);
    let (count, decoys, alibis) = size(level);
    let crime = *dice.pick(CRIMES);
    let motive = *dice.pick(MOTIVES);

    // People: names, jobs, manners, secrets all different within a case.
    let names = dice.several(NAMES.len(), count + 1);
    let roles = dice.several(ROLES.len(), count);
    let manners = dice.several(MANNERS.len(), count + 1);
    let secrets = dice.several(SECRETS.len(), count);
    // Where each hangs out: places other than the scene, spread out.
    let elsewhere: Vec<PlaceKind> = PlaceKind::ALL
        .iter()
        .copied()
        .filter(|p| *p != crime.at)
        .collect();
    let mut suspects: Vec<Suspect> = (0..count)
        .map(|i| {
            let (role, role_trace) = ROLES[roles[i]];
            Suspect {
                name: NAMES[names[i]],
                role,
                manner: MANNERS[manners[i]],
                secret: SECRETS[secrets[i]],
                look: Look::random(dice.next()),
                hangs_out: elsewhere[i % elsewhere.len()],
                traces: role_trace.into_iter().collect(),
                alibi: None,
            }
        })
        .collect();
    let culprit = dice.below(count);
    if !suspects[culprit].traces.contains(&crime.trace) {
        suspects[culprit].traces.push(crime.trace);
    }
    // From level 2, someone innocent carries the same trace (the baker is
    // always floury): the trace alone no longer settles it.
    let innocents: Vec<usize> = (0..count).filter(|s| *s != culprit).collect();
    if decoys {
        let how_many = 1
            + (level as usize)
                .saturating_sub(2)
                .min(innocents.len().saturating_sub(1));
        for &s in dice
            .several(innocents.len(), how_many)
            .iter()
            .map(|i| &innocents[*i])
        {
            if !suspects[s].traces.contains(&crime.trace) {
                suspects[s].traces.push(crime.trace);
            }
        }
    } else {
        // At level 1 only the culprit carries it.
        for &s in &innocents {
            suspects[s].traces.retain(|t| *t != crime.trace);
        }
    }

    let mut case = Case {
        number,
        level,
        crime,
        motive,
        client: (NAMES[names[count]], MANNERS[manners[count]]),
        suspects,
        culprit,
        clues: vec![Clue::Trace(crime.trace)],
    };

    // Alibis, from level 3: the innocents were somewhere in twos (or a
    // three), and each says so naming the other, so asking either confirms
    // it. The culprit claims to have been with somebody who was in fact
    // elsewhere with someone else, and who will say so.
    if alibis {
        let order: Vec<usize> = dice
            .several(innocents.len(), innocents.len())
            .into_iter()
            .map(|i| innocents[i])
            .collect();
        let mut groups: Vec<Vec<usize>> = order.chunks(2).map(|c| c.to_vec()).collect();
        if groups.len() > 1 && groups.last().is_some_and(|g| g.len() == 1) {
            let lone = groups.pop().unwrap();
            groups.last_mut().unwrap().extend(lone);
        }
        let spots = dice.several(elsewhere.len(), groups.len() + 1);
        for (group, spot) in groups.iter().zip(&spots) {
            for (k, &s) in group.iter().enumerate() {
                let with = group[(k + 1) % group.len()];
                if with != s {
                    case.suspects[s].alibi = Some(Alibi {
                        place: elsewhere[*spot],
                        with,
                        true_: true,
                    });
                }
            }
        }
        // The culprit's story: with an innocent, at a place they were not.
        let with = innocents[dice.below(innocents.len())];
        let theirs = case.suspects[with].alibi.as_ref().map(|a| a.place);
        let place = elsewhere
            .iter()
            .copied()
            .find(|p| Some(*p) != theirs && *p != elsewhere[spots[spots.len() - 1]])
            .unwrap_or(elsewhere[0]);
        case.suspects[culprit].alibi = Some(Alibi {
            place,
            with,
            true_: false,
        });
    }

    // Rule innocents out one clue at a time until only the culprit fits.
    // Higher levels prefer clues that rule out fewer at once, so more are
    // needed.
    let culprit_features = features(&case.suspects[culprit].look);
    let mut guard = 0;
    while (0..count).any(|s| s != culprit && case.fits(s)) {
        guard += 1;
        if guard > 20 {
            return None;
        }
        let remaining: Vec<usize> = (0..count)
            .filter(|s| *s != culprit && case.fits(*s))
            .collect();
        // Sightings that would rule somebody out.
        let mut sightings: Vec<(Feature, usize)> = culprit_features
            .iter()
            .map(|f| {
                (
                    *f,
                    remaining
                        .iter()
                        .filter(|s| !features(&case.suspects[**s].look).contains(f))
                        .count(),
                )
            })
            .filter(|(_, n)| *n > 0)
            .filter(|(f, _)| {
                !case
                    .clues
                    .iter()
                    .any(|c| matches!(c, Clue::Sighting { feature, .. } if feature == f))
            })
            .collect();
        let use_alibi = alibis && dice.chance(40);
        if use_alibi || sightings.is_empty() {
            if !alibis {
                return None;
            }
            let of = remaining[dice.below(remaining.len())];
            let by = case.suspects[of].alibi.as_ref().map(|a| a.with)?;
            case.clues.push(Clue::AlibiCheck { of, by });
            continue;
        }
        // Easy levels: the sighting that rules out most; harder: the least.
        sightings.sort_by_key(|(_, n)| if level <= 2 { usize::MAX - n } else { *n });
        let (feature, _) = sightings[0];
        // A witness: the client at first, a suspect who is not the culprit
        // later (so it has to be asked for).
        let by = if level >= 2 && dice.chance(60) {
            Some(innocents[dice.below(innocents.len())])
        } else {
            None
        };
        case.clues.push(Clue::Sighting { feature, by });
    }
    case.is_fair().then_some(case)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_case_is_fair_at_every_level() {
        for seed in [1u64, 7, 2026, 99_999] {
            for level in 1..=8 {
                for number in 0..60 {
                    let case = case(seed, number, level);
                    assert!(case.is_fair(), "seed {seed} level {level} case {number}");
                    assert_eq!(case.suspects.len(), size(case.level).0);
                }
            }
        }
    }

    #[test]
    fn the_same_city_and_number_is_the_same_case() {
        assert_eq!(case(5, 3, 2), case(5, 3, 2));
        assert_ne!(case(5, 3, 2), case(5, 4, 2));
        assert_ne!(case(5, 3, 2).suspects, case(6, 3, 2).suspects);
    }

    #[test]
    fn cases_grow_with_the_level() {
        let average = |level: u32| -> f32 {
            (0..200)
                .map(|n| case(11, n, level).clues.len() as f32)
                .sum::<f32>()
                / 200.0
        };
        let (one, three, six) = (average(1), average(3), average(6));
        assert!(one < three && three < six, "{one} {three} {six}");
        // At level 1 the trace and one sighting are about enough.
        assert!(one <= 2.5, "{one}");
        // Alibis appear from level 3.
        assert!((0..50).all(|n| case(11, n, 2).suspects.iter().all(|s| s.alibi.is_none())));
        assert!((0..50).any(|n| case(11, n, 4)
            .clues
            .iter()
            .any(|c| matches!(c, Clue::AlibiCheck { .. }))));
    }

    #[test]
    fn from_level_two_the_trace_alone_does_not_settle_it() {
        for number in 0..100 {
            let case = case(3, number, 2);
            let carriers = case
                .suspects
                .iter()
                .filter(|s| s.traces.contains(&case.crime.trace))
                .count();
            assert!(carriers >= 2, "case {number}");
        }
        for number in 0..100 {
            let case = case(3, number, 1);
            let carriers = case
                .suspects
                .iter()
                .filter(|s| s.traces.contains(&case.crime.trace))
                .count();
            assert_eq!(carriers, 1, "case {number}");
        }
    }

    #[test]
    fn people_in_a_case_are_all_different() {
        for number in 0..100 {
            let case = case(8, number, 5);
            let names: std::collections::HashSet<_> = case
                .suspects
                .iter()
                .map(|s| s.name)
                .chain([case.client.0])
                .collect();
            assert_eq!(names.len(), case.suspects.len() + 1);
            let roles: std::collections::HashSet<_> =
                case.suspects.iter().map(|s| s.role).collect();
            assert_eq!(roles.len(), case.suspects.len());
            let looks: std::collections::HashSet<_> =
                case.suspects.iter().map(|s| s.look.visible()).collect();
            assert_eq!(looks.len(), case.suspects.len());
            for suspect in &case.suspects {
                assert_ne!(suspect.hangs_out, case.crime.at);
            }
        }
    }

    #[test]
    fn alibis_agree_with_each_other() {
        for number in 0..200 {
            let case = case(12, number, 5);
            for (s, suspect) in case.suspects.iter().enumerate() {
                let Some(alibi) = &suspect.alibi else {
                    continue;
                };
                let other = case.suspects[alibi.with].alibi.as_ref();
                if alibi.true_ {
                    // The person named was there too, and says so.
                    let other = other.expect("an alibi partner with an alibi");
                    assert_eq!(other.place, alibi.place, "case {number}");
                    assert!(other.true_);
                    let named_back = case.suspects.iter().enumerate().any(|(k, x)| {
                        k != s
                            && x.alibi.as_ref().is_some_and(|a| {
                                a.place == alibi.place && (a.with == s || k == alibi.with)
                            })
                    });
                    assert!(named_back, "case {number}");
                } else {
                    // The culprit names someone who was somewhere else.
                    assert_eq!(s, case.culprit);
                    assert_ne!(other.map(|o| o.place), Some(alibi.place), "case {number}");
                }
            }
        }
    }

    #[test]
    fn an_alibi_only_rules_out_who_it_belongs_to() {
        let case = (0..200)
            .map(|n| case(4, n, 4))
            .find(|c| c.clues.iter().any(|c| matches!(c, Clue::AlibiCheck { .. })))
            .unwrap();
        for clue in &case.clues {
            if let Clue::AlibiCheck { of, by } = clue {
                assert_ne!(of, by);
                assert_ne!(*of, case.culprit);
                for s in 0..case.suspects.len() {
                    assert_eq!(case.rules_out(clue, s), s == *of);
                }
            }
        }
        assert!(!case.suspects[case.culprit].alibi.as_ref().unwrap().true_);
    }

    #[test]
    fn every_crime_happens_somewhere_a_city_has() {
        use crate::gen;
        for crime in CRIMES {
            assert!(
                gen::nearest_place(2026, bevy::math::Vec2::ZERO, crime.at, 8).is_some(),
                "{}",
                crime.what
            );
        }
        let _ = IVec2::ZERO;
    }
}
