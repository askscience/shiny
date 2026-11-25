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

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::gen::{self, LotKind};
pub use crate::gen::{Solid, SolidKind, PAVED, PAVEMENT, PITCH};
use crate::player::Player;
use crate::vox::VoxLibrary;

/// The seed the city grows from.
#[derive(Resource, Clone, Copy, Debug)]
pub struct CitySeed(pub u64);

/// Where the walk starts: the middle of a crossing, looking up a street.
pub const START: Vec3 = Vec3::new(PITCH / 2.0, 0.0, PITCH / 2.0);

/// Chunks this many steps from the player's are on screen...
const NEAR: i32 = 2;
/// ...and are dropped once they are this far.
const FAR: i32 = 3;
/// Chunks spawned in one frame, so walking never stutters.
const PER_FRAME: usize = 2;

/// Everything that stops the character, anywhere in the city.
#[derive(Resource)]
pub struct Solids {
    seed: u64,
    sizes: HashMap<&'static str, UVec3>,
    loaded: HashMap<IVec2, Vec<Solid>>,
}

impl Solids {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            sizes: crate::vox::sizes(),
            loaded: HashMap::new(),
        }
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
        // A lot's solids stay on its pavement, so only the chunks the square
        // itself lies in can matter.
        let low = gen::chunk_of(at - Vec2::splat(radius));
        let high = gen::chunk_of(at + Vec2::splat(radius));
        for x in low.x..=high.x {
            for z in low.y..=high.y {
                let coord = IVec2::new(x, z);
                let found = match self.loaded.get(&coord) {
                    Some(solids) => solids
                        .iter()
                        .find(|s| s.touches(at, radius))
                        .map(|s| s.kind),
                    None => self
                        .of(&gen::chunk(self.seed, coord))
                        .iter()
                        .find(|s| s.touches(at, radius))
                        .map(|s| s.kind),
                };
                if found.is_some() {
                    return found;
                }
            }
        }
        None
    }
}

/// The chunks on screen, and what was spawned for each.
#[derive(Resource, Default)]
struct Loaded(HashMap<IVec2, Vec<Entity>>);

/// What every chunk's ground is made of, made once.
#[derive(Resource)]
struct Ground {
    slab: Handle<Mesh>,
    dashes: Handle<Mesh>,
    asphalt: Handle<StandardMaterial>,
    stone: Handle<StandardMaterial>,
    grass: Handle<StandardMaterial>,
    square: Handle<StandardMaterial>,
    line: Handle<StandardMaterial>,
}

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
            .add_systems(Startup, prepare_ground)
            .add_systems(Update, stream);
    }
}

/// The dashes down the middle of the two streets a chunk owns (the one along
/// its `+x` side and the one along its `+z` side), as one mesh.
fn dashes_mesh() -> Mesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    let mut quad = |centre: Vec2, half: Vec2| {
        let first = positions.len() as u32;
        for (dx, dz) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, 1.0), (1.0, -1.0)] {
            positions.push([centre.x + dx * half.x, 0.012, centre.y + dz * half.y]);
        }
        indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
    };
    let street = PITCH / 2.0;
    let mut along = -PITCH / 2.0 + 1.1;
    while along < PITCH / 2.0 {
        // Not across the crossing itself.
        if along < PAVED / 2.0 {
            quad(Vec2::new(street, along), Vec2::new(0.08, 0.7));
            quad(Vec2::new(along, street), Vec2::new(0.7, 0.08));
        }
        along += 3.3;
    }
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices))
}

fn prepare_ground(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut matte = |r: u8, g: u8, b: u8| {
        materials.add(StandardMaterial {
            base_color: Color::srgb_u8(r, g, b),
            perceptual_roughness: 1.0,
            reflectance: 0.05,
            ..default()
        })
    };
    commands.insert_resource(Ground {
        asphalt: matte(184, 188, 204),
        stone: matte(229, 226, 220),
        grass: matte(190, 228, 196),
        square: matte(244, 232, 214),
        line: matte(247, 245, 230),
        slab: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        dashes: meshes.add(dashes_mesh()),
    });
}

/// Keep the chunks around the player on screen.
fn stream(
    mut commands: Commands,
    seed: Res<CitySeed>,
    library: Res<VoxLibrary>,
    ground: Res<Ground>,
    players: Query<&Transform, With<Player>>,
    mut loaded: ResMut<Loaded>,
    mut solids: ResMut<Solids>,
    mut here: Local<Option<IVec2>>,
) {
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

    let far: Vec<IVec2> = loaded
        .0
        .keys()
        .copied()
        .filter(|c| (*c - home).abs().max_element() > FAR)
        .collect();
    for coord in far {
        for entity in loaded.0.remove(&coord).unwrap_or_default() {
            commands.entity(entity).despawn();
        }
        solids.unload(coord);
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
        let lot = gen::centre(coord);
        let mut entities = Vec::with_capacity(chunk.props.len() + 3);
        let slab =
            |commands: &mut Commands, y: f32, size: Vec3, material: &Handle<StandardMaterial>| {
                commands
                    .spawn((
                        Mesh3d(ground.slab.clone()),
                        MeshMaterial3d(material.clone()),
                        Transform::from_xyz(lot.x, y, lot.y).with_scale(size),
                    ))
                    .id()
            };
        entities.push(slab(
            &mut commands,
            -0.25,
            Vec3::new(PITCH, 0.5, PITCH),
            &ground.asphalt,
        ));
        let paving = match chunk.kind {
            LotKind::Park => &ground.grass,
            LotKind::Square => &ground.square,
            _ => &ground.stone,
        };
        entities.push(slab(
            &mut commands,
            PAVEMENT / 2.0,
            Vec3::new(PAVED, PAVEMENT, PAVED),
            paving,
        ));
        entities.push(
            commands
                .spawn((
                    Mesh3d(ground.dashes.clone()),
                    MeshMaterial3d(ground.line.clone()),
                    Transform::from_xyz(lot.x, 0.0, lot.y),
                ))
                .id(),
        );
        for prop in &chunk.props {
            entities.push(
                commands
                    .spawn(library.place(
                        prop.model,
                        Vec3::new(prop.at.x, prop.lift, prop.at.y),
                        prop.voxel,
                        prop.quarter_turns,
                    ))
                    .id(),
            );
        }
        solids.load(&chunk);
        loaded.0.insert(coord, entities);
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
        let mesh = dashes_mesh();
        let positions = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .unwrap()
            .as_float3()
            .unwrap();
        assert!(!positions.is_empty());
        for p in positions {
            let (x, z) = (p[0], p[2]);
            assert!(!(x > PAVED / 2.0 && z > PAVED / 2.0), "{p:?}");
        }
    }
}
