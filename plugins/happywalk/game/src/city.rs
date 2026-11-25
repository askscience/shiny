//! `CityPlugin`: the place. For now one fixed neighbourhood: nine lots of
//! cube buildings with streets between them, lamp posts at the crossings and
//! a hedge around it all.
//!
//! Everything that can be walked into is also a [`Solid`]: a box aligned to
//! the ground grid. The player's movement tests against these, nothing else.

use bevy::prelude::*;

/// What kind of thing a solid is (the character's lines differ).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolidKind {
    Building,
    Lamp,
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

/// Everything that stops the character.
#[derive(Resource, Default)]
pub struct Solids(pub Vec<Solid>);

impl Solids {
    /// The first solid a square at `at` would touch.
    pub fn hit(&self, at: Vec2, radius: f32) -> Option<SolidKind> {
        self.0
            .iter()
            .find(|solid| solid.touches(at, radius))
            .map(|solid| solid.kind)
    }
}

/// Lots are `LOT` wide and `PITCH` apart, so the streets are `PITCH - LOT`
/// wide and run along `±PITCH / 2`.
pub const PITCH: f32 = 22.0;
pub const LOT: f32 = 14.0;
/// Half the side of the whole neighbourhood, where the hedge stands.
pub const EDGE: f32 = 37.0;
/// Where the walk starts: the middle of a crossing, looking up a street.
pub const START: Vec3 = Vec3::new(PITCH / 2.0, 0.0, PITCH / 2.0);

const WALLS: [Color; 6] = [
    Color::srgb(0.98, 0.80, 0.82),
    Color::srgb(0.99, 0.88, 0.72),
    Color::srgb(0.80, 0.90, 0.98),
    Color::srgb(0.84, 0.82, 0.96),
    Color::srgb(0.78, 0.93, 0.86),
    Color::srgb(0.99, 0.93, 0.78),
];
const ROOFS: [Color; 3] = [
    Color::srgb(0.93, 0.60, 0.62),
    Color::srgb(0.55, 0.72, 0.86),
    Color::srgb(0.62, 0.80, 0.70),
];

/// A small fixed "random": the same city every time.
fn pick(seed: i32, modulo: u32) -> u32 {
    let mut x = (seed as u32).wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x % modulo
}

/// The buildings of the neighbourhood: centre, half size on the ground,
/// height, and which colours.
pub fn buildings() -> Vec<(Vec2, Vec2, f32, usize, usize)> {
    let mut out = Vec::new();
    for lot_x in -1..=1 {
        for lot_z in -1..=1 {
            let lot = Vec2::new(lot_x as f32, lot_z as f32) * PITCH;
            // Four buildings to a lot, each a little different.
            for (i, corner) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
                .into_iter()
                .enumerate()
            {
                let seed = (lot_x + 2) * 97 + (lot_z + 2) * 31 + i as i32 * 7;
                let half = Vec2::new(
                    2.6 + pick(seed, 5) as f32 * 0.15,
                    2.6 + pick(seed + 1, 5) as f32 * 0.15,
                );
                let centre = lot + Vec2::new(corner.0, corner.1) * (LOT / 4.0);
                let height = 4.0 + pick(seed + 2, 5) as f32 * 2.0;
                out.push((
                    centre,
                    half,
                    height,
                    pick(seed + 3, WALLS.len() as u32) as usize,
                    pick(seed + 4, ROOFS.len() as u32) as usize,
                ));
            }
        }
    }
    out
}

/// Lamp posts: one at a corner of every crossing.
pub fn lamps() -> Vec<Vec2> {
    let mut out = Vec::new();
    for x in [-1.0, 1.0] {
        for z in [-1.0, 1.0] {
            out.push(Vec2::new(x, z) * (PITCH / 2.0) + Vec2::new(x, z) * 3.3);
        }
    }
    out
}

/// Every solid of the neighbourhood.
pub fn solids() -> Vec<Solid> {
    let mut out: Vec<Solid> = buildings()
        .into_iter()
        .map(|(centre, half, ..)| Solid::around(centre, half, SolidKind::Building))
        .collect();
    out.extend(
        lamps()
            .into_iter()
            .map(|at| Solid::around(at, Vec2::splat(0.18), SolidKind::Lamp)),
    );
    let long = Vec2::new(EDGE + 1.0, 0.5);
    let tall = Vec2::new(0.5, EDGE + 1.0);
    out.push(Solid::around(Vec2::new(0.0, -EDGE), long, SolidKind::Edge));
    out.push(Solid::around(Vec2::new(0.0, EDGE), long, SolidKind::Edge));
    out.push(Solid::around(Vec2::new(-EDGE, 0.0), tall, SolidKind::Edge));
    out.push(Solid::around(Vec2::new(EDGE, 0.0), tall, SolidKind::Edge));
    out
}

pub struct CityPlugin;

impl Plugin for CityPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(0.80, 0.90, 0.97)))
            .insert_resource(Solids(solids()))
            .add_systems(Startup, build_city);
    }
}

fn build_city(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut matte = |color: Color| {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: 0.95,
            reflectance: 0.1,
            ..default()
        })
    };
    let unit = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let block =
        |commands: &mut Commands, centre: Vec3, size: Vec3, material: Handle<StandardMaterial>| {
            commands.spawn((
                Mesh3d(unit.clone()),
                MeshMaterial3d(material),
                Transform::from_translation(centre).with_scale(size),
            ));
        };

    commands.spawn((
        DirectionalLight {
            illuminance: 9500.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(18.0, 30.0, 12.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.92, 0.94, 1.0),
        brightness: 420.0,
        ..default()
    });

    // The street is the ground; lots are raised pavements on it.
    let street = matte(Color::srgb(0.72, 0.74, 0.80));
    let pavement = matte(Color::srgb(0.90, 0.89, 0.86));
    let stripe = matte(Color::srgb(0.97, 0.96, 0.90));
    let side = EDGE * 2.0;
    block(
        &mut commands,
        Vec3::new(0.0, -0.25, 0.0),
        Vec3::new(side, 0.5, side),
        street,
    );
    for lot_x in -1..=1 {
        for lot_z in -1..=1 {
            let centre = Vec3::new(lot_x as f32 * PITCH, 0.06, lot_z as f32 * PITCH);
            block(
                &mut commands,
                centre,
                Vec3::new(LOT + 1.6, 0.12, LOT + 1.6),
                pavement.clone(),
            );
        }
    }
    // Dashes down the middle of the four streets.
    for line in [-PITCH / 2.0, PITCH / 2.0] {
        let mut along = -EDGE + 2.0;
        while along < EDGE - 2.0 {
            block(
                &mut commands,
                Vec3::new(line, 0.011, along),
                Vec3::new(0.18, 0.02, 1.4),
                stripe.clone(),
            );
            block(
                &mut commands,
                Vec3::new(along, 0.011, line),
                Vec3::new(1.4, 0.02, 0.18),
                stripe.clone(),
            );
            along += 3.6;
        }
    }

    let walls: Vec<_> = WALLS.iter().map(|c| matte(*c)).collect();
    let roofs: Vec<_> = ROOFS.iter().map(|c| matte(*c)).collect();
    let glass = matte(Color::srgb(0.62, 0.78, 0.90));
    let door = matte(Color::srgb(0.58, 0.44, 0.40));
    for (centre, half, height, wall, roof) in buildings() {
        let base = Vec3::new(centre.x, 0.12 + height / 2.0, centre.y);
        block(
            &mut commands,
            base,
            Vec3::new(half.x * 2.0, height, half.y * 2.0),
            walls[wall].clone(),
        );
        // A roof slab a little wider than the walls.
        block(
            &mut commands,
            Vec3::new(centre.x, 0.12 + height + 0.2, centre.y),
            Vec3::new(half.x * 2.0 + 0.5, 0.4, half.y * 2.0 + 0.5),
            roofs[roof].clone(),
        );
        // Bands of windows on the four sides, floor by floor, and a door.
        let mut floor = 2.6;
        while floor < height - 0.8 {
            block(
                &mut commands,
                Vec3::new(centre.x, 0.12 + floor, centre.y),
                Vec3::new(half.x * 2.0 + 0.06, 0.9, half.y * 1.5),
                glass.clone(),
            );
            block(
                &mut commands,
                Vec3::new(centre.x, 0.12 + floor, centre.y),
                Vec3::new(half.x * 1.5, 0.9, half.y * 2.0 + 0.06),
                glass.clone(),
            );
            floor += 2.0;
        }
        block(
            &mut commands,
            Vec3::new(centre.x, 0.12 + 0.9, centre.y),
            Vec3::new(1.1, 1.8, half.y * 2.0 + 0.08),
            door.clone(),
        );
    }

    let post = matte(Color::srgb(0.45, 0.48, 0.58));
    let hedge = matte(Color::srgb(0.62, 0.82, 0.66));
    let bulb = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.96, 0.78),
        emissive: LinearRgba::rgb(1.6, 1.4, 0.8),
        ..default()
    });
    for at in lamps() {
        block(
            &mut commands,
            Vec3::new(at.x, 1.7, at.y),
            Vec3::new(0.2, 3.4, 0.2),
            post.clone(),
        );
        block(
            &mut commands,
            Vec3::new(at.x, 3.55, at.y),
            Vec3::new(0.5, 0.35, 0.5),
            bulb.clone(),
        );
    }

    for (centre, size) in [
        (Vec3::new(0.0, 0.6, -EDGE), Vec3::new(side + 1.0, 1.2, 1.0)),
        (Vec3::new(0.0, 0.6, EDGE), Vec3::new(side + 1.0, 1.2, 1.0)),
        (Vec3::new(-EDGE, 0.6, 0.0), Vec3::new(1.0, 1.2, side + 1.0)),
        (Vec3::new(EDGE, 0.6, 0.0), Vec3::new(1.0, 1.2, side + 1.0)),
    ] {
        block(&mut commands, centre, size, hedge.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_walk_starts_in_the_open() {
        let solids = Solids(solids());
        assert_eq!(solids.hit(Vec2::new(START.x, START.z), 0.4), None);
    }

    #[test]
    fn streets_are_clear_and_lots_are_not() {
        let solids = Solids(solids());
        // Down the middle of a street, from hedge to hedge.
        let mut z = -EDGE + 2.0;
        while z < EDGE - 2.0 {
            assert_eq!(solids.hit(Vec2::new(PITCH / 2.0, z), 0.4), None, "z = {z}");
            z += 0.5;
        }
        // A building stands in each quarter of a lot.
        assert_eq!(
            solids.hit(Vec2::new(LOT / 4.0, LOT / 4.0), 0.4),
            Some(SolidKind::Building)
        );
        assert_eq!(solids.hit(Vec2::new(0.0, EDGE), 0.4), Some(SolidKind::Edge));
        assert_eq!(solids.hit(lamps()[0], 0.4), Some(SolidKind::Lamp));
    }

    #[test]
    fn buildings_stay_inside_their_lots() {
        for (centre, half, ..) in buildings() {
            let lot = (centre / PITCH).round() * PITCH;
            let reach = (centre - lot).abs() + half;
            assert!(reach.x <= LOT / 2.0 && reach.y <= LOT / 2.0, "{centre:?}");
        }
    }
}
