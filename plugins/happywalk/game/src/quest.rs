//! `QuestPlugin`: errands. Small everyday things to do in the city, one at a
//! time: buy bread, fetch the paper, sit a while in the park. Each sends the
//! character to the nearest place of a kind; getting there does it. Five
//! make a round, with a beginning and an end.
//!
//! An errand is a row in [`ERRANDS`]: to add one, add a row.

use bevy::prelude::*;

use crate::city::CitySeed;
use crate::gen::{self, Place, PlaceKind};
use crate::hud::GameFont;
use crate::nav::REACH;
use crate::player::{Player, PlayerSet};

/// One thing to do.
pub struct Errand {
    pub at: PlaceKind,
    /// What to tell the model the errand is.
    pub task: &'static str,
    pub italian: &'static str,
    pub english: &'static str,
}

pub const ERRANDS: &[Errand] = &[
    Errand {
        at: PlaceKind::Bakery,
        task: "buy bread at the bakery",
        italian: "Compra il pane al forno",
        english: "Buy bread at the bakery",
    },
    Errand {
        at: PlaceKind::Newsstand,
        task: "fetch the newspaper from the newsstand",
        italian: "Prendi il giornale all'edicola",
        english: "Fetch the paper at the newsstand",
    },
    Errand {
        at: PlaceKind::Florist,
        task: "buy flowers at the florist",
        italian: "Compra dei fiori dal fiorista",
        english: "Buy flowers at the florist",
    },
    Errand {
        at: PlaceKind::Grocer,
        task: "buy fruit at the greengrocer",
        italian: "Prendi la frutta dal fruttivendolo",
        english: "Get fruit at the greengrocer",
    },
    Errand {
        at: PlaceKind::Cafe,
        task: "have a coffee at the café",
        italian: "Bevi un caffè al bar",
        english: "Have a coffee at the café",
    },
    Errand {
        at: PlaceKind::Park,
        task: "take a walk in the park",
        italian: "Fai due passi al parco",
        english: "Take a stroll in the park",
    },
    Errand {
        at: PlaceKind::Square,
        task: "go and see the fountain in the square",
        italian: "Vai a vedere la fontana in piazza",
        english: "Go and see the fountain in the square",
    },
];

/// How many errands make a round.
pub const ROUND: u32 = 5;
/// Within this many metres of the place, the errand is done.
const THERE: f32 = 3.2;

/// An errand was done.
#[derive(Message, Clone, Copy, Debug)]
pub struct ErrandDone {
    /// Which, in [`ERRANDS`].
    pub errand: usize,
    /// How many are done in this round, this one included.
    pub done: u32,
    /// The round is complete.
    pub round_over: bool,
}

/// What the character is out to do.
#[derive(Resource, Default, Debug)]
pub struct Quest {
    /// The errand in hand and where it is done.
    pub current: Option<(usize, Place)>,
    pub done: u32,
    pub rounds: u32,
    /// How many were ever given: the next one's turn in the seed.
    given: u32,
}

impl Quest {
    pub fn errand(&self) -> Option<&'static Errand> {
        self.current.map(|(index, _)| &ERRANDS[index])
    }
}

/// The `turn`-th errand of a city: from the seed, never the same place twice
/// running, and only where there is such a place within reach of `from`.
pub fn next_errand(
    seed: u64,
    turn: u32,
    last: Option<usize>,
    from: Vec2,
) -> Option<(usize, Place)> {
    let start = gen::chance(seed, IVec2::new(turn as i32, 0), 1300) as usize;
    (0..ERRANDS.len())
        .map(|i| (start + i) % ERRANDS.len())
        .filter(|index| Some(*index) != last)
        .find_map(|index| {
            gen::nearest_place(seed, from, ERRANDS[index].at, REACH).map(|place| (index, place))
        })
}

/// Which way a place is from where the character stands and faces: an arrow.
pub fn arrow(from: Vec2, facing: f32, to: Vec2) -> char {
    let towards = (-(to.x - from.x)).atan2(-(to.y - from.y));
    let turn = (towards - facing + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI;
    match turn {
        t if t.abs() < 0.6 => '↑',
        t if t.abs() > 2.5 => '↓',
        t if t > 0.0 => '←',
        _ => '→',
    }
}

#[derive(Component)]
struct QuestLabel;

#[derive(Component)]
struct QuestPanel;

pub struct QuestPlugin;

impl Plugin for QuestPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Quest>()
            .add_message::<ErrandDone>()
            .add_systems(Update, (progress, show).chain().after(PlayerSet));
    }
}

/// Give the first errand, notice when one is done, give the next.
fn progress(
    seed: Res<CitySeed>,
    players: Query<&Transform, With<Player>>,
    mut quest: ResMut<Quest>,
    mut done: MessageWriter<ErrandDone>,
    wardrobe: Res<crate::bridge::Wardrobe>,
    mystery: Res<crate::mystery::Mystery>,
) {
    if seed.is_changed() && !seed.is_added() {
        *quest = Quest::default();
    }
    // While there is a case to solve, errands wait.
    if wardrobe.0 || mystery.case.is_some() {
        return;
    }
    let Ok(player) = players.single() else { return };
    let at = Vec2::new(player.translation.x, player.translation.z);
    let last = match quest.current {
        Some((index, place)) => {
            if place.at.distance(at) > THERE {
                return;
            }
            quest.done += 1;
            let round_over = quest.done >= ROUND;
            done.write(ErrandDone {
                errand: index,
                done: quest.done,
                round_over,
            });
            if round_over {
                quest.done = 0;
                quest.rounds += 1;
            }
            Some(index)
        }
        None => None,
    };
    quest.current = next_errand(seed.0, quest.given, last, at);
    quest.given += 1;
}

/// The errand in the corner: what, how far, which way.
fn show(
    mut commands: Commands,
    font: Option<Res<GameFont>>,
    status: Res<crate::bridge::HostStatus>,
    quest: Res<Quest>,
    players: Query<(&Player, &Transform)>,
    mut labels: Query<&mut Text, With<QuestLabel>>,
    mut shown: Local<String>,
    mystery: Res<crate::mystery::Mystery>,
    mut panels: Query<&mut Visibility, With<QuestPanel>>,
) {
    // The notebook of the case takes the corner while there is one.
    let hidden = mystery.case.is_some();
    for mut visibility in &mut panels {
        let want = if hidden {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *visibility != want {
            *visibility = want;
        }
    }
    let Some(font) = font else { return };
    let Ok(mut label) = labels.single_mut() else {
        commands
            .spawn((
                crate::hud::HudRoot,
                QuestPanel,
                Node {
                    position_type: PositionType::Absolute,
                    right: Val::Px(20.0),
                    top: Val::Px(16.0),
                    padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)),
                    border_radius: BorderRadius::all(Val::Px(14.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.82)),
            ))
            .with_children(|panel| {
                panel.spawn((
                    Text::new(""),
                    TextFont {
                        font: font.0.clone().into(),
                        font_size: FontSize::Px(16.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.20, 0.22, 0.30)),
                    QuestLabel,
                ));
            });
        return;
    };
    let (Some(errand), Some((_, place)), Ok((player, transform))) =
        (quest.errand(), quest.current, players.single())
    else {
        return;
    };
    let at = Vec2::new(transform.translation.x, transform.translation.z);
    let what = if status.lang.starts_with("it") {
        errand.italian
    } else {
        errand.english
    };
    // To the nearest five metres: the text changes a few times a minute, not
    // sixty times a second.
    let metres = (place.at.distance(at) / 5.0).round() as i32 * 5;
    let text = format!(
        "{what}\n{} {metres} m   {}/{ROUND}",
        arrow(at, player.facing, place.at),
        quest.done
    );
    if *shown != text {
        **label = text.clone();
        *shown = text;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errands_come_from_the_seed_and_never_repeat_at_once() {
        let from = Vec2::new(11.0, 11.0);
        for seed in [1u64, 2026] {
            let mut last = None;
            let mut seen = std::collections::HashSet::new();
            for turn in 0..30 {
                let (index, place) = next_errand(seed, turn, last, from).expect("an errand");
                assert_eq!(next_errand(seed, turn, last, from), Some((index, place)));
                assert_ne!(Some(index), last);
                assert_eq!(place.kind, ERRANDS[index].at);
                seen.insert(index);
                last = Some(index);
            }
            assert!(seen.len() >= 5, "seed {seed}: only {seen:?}");
        }
    }

    #[test]
    fn the_arrow_points_the_way() {
        let here = Vec2::ZERO;
        // Facing `-z` (0): ahead is `-z`, right is `+x`.
        assert_eq!(arrow(here, 0.0, Vec2::new(0.0, -10.0)), '↑');
        assert_eq!(arrow(here, 0.0, Vec2::new(0.0, 10.0)), '↓');
        assert_eq!(arrow(here, 0.0, Vec2::new(10.0, 0.0)), '→');
        assert_eq!(arrow(here, 0.0, Vec2::new(-10.0, 0.0)), '←');
        // Turned to face `-x`, what was to the left is now ahead.
        assert_eq!(
            arrow(here, std::f32::consts::FRAC_PI_2, Vec2::new(-10.0, 0.0)),
            '↑'
        );
    }

    #[test]
    fn every_errand_has_its_words() {
        for errand in ERRANDS {
            assert!(
                !errand.italian.is_empty() && !errand.english.is_empty() && !errand.task.is_empty()
            );
        }
        let kinds: std::collections::HashSet<_> = ERRANDS.iter().map(|e| e.at).collect();
        assert_eq!(kinds.len(), PlaceKind::ALL.len());
    }
}
