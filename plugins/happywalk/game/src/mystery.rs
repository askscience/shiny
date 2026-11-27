//! `MysteryPlugin`: the case being played.
//!
//! The generator (`case.rs`) writes a case; this module puts it in the
//! town and runs it:
//!
//! 1. **The office.** The character starts the day in the detective's
//!    office (`rooms.rs`). While no case is open, a client comes in and
//!    tells what happened: they are played by the language model, and the
//!    player can question them in the chat.
//! 2. **The people.** Every suspect has a place of their own: the shop where
//!    they work, or their home ([`Mystery::spaces`]). The notebook says
//!    where, how far, and has a button to walk there; their door opens and
//!    they are inside.
//! 3. **The chat.** Next to someone, the notebook becomes a conversation:
//!    the player writes (or speaks, and the page writes it down) and the
//!    model answers as that person, from the facts the case gives them and
//!    everything said so far: an innocent tells the truth, the culprit lies
//!    about where they were. After the first answer the notebook notes what
//!    shows (the traces they carry), where they say they were and what they
//!    saw. The scene, outside, is marked by a question mark; going there
//!    finds the trace.
//! 4. **The accusation**, with the notebook's button. Right: the culprit
//!    confesses. Wrong: they get away. Back at the office, the next client.
//!
//! Nobody else speaks: the character has no lines of its own.

use std::collections::HashMap;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::bridge::{BrainAnswered, BrainRequest, Host, HostStatus, Turn, Wardrobe};
use crate::camera::FollowCamera;
use crate::case::{self, role_in, Case, Clue};
use crate::city::{CityName, CitySeed, Solids, PAVED};
use crate::gen::{self, PlaceKind};
use crate::hud::GameFont;
use crate::nav::REACH;
use crate::player::{Player, PlayerSet};
use crate::rooms::{self, Doors, GoIn, Inside, Passage, RoomKind, Space};
use crate::vox::VoxLibrary;

/// Within this many metres of someone, the character is talking to them.
const NEAR: f32 = 2.6;
/// Within this many metres of the scene, the trace is found.
const AT_SCENE: f32 = 3.0;
/// Name tags show within this distance.
const TAGS_WITHIN: f32 = 16.0;
/// Seconds in the office before the next client comes in.
const CLIENT_AFTER: f32 = 2.5;
/// Seconds to wait for the model before saying it did not answer.
const PATIENCE: f32 = 25.0;

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
    /// No case: the next client comes to the office.
    #[default]
    Waiting,
    Investigating,
    Solved,
    Failed,
}

/// Someone to talk to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Person {
    Client,
    Suspect(usize),
}

/// One line of a conversation.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Line {
    /// `"you"`, `"them"`, or `"note"` (the game telling something, such as
    /// the model not answering).
    pub from: &'static str,
    pub text: String,
}

/// The case in play and what has been found out.
#[derive(Resource, Default)]
pub struct Mystery {
    pub case: Option<Case>,
    pub phase: Phase,
    pub progress: Progress,
    /// Where it happened.
    pub scene: Vec2,
    /// Where each suspect is to be found.
    pub spaces: Vec<Space>,
    /// Clue indices (into the case's clues) found so far.
    pub found: Vec<bool>,
    /// Suspects talked to.
    pub talked: Vec<bool>,
    /// The notebook, line by line, in the player's language.
    pub notes: Vec<String>,
    /// Who was accused, once someone is.
    pub accused: Option<usize>,
    /// Every conversation so far.
    pub chats: HashMap<Person, Vec<Line>>,
}

impl Mystery {
    fn clear(&mut self) {
        let progress = self.progress;
        *self = Mystery {
            progress,
            ..default()
        };
    }
}

/// Who the character stands next to, if anyone: the chat is with them.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Talking(pub Option<Person>);

/// From the page: a chat message, an accusation, the next case, a walk to
/// somebody's door.
#[derive(Message, Clone, Debug)]
pub enum Detective {
    Chat(String),
    Accuse(usize),
    NextCase,
    Progress(Progress),
    /// Walk to a suspect's door (`None`: to the office).
    GoTo(Option<usize>),
}

/// Questions the notebook suggests, to start with.
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
struct Client;

#[derive(Component)]
struct ClueMarker;

#[derive(Component)]
struct NameTag(usize);

/// The system set the case runs in.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MysterySet;

/// Answers being waited for: who will speak, since when.
#[derive(Resource, Default)]
struct Pending(HashMap<u32, (Person, f32)>);

/// The office's door, once known.
#[derive(Resource, Default)]
struct Office(Option<Space>);

pub struct MysteryPlugin;

impl Plugin for MysteryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Mystery>()
            .init_resource::<Talking>()
            .init_resource::<Pending>()
            .init_resource::<Office>()
            .add_message::<Detective>()
            .configure_sets(Update, MysterySet.after(PlayerSet))
            .add_systems(
                Update,
                (
                    begin, requests, arrive, occupy, notice, chat, answers, publish,
                )
                    .chain()
                    .in_set(MysterySet),
            )
            .add_systems(Update, (idle, spin))
            .add_systems(PostUpdate, tags.after(TransformSystems::Propagate));
    }
}

fn italian(status: &HostStatus) -> bool {
    status.lang.starts_with("it")
}

fn language(status: &HostStatus) -> &'static str {
    if italian(status) {
        "Italian"
    } else {
        "English"
    }
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

/// The look of case `number`'s client.
fn client_look(seed: u64, number: u32) -> crate::figure::Look {
    crate::figure::Look::random(seed ^ (number as u64).wrapping_mul(0xA24B_AED4_963E_E407))
}

/// A person of the case standing at `at`, facing `facing`.
fn spawn_person(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    library: &VoxLibrary,
    look: &crate::figure::Look,
    at: Vec2,
    facing: f32,
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
        .spawn((
            Transform::from_xyz(at.x, 0.0, at.y).with_rotation(Quat::from_rotation_y(facing)),
            Visibility::default(),
        ))
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

/// Where each suspect is found: at work if they work in a shop and nobody
/// of the case is there already, else at home. Every one a door of their
/// own.
pub fn spaces_of(seed: u64, case: &Case, near: Vec2, library: &VoxLibrary) -> Vec<Space> {
    let mut taken: Vec<Vec2> = Vec::new();
    let mut out = Vec::new();
    for who in &case.suspects {
        let at_work = match who.hangs_out {
            PlaceKind::Park | PlaceKind::Square | PlaceKind::Home => None,
            kind => gen::nearest_place(seed, near, kind, REACH)
                .and_then(|p| rooms::space_of(seed, &p, library, RoomKind::Shop(kind))),
        }
        .filter(|s| !taken.iter().any(|t| t.distance(s.door) < 1.0));
        let space = at_work.or_else(|| {
            // Home: the nearest free front door around their usual place.
            let around = gen::nearest_place(seed, near, who.hangs_out, REACH)
                .map(|p| p.at)
                .unwrap_or(near);
            homes_around(seed, around)
                .into_iter()
                .filter_map(|p| rooms::space_of(seed, &p, library, RoomKind::Home))
                .find(|s| !taken.iter().any(|t| t.distance(s.door) < 1.0))
        });
        let space = space.unwrap_or(Space {
            kind: RoomKind::Home,
            door: near,
            out_facing: 0.0,
            salt: out.len() as u64,
        });
        taken.push(space.door);
        out.push(space);
    }
    out
}

/// Front doors of homes around a point, nearest first.
fn homes_around(seed: u64, at: Vec2) -> Vec<gen::Place> {
    let home = gen::chunk_of(at);
    let mut found: Vec<gen::Place> = (-3..=3)
        .flat_map(|dx| (-3..=3).map(move |dz| IVec2::new(dx, dz)))
        .flat_map(|d| gen::chunk(seed, home + d).places)
        .filter(|p| p.kind == PlaceKind::Home)
        .collect();
    found.sort_by(|a, b| a.at.distance(at).total_cmp(&b.at.distance(at)));
    found
}

/// Who the client is, for the model.
pub fn client_persona(case: &Case, city: &str, lang: &str) -> String {
    let seen: Vec<String> = case
        .clues
        .iter()
        .filter_map(|c| match c {
            Clue::Sighting { feature, by: None } => Some(feature.words()),
            _ => None,
        })
        .collect();
    format!(
        "You are {name}, a resident of {town} in a funny detective game, {manner}. You have come to the detective's office \
         because {what}. {seen}You know nothing more about who did it: never invent clues (objects, smells, sounds, \
         people seen) beyond what you are told here; asked about anything else, you did not notice. Talk to the detective's assistant, who questions you: \
         answer in character, upset and a little theatrical, in a few short sentences, as in a real conversation, in {lang}. \
         Never mention being a game character.",
        name = case.client.0,
        town = if city.is_empty() { "a small pastel town" } else { city },
        manner = case.client.1,
        what = case.crime.what,
        seen = if seen.is_empty() {
            String::new()
        } else {
            format!("You glimpsed whoever did it: they {}. ", seen.join(" and "))
        },
        lang = lang,
    )
}

/// Into the office when the game starts, and again in a new town.
fn begin(
    seed: Res<CitySeed>,
    status: Res<HostStatus>,
    wardrobe: Res<Wardrobe>,
    library: Res<VoxLibrary>,
    mut office: ResMut<Office>,
    mut doors: ResMut<Doors>,
    mut go_in: MessageWriter<GoIn>,
    mut mystery: ResMut<Mystery>,
    mut started: Local<bool>,
) {
    if seed.is_changed() && !seed.is_added() {
        *started = false;
        mystery.clear();
    }
    if *started || wardrobe.0 || status.lang.is_empty() {
        return;
    }
    *started = true;
    office.0 = rooms::office(seed.0, &library);
    doors.0 = office
        .0
        .into_iter()
        .chain(mystery.spaces.iter().copied())
        .collect();
    if let Some(space) = office.0 {
        go_in.write(GoIn(space));
    }
}

/// The page's moves that are not chat.
#[allow(clippy::too_many_arguments)]
fn requests(
    mut commands: Commands,
    status: Res<HostStatus>,
    city: Res<CityName>,
    office: Res<Office>,
    inside: Res<Inside>,
    time: Res<Time>,
    mut incoming: MessageReader<Detective>,
    mut mystery: ResMut<Mystery>,
    mut doors: ResMut<Doors>,
    mut host: ResMut<Host>,
    mut pending: ResMut<Pending>,
    mut walk: MessageWriter<crate::control::TapTo>,
    leftovers: Query<Entity, Or<(With<ClueMarker>, With<NameTag>)>>,
) {
    for request in incoming.read() {
        match request {
            Detective::Progress(progress) => mystery.progress = *progress,
            Detective::NextCase if matches!(mystery.phase, Phase::Solved | Phase::Failed) => {
                mystery.clear();
                for entity in &leftovers {
                    commands.entity(entity).despawn();
                }
                doors.0 = office.0.into_iter().collect();
            }
            Detective::GoTo(who) if inside.0.is_none() => {
                let door = match who {
                    Some(s) => mystery.spaces.get(*s).map(|space| space.door),
                    None => office.0.map(|space| space.door),
                };
                if let Some(door) = door {
                    walk.write(crate::control::TapTo(door));
                }
            }
            Detective::Accuse(s) => accuse(
                *s,
                &status,
                &city,
                &time,
                &mut mystery,
                &mut host,
                &mut pending,
            ),
            _ => {}
        }
    }
}

/// The accusation: right, the culprit confesses; wrong, they get away.
fn accuse(
    s: usize,
    status: &HostStatus,
    city: &CityName,
    time: &Time,
    mystery: &mut Mystery,
    host: &mut Host,
    pending: &mut Pending,
) {
    if mystery.phase != Phase::Investigating {
        return;
    }
    let Some(case) = mystery.case.clone() else {
        return;
    };
    if s >= case.suspects.len() {
        return;
    }
    let it = italian(status);
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
    if right {
        // The culprit confesses, in their own conversation.
        let who = Person::Suspect(case.culprit);
        let persona = format!(
            "{}\nYou have just been unmasked. Confess at last, theatrically, with your reason: you {}. A few short sentences, in {}.",
            suspect_persona(&case, case.culprit, &city.0, it),
            case.motive,
            language(status)
        );
        let id = host.ask_brain(&BrainRequest {
            persona,
            history: history(mystery.chats.get(&who)),
            situation: if it {
                "Sei stato smascherato: confessa.".into()
            } else {
                "You have been unmasked: confess.".into()
            },
            lang: status.lang.clone(),
            ..default()
        });
        pending.0.insert(id, (who, time.elapsed_secs()));
    }
}

/// A conversation as the model is told it.
fn history(lines: Option<&Vec<Line>>) -> Vec<Turn> {
    lines
        .into_iter()
        .flatten()
        .filter(|l| l.from != "note")
        .map(|l| Turn {
            from: if l.from == "you" { "detective" } else { "them" },
            text: l.text.clone(),
        })
        .collect()
}

/// In the office with no case open: the next client comes in, and the case
/// opens with them telling what happened.
#[allow(clippy::too_many_arguments)]
fn arrive(
    mut commands: Commands,
    seed: Res<CitySeed>,
    city: Res<CityName>,
    status: Res<HostStatus>,
    inside: Res<Inside>,
    office: Res<Office>,
    solids: Res<Solids>,
    library: Res<VoxLibrary>,
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mystery: ResMut<Mystery>,
    mut doors: ResMut<Doors>,
    mut host: ResMut<Host>,
    mut pending: ResMut<Pending>,
    mut waited: Local<f32>,
) {
    let in_office = inside
        .0
        .as_ref()
        .is_some_and(|room| room.space.kind == RoomKind::Office);
    if !in_office || mystery.phase != Phase::Waiting || status.lang.is_empty() {
        *waited = 0.0;
        return;
    }
    *waited += time.delta_secs();
    if *waited < CLIENT_AFTER {
        return;
    }
    *waited = 0.0;
    let (Some(room), Some(office)) = (inside.0.as_ref(), office.0) else {
        return;
    };
    // The case: the first whose scene is in reach of the office.
    let mut number = mystery.progress.case;
    let (case, scene) = loop {
        let case = case::case(seed.0, number, mystery.progress.level());
        if let Some(place) = gen::nearest_place(seed.0, office.door, case.crime.at, REACH) {
            break (case, place.at);
        }
        number += 1;
    };
    mystery.progress.case = number;
    let it = italian(&status);
    let lang = status.lang.as_str();
    mystery.spaces = spaces_of(seed.0, &case, scene, &library);
    doors.0 = std::iter::once(office)
        .chain(mystery.spaces.iter().copied())
        .collect();
    let marker = pavement_spot(&solids, scene);
    commands.spawn((
        ClueMarker,
        library.place("clue", Vec3::new(marker.x, 1.2, marker.y), 0.06, 0),
        NotShadowCaster,
    ));
    for (s, suspect) in case.suspects.iter().enumerate() {
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
    // The client, before the desk.
    let (at, facing) = room.host;
    let client = spawn_person(
        &mut commands,
        &mut meshes,
        &library,
        &client_look(seed.0, number),
        room.at(at),
        facing,
    );
    commands.entity(client).insert(Client);

    mystery.notes = vec![
        if it {
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
        },
        if it {
            format!(
                "Scena: {}. Cliente: {}.",
                case.crime.at.name_in(lang),
                case.client.0
            )
        } else {
            format!(
                "Scene: {}. Client: {}.",
                case.crime.at.name(),
                case.client.0
            )
        },
    ];
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
    // They speak first.
    let id = host.ask_brain(&BrainRequest {
        persona: client_persona(&case, &city.0, language(&status)),
        situation: if it {
            "Sei appena entrato nell'ufficio. Racconta cosa è successo.".into()
        } else {
            "You have just come into the office. Tell what happened.".into()
        },
        lang: status.lang.clone(),
        ..default()
    });
    pending.0.insert(id, (Person::Client, time.elapsed_secs()));
    mystery.talked = vec![false; case.suspects.len()];
    mystery.scene = scene;
    mystery.accused = None;
    mystery.chats.clear();
    mystery.case = Some(case);
    mystery.phase = Phase::Investigating;
    crate::bridge::host_log(&format!(
        "case {} opened at level {}",
        number + 1,
        mystery.progress.level()
    ));
}

/// Into a room: whoever belongs there is in. Out: they stay behind.
fn occupy(
    mut commands: Commands,
    seed: Res<CitySeed>,
    inside: Res<Inside>,
    library: Res<VoxLibrary>,
    mystery: Res<Mystery>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut passages: MessageReader<Passage>,
    people: Query<Entity, Or<(With<Suspect>, With<Client>)>>,
) {
    for passage in passages.read() {
        for entity in &people {
            commands.entity(entity).despawn();
        }
        let (Passage::In(space), Some(room), Some(case)) =
            (passage, inside.0.as_ref(), mystery.case.as_ref())
        else {
            continue;
        };
        if space.kind == RoomKind::Office {
            // The client waits in the office until the case is over.
            if mystery.phase == Phase::Investigating {
                let (at, facing) = room.host;
                let client = spawn_person(
                    &mut commands,
                    &mut meshes,
                    &library,
                    &client_look(seed.0, case.number),
                    room.at(at),
                    facing,
                );
                commands.entity(client).insert(Client);
            }
            continue;
        }
        let here: Vec<usize> = (0..case.suspects.len())
            .filter(|s| {
                mystery
                    .spaces
                    .get(*s)
                    .is_some_and(|sp| sp.door.distance(space.door) < 0.5)
            })
            .collect();
        for (n, s) in here.iter().enumerate() {
            let (at, facing) = room.host;
            let at = at + Vec2::new(n as f32 * 1.1, 0.0);
            let who = spawn_person(
                &mut commands,
                &mut meshes,
                &library,
                &case.suspects[*s].look,
                room.at(at),
                facing,
            );
            commands.entity(who).insert(Suspect(*s));
        }
    }
}

/// Who is near, and the scene.
fn notice(
    status: Res<HostStatus>,
    players: Query<&Transform, With<Player>>,
    people: Query<
        (&Transform, Option<&Suspect>),
        (Or<(With<Suspect>, With<Client>)>, Without<Player>),
    >,
    mut mystery: ResMut<Mystery>,
    mut talking: ResMut<Talking>,
) {
    let Ok(player) = players.single() else { return };
    let at = Vec2::new(player.translation.x, player.translation.z);
    let near = people
        .iter()
        .map(|(t, s)| {
            (
                s.map(|s| Person::Suspect(s.0)).unwrap_or(Person::Client),
                Vec2::new(t.translation.x, t.translation.z).distance(at),
            )
        })
        .filter(|(_, d)| *d < NEAR)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(p, _)| p);
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
    let trace = case.crime.trace;
    mystery.notes.push(if italian(&status) {
        format!("Sulla scena: {}.", trace.at_scene_in(lang))
    } else {
        format!("At the scene: {}.", trace.at_scene())
    });
}

/// The player's chat message, to whoever is near.
#[allow(clippy::too_many_arguments)]
fn chat(
    status: Res<HostStatus>,
    city: Res<CityName>,
    time: Res<Time>,
    talking: Res<Talking>,
    mut incoming: MessageReader<Detective>,
    mut mystery: ResMut<Mystery>,
    mut host: ResMut<Host>,
    mut pending: ResMut<Pending>,
) {
    for request in incoming.read() {
        let Detective::Chat(text) = request else {
            continue;
        };
        let (Some(who), Some(case)) = (talking.0, mystery.case.clone()) else {
            continue;
        };
        if text.is_empty() || pending.0.values().any(|(p, _)| *p == who) {
            continue;
        }
        let persona = match who {
            Person::Client => client_persona(&case, &city.0, language(&status)),
            Person::Suspect(s) => suspect_persona(&case, s, &city.0, italian(&status)),
        };
        let earlier = history(mystery.chats.get(&who));
        mystery.chats.entry(who).or_default().push(Line {
            from: "you",
            text: text.clone(),
        });
        let id = host.ask_brain(&BrainRequest {
            persona,
            history: earlier,
            situation: text.clone(),
            lang: status.lang.clone(),
            ..default()
        });
        pending.0.insert(id, (who, time.elapsed_secs()));
    }
}

/// The model's answers, into the conversation.
#[allow(clippy::too_many_arguments)]
fn answers(
    status: Res<HostStatus>,
    time: Res<Time>,
    mut answered: MessageReader<BrainAnswered>,
    mut pending: ResMut<Pending>,
    mut mystery: ResMut<Mystery>,
) {
    let it = italian(&status);
    let mut lines: Vec<(Person, Option<String>)> = Vec::new();
    for reply in answered.read() {
        if let Some((who, _)) = pending.0.remove(&reply.id) {
            let line = reply
                .answer
                .as_ref()
                .map(|a| a.say.trim().to_string())
                .filter(|s| !s.is_empty());
            lines.push((who, line));
        }
    }
    let now = time.elapsed_secs();
    let late: Vec<u32> = pending
        .0
        .iter()
        .filter(|(_, (_, at))| now - at > PATIENCE)
        .map(|(id, _)| *id)
        .collect();
    for id in late {
        if let Some((who, _)) = pending.0.remove(&id) {
            lines.push((who, None));
        }
    }
    for (who, line) in lines {
        let Some(text) = line else {
            mystery.chats.entry(who).or_default().push(Line {
                from: "note",
                text: if it {
                    "Nessuna risposta: controlla l'IA in Impostazioni.".into()
                } else {
                    "No answer: check the AI in Settings.".into()
                },
            });
            continue;
        };
        mystery
            .chats
            .entry(who)
            .or_default()
            .push(Line { from: "them", text });
        // The first time someone answers, the notebook notes what showed.
        if let (Person::Suspect(s), Some(case)) = (who, mystery.case.clone()) {
            if mystery.talked.get(s) == Some(&false) {
                mystery.talked[s] = true;
                let notes = suspect_notes(&case, s, &status.lang);
                mystery.notes.extend(notes);
            }
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
    progress: Progress,
    /// Who the character is talking to, and the conversation.
    talking: Option<NotebookPerson>,
    chat: &'a [Line],
    /// An answer is on its way.
    thinking: bool,
    questions: Vec<&'static str>,
    /// Where the character is: `"office"`, `"shop"`, `"home"`, or `"town"`.
    room: &'static str,
    /// How far the office is, outside.
    office: u32,
}

#[derive(Serialize)]
struct NotebookPerson {
    /// `"client"`, or the suspect's index.
    who: String,
    name: &'static str,
    role: &'static str,
}

#[derive(Serialize)]
struct NotebookSuspect {
    name: &'static str,
    role: &'static str,
    /// Where they are found ("il forno", "a casa").
    place: String,
    talked: bool,
    /// How far their door is, in metres, rounded; 0 inside.
    distance: u32,
}

#[allow(clippy::too_many_arguments)]
fn publish(
    status: Res<HostStatus>,
    mystery: Res<Mystery>,
    talking: Res<Talking>,
    pending: Res<Pending>,
    inside: Res<Inside>,
    office: Res<Office>,
    players: Query<&Transform, With<Player>>,
    mut last: Local<String>,
) {
    let at = players
        .single()
        .map(|t| Vec2::new(t.translation.x, t.translation.z))
        .unwrap_or_default();
    let lang = status.lang.as_str();
    let it = italian(&status);
    let away = |door: Vec2| {
        if inside.0.is_some() {
            0
        } else {
            ((door.distance(at) / 5.0).round() * 5.0) as u32
        }
    };
    let suspects = mystery
        .case
        .as_ref()
        .map(|case| {
            case.suspects
                .iter()
                .enumerate()
                .map(|(s, who)| {
                    let space = mystery.spaces.get(s);
                    NotebookSuspect {
                        name: who.name,
                        role: role_in(who.role, lang),
                        place: match space.map(|sp| sp.kind) {
                            Some(RoomKind::Shop(kind)) => kind.name_in(lang).to_string(),
                            _ if it => "a casa".to_string(),
                            _ => "at home".to_string(),
                        },
                        talked: mystery.talked.get(s).copied().unwrap_or(false),
                        distance: space.map(|sp| away(sp.door)).unwrap_or(0),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let talking_to = match (talking.0, mystery.case.as_ref()) {
        (Some(Person::Client), Some(case)) => Some(NotebookPerson {
            who: "client".into(),
            name: case.client.0,
            role: if it { "cliente" } else { "client" },
        }),
        (Some(Person::Suspect(s)), Some(case)) => Some(NotebookPerson {
            who: s.to_string(),
            name: case.suspects[s].name,
            role: role_in(case.suspects[s].role, lang),
        }),
        _ => None,
    };
    let empty = Vec::new();
    let chat = talking
        .0
        .and_then(|who| mystery.chats.get(&who))
        .unwrap_or(&empty);
    let notebook = Notebook {
        phase: match mystery.phase {
            Phase::Waiting => "waiting",
            Phase::Investigating => "investigating",
            Phase::Solved => "solved",
            Phase::Failed => "failed",
        },
        case: mystery.progress.case + u32::from(mystery.phase == Phase::Investigating),
        level: mystery
            .case
            .as_ref()
            .map(|c| c.level)
            .unwrap_or(mystery.progress.level()),
        notes: &mystery.notes,
        suspects,
        progress: mystery.progress,
        thinking: talking
            .0
            .is_some_and(|who| pending.0.values().any(|(p, _)| *p == who)),
        talking: talking_to,
        chat,
        questions: QUESTIONS
            .iter()
            .map(|(en, itq)| if it { *itq } else { *en })
            .collect(),
        room: match inside.0.as_ref().map(|r| r.space.kind) {
            Some(RoomKind::Office) => "office",
            Some(RoomKind::Shop(_)) => "shop",
            Some(RoomKind::Home) => "home",
            None => "town",
        },
        office: office.0.map(|sp| away(sp.door)).unwrap_or(0),
    };
    let json = serde_json::to_string(&notebook).unwrap_or_default();
    if *last != json {
        crate::bridge::publish_case(json.clone());
        *last = json;
    }
}

/// People of the case turn to the character when it is near.
fn idle(
    time: Res<Time>,
    players: Query<&Transform, (With<Player>, Without<Suspect>, Without<Client>)>,
    mut people: Query<&mut Transform, Or<(With<Suspect>, With<Client>)>>,
) {
    let Ok(player) = players.single() else { return };
    for mut transform in &mut people {
        if transform.translation.distance(player.translation) > 5.0 {
            continue;
        }
        let d = player.translation - transform.translation;
        let want = Quat::from_rotation_y((-d.x).atan2(-d.z));
        transform.rotation = transform
            .rotation
            .slerp(want, (time.delta_secs() * 5.0).min(1.0));
    }
}

/// The scene's question mark turns and bobs.
fn spin(time: Res<Time>, mut markers: Query<&mut Transform, With<ClueMarker>>) {
    let t = time.elapsed_secs();
    for mut marker in &mut markers {
        marker.rotation = Quat::from_rotation_y(t * 1.5);
        marker.translation.y = 1.2 + (t * 2.0).sin() * 0.12;
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
            *visibility = Visibility::Hidden;
            continue;
        };
        let close = at.translation().distance(player.translation()) < TAGS_WITHIN;
        let shown = close
            && mystery.phase == Phase::Investigating
            && talking.0 != Some(Person::Suspect(tag.0));
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
         who works for a detective) is questioning you about this: {what}. They came to find you where you are.\n",
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
        "Never invent clues about the case (objects, smells, sounds, people seen) beyond what you are told here. Answer what you are asked, in character, as in a real conversation: a few short sentences; you may be evasive, digress, ask back. Remember what was said before. Answer in {}. Never mention being a game character.",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city::seed_for;

    fn a_case(level: u32) -> Case {
        case::case(seed_for("Borgo Pastello"), 3, level)
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
    fn every_suspect_has_a_door_of_their_own() {
        let seed = seed_for("Borgo Pastello");
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .add_plugins(crate::vox::VoxPlugin);
        app.update();
        let library = app.world().resource::<VoxLibrary>();
        for number in 0..6 {
            let case = case::case(seed, number, 4);
            let scene = gen::nearest_place(seed, Vec2::ZERO, case.crime.at, REACH)
                .unwrap()
                .at;
            let spaces = spaces_of(seed, &case, scene, library);
            assert_eq!(spaces.len(), case.suspects.len());
            for (i, a) in spaces.iter().enumerate() {
                for b in &spaces[i + 1..] {
                    assert!(
                        a.door.distance(b.door) > 1.0,
                        "case {number}: two people at one door"
                    );
                }
                assert!(a.door.distance(scene) < 200.0, "case {number}: too far");
            }
        }
    }

    #[test]
    fn the_client_knows_what_happened_and_no_more() {
        let case = a_case(3);
        let persona = client_persona(&case, "Borgo Pastello", "Italian");
        assert!(persona.contains(case.crime.what));
        assert!(persona.contains(case.client.0));
        assert!(!persona.contains(case.motive));
        assert!(!persona.contains(case.suspects[case.culprit].name));
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
