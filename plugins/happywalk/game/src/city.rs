//! `CityPlugin`: the city around the player.
//!
//! The city itself is written by the generator (`gen.rs`) from a seed, one
//! chunk at a time, and has no edge. This plugin keeps the chunks near the
//! player on screen: it spawns the ones that come within reach, a few per
//! frame, and drops the ones left behind. Fog (`sky.rs`) hides the border.
//!
//! [`Solids`] answers "is there something here?" for any place in the city,
//! loaded or not: the chunks on screen are remembered, any other is worked
//! out from the seed on the spot.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::gen::{self, LotKind};
pub use crate::gen::{Solid, SolidKind, PAVED, PAVEMENT, PITCH};
use crate::player::Player;
use crate::vox::{Geometry, VoxLibrary};
use bevy::camera::visibility::VisibilityRange;
use bevy::light::NotShadowCaster;

/// What the player called their city.
#[derive(Resource, Clone, Debug, Default)]
pub struct CityName(pub String);

/// The player named their city (or renamed it).
#[derive(Message, Clone, Debug)]
pub struct CityNamed(pub String);

/// The seed of the city a name stands for: the same name, the same city,
/// whatever the case or the spaces around it.
pub fn seed_for(name: &str) -> u64 {
    let name = name.trim().to_lowercase();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    // Never 0: the generator's dice want a nonzero start.
    hash | 1
}

/// The seed the city grows from.
#[derive(Resource, Clone, Copy, Debug)]
pub struct CitySeed(pub u64);

/// Where the walk starts: the middle of a crossing, looking up a street.
pub const START: Vec3 = Vec3::new(PITCH / 2.0, 0.0, PITCH / 2.0);

/// Chunks this many steps from the player's are on screen...
const NEAR: i32 = 2;
/// ...and are dropped once they are this far.
const FAR: i32 = 3;
/// Chunks this near get everything; farther ones only what shows from a
/// distance (buildings, trees, lamp posts).
const DETAILED: i32 = 1;
/// Chunks built in one frame, so walking never stutters (merging a
/// chunk's geometry takes a moment).
const PER_FRAME: usize = 1;

/// Everything that stops the character, anywhere in the city.
#[derive(Resource)]
pub struct Solids {
    seed: u64,
    sizes: HashMap<&'static str, UVec3>,
    loaded: HashMap<IVec2, Vec<Solid>>,
    /// The walls and furniture of the room the character is in: while there
    /// are any, they are all there is.
    room: Vec<Solid>,
}

impl Solids {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            sizes: crate::vox::sizes(),
            loaded: HashMap::new(),
            room: Vec::new(),
        }
    }

    /// The solids of the room the character is in (none: back in the city).
    pub fn set_room(&mut self, room: Vec<Solid>) {
        self.room = room;
    }

    fn of(&self, chunk: &gen::Chunk) -> Vec<Solid> {
        gen::chunk_solids(chunk, &self.sizes)
    }

    /// Remember a chunk's solids while it is on screen.
    pub fn load(&mut self, chunk: &gen::Chunk) {
        let solids = self.of(chunk);
        self.loaded.insert(chunk.coord, solids);
    }

    pub fn unload(&mut self, coord: IVec2) {
        self.loaded.remove(&coord);
    }

    /// The first solid a square at `at` would touch.
    pub fn hit(&self, at: Vec2, radius: f32) -> Option<SolidKind> {
        self.what(at, radius).map(|solid| solid.kind)
    }

    /// The first solid a square at `at` would touch, whole.
    pub fn what(&self, at: Vec2, radius: f32) -> Option<Solid> {
        self.find(at, radius, |_| true)
    }

    /// Is there a solid of this kind within the square?
    pub fn any(&self, at: Vec2, radius: f32, kind: SolidKind) -> bool {
        self.find(at, radius, |s| s.kind == kind).is_some()
    }

    fn find(&self, at: Vec2, radius: f32, wanted: impl Fn(&Solid) -> bool) -> Option<Solid> {
        // A lot's solids stay on its pavement, so only the chunks the square
        // itself lies in can matter.
        let low = gen::chunk_of(at - Vec2::splat(radius));
        let high = gen::chunk_of(at + Vec2::splat(radius));
        let fits = |s: &Solid| s.touches(at, radius) && wanted(s);
        if !self.room.is_empty() {
            return self.room.iter().find(|s| fits(s)).copied();
        }
        for x in low.x..=high.x {
            for z in low.y..=high.y {
                let coord = IVec2::new(x, z);
                let found = match self.loaded.get(&coord) {
                    Some(solids) => solids.iter().find(|s| fits(s)).copied(),
                    None => self
                        .of(&gen::chunk(self.seed, coord))
                        .into_iter()
                        .find(|s| fits(s)),
                };
                if found.is_some() {
                    return found;
                }
            }
        }
        None
    }
}

/// A name from the player: the city it stands for replaces this one.
fn rename(
    mut named: MessageReader<CityNamed>,
    mut name: ResMut<CityName>,
    mut seed: ResMut<CitySeed>,
) {
    for new in named.read() {
        let clean = new.0.trim();
        if clean.is_empty() || clean == name.0 {
            continue;
        }
        name.0 = clean.to_string();
        let next = seed_for(clean);
        if seed.0 != next {
            seed.0 = next;
        }
    }
}

/// The chunks on screen, and what was spawned for each.
#[derive(Resource, Default)]
struct Loaded(HashMap<IVec2, (bool, Vec<Entity>)>);

/// The dashes down the middle of the two streets a chunk owns (the one along
/// its `+x` side and the one along its `+z` side): centre and half size,
/// around the middle of the lot.
fn dashes() -> Vec<(Vec2, Vec2)> {
    let mut out = Vec::new();
    let street = PITCH / 2.0;
    let mut along = -PITCH / 2.0 + 1.1;
    while along < PITCH / 2.0 {
        // Not across the crossing itself.
        if along < PAVED / 2.0 {
            out.push((Vec2::new(street, along), Vec2::new(0.08, 0.7)));
            out.push((Vec2::new(along, street), Vec2::new(0.7, 0.08)));
        }
        along += 3.3;
    }
    out
}

/// A colour from sRGB bytes, as a vertex colour.
fn rgb(r: u8, g: u8, b: u8) -> [f32; 4] {
    let c = Color::srgb_u8(r, g, b).to_linear();
    [c.red, c.green, c.blue, 1.0]
}

/// Everything of a chunk that stands still, as two meshes around the middle
/// of its lot: the ground (the street, the pavement of stone, grass or the
/// square's paving, the dashes) and every prop standing on it. A far chunk takes the halved models
/// and leaves the small things out.
pub fn chunk_geometry(chunk: &gen::Chunk, library: &VoxLibrary, far: bool) -> (Geometry, Geometry) {
    let lot = gen::centre(chunk.coord);
    let mut out = Geometry::default();
    let half = PITCH / 2.0;
    out.floor(
        Vec2::splat(-half),
        Vec2::splat(half),
        0.0,
        rgb(122, 126, 144),
    );
    let paving = match chunk.kind {
        LotKind::Park => rgb(134, 206, 128),
        LotKind::Square => rgb(240, 218, 180),
        _ => rgb(222, 214, 200),
    };
    out.block(
        Vec3::new(-PAVED / 2.0, 0.0, -PAVED / 2.0),
        Vec3::new(PAVED / 2.0, PAVEMENT, PAVED / 2.0),
        paving,
    );
    let line = rgb(247, 245, 230);
    for (centre, size) in dashes() {
        out.floor(centre - size, centre + size, 0.012, line);
    }
    let ground = out;
    let mut out = Geometry::default();
    for prop in chunk
        .props
        .iter()
        .filter(|prop| !far || prop.shows_from_afar())
    {
        let Some(model) = library.geometry(prop.model, far) else {
            continue;
        };
        let voxel = if far { prop.voxel * 2.0 } else { prop.voxel };
        let place = Transform::from_xyz(prop.at.x - lot.x, prop.lift, prop.at.y - lot.y)
            .with_rotation(Quat::from_rotation_y(
                prop.quarter_turns as f32 * std::f32::consts::FRAC_PI_2,
            ))
            .with_scale(Vec3::splat(voxel));
        out.append(model, &place);
    }
    (ground, out)
}

/// Chunks whose middle is farther than this from the camera are not drawn:
/// all of them is in the fog by then.
const DRAWN_WITHIN: f32 = 72.0;

pub struct CityPlugin;

impl Plugin for CityPlugin {
    fn build(&self, app: &mut App) {
        let seed = app
            .world()
            .get_resource::<CitySeed>()
            .map(|s| s.0)
            .unwrap_or(1);
        app.insert_resource(CitySeed(seed))
            .insert_resource(Solids::new(seed))
            .init_resource::<Loaded>()
            .init_resource::<CityName>()
            .add_message::<CityNamed>()
            .add_systems(PreUpdate, rename)
            .add_systems(Update, stream);
    }
}

/// Keep the chunks around the player on screen.
fn stream(
    mut commands: Commands,
    seed: Res<CitySeed>,
    library: Res<VoxLibrary>,
    mut meshes: ResMut<Assets<Mesh>>,
    players: Query<&Transform, With<Player>>,
    mut loaded: ResMut<Loaded>,
    mut solids: ResMut<Solids>,
    mut here: Local<Option<IVec2>>,
    wardrobe: Res<crate::bridge::Wardrobe>,
    inside: Res<crate::rooms::Inside>,
) {
    // A new name is a new city: everything of the old one goes.
    if seed.is_changed() && !seed.is_added() {
        for (_, (_, entities)) in loaded.0.drain() {
            for entity in entities {
                commands.entity(entity).despawn();
            }
        }
        *solids = Solids::new(seed.0);
        *here = None;
    }
    // While the character is being dressed (in the studio) or is in a room,
    // it is far away: the city stays as it is.
    if wardrobe.0 || inside.0.is_some() {
        return;
    }
    let at = players
        .single()
        .map(|t| Vec2::new(t.translation.x, t.translation.z))
        .unwrap_or(Vec2::new(START.x, START.z));
    let home = gen::chunk_of(at);
    let first = loaded.0.is_empty();
    // Nothing to do while the player stays in a chunk whose surroundings
    // are all there.
    if *here == Some(home) {
        return;
    }

    // Drop what is too far, and what is on screen at the wrong detail (it is
    // spawned again below at the right one).
    let stale: Vec<IVec2> = loaded
        .0
        .iter()
        .filter(|(coord, (detailed, _))| {
            let ring = (**coord - home).abs().max_element();
            ring > FAR || (ring <= NEAR && *detailed != (ring <= DETAILED))
        })
        .map(|(coord, _)| *coord)
        .collect();
    for coord in stale {
        for entity in loaded
            .0
            .remove(&coord)
            .map(|(_, entities)| entities)
            .unwrap_or_default()
        {
            commands.entity(entity).despawn();
        }
        if (coord - home).abs().max_element() > FAR {
            solids.unload(coord);
        }
    }

    let mut missing: Vec<IVec2> = (-NEAR..=NEAR)
        .flat_map(|x| (-NEAR..=NEAR).map(move |z| home + IVec2::new(x, z)))
        .filter(|coord| !loaded.0.contains_key(coord))
        .collect();
    missing.sort_by_key(|coord| (*coord - home).length_squared());
    // Everything at once the first time (the game is still loading), then a
    // few a frame, nearest first.
    let budget = if first { missing.len() } else { PER_FRAME };
    if missing.len() <= budget {
        *here = Some(home);
    }
    for coord in missing.into_iter().take(budget) {
        let chunk = gen::chunk(seed.0, coord);
        let detailed = (coord - home).abs().max_element() <= DETAILED;
        let lot = gen::centre(coord);
        // The whole chunk in two draws: the ground, which casts no shadow
        // (on itself it would only darken everything), and what stands on it.
        let (ground, things) = chunk_geometry(&chunk, &library, !detailed);
        let mut entities = Vec::with_capacity(2);
        for (geometry, casts) in [(ground, false), (things, true)] {
            if geometry.indices.is_empty() {
                continue;
            }
            let mut entity = commands.spawn((
                Mesh3d(meshes.add(geometry.mesh())),
                MeshMaterial3d(library.material.clone()),
                Transform::from_xyz(lot.x, 0.0, lot.y),
                VisibilityRange::abrupt(0.0, DRAWN_WITHIN),
                crate::vox::Triangles(geometry.triangles() as u32),
            ));
            if !casts {
                entity.insert(NotShadowCaster);
            }
            entities.push(entity.id());
        }
        solids.load(&chunk);
        loaded.0.insert(coord, (detailed, entities));
    }
}

/// A lot with buildings near the start, for tests: its chunk.
#[cfg(test)]
pub fn a_built_lot(seed: u64) -> IVec2 {
    (0..40)
        .flat_map(|x| (0..40).map(move |z| IVec2::new(x, z)))
        .find(|c| matches!(gen::lot_kind(seed, *c), LotKind::Homes | LotKind::Shops))
        .expect("a city with no buildings")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_always_the_same_city() {
        assert_eq!(seed_for("Borgo Pastello"), seed_for("  borgo pastello "));
        assert_ne!(seed_for("Borgo Pastello"), seed_for("Villa Confetto"));
        assert_ne!(seed_for(""), 0);
        // Every name gives a city the walk can start in.
        for name in [
            "Borgo Pastello",
            "Villa Confetto",
            "Pandolce",
            "Marshmallow Bay",
            "Zzz",
        ] {
            assert_eq!(
                Solids::new(seed_for(name)).hit(Vec2::new(START.x, START.z), 0.4),
                None,
                "{name}"
            );
        }
    }

    #[test]
    fn the_walk_starts_in_the_open() {
        for seed in [1, 2, 3] {
            assert_eq!(
                Solids::new(seed).hit(Vec2::new(START.x, START.z), 0.4),
                None
            );
        }
    }

    #[test]
    fn streets_are_clear_and_built_lots_are_not() {
        let seed = 11;
        let city = Solids::new(seed);
        // Down the middle of a street, a long way.
        let mut z = -200.0;
        while z < 200.0 {
            assert_eq!(city.hit(Vec2::new(PITCH / 2.0, z), 0.4), None, "z = {z}");
            assert_eq!(city.hit(Vec2::new(z, -PITCH / 2.0), 0.4), None, "x = {z}");
            z += 0.5;
        }
        let lot = gen::centre(a_built_lot(seed));
        assert_eq!(
            city.hit(lot + Vec2::splat(3.5), 0.4),
            Some(SolidKind::Building)
        );
    }

    #[test]
    fn a_loaded_chunk_answers_like_the_generator() {
        let seed = 5;
        let fresh = Solids::new(seed);
        let mut warm = Solids::new(seed);
        for x in -1..=1 {
            for z in -1..=1 {
                warm.load(&gen::chunk(seed, IVec2::new(x, z)));
            }
        }
        let mut x = -40.0;
        while x < 40.0 {
            let mut z = -40.0;
            while z < 40.0 {
                let at = Vec2::new(x, z);
                assert_eq!(warm.hit(at, 0.38), fresh.hit(at, 0.38), "{at:?}");
                z += 0.9;
            }
            x += 0.9;
        }
        warm.unload(IVec2::ZERO);
        assert_eq!(warm.hit(Vec2::ZERO, 0.38), fresh.hit(Vec2::ZERO, 0.38));
    }

    #[test]
    fn the_dashes_stay_out_of_the_crossing() {
        let dashes = dashes();
        assert!(!dashes.is_empty());
        for (centre, half) in dashes {
            let far = centre + half;
            assert!(!(far.x > PAVED / 2.0 && far.y > PAVED / 2.0), "{centre:?}");
        }
    }
}
