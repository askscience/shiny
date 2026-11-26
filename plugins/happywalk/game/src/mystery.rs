//! `MysteryPlugin`: the case being played.
//!
//! The generator (`case.rs`) writes a case; this module puts it in the
//! town and runs it:
//!
//! 0. **Someone needs help.** Cases are met on the way: after a few errands
//!    (`ERRANDS_BEFORE_CALL`), someone waits on the pavement along the way to
//!    the next one (`caller_spot`), an exclamation mark over their head, and
//!    calls out when the character comes near. Errands go on meanwhile.
//! 1. **The case opens** when the character reaches them: they tell what
//!    happened (the model plays them); the notebook gets the crime and what
//!    they saw. Errands wait until it is over.
//! 2. **The scene.** A floating question mark marks where it happened;
//!    going there finds the trace.
//! 3. **The suspects** wait on the pavement by their usual place, a name
//!    over their heads. Near one, the player can ask a question (a quick one
//!    from the notebook, or anything by voice: the recogniser listens freely
//!    there). The model answers as that person, with the facts the case
//!    gives them: an innocent tells the truth, the culprit lies about where
//!    they were. Talking to someone fills the notebook with what shows (the
//!    traces they carry), where they say they were, and what they saw.
//! 4. **The accusation**, by voice ("è stato Leo", "accuso Leo") or from the
//!    notebook. Right: a confession and the next case, a level up every two
//!    solved. Wrong: the culprit gets away, and the character says so.
//!
//! The notebook itself is drawn by the page (`web/notebook.js`) from what
//! this module publishes (`hw_case`), and its buttons come back through the
//! bridge.

use std::collections::HashMap;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::brain::Said;
use crate::bridge::{BrainAnswered, BrainRequest, Heard, Host, HostStatus, Wardrobe};
use crate::camera::FollowCamera;
use crate::case::{self, role_in, Case, Clue};
use crate::city::{CityName, CitySeed, Solids, PAVED};
use crate::gen;
use crate::hud::{GameFont, OtherSaid};
use crate::intent::NotUnderstood;
use crate::nav::{self, REACH};
use crate::player::{Player, PlayerSet};
use crate::vox::VoxLibrary;

/// Within this many metres of a suspect, the character is talking to them.
const NEAR: f32 = 2.6;
/// Within this many metres of the scene, the trace is found.
const AT_SCENE: f32 = 3.0;
/// Name tags show within this distance.
const TAGS_WITHIN: f32 = 16.0;
/// Seconds to wait for the model before a stand-in line.
const PATIENCE: f32 = 4.0;
/// Errands done between one case and someone else needing help.
pub const ERRANDS_BEFORE_CALL: u32 = 2;
/// Someone who needs help waits this far along the way to the errand (or
/// most of the way, on a short walk): ahead, where the character will pass.
const CALLER_AHEAD: f32 = 30.0;
/// They call out when the character is this near...
const HEARD_WITHIN: f32 = 14.0;
/// ...and tell their story when it is this near.
const AT_CALLER: f32 = 2.6;
/// How high the exclamation mark floats: just over a head.
const ALERT_HEIGHT: f32 = 1.55;
/// From the middle of a street to a spot on the pavement beside it.
const KERBSIDE: f32 = (gen::PITCH - PAVED) / 2.0 + 0.9;

/// How far the player has come, kept by the page between games.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Progress {
    /// The next case's number.
    pub case: u32,
    pub solved: u32,
    pub failed: u32,
}

impl Progress {
    /// The level of the next case: one up every two cases solved.
    pub fn level(&self) -> u32 {
        (1 + self.solved / 2).min(8)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Phase {
    /// No case: errands.
    #[default]
    Waiting,
    /// Someone needs help, at `Mystery::caller`; errands go on.
    Calling,
    Investigating,
    Solved,
    Failed,
}

/// The case in play and what has been found out.
#[derive(Resource, Default)]
pub struct Mystery {
    pub case: Option<Case>,
    pub phase: Phase,
    pub progress: Progress,
    /// Where it happened, and where each suspect waits.
    pub scene: Vec2,
    pub spots: Vec<Vec2>,
    /// Clue indices (into the case's clues) found so far.
    pub found: Vec<bool>,
    /// Suspects talked to.
    pub talked: Vec<bool>,
    /// The notebook, line by line, in the player's language.
    pub notes: Vec<String>,
    /// Who was accused, once someone is.
    pub accused: Option<usize>,
    /// Errands done since the last case.
    pub errands: u32,
    /// Where whoever needs help waits, and whether they have called out.
    pub caller: Vec2,
    pub called: bool,
}

impl Mystery {
    /// A case is being solved, or its outcome is on show: errands wait.
    pub fn busy(&self) -> bool {
        matches!(
            self.phase,
            Phase::Investigating | Phase::Solved | Phase::Failed
        )
    }

    /// A short description for the character's brain.
    pub fn briefing(&self) -> String {
        match &self.case {
            Some(case) if self.phase == Phase::Investigating => {
                format!("solving a mystery: {}", case.crime.what)
            }
            _ => String::new(),
        }
    }
}

/// The suspect the character stands next to, if any: what is said then is
/// for them.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Talking(pub Option<usize>);

/// Asked from the page: a quick question, an accusation, the next case.
#[derive(Message, Clone, Debug)]
pub enum Detective {
    Ask { suspect: usize, question: usize },
    Accuse(usize),
    NextCase,
    Progress(Progress),
}

/// A question for a suspect, in words.
#[derive(Message, Clone, Debug)]
struct Question {
    suspect: usize,
    text: String,
}

/// The quick questions the notebook offers.
pub const QUESTIONS: [(&str, &str); 3] = [
    (
        "Where were you when it happened?",
        "Dov'eri quando è successo?",
    ),
    (
        "Did you see anything strange?",
        "Hai visto qualcosa di strano?",
    ),
    ("What is that on you?", "Cos'hai addosso?"),
];

#[derive(Component)]
struct Suspect(usize);

#[derive(Component)]
struct Client {
    left: f32,
}

#[derive(Component)]
struct ClueMarker;

/// The exclamation mark over whoever needs help.
#[derive(Component)]
struct Alert;

#[derive(Component)]
struct NameTag(usize);

/// What the character asked the model and is waiting for.
struct Waiting {
    asked_at: f32,
    /// Who says the answer: a suspect, the client, or the character.
    speaker: Speaker,
    stand_in: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Speaker {
    Suspect(usize),
    Client,
    Character,
}

#[derive(Resource, Default)]
struct Pending(HashMap<u32, Waiting>);

pub struct MysteryPlugin;

impl Plugin for MysteryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Mystery>()
            .init_resource::<Talking>()
            .init_resource::<Pending>()
            .add_message::<Detective>()
            .add_message::<Question>()
            .add_systems(
                Update,
                (call, open, notice, listen, ask, accuse, answers, publish)
                    .chain()
                    .after(PlayerSet),
            )
            .add_systems(Update, (idle, spin))
            .add_systems(PostUpdate, tags.after(TransformSystems::Propagate));
    }
}

fn italian(status: &HostStatus) -> bool {
    status.lang.starts_with("it")
}

/// A spot on the pavement by a place, clear of what stands there.
pub fn pavement_spot(solids: &Solids, place: Vec2) -> Vec2 {
    for direction in [Vec2::X, Vec2::NEG_X, Vec2::Y, Vec2::NEG_Y] {
        let kerb = place + direction * 2.3;
        if !on_pavement(kerb) {
            continue;
        }
        let along = direction.perp();
        for step in [0.0, 1.2, -1.2, 2.4, -2.4, 3.6, -3.6] {
            let p = kerb + along * step;
            if on_pavement(p) && solids.hit(p, 0.45).is_none() {
                return p;
            }
        }
    }
    place
}

/// Is a point on a lot's pavement (not the street)?
fn on_pavement(p: Vec2) -> bool {
    let off = (p - gen::centre(gen::chunk_of(p))).abs();
    off.x < PAVED / 2.0 - 0.3 && off.y < PAVED / 2.0 - 0.3
}

/// Where someone who needs help waits: on the pavement beside the way from
/// `from` along `points` (a route), `CALLER_AHEAD` metres on (most of the
/// way, on a short walk), clear of what stands there. The character walks
/// past it doing the errand.
pub fn caller_spot(solids: &Solids, from: Vec2, points: &[Vec2]) -> Vec2 {
    let mut want = CALLER_AHEAD.min(nav::length(from, points) * 0.7);
    let (mut at, mut point, mut way) = (from, from, Vec2::X);
    for p in points {
        let d = at.distance(*p);
        if d > 0.01 {
            way = (*p - at) / d;
            if want <= d {
                point = at + way * want;
                break;
            }
            want -= d;
        }
        at = *p;
        point = at;
    }
    // Out in the open if there is room: under a tree, the exclamation mark
    // over their head would be lost in the leaves.
    for open in [true, false] {
        for step in [0.0, 1.5, -1.5, 3.0, -3.0, 4.5, -4.5, 6.0, -6.0] {
            for side in [1.0, -1.0] {
                let spot = point + way * step + way.perp() * side * KERBSIDE;
                if on_pavement(spot)
                    && solids.hit(spot, 0.45).is_none()
                    && !(open && solids.any(spot, 2.2, gen::SolidKind::Tree))
                {
                    return spot;
                }
            }
        }
    }
    pavement_spot(solids, point)
}

/// The look of case `number`'s client.
fn client_look(seed: u64, number: u32) -> crate::figure::Look {
    crate::figure::Look::random(seed ^ (number as u64).wrapping_mul(0xA24B_AED4_963E_E407))
}

/// A person of the case (a suspect, the client) standing at `at`.
fn spawn_person(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    library: &VoxLibrary,
    look: &crate::figure::Look,
    at: Vec3,
) -> Entity {
    let voxel = look.voxel();
    let (body, leg) = (look.body(), look.leg());
    let size = leg.size.as_vec3() * voxel;
    let hip = size.y - 0.02;
    let piece = |mesh: Handle<Mesh>, offset: Vec3| {
        (
            Mesh3d(mesh),
            MeshMaterial3d(library.material.clone()),
            Transform::from_translation(offset).with_scale(Vec3::splat(voxel)),
        )
    };
    let (body, leg) = (meshes.add(body.mesh()), meshes.add(leg.mesh()));
    commands
        .spawn((Transform::from_translation(at), Visibility::default()))
        .with_children(|figure| {
            figure.spawn(piece(body, Vec3::new(0.0, hip, 3.0 * voxel)));
            for side in [-1.0, 1.0] {
                figure.spawn(piece(
                    leg.clone(),
                    Vec3::new(side * (size.x / 2.0 + 0.02), 0.0, -0.02),
                ));
            }
        })
        .id()
}

/// After a few errands, someone needs help: they wait along the way to the
/// next errand, and call out when the character comes near.
#[allow(clippy::too_many_arguments)]
fn call(
    mut commands: Commands,
    seed: Res<CitySeed>,
    city: Res<CityName>,
    status: Res<HostStatus>,
    wardrobe: Res<Wardrobe>,
    solids: Res<Solids>,
    library: Res<VoxLibrary>,
    quest: Res<crate::quest::Quest>,
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mystery: ResMut<Mystery>,
    mut host: ResMut<Host>,
    mut pending: ResMut<Pending>,
    mut errands: MessageReader<crate::quest::ErrandDone>,
    players: Query<&Transform, With<Player>>,
    // Whoever is calling, and what is left of the last case.
    callers: Query<
        Entity,
        Or<(
            With<Client>,
            With<Alert>,
            With<Suspect>,
            With<ClueMarker>,
            With<NameTag>,
        )>,
    >,
) {
    for _ in errands.read() {
        if mystery.phase == Phase::Waiting {
            mystery.errands += 1;
        }
    }
    // A new town, or the wardrobe: whoever was calling goes home.
    let new_town = seed.is_changed() && !seed.is_added();
    if new_town {
        mystery.errands = 0;
    }
    if mystery.phase == Phase::Calling && (new_town || wardrobe.0) {
        for entity in &callers {
            commands.entity(entity).despawn();
        }
        mystery.phase = Phase::Waiting;
        return;
    }
    // Back to the errands: the last case's people go home.
    if mystery.phase == Phase::Waiting && mystery.case.is_some() {
        for entity in &callers {
            commands.entity(entity).despawn();
        }
        mystery.case = None;
    }
    let Ok(player) = players.single() else { return };
    let at = Vec2::new(player.translation.x, player.translation.z);
    let it = italian(&status);
    match mystery.phase {
        Phase::Waiting => {
            if wardrobe.0 || status.lang.is_empty() || mystery.errands < ERRANDS_BEFORE_CALL {
                return;
            }
            let Some((_, place)) = quest.current else {
                return;
            };
            let spot = caller_spot(&solids, at, &nav::route(at, place.at));
            // The case they bring: the first that has its scene near them.
            let mut number = mystery.progress.case;
            while gen::nearest_place(
                seed.0,
                spot,
                case::case(seed.0, number, mystery.progress.level())
                    .crime
                    .at,
                REACH,
            )
            .is_none()
            {
                number += 1;
            }
            mystery.progress.case = number;
            for entity in &callers {
                commands.entity(entity).despawn();
            }
            let caller = spawn_person(
                &mut commands,
                &mut meshes,
                &library,
                &client_look(seed.0, number),
                Vec3::new(spot.x, gen::PAVEMENT, spot.y),
            );
            commands.entity(caller).insert(Client {
                left: f32::INFINITY,
            });
            commands.spawn((
                Alert,
                library.place("alert", Vec3::new(spot.x, ALERT_HEIGHT, spot.y), 0.08, 0),
                NotShadowCaster,
            ));
            mystery.caller = spot;
            mystery.called = false;
            mystery.errands = 0;
            mystery.phase = Phase::Calling;
            crate::bridge::host_log(&format!(
                "someone needs help at {:.0},{:.0}",
                spot.x, spot.y
            ));
        }
        Phase::Calling if !mystery.called && mystery.caller.distance(at) < HEARD_WITHIN => {
            mystery.called = true;
            let case = case::case(seed.0, mystery.progress.case, mystery.progress.level());
            let town = if city.0.is_empty() {
                "the town".to_string()
            } else {
                city.0.clone()
            };
            let persona = format!(
                "You are {name}, a resident of {town} in a funny video game, {manner}. Something has just happened to you ({what})                  and you spot a detective's assistant (a small voxel person) across the street. Call them over: one very short shouted line,                  in {lang_name}, without telling what happened yet. Never mention being a game character.",
                name = case.client.0,
                manner = case.client.1,
                what = case.crime.what,
                lang_name = if it { "Italian" } else { "English" },
            );
            let id = host.ask_brain(&BrainRequest {
                persona,
                situation: "Call the detective's assistant over.".into(),
                ..default()
            });
            pending.0.insert(
                id,
                Waiting {
                    asked_at: time.elapsed_secs(),
                    speaker: Speaker::Client,
                    stand_in: if it {
                        "Ehi! Aiuto, di qua!".into()
                    } else {
                        "Hey! Help, over here!".into()
                    },
                },
            );
        }
        _ => {}
    }
}

/// Who a suspect is, for the model: their facts and how to behave.
pub fn suspect_persona(case: &Case, s: usize, city: &str, italian: bool) -> String {
    let who = &case.suspects[s];
    let town = if city.is_empty() {
        "a small pastel town".to_string()
    } else {
        format!("the town of {city}")
    };
    let mut out = format!(
        "You are {name}, the {role} of {town} in a funny video game, {manner}. A detective's assistant (a small voxel person, \
         steered by a detective's voice) is questioning you about this: {what}.\n",
        name = who.name,
        role = who.role,
        manner = who.manner,
        what = case.crime.what,
    );
    let traces: Vec<&str> = who.traces.iter().map(|t| t.on_someone()).collect();
    if !traces.is_empty() {
        out.push_str(&format!(
            "You have {} (because of your job, or because you did it).\n",
            traces.join(" and ")
        ));
    }
    if let Some(alibi) = &who.alibi {
        out.push_str(&format!(
            "When it happened you say you were at {} with {}.\n",
            alibi.place.name(),
            case.suspects[alibi.with].name
        ));
    } else {
        // Small cases give nobody an alibi to check: everyone was at their
        // usual place (the culprit says so too, untrue). Never the scene.
        out.push_str(&format!(
            "When it happened you say you were at {}, as usual, alone. Never say you were at {}.\n",
            who.hangs_out.name(),
            case.crime.at.name()
        ));
    }
    if s == case.culprit {
        out.push_str(&format!(
            "You did it: you {}. Never admit it. Deny it, act offended or charming, change the subject. Your story about where you were is a lie, but stick to it.\n",
            case.motive
        ));
    } else {
        out.push_str(
            "You did not do it, and you tell the truth about where you were and with whom.\n",
        );
        let seen: Vec<String> = case
            .clues
            .iter()
            .filter_map(|c| match c {
                Clue::Sighting {
                    feature,
                    by: Some(b),
                } if *b == s => Some(feature.words()),
                _ => None,
            })
            .collect();
        if !seen.is_empty() {
            out.push_str(&format!(
                "You glimpsed the culprit: they {}. Say so if asked what you saw.\n",
                seen.join(" and ")
            ));
        }
        // Who they were really with, so they can confirm it.
        if let Some(alibi) = &who.alibi {
            out.push_str(&format!(
                "If asked about {}, you confirm you were together at {}.\n",
                case.suspects[alibi.with].name,
                alibi.place.name()
            ));
        }
        out.push_str(&format!("You have a little embarrassing secret, nothing to do with this: you {}. You might let it slip.\n", who.secret));
    }
    out.push_str(&format!(
        "Answer what you are asked, in character, in one or two short spoken lines, in {}. Never mention being a game character.",
        if italian { "Italian" } else { "English" }
    ));
    out
}

/// The notebook's lines about a suspect, once talked to.
pub fn suspect_notes(case: &Case, s: usize, lang: &str) -> Vec<String> {
    let it = lang.starts_with("it");
    let who = &case.suspects[s];
    let mut out = Vec::new();
    let traces: Vec<&str> = who.traces.iter().map(|t| t.on_someone_in(lang)).collect();
    out.push(if traces.is_empty() {
        if it {
            format!("{}: niente addosso", who.name)
        } else {
            format!("{}: nothing on them", who.name)
        }
    } else if it {
        format!("{}: ha {}", who.name, traces.join(" e "))
    } else {
        format!("{} has {}", who.name, traces.join(" and "))
    });
    if let Some(alibi) = &who.alibi {
        let with = case.suspects[alibi.with].name;
        out.push(if it {
            format!(
                "{} dice di essere stato a {} con {}",
                who.name,
                alibi
                    .place
                    .name_in(lang)
                    .trim_start_matches("il ")
                    .trim_start_matches("la ")
                    .trim_start_matches("l'"),
                with
            )
        } else {
            format!(
                "{} says they were at {} with {}",
                who.name,
                alibi.place.name(),
                with
            )
        });
    }
    for clue in &case.clues {
        if let Clue::Sighting {
            feature,
            by: Some(b),
        } = clue
        {
            if *b == s {
                out.push(if it {
                    format!(
                        "{} ha visto che il colpevole {}",
                        who.name,
                        feature.words_in(lang)
                    )
                } else {
                    format!("{} saw that the culprit {}", who.name, feature.words())
                });
            }
        }
    }
    out
}

/// Words the recogniser should know while a case is on, besides names.
const ACCUSING: [&str; 6] = [
    "accuso",
    "è stato",
    "è stata",
    "colpevole",
    "accuse",
    "it was",
];

/// Is `text` an accusation, and of whom?
pub fn accusation(case: &Case, text: &str) -> Option<usize> {
    let text = text.to_lowercase();
    let accusing = [
        "accus",
        "colpevole",
        "è stato",
        "e stato",
        "è stata",
        "arrest",
        "it was",
        "culprit",
        "guilty",
    ]
    .iter()
    .any(|w| text.contains(w));
    if !accusing {
        return None;
    }
    let words: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    case.suspects
        .iter()
        .position(|s| words.contains(&s.name.to_lowercase().as_str()))
}

/// Open the case when the character reaches whoever needs help.
#[allow(clippy::too_many_arguments)]
fn open(
    mut commands: Commands,
    seed: Res<CitySeed>,
    city: Res<CityName>,
    status: Res<HostStatus>,
    wardrobe: Res<Wardrobe>,
    solids: Res<Solids>,
    library: Res<VoxLibrary>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mystery: ResMut<Mystery>,
    mut host: ResMut<Host>,
    mut pending: ResMut<Pending>,
    mut requests: MessageReader<Detective>,
    mut words: ResMut<crate::intent::ExtraWords>,
    players: Query<(&Player, &Transform)>,
    mut people: Query<
        (Entity, Option<&mut Client>),
        Or<(
            With<Suspect>,
            With<ClueMarker>,
            With<NameTag>,
            With<Alert>,
            With<Client>,
        )>,
    >,
    time: Res<Time>,
) {
    for request in requests.read() {
        match request {
            Detective::Progress(progress) => mystery.progress = *progress,
            Detective::NextCase if matches!(mystery.phase, Phase::Solved | Phase::Failed) => {
                mystery.phase = Phase::Waiting
            }
            _ => {}
        }
    }
    // A new town is a new case, from the top.
    if seed.is_changed() && !seed.is_added() && mystery.case.is_some() {
        mystery.phase = Phase::Waiting;
        for (entity, _) in &people {
            commands.entity(entity).despawn();
        }
    }
    let Ok((player, transform)) = players.single() else {
        return;
    };
    let at = Vec2::new(transform.translation.x, transform.translation.z);
    if mystery.phase != Phase::Calling
        || mystery.caller.distance(at) > AT_CALLER
        || wardrobe.0
        || status.lang.is_empty()
    {
        return;
    }
    // Whoever called stays a while to see how it goes; the rest of the last
    // case goes.
    let mut caller_here = false;
    for (entity, client) in &mut people {
        match client {
            Some(mut client) => {
                client.left = 30.0;
                caller_here = true;
            }
            None => commands.entity(entity).despawn(),
        }
    }
    // The case: its scene and its people placed in this town.
    let mut number = mystery.progress.case;
    let (case, scene) = loop {
        let case = case::case(seed.0, number, mystery.progress.level());
        if let Some(place) = gen::nearest_place(seed.0, at, case.crime.at, REACH) {
            break (case, place.at);
        }
        number += 1;
    };
    mystery.progress.case = number;
    let spots: Vec<Vec2> = case
        .suspects
        .iter()
        .map(|s| {
            gen::nearest_place(seed.0, scene, s.hangs_out, REACH)
                .map(|p| pavement_spot(&solids, p.at))
                .unwrap_or(scene + Vec2::splat(4.0))
        })
        .collect();
    let marker = pavement_spot(&solids, scene);
    commands.spawn((
        ClueMarker,
        library.place("clue", Vec3::new(marker.x, 1.2, marker.y), 0.06, 0),
        NotShadowCaster,
    ));
    for (s, (suspect, spot)) in case.suspects.iter().zip(&spots).enumerate() {
        let who = spawn_person(
            &mut commands,
            &mut meshes,
            &library,
            &suspect.look,
            Vec3::new(spot.x, gen::PAVEMENT, spot.y),
        );
        commands.entity(who).insert(Suspect(s));
        commands.spawn((
            NameTag(s),
            Text::new(suspect.name),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            Visibility::Hidden,
        ));
    }
    // If they are gone, someone comes up to the character instead.
    if !caller_here {
        let ahead = Vec2::new(-player.facing.sin(), -player.facing.cos());
        // In front and to one side, so the two speech bubbles do not cover
        // each other.
        let client_at = at + ahead * 1.6 + ahead.perp() * 1.3;
        let client = spawn_person(
            &mut commands,
            &mut meshes,
            &library,
            &client_look(seed.0, number),
            Vec3::new(client_at.x, 0.0, client_at.y),
        );
        commands.entity(client).insert(Client { left: 30.0 });
    }
    let it = italian(&status);
    let lang = status.lang.as_str();
    let client_sightings: Vec<String> = case
        .clues
        .iter()
        .filter_map(|c| match c {
            Clue::Sighting { feature, by: None } => Some(*feature),
            _ => None,
        })
        .map(|f| f.words())
        .collect();
    mystery.notes = vec![if it {
        format!(
            "Caso {}, livello {}: {}.",
            number + 1,
            case.level,
            case.crime.what_in(lang)
        )
    } else {
        format!(
            "Case {}, level {}: {}.",
            number + 1,
            case.level,
            case.crime.what_in(lang)
        )
    }];
    mystery.notes.push(if it {
        format!(
            "Scena: {}. Indiziati: {}.",
            case.crime.at.name_in(lang),
            case.suspects.len()
        )
    } else {
        format!(
            "Scene: {}. Suspects: {}.",
            case.crime.at.name(),
            case.suspects.len()
        )
    });
    mystery.found = vec![false; case.clues.len()];
    for (i, clue) in case.clues.iter().enumerate() {
        if let Clue::Sighting { feature, by: None } = clue {
            mystery.found[i] = true;
            mystery.notes.push(if it {
                format!(
                    "{} ha visto che il colpevole {}",
                    case.client.0,
                    feature.words_in(lang)
                )
            } else {
                format!("{} saw that the culprit {}", case.client.0, feature.words())
            });
        }
    }
    let town = if city.0.is_empty() {
        "the town".to_string()
    } else {
        city.0.clone()
    };
    let persona = format!(
        "You are {name}, a resident of {town} in a funny video game, {manner}. You run up to a detective's assistant (a small voxel person) \
         because {what}. {seen}Tell them, upset and a little theatrical, in two short spoken lines, in {lang_name}. Never mention being a game character.",
        name = case.client.0,
        manner = case.client.1,
        what = case.crime.what,
        seen = if client_sightings.is_empty() { String::new() } else { format!("You glimpsed whoever did it: they {}. ", client_sightings.join(" and ")) },
        lang_name = if it { "Italian" } else { "English" },
    );
    let stand_in = if it {
        format!("Aiuto! {}!", capitalise(case.crime.what_in(lang)))
    } else {
        format!("Help! {}!", capitalise(case.crime.what))
    };
    let id = host.ask_brain(&BrainRequest {
        persona,
        situation: "Tell them what happened.".into(),
        ..default()
    });
    pending.0.insert(
        id,
        Waiting {
            asked_at: time.elapsed_secs(),
            speaker: Speaker::Client,
            stand_in,
        },
    );

    words.0 = case
        .suspects
        .iter()
        .map(|s| s.name.to_lowercase())
        .chain(ACCUSING.iter().map(|w| w.to_string()))
        .collect();
    mystery.talked = vec![false; case.suspects.len()];
    mystery.scene = scene;
    mystery.spots = spots;
    mystery.accused = None;
    mystery.case = Some(case);
    mystery.phase = Phase::Investigating;
    crate::bridge::host_log(&format!(
        "case {} opened at level {}",
        number + 1,
        mystery.progress.level()
    ));
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// The scene, and who is near.
fn notice(
    status: Res<HostStatus>,
    time: Res<Time>,
    players: Query<&Transform, With<Player>>,
    mut mystery: ResMut<Mystery>,
    mut talking: ResMut<Talking>,
    mut host: ResMut<Host>,
    mut pending: ResMut<Pending>,
) {
    let Ok(player) = players.single() else { return };
    let at = Vec2::new(player.translation.x, player.translation.z);
    let near = if mystery.phase == Phase::Investigating {
        mystery
            .spots
            .iter()
            .enumerate()
            .map(|(s, p)| (s, p.distance(at)))
            .filter(|(_, d)| *d < NEAR)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(s, _)| s)
    } else {
        None
    };
    if talking.0 != near {
        talking.0 = near;
    }
    if mystery.phase != Phase::Investigating || mystery.scene.distance(at) > AT_SCENE {
        return;
    }
    let Some(case) = mystery.case.clone() else {
        return;
    };
    let Some(index) = case.clues.iter().position(|c| matches!(c, Clue::Trace(_))) else {
        return;
    };
    if mystery.found[index] {
        return;
    }
    mystery.found[index] = true;
    let lang = status.lang.as_str();
    let it = italian(&status);
    let trace = case.crime.trace;
    mystery.notes.push(if it {
        format!("Sulla scena: {}.", trace.at_scene_in(lang))
    } else {
        format!("At the scene: {}.", trace.at_scene())
    });
    let id = host.ask_brain(&BrainRequest {
        situation: format!(
            "You are investigating a case with the detective (the voice): {}. At the scene you just found {}. Say your one line about this clue.",
            case.crime.what,
            trace.at_scene()
        ),
        ..default()
    });
    let stand_in = if it {
        format!("Guarda qui: {}. Interessante.", trace.at_scene_in(lang))
    } else {
        format!("Look: {}. Interesting.", trace.at_scene())
    };
    pending.0.insert(
        id,
        Waiting {
            asked_at: time.elapsed_secs(),
            speaker: Speaker::Character,
            stand_in,
        },
    );
}

/// What is said: a question for the suspect nearby, or an accusation.
fn listen(
    mut heard: MessageReader<Heard>,
    mut lost: MessageReader<NotUnderstood>,
    talking: Res<Talking>,
    mystery: Res<Mystery>,
    status: Res<HostStatus>,
    mut requests: MessageWriter<Detective>,
    mut questions: MessageWriter<Question>,
) {
    let (Some(case), Phase::Investigating) = (&mystery.case, mystery.phase) else {
        heard.clear();
        lost.clear();
        return;
    };
    for h in heard.read() {
        if h.is_final {
            if let Some(suspect) = accusation(case, &h.text) {
                requests.write(Detective::Accuse(suspect));
            }
        }
    }
    for sentence in lost.read() {
        if let (Some(suspect), None) = (talking.0, accusation(case, &sentence.text)) {
            questions.write(Question {
                suspect,
                text: sentence.text.clone(),
            });
        }
    }
    // Quick questions from the notebook, to whoever is next to the character.
    let _ = &status;
}

/// A question to a suspect: the notebook fills in, and the model answers as
/// them.
#[allow(clippy::too_many_arguments)]
fn ask(
    status: Res<HostStatus>,
    city: Res<CityName>,
    time: Res<Time>,
    mut requests: MessageReader<Detective>,
    mut questions: MessageReader<Question>,
    mut mystery: ResMut<Mystery>,
    mut host: ResMut<Host>,
    mut pending: ResMut<Pending>,
    talking: Res<Talking>,
) {
    let mut asked: Vec<(usize, String)> = questions
        .read()
        .map(|q| (q.suspect, q.text.clone()))
        .collect();
    let it = italian(&status);
    for request in requests.read() {
        if let Detective::Ask { suspect, question } = request {
            // Only someone the character is standing next to.
            if talking.0 == Some(*suspect) {
                let (en, itq) = QUESTIONS[(*question).min(QUESTIONS.len() - 1)];
                asked.push((*suspect, if it { itq.to_string() } else { en.to_string() }));
            }
        }
    }
    if mystery.phase != Phase::Investigating {
        return;
    }
    let Some(case) = mystery.case.clone() else {
        return;
    };
    for (s, question) in asked {
        if s >= case.suspects.len() {
            continue;
        }
        if !mystery.talked[s] {
            mystery.talked[s] = true;
            let notes = suspect_notes(&case, s, &status.lang);
            mystery.notes.extend(notes);
            // What they confirm or deny counts as checking an alibi.
            for (i, clue) in case.clues.iter().enumerate() {
                if matches!(clue, Clue::AlibiCheck { by, .. } if *by == s)
                    || matches!(clue, Clue::Sighting { by: Some(b), .. } if *b == s)
                {
                    mystery.found[i] = true;
                }
            }
        }
        let persona = suspect_persona(&case, s, &city.0, it);
        let id = host.ask_brain(&BrainRequest {
            persona,
            situation: format!("The assistant asks you: \"{question}\""),
            ..default()
        });
        let stand_in = if it {
            "Hmm. Non so niente, io.".to_string()
        } else {
            "Hmm. I know nothing.".to_string()
        };
        pending.0.insert(
            id,
            Waiting {
                asked_at: time.elapsed_secs(),
                speaker: Speaker::Suspect(s),
                stand_in,
            },
        );
    }
}

/// An accusation: right or wrong, the case closes.
fn accuse(
    status: Res<HostStatus>,
    time: Res<Time>,
    mut requests: MessageReader<Detective>,
    mut mystery: ResMut<Mystery>,
    mut host: ResMut<Host>,
    mut pending: ResMut<Pending>,
    city: Res<CityName>,
) {
    let mut accused = None;
    for request in requests.read() {
        if let Detective::Accuse(s) = request {
            accused = Some(*s);
        }
    }
    let Some(s) = accused else { return };
    if mystery.phase != Phase::Investigating {
        return;
    }
    let Some(case) = mystery.case.clone() else {
        return;
    };
    if s >= case.suspects.len() {
        return;
    }
    let it = italian(&status);
    let lang = status.lang.as_str();
    let culprit = &case.suspects[case.culprit];
    let right = s == case.culprit;
    mystery.accused = Some(s);
    mystery.phase = if right { Phase::Solved } else { Phase::Failed };
    if right {
        mystery.progress.solved += 1;
    } else {
        mystery.progress.failed += 1;
    }
    mystery.progress.case += 1;
    mystery.notes.push(match (right, it) {
        (true, true) => format!(
            "Risolto: è stato {}, {}.",
            culprit.name,
            role_in(culprit.role, lang)
        ),
        (true, false) => format!("Solved: it was {} the {}.", culprit.name, culprit.role),
        (false, true) => format!(
            "Sbagliato: {} era innocente. È stato {}, che è scappato.",
            case.suspects[s].name, culprit.name
        ),
        (false, false) => format!(
            "Wrong: {} was innocent. It was {}, who got away.",
            case.suspects[s].name, culprit.name
        ),
    });
    crate::bridge::host_log(&format!(
        "case {} {}",
        case.number + 1,
        if right { "solved" } else { "failed" }
    ));
    let now = time.elapsed_secs();
    if right {
        // The culprit confesses, then the character gloats.
        let persona = format!(
            "{}\nYou have just been caught. Confess at last, theatrically, with your reason: you {}. Two short spoken lines, in {}.",
            suspect_persona(&case, case.culprit, &city.0, it),
            case.motive,
            if it { "Italian" } else { "English" }
        );
        let id = host.ask_brain(&BrainRequest {
            persona,
            situation: "The detective has unmasked you.".into(),
            ..default()
        });
        let stand_in = if it {
            "Va bene, va bene! Sono stato io!".into()
        } else {
            "All right, all right! It was me!".into()
        };
        pending.0.insert(
            id,
            Waiting {
                asked_at: now,
                speaker: Speaker::Suspect(case.culprit),
                stand_in,
            },
        );
    }
    let situation = if right {
        format!("You and the detective just solved the case ({}): it was {} the {}, who {}. Say your one triumphant line.", case.crime.what, culprit.name, culprit.role, case.motive)
    } else {
        format!(
            "The detective accused {} the {} of this ({}), but they were innocent, and the real culprit, {}, got away. Say your one line about it.",
            case.suspects[s].name, case.suspects[s].role, case.crime.what, culprit.name
        )
    };
    let id = host.ask_brain(&BrainRequest {
        situation,
        ..default()
    });
    let stand_in = match (right, it) {
        (true, true) => "Caso chiuso. Siamo bravissimi, eh.".into(),
        (true, false) => "Case closed. We are brilliant.".into(),
        (false, true) => "Complimenti. Il colpevole ringrazia.".into(),
        (false, false) => "Well done. The culprit thanks you.".into(),
    };
    pending.0.insert(
        id,
        Waiting {
            asked_at: now + 0.8,
            speaker: Speaker::Character,
            stand_in,
        },
    );
}

/// The model's answers (or stand-ins, when late), said by whoever speaks.
#[allow(clippy::too_many_arguments)]
fn answers(
    time: Res<Time>,
    mut answered: MessageReader<BrainAnswered>,
    mut pending: ResMut<Pending>,
    mut host: ResMut<Host>,
    mut said: MessageWriter<Said>,
    mut others: MessageWriter<OtherSaid>,
    suspects: Query<(&Suspect, &Transform)>,
    clients: Query<&Transform, With<Client>>,
) {
    let now = time.elapsed_secs();
    let mut speak = |speaker: Speaker, line: String, host: &mut Host| {
        if line.is_empty() {
            return;
        }
        host.say(&line);
        let at = match speaker {
            Speaker::Character => {
                said.write(Said { text: line });
                return;
            }
            Speaker::Client => clients.iter().next().map(|t| t.translation),
            Speaker::Suspect(s) => suspects
                .iter()
                .find(|(x, _)| x.0 == s)
                .map(|(_, t)| t.translation),
        };
        if let Some(at) = at {
            others.write(OtherSaid {
                text: line,
                at: at + Vec3::Y * 1.8,
            });
        }
    };
    for reply in answered.read() {
        let Some(waiting) = pending.0.remove(&reply.id) else {
            continue;
        };
        let line = reply
            .answer
            .as_ref()
            .map(|a| a.say.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or(waiting.stand_in);
        speak(waiting.speaker, line, &mut host);
    }
    let late: Vec<u32> = pending
        .0
        .iter()
        .filter(|(_, w)| now - w.asked_at > PATIENCE)
        .map(|(id, _)| *id)
        .collect();
    for id in late {
        if let Some(waiting) = pending.0.remove(&id) {
            speak(waiting.speaker, waiting.stand_in, &mut host);
        }
    }
}

/// What the page's notebook shows.
#[derive(Serialize)]
struct Notebook<'a> {
    phase: &'static str,
    case: u32,
    level: u32,
    notes: &'a [String],
    suspects: Vec<NotebookSuspect>,
    near: Option<usize>,
    progress: Progress,
    questions: Vec<&'static str>,
    /// Where the scene is (x, z), for the page to point the way.
    scene: [f32; 2],
    /// Where whoever needs help waits, while they do; and where the errand
    /// in hand is done.
    caller: Option<[f32; 2]>,
    errand: Option<[f32; 2]>,
}

#[derive(Serialize)]
struct NotebookSuspect {
    name: &'static str,
    role: &'static str,
    place: &'static str,
    talked: bool,
    distance: u32,
    /// Where they wait (x, z).
    at: [f32; 2],
}

fn publish(
    status: Res<HostStatus>,
    mystery: Res<Mystery>,
    talking: Res<Talking>,
    quest: Res<crate::quest::Quest>,
    players: Query<&Transform, With<Player>>,
    mut last: Local<String>,
) {
    let at = players
        .single()
        .map(|t| Vec2::new(t.translation.x, t.translation.z))
        .unwrap_or_default();
    let lang = status.lang.as_str();
    let it = italian(&status);
    let suspects = mystery
        .case
        .as_ref()
        .map(|case| {
            case.suspects
                .iter()
                .enumerate()
                .map(|(s, who)| NotebookSuspect {
                    name: who.name,
                    role: role_in(who.role, lang),
                    place: who.hangs_out.name_in(lang),
                    talked: mystery.talked.get(s).copied().unwrap_or(false),
                    at: mystery
                        .spots
                        .get(s)
                        .map(|p| [p.x.round(), p.y.round()])
                        .unwrap_or_default(),
                    // To the nearest five metres: the page redraws when it changes.
                    distance: mystery
                        .spots
                        .get(s)
                        .map(|p| ((p.distance(at) / 5.0).round() * 5.0) as u32)
                        .unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default();
    let notebook = Notebook {
        phase: match mystery.phase {
            Phase::Waiting => "waiting",
            Phase::Calling => "calling",
            Phase::Investigating => "investigating",
            Phase::Solved => "solved",
            Phase::Failed => "failed",
        },
        case: mystery.progress.case
            + if mystery.phase == Phase::Investigating {
                1
            } else {
                0
            },
        level: mystery.case.as_ref().map(|c| c.level).unwrap_or(1),
        notes: &mystery.notes,
        suspects,
        near: talking.0,
        progress: mystery.progress,
        questions: QUESTIONS
            .iter()
            .map(|(en, itq)| if it { *itq } else { *en })
            .collect(),
        scene: [mystery.scene.x.round(), mystery.scene.y.round()],
        caller: (mystery.phase == Phase::Calling)
            .then(|| [mystery.caller.x.round(), mystery.caller.y.round()]),
        errand: quest
            .current
            .map(|(_, place)| [place.at.x.round(), place.at.y.round()]),
    };
    let json = serde_json::to_string(&notebook).unwrap_or_default();
    if *last != json {
        crate::bridge::publish_case(json.clone());
        *last = json;
    }
}

/// Suspects stand by their place, look at the character when it is near;
/// the client hangs about a while, then goes.
fn idle(
    mut commands: Commands,
    time: Res<Time>,
    talking: Res<Talking>,
    players: Query<&Transform, (With<Player>, Without<Suspect>, Without<Client>)>,
    mut suspects: Query<(&Suspect, &mut Transform), Without<Client>>,
    mut clients: Query<(Entity, &mut Client, &mut Transform), Without<Suspect>>,
) {
    let Ok(player) = players.single() else { return };
    let t = time.elapsed_secs();
    let face = |from: Vec3, to: Vec3| {
        let d = to - from;
        Quat::from_rotation_y((-d.x).atan2(-d.z))
    };
    for (suspect, mut transform) in &mut suspects {
        let near = talking.0 == Some(suspect.0)
            || transform.translation.distance(player.translation) < 6.0;
        if near {
            let want = face(transform.translation, player.translation);
            transform.rotation = transform
                .rotation
                .slerp(want, (time.delta_secs() * 6.0).min(1.0));
        } else {
            // A little restless: turning this way and that.
            transform.rotation =
                Quat::from_rotation_y((t * 0.4 + suspect.0 as f32 * 1.7).sin() * 0.8);
        }
    }
    for (entity, mut client, mut transform) in &mut clients {
        client.left -= time.delta_secs();
        if client.left <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let want = face(transform.translation, player.translation);
        transform.rotation = transform
            .rotation
            .slerp(want, (time.delta_secs() * 6.0).min(1.0));
    }
}

/// The scene's question mark turns and bobs; the exclamation mark over
/// whoever needs help jumps.
fn spin(
    time: Res<Time>,
    mut markers: Query<&mut Transform, (With<ClueMarker>, Without<Alert>)>,
    mut alerts: Query<&mut Transform, (With<Alert>, Without<FollowCamera>)>,
    cameras: Query<&Transform, (With<FollowCamera>, Without<Alert>, Without<ClueMarker>)>,
) {
    let t = time.elapsed_secs();
    for mut marker in &mut markers {
        marker.rotation = Quat::from_rotation_y(t * 1.5);
        marker.translation.y = 1.2 + (t * 2.0).sin() * 0.12;
    }
    let view = cameras.single().map(|c| c.translation).ok();
    for mut alert in &mut alerts {
        // Always facing the camera, so it reads from any side.
        if let Some(view) = view {
            let d = view - alert.translation;
            // (The glyph's face looks along the model's x.)
            alert.rotation = Quat::from_rotation_y(d.x.atan2(d.z) - std::f32::consts::FRAC_PI_2);
        }
        alert.translation.y = ALERT_HEIGHT + (t * 5.0).sin().abs() * 0.2;
    }
}

/// Names over the suspects' heads, when they are near enough to matter.
fn tags(
    font: Option<Res<GameFont>>,
    mystery: Res<Mystery>,
    talking: Res<Talking>,
    suspects: Query<(&Suspect, &GlobalTransform)>,
    cameras: Query<(&Camera, &GlobalTransform), With<FollowCamera>>,
    players: Query<&GlobalTransform, With<Player>>,
    mut labels: Query<(
        &NameTag,
        &mut Node,
        &mut Visibility,
        &mut TextFont,
        &mut TextColor,
    )>,
) {
    let (Ok((camera, view)), Ok(player)) = (cameras.single(), players.single()) else {
        return;
    };
    for (tag, mut node, mut visibility, mut text_font, mut colour) in &mut labels {
        let Some((_, at)) = suspects.iter().find(|(s, _)| s.0 == tag.0) else {
            continue;
        };
        let close = at.translation().distance(player.translation()) < TAGS_WITHIN;
        // Not over whoever the character is talking to: their speech bubble
        // is there, and the notebook names them.
        let shown = close && mystery.phase == Phase::Investigating && talking.0 != Some(tag.0);
        match camera.world_to_viewport(view, at.translation() + Vec3::Y * 1.75) {
            Ok(screen) if shown => {
                *visibility = Visibility::Visible;
                node.left = Val::Px(screen.x - 30.0);
                node.top = Val::Px(screen.y - 24.0);
            }
            _ => *visibility = Visibility::Hidden,
        }
        if let Some(font) = &font {
            if text_font.font_size != FontSize::Px(15.0) {
                *text_font = TextFont {
                    font: font.0.clone().into(),
                    font_size: FontSize::Px(15.0),
                    ..default()
                };
                colour.0 = Color::srgb(0.2, 0.22, 0.3);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city::seed_for;
    use crate::gen::PlaceKind;

    fn a_case(level: u32) -> Case {
        case::case(seed_for("Borgo Pastello"), 3, level)
    }

    #[test]
    fn accusations_are_understood() {
        let case = a_case(3);
        let name = case.suspects[1].name;
        assert_eq!(accusation(&case, &format!("è stato {name}!")), Some(1));
        assert_eq!(
            accusation(&case, &format!("Accuso {}", name.to_lowercase())),
            Some(1)
        );
        assert_eq!(accusation(&case, &format!("it was {name}")), Some(1));
        // Naming someone is not accusing them.
        assert_eq!(accusation(&case, &format!("ciao {name}, dove eri?")), None);
        assert_eq!(accusation(&case, "è stato il maggiordomo"), None);
    }

    #[test]
    fn a_suspect_knows_their_part() {
        let case = a_case(4);
        let culprit = suspect_persona(&case, case.culprit, "Borgo Pastello", true);
        assert!(culprit.contains("You did it"));
        assert!(culprit.contains(case.motive));
        assert!(culprit.contains("Italian"));
        assert!(culprit.contains("Borgo Pastello"));
        let innocent = (0..case.suspects.len())
            .find(|s| *s != case.culprit)
            .unwrap();
        let persona = suspect_persona(&case, innocent, "", false);
        assert!(persona.contains("You did not do it"));
        assert!(!persona.contains(case.motive));
        assert!(persona.contains(case.suspects[innocent].secret));
    }

    #[test]
    fn the_notebook_tells_what_talking_reveals() {
        let case = a_case(4);
        for s in 0..case.suspects.len() {
            let notes = suspect_notes(&case, s, "it");
            assert!(notes[0].starts_with(case.suspects[s].name));
            if case.suspects[s].alibi.is_some() {
                assert!(notes.iter().any(|n| n.contains("dice di essere stato")));
            }
        }
        // Every clue can be found: the trace at the scene, the client's
        // sightings at the start, the rest by talking to someone.
        for clue in &case.clues {
            match clue {
                Clue::Sighting { by: Some(b), .. } => {
                    assert!(suspect_notes(&case, *b, "en")
                        .iter()
                        .any(|n| n.contains("saw that the culprit")));
                }
                Clue::AlibiCheck { of, by } => {
                    assert!(case.suspects[*of]
                        .alibi
                        .as_ref()
                        .is_some_and(|a| a.with == *by));
                }
                _ => {}
            }
        }
    }

    #[test]
    fn suspects_wait_on_the_pavement() {
        let seed = seed_for("Borgo Pastello");
        let solids = Solids::new(seed);
        for kind in PlaceKind::ALL {
            let place = gen::nearest_place(seed, Vec2::new(11.0, 11.0), kind, REACH).unwrap();
            let spot = pavement_spot(&solids, place.at);
            assert_eq!(solids.hit(spot, 0.4), None, "{kind:?}");
            assert!(spot.distance(place.at) < 6.0, "{kind:?}");
        }
    }

    #[test]
    fn whoever_needs_help_waits_on_the_way() {
        for town in ["Borgo Pastello", "Valfiorita", "Ponte Lungo"] {
            let seed = seed_for(town);
            let solids = Solids::new(seed);
            let from = Vec2::new(11.0, 3.0);
            for kind in PlaceKind::ALL {
                let place = gen::nearest_place(seed, from, kind, REACH).unwrap();
                let points = nav::route(from, place.at);
                let spot = caller_spot(&solids, from, &points);
                assert_eq!(solids.hit(spot, 0.4), None, "{town} {kind:?}");
                // Beside the way, not off somewhere else...
                let mut nearest = f32::MAX;
                let mut at = from;
                for p in &points {
                    for k in 0..=20 {
                        nearest = nearest.min(spot.distance(at.lerp(*p, k as f32 / 20.0)));
                    }
                    at = *p;
                }
                assert!(nearest < KERBSIDE + 5.0, "{town} {kind:?}: {nearest}");
                // ...and ahead: a walk to reach, unless the errand is near.
                let walk = nav::length(from, &points);
                if walk > 20.0 {
                    assert!(spot.distance(from) > 8.0, "{town} {kind:?}");
                }
            }
        }
    }

    #[test]
    fn levels_follow_the_cases_solved() {
        assert_eq!(Progress::default().level(), 1);
        assert_eq!(
            Progress {
                solved: 3,
                ..default()
            }
            .level(),
            2
        );
        assert_eq!(
            Progress {
                solved: 100,
                ..default()
            }
            .level(),
            8
        );
    }

    #[test]
    fn the_quick_questions_are_in_both_languages() {
        for (en, it) in QUESTIONS {
            assert!(en.ends_with('?') && it.ends_with('?'));
        }
    }
}
