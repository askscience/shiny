//! `CityPlugin`: the place, as data.
//!
//! A neighbourhood is a list of [`Prop`]s: which voxel model stands where,
//! turned which way, how big its voxels are, and whether it stops the
//! character. This module only lays the list out (for now one fixed
//! neighbourhood: nine lots of buildings with streets between them) and
//! spawns it; whatever writes the list later (a generator working from a
//! seed) changes nothing else.
//!
//! Everything that can be walked into is also a [`Solid`]: a box aligned to
//! the ground grid, taken from the model's own footprint. The player's
//! movement tests against these, nothing else.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::vox::VoxLibrary;

/// What kind of thing a solid is (what the character says differs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SolidKind {
    Building,
    Lamp,
    Tree,
    Bench,
    Planter,
    /// The hedge at the end of the neighbourhood.
    Edge,
}

/// A box on the ground, as seen from above (`x`, `z`).
#[derive(Clone, Copy, Debug)]
pub struct Solid {
    pub min: Vec2,
    pub max: Vec2,
    pub kind: SolidKind,
}

impl Solid {
    pub fn around(centre: Vec2, half: Vec2, kind: SolidKind) -> Self {
        Self {
            min: centre - half,
            max: centre + half,
            kind,
        }
    }

    /// Does a square of half-side `radius` centred on `at` touch this box?
    pub fn touches(&self, at: Vec2, radius: f32) -> bool {
        at.x + radius > self.min.x
            && at.x - radius < self.max.x
            && at.y + radius > self.min.y
            && at.y - radius < self.max.y
    }
}

/// Everything that stops the character, in a grid of buckets so a step only
/// looks at what is near it however big the city gets.
#[derive(Resource, Default)]
pub struct Solids {
    all: Vec<Solid>,
    buckets: HashMap<(i32, i32), Vec<u32>>,
}

/// Side of a bucket, in metres.
const BUCKET: f32 = 8.0;

impl Solids {
    pub fn new(all: Vec<Solid>) -> Self {
        let mut buckets: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (index, solid) in all.iter().enumerate() {
            let (x0, z0) = Self::bucket(solid.min);
            let (x1, z1) = Self::bucket(solid.max);
            for x in x0..=x1 {
                for z in z0..=z1 {
                    buckets.entry((x, z)).or_default().push(index as u32);
                }
            }
        }
        Self { all, buckets }
    }

    fn bucket(at: Vec2) -> (i32, i32) {
        (
            (at.x / BUCKET).floor() as i32,
            (at.y / BUCKET).floor() as i32,
        )
    }

    pub fn len(&self) -> usize {
        self.all.len()
    }

    pub fn is_empty(&self) -> bool {
        self.all.is_empty()
    }

    /// The first solid a square at `at` would touch.
    pub fn hit(&self, at: Vec2, radius: f32) -> Option<SolidKind> {
        let (x0, z0) = Self::bucket(at - Vec2::splat(radius));
        let (x1, z1) = Self::bucket(at + Vec2::splat(radius));
        for x in x0..=x1 {
            for z in z0..=z1 {
                let Some(bucket) = self.buckets.get(&(x, z)) else {
                    continue;
                };
                if let Some(solid) = bucket
                    .iter()
                    .map(|i| &self.all[*i as usize])
                    .find(|s| s.touches(at, radius))
                {
                    return Some(solid.kind);
                }
            }
        }
        None
    }
}

/// A voxel model standing somewhere.
#[derive(Clone, Debug)]
pub struct Prop {
    pub model: &'static str,
    /// The centre of its base on the ground (`x`, `z`), and how high it
    /// stands above the street.
    pub at: Vec2,
    pub lift: f32,
    /// Quarter turns anticlockwise seen from above; 0 faces `-z`.
    pub quarter_turns: i32,
    /// Metres to a voxel.
    pub voxel: f32,
    /// What it is to walk into, and half the box it takes on the ground;
    /// `None` half means the model's own footprint.
    pub solid: Option<(SolidKind, Option<Vec2>)>,
}

impl Prop {
    fn new(model: &'static str, at: Vec2, voxel: f32) -> Self {
        Self {
            model,
            at,
            lift: PAVEMENT,
            quarter_turns: 0,
            voxel,
            solid: None,
        }
    }

    fn turned(mut self, quarter_turns: i32) -> Self {
        self.quarter_turns = quarter_turns;
        self
    }

    /// Solid over its whole footprint.
    fn solid(mut self, kind: SolidKind) -> Self {
        self.solid = Some((kind, None));
        self
    }

    /// Solid only around its foot (a tree's trunk, not its crown).
    fn solid_foot(mut self, kind: SolidKind, half: f32) -> Self {
        self.solid = Some((kind, Some(Vec2::splat(half))));
        self
    }

    fn on_street(mut self) -> Self {
        self.lift = 0.0;
        self
    }

    /// The box it takes on the ground, given its model's size in voxels.
    pub fn footprint(&self, size: UVec3) -> Option<Solid> {
        let (kind, half) = self.solid?;
        let own = Vec2::new(size.x as f32, size.z as f32) * self.voxel / 2.0;
        let own = if self.quarter_turns.rem_euclid(2) == 1 {
            Vec2::new(own.y, own.x)
        } else {
            own
        };
        Some(Solid::around(self.at, half.unwrap_or(own), kind))
    }
}

/// Lots are `PITCH` apart with pavements `PAVED` wide, so the streets are
/// `PITCH - PAVED` wide and run along `±PITCH / 2`.
pub const PITCH: f32 = 22.0;
pub const PAVED: f32 = 16.8;
/// How high a pavement stands above the street.
pub const PAVEMENT: f32 = 0.12;
/// Half the side of the whole neighbourhood, where the hedge stands.
pub const EDGE: f32 = 36.0;
/// Where the walk starts: the middle of a crossing, looking up a street.
pub const START: Vec3 = Vec3::new(PITCH / 2.0, 0.0, PITCH / 2.0);

const BUILDINGS: [&str; 7] = [
    "building_rose_shop",
    "building_peach",
    "building_sky",
    "building_lilac_shop",
    "building_mint",
    "building_cream",
    "building_peach_shop",
];

/// A small fixed "random": the same city every time.
fn pick(seed: i32, modulo: u32) -> u32 {
    let mut x = (seed as u32).wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x % modulo
}

/// The neighbourhood: nine lots, each with four buildings facing its
/// streets and trees, benches and planters on its pavement; lamp posts at
/// the crossings; a hedge around it all.
pub fn neighbourhood() -> Vec<Prop> {
    let mut props = Vec::new();
    for lot_x in -1..=1 {
        for lot_z in -1..=1 {
            let lot = Vec2::new(lot_x as f32, lot_z as f32) * PITCH;
            let seed = (lot_x + 2) * 97 + (lot_z + 2) * 31;
            for (i, corner) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
                .into_iter()
                .enumerate()
            {
                let model = BUILDINGS[pick(seed + i as i32 * 7, BUILDINGS.len() as u32) as usize];
                // Fronts look away from the middle of the lot.
                let turns = if corner.1 < 0.0 { 0 } else { 2 };
                props.push(
                    Prop::new(model, lot + Vec2::new(corner.0, corner.1) * 3.5, 0.25)
                        .turned(turns)
                        .solid(SolidKind::Building),
                );
            }
            // The pavement between the walls and the kerb.
            let strip = PAVED / 2.0 - 0.75;
            for (i, (side, along)) in [
                (Vec2::X, Vec2::Y),
                (Vec2::NEG_X, Vec2::Y),
                (Vec2::Y, Vec2::X),
                (Vec2::NEG_Y, Vec2::X),
            ]
            .into_iter()
            .enumerate()
            {
                let seed = seed + 500 + i as i32 * 13;
                let tree = if pick(seed, 2) == 0 {
                    "tree"
                } else {
                    "tree_dark"
                };
                props.push(
                    Prop::new(tree, lot + side * strip, 0.1875).solid_foot(SolidKind::Tree, 0.3),
                );
                let turns = if side.x != 0.0 { 1 } else { 0 };
                match pick(seed + 1, 3) {
                    0 => props.push(
                        Prop::new("bench", lot + side * strip + along * 3.6, 0.125)
                            .turned(turns)
                            .solid(SolidKind::Bench),
                    ),
                    1 => props.push(
                        Prop::new("planter", lot + side * strip - along * 3.9, 0.125)
                            .solid(SolidKind::Planter),
                    ),
                    _ => {}
                }
            }
        }
    }
    // Lamp posts: on the pavement at two corners of every crossing.
    for x in [-1.0, 1.0] {
        for z in [-1.0, 1.0] {
            let crossing = Vec2::new(x, z) * (PITCH / 2.0);
            let reach = (PITCH - PAVED) / 2.0 + 0.35;
            for corner in [Vec2::new(1.0, 1.0), Vec2::new(-1.0, -1.0)] {
                props.push(
                    Prop::new("lamp", crossing + corner * reach, 0.125)
                        .solid_foot(SolidKind::Lamp, 0.16),
                );
            }
        }
    }
    // The hedge: 4 m pieces all the way round. One long solid per side
    // stands for them (see `edges`).
    let mut along = -EDGE + 2.0;
    while along < EDGE {
        for (at, turns) in [
            (Vec2::new(along, -EDGE), 0),
            (Vec2::new(along, EDGE), 0),
            (Vec2::new(-EDGE, along), 1),
            (Vec2::new(EDGE, along), 1),
        ] {
            props.push(Prop::new("hedge", at, 0.25).turned(turns).on_street());
        }
        along += 4.0;
    }
    props
}

/// The four sides of the hedge, as solids.
fn edges() -> [Solid; 4] {
    let long = Vec2::new(EDGE + 1.0, 0.5);
    let tall = Vec2::new(0.5, EDGE + 1.0);
    [
        Solid::around(Vec2::new(0.0, -EDGE), long, SolidKind::Edge),
        Solid::around(Vec2::new(0.0, EDGE), long, SolidKind::Edge),
        Solid::around(Vec2::new(-EDGE, 0.0), tall, SolidKind::Edge),
        Solid::around(Vec2::new(EDGE, 0.0), tall, SolidKind::Edge),
    ]
}

/// Every solid of a list of props, given the models' sizes.
pub fn solids(props: &[Prop], sizes: &HashMap<&'static str, UVec3>) -> Solids {
    let mut all: Vec<Solid> = props
        .iter()
        .filter_map(|prop| prop.footprint(sizes.get(prop.model).copied().unwrap_or(UVec3::ONE)))
        .collect();
    all.extend(edges());
    Solids::new(all)
}

pub struct CityPlugin;

impl Plugin for CityPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(solids(&neighbourhood(), &crate::vox::sizes()))
            .add_systems(Startup, build_city);
    }
}

fn build_city(
    mut commands: Commands,
    library: Res<VoxLibrary>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // The ground is three flat colours: plain boxes, three materials.
    let mut matte = |r: u8, g: u8, b: u8| {
        materials.add(StandardMaterial {
            base_color: Color::srgb_u8(r, g, b),
            perceptual_roughness: 1.0,
            reflectance: 0.05,
            ..default()
        })
    };
    let (asphalt, stone, line) = (
        matte(184, 188, 204),
        matte(229, 226, 220),
        matte(247, 245, 230),
    );
    let unit = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let mut slab = |centre: Vec3, size: Vec3, material: &Handle<StandardMaterial>| {
        commands.spawn((
            Mesh3d(unit.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(centre).with_scale(size),
        ));
    };
    let side = EDGE * 2.0 + 2.0;
    slab(
        Vec3::new(0.0, -0.25, 0.0),
        Vec3::new(side, 0.5, side),
        &asphalt,
    );
    for lot_x in -1..=1 {
        for lot_z in -1..=1 {
            let centre = Vec3::new(lot_x as f32 * PITCH, PAVEMENT / 2.0, lot_z as f32 * PITCH);
            slab(centre, Vec3::new(PAVED, PAVEMENT, PAVED), &stone);
        }
    }
    for street in [-PITCH / 2.0, PITCH / 2.0] {
        let mut along = -EDGE + 2.0;
        while along < EDGE - 2.0 {
            slab(
                Vec3::new(street, 0.011, along),
                Vec3::new(0.16, 0.02, 1.4),
                &line,
            );
            slab(
                Vec3::new(along, 0.011, street),
                Vec3::new(1.4, 0.02, 0.16),
                &line,
            );
            along += 3.6;
        }
    }

    for prop in neighbourhood() {
        commands.spawn(library.place(
            prop.model,
            Vec3::new(prop.at.x, prop.lift, prop.at.y),
            prop.voxel,
            prop.quarter_turns,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn city() -> Solids {
        solids(&neighbourhood(), &crate::vox::sizes())
    }

    #[test]
    fn the_walk_starts_in_the_open() {
        assert_eq!(city().hit(Vec2::new(START.x, START.z), 0.4), None);
    }

    #[test]
    fn streets_are_clear_and_lots_are_not() {
        let city = city();
        // Down the middle of a street, from hedge to hedge.
        let mut z = -EDGE + 2.0;
        while z < EDGE - 2.0 {
            assert_eq!(city.hit(Vec2::new(PITCH / 2.0, z), 0.4), None, "z = {z}");
            assert_eq!(city.hit(Vec2::new(z, -PITCH / 2.0), 0.4), None, "x = {z}");
            z += 0.5;
        }
        // A building stands in each quarter of a lot.
        assert_eq!(
            city.hit(Vec2::new(3.5, 3.5), 0.4),
            Some(SolidKind::Building)
        );
        assert_eq!(city.hit(Vec2::new(0.0, EDGE), 0.4), Some(SolidKind::Edge));
    }

    #[test]
    fn every_kind_of_thing_can_be_walked_into() {
        let city = city();
        let sizes = crate::vox::sizes();
        for prop in neighbourhood() {
            let Some(solid) = prop.footprint(sizes[prop.model]) else {
                continue;
            };
            assert_eq!(city.hit(prop.at, 0.05), Some(solid.kind), "{prop:?}");
        }
        for kind in [SolidKind::Lamp, SolidKind::Tree, SolidKind::Building] {
            assert!(
                neighbourhood()
                    .iter()
                    .any(|p| p.solid.map(|s| s.0) == Some(kind)),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn props_use_models_that_exist_and_buildings_stay_on_their_lots() {
        let sizes = crate::vox::sizes();
        for prop in neighbourhood() {
            let size = sizes
                .get(prop.model)
                .unwrap_or_else(|| panic!("no model {}", prop.model));
            if prop.solid.map(|s| s.0) == Some(SolidKind::Building) {
                let solid = prop.footprint(*size).unwrap();
                let lot = (prop.at / PITCH).round() * PITCH;
                let reach = (solid.max - lot).abs().max((solid.min - lot).abs());
                assert!(
                    reach.x <= PAVED / 2.0 - 1.0 && reach.y <= PAVED / 2.0 - 1.0,
                    "{prop:?}"
                );
            }
        }
    }

    #[test]
    fn a_turned_prop_turns_its_footprint() {
        let bench = Prop::new("bench", Vec2::ZERO, 0.125).solid(SolidKind::Bench);
        let size = UVec3::new(14, 8, 5);
        let straight = bench.clone().footprint(size).unwrap();
        let turned = bench.turned(1).footprint(size).unwrap();
        assert!(straight.max.x > straight.max.y);
        assert!(turned.max.y > turned.max.x);
    }

    #[test]
    fn buckets_find_what_a_plain_search_finds() {
        let city = city();
        let mut x = -EDGE;
        while x < EDGE {
            let mut z = -EDGE;
            while z < EDGE {
                let at = Vec2::new(x, z);
                let plain = city
                    .all
                    .iter()
                    .find(|s| s.touches(at, 0.38))
                    .map(|s| s.kind);
                assert_eq!(city.hit(at, 0.38).is_some(), plain.is_some(), "{at:?}");
                z += 0.7;
            }
            x += 0.7;
        }
    }
}
