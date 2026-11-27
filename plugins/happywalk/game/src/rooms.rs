//! `RoomsPlugin`: inside places. The detective's office, the shops where
//! suspects work, the homes where they live.
//!
//! A room is never stored or loaded: it is built the moment the character
//! walks through its door, from the kind of place and the town's seed, as two
//! merged meshes (the shell, and the furniture), and thrown away as soon as
//! the character steps back out. Only one exists at a time, far from the
//! city (`ROOMS`), and while it does the city around the door is left as it
//! was: nothing streams, the camera looks into the room from its open front
//! like a doll's house.
//!
//! Which doors open is up to the case ([`Doors`]): the office, and the places
//! of the people the detective may want to see.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::city::{CitySeed, Solids};
use crate::gen::{self, PlaceKind, Solid, SolidKind};
use crate::player::Player;
use crate::vox::{Geometry, VoxLibrary};

/// Where rooms are built: far from the city and from the studio.
pub const ROOMS: Vec3 = Vec3::new(-20000.0, 0.0, -20000.0);
/// Within this many metres of a door the character goes in.
const DOOR: f32 = 0.9;
/// How high the walls are.
const WALL: f32 = 2.9;

/// What kind of place a room is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoomKind {
    Office,
    Home,
    Shop(PlaceKind),
}

/// A place that can be entered: what it is, and its door on the street.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Space {
    pub kind: RoomKind,
    /// The point in front of the door that takes the character in.
    pub door: Vec2,
    /// Which way out of the door is (towards the street), as a facing.
    pub out_facing: f32,
    /// A number of its own, so two homes are not furnished alike.
    pub salt: u64,
}

impl Space {
    /// Where the character stands after coming out: a step from the door.
    pub fn outside(&self) -> Vec2 {
        self.door + Vec2::new(-self.out_facing.sin(), -self.out_facing.cos()) * 1.4
    }
}

/// The doors that open now.
#[derive(Resource, Default, Debug)]
pub struct Doors(pub Vec<Space>);

/// The room the character is in, if any.
#[derive(Resource, Default, Debug)]
pub struct Inside(pub Option<Room>);

#[derive(Clone, Debug)]
pub struct Room {
    pub space: Space,
    /// Half its floor, in metres (`x` across, `y` deep).
    pub half: Vec2,
    /// Where whoever is at home stands, and which way they face.
    pub host: (Vec2, f32),
}

impl Room {
    /// A point of the room, in the world.
    pub fn at(&self, local: Vec2) -> Vec2 {
        Vec2::new(ROOMS.x, ROOMS.z) + local
    }

    /// The doormat, on the open front: stepping on it goes out.
    pub fn mat(&self) -> Vec2 {
        self.at(Vec2::new(0.0, self.half.y - 0.25))
    }
}

/// The character went into a room, or came out of one.
#[derive(Message, Clone, Copy, Debug)]
pub enum Passage {
    In(Space),
    Out(Space),
}

#[derive(Component)]
struct RoomPart;

pub struct RoomsPlugin;

impl Plugin for RoomsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Doors>()
            .init_resource::<Inside>()
            .add_message::<Passage>()
            .add_message::<GoIn>()
            .add_systems(
                Update,
                pass.after(crate::player::PlayerSet)
                    .before(crate::mystery::MysterySet),
            );
    }
}

/// Go into a space at once (the game starts in the office).
#[derive(Message, Clone, Copy, Debug)]
pub struct GoIn(pub Space);

/// The door of the building a place belongs to: in front of its facade,
/// on the pavement. `None` when the chunk has no building there.
pub fn space_of(
    seed: u64,
    place: &gen::Place,
    library: &VoxLibrary,
    kind: RoomKind,
) -> Option<Space> {
    let chunk = gen::chunk(seed, gen::chunk_of(place.at));
    let lot = gen::centre(chunk.coord);
    // The place stands on the street before its building: the building is the
    // one on the same side of the lot, at the same `x`.
    let facing = (place.at.y - lot.y).signum();
    let building = chunk.props.iter().find(|p| {
        p.solid.is_some_and(|(k, _)| k == SolidKind::Building)
            && (p.at.x - place.at.x).abs() < 0.5
            && (p.at.y - lot.y).signum() == facing
    })?;
    let size = library.size(building.model).as_vec3() * building.voxel;
    let depth = if building.quarter_turns % 2 == 0 {
        size.z
    } else {
        size.x
    };
    let door = Vec2::new(building.at.x, building.at.y + facing * (depth / 2.0 + 0.55));
    let out_facing = if facing > 0.0 {
        std::f32::consts::PI
    } else {
        0.0
    };
    let salt = ((door.x.to_bits() as u64) << 32) ^ door.y.to_bits() as u64 ^ seed;
    Some(Space {
        kind,
        door,
        out_facing,
        salt,
    })
}

/// The detective's office: the home nearest the start of the walk.
pub fn office(seed: u64, library: &VoxLibrary) -> Option<Space> {
    let start = Vec2::new(crate::city::START.x, crate::city::START.z);
    let place = gen::nearest_place(seed, start, PlaceKind::Home, 4)?;
    space_of(seed, &place, library, RoomKind::Office)
}

/// Walk through doors: in through an open one, out over the doormat.
#[allow(clippy::too_many_arguments)]
fn pass(
    mut commands: Commands,
    seed: Res<CitySeed>,
    doors: Res<Doors>,
    wardrobe: Res<crate::bridge::Wardrobe>,
    library: Res<VoxLibrary>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut inside: ResMut<Inside>,
    mut solids: ResMut<Solids>,
    mut go_in: MessageReader<GoIn>,
    mut passages: MessageWriter<Passage>,
    mut route: ResMut<crate::nav::Route>,
    mut players: Query<(&mut Player, &mut Transform)>,
    parts: Query<Entity, With<RoomPart>>,
) {
    let Ok((mut player, mut transform)) = players.single_mut() else {
        return;
    };
    if wardrobe.0 {
        return;
    }
    let at = Vec2::new(transform.translation.x, transform.translation.z);
    let mut enter = go_in.read().last().map(|g| g.0);
    // A new town: out of any room, onto its street.
    if seed.is_changed() && !seed.is_added() {
        if let Some(room) = inside.0.take() {
            for part in &parts {
                commands.entity(part).despawn();
            }
            solids.set_room(Vec::new());
            passages.write(Passage::Out(room.space));
        }
    }
    match &inside.0 {
        Some(room) => {
            if enter.is_none() && at.distance(room.mat()) < DOOR * 0.7 {
                let space = room.space;
                for part in &parts {
                    commands.entity(part).despawn();
                }
                solids.set_room(Vec::new());
                inside.0 = None;
                let out = space.outside();
                transform.translation = Vec3::new(out.x, 0.0, out.y);
                (player.heading, player.facing, player.walking) =
                    (space.out_facing, space.out_facing, false);
                *route = crate::nav::Route::default();
                passages.write(Passage::Out(space));
                return;
            }
        }
        None => {
            if enter.is_none() {
                enter = doors.0.iter().find(|d| d.door.distance(at) < DOOR).copied();
            }
        }
    }
    let Some(space) = enter else { return };
    for part in &parts {
        commands.entity(part).despawn();
    }
    let built = build(space);
    let origin = Vec3::new(ROOMS.x, 0.0, ROOMS.z);
    for (geometry, casts) in [(built.shell, false), (built.furniture, true)] {
        let mut part = commands.spawn((
            RoomPart,
            Mesh3d(meshes.add(geometry.mesh())),
            MeshMaterial3d(library.material.clone()),
            Transform::from_translation(origin),
            crate::vox::Triangles(geometry.triangles() as u32),
        ));
        if !casts {
            part.insert(NotShadowCaster);
        }
    }
    solids.set_room(
        built
            .solids
            .iter()
            .map(|s| Solid {
                min: s.min + Vec2::new(ROOMS.x, ROOMS.z),
                max: s.max + Vec2::new(ROOMS.x, ROOMS.z),
                ..*s
            })
            .collect(),
    );
    let room = Room {
        space,
        half: built.half,
        host: built.host,
    };
    // Just inside, looking in.
    let start = room.at(Vec2::new(0.0, built.half.y - 1.3));
    transform.translation = Vec3::new(start.x, 0.0, start.y);
    (player.heading, player.facing, player.walking) = (0.0, 0.0, false);
    // A walk to the door ends at the door.
    *route = crate::nav::Route::default();
    inside.0 = Some(room);
    passages.write(Passage::In(space));
}

/// A room, built: its shell (floor, walls, the ground around), its
/// furniture, what cannot be walked through, and where its host stands.
pub struct Built {
    pub shell: Geometry,
    pub furniture: Geometry,
    pub solids: Vec<Solid>,
    pub half: Vec2,
    pub host: (Vec2, f32),
}

fn rgb(r: u8, g: u8, b: u8) -> [f32; 4] {
    let c = Color::srgb_u8(r, g, b).to_linear();
    [c.red, c.green, c.blue, 1.0]
}

/// A number from a salt, 0..n.
fn pick(salt: u64, what: u64, n: usize) -> usize {
    let mut z = salt ^ what.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((z ^ (z >> 31)) % n as u64) as usize
}

const WALLS: [(u8, u8, u8); 6] = [
    (247, 226, 200),
    (220, 236, 226),
    (232, 222, 246),
    (250, 220, 214),
    (214, 230, 246),
    (248, 240, 200),
];
const FLOORS: [(u8, u8, u8); 4] = [
    (196, 146, 104),
    (178, 128, 92),
    (226, 214, 196),
    (204, 188, 214),
];
const FABRICS: [(u8, u8, u8); 5] = [
    (226, 104, 112),
    (92, 146, 214),
    (96, 180, 136),
    (240, 180, 70),
    (170, 120, 200),
];

/// Furniture, as boxes: the room's own furniture maker.
struct Maker {
    furniture: Geometry,
    solids: Vec<Solid>,
}

impl Maker {
    /// A box standing on the floor at `at` (its middle), `size` big; solid
    /// when it is more than ankle high.
    fn put(&mut self, at: Vec2, size: Vec3, colour: (u8, u8, u8)) {
        self.lay(at, 0.0, size, colour);
    }

    /// A box at `at`, its bottom `lift` above the floor.
    fn lay(&mut self, at: Vec2, lift: f32, size: Vec3, colour: (u8, u8, u8)) {
        let half = Vec2::new(size.x, size.z) / 2.0;
        self.furniture.block(
            Vec3::new(at.x - half.x, lift, at.y - half.y),
            Vec3::new(at.x + half.x, lift + size.y, at.y + half.y),
            rgb(colour.0, colour.1, colour.2),
        );
        if lift < 0.3 && size.y > 0.25 {
            self.solids.push(Solid::around(at, half, SolidKind::Bench));
        }
    }

    /// Something flat on the floor (a rug), never in the way.
    fn rug(&mut self, at: Vec2, size: Vec2, colour: (u8, u8, u8)) {
        self.furniture.floor(
            at - size / 2.0,
            at + size / 2.0,
            0.012,
            rgb(colour.0, colour.1, colour.2),
        );
    }

    /// A rectangle on the back wall (a window, a picture), `y` its middle.
    fn on_wall(&mut self, back: f32, x: f32, y: f32, size: Vec2, colour: (u8, u8, u8)) {
        self.furniture.block(
            Vec3::new(x - size.x / 2.0, y - size.y / 2.0, back),
            Vec3::new(x + size.x / 2.0, y + size.y / 2.0, back + 0.05),
            rgb(colour.0, colour.1, colour.2),
        );
    }
}

/// Build the room of a space.
pub fn build(space: Space) -> Built {
    let s = space.salt;
    let half = match space.kind {
        RoomKind::Office => Vec2::new(4.0, 3.2),
        RoomKind::Home => Vec2::new(4.2, 3.4),
        RoomKind::Shop(PlaceKind::Cafe) => Vec2::new(4.4, 3.4),
        RoomKind::Shop(_) => Vec2::new(3.8, 3.0),
    };
    let wall = match space.kind {
        RoomKind::Office => (240, 226, 196),
        _ => WALLS[pick(s, 1, WALLS.len())],
    };
    let floor = match space.kind {
        RoomKind::Office => (150, 104, 76),
        _ => FLOORS[pick(s, 2, FLOORS.len())],
    };
    let mut shell = Geometry::default();
    // The table it stands on: a wide pale ground, so the room reads as a
    // model on a table rather than floating in the sky.
    shell.floor(
        Vec2::splat(-24.0),
        Vec2::splat(24.0),
        -0.02,
        rgb(222, 218, 232),
    );
    shell.floor(-half, half, 0.0, rgb(floor.0, floor.1, floor.2));
    let w = rgb(wall.0, wall.1, wall.2);
    let t = 0.2;
    // The back wall and the two sides, full height; the front is open but
    // for a low wall each side of the door.
    shell.block(
        Vec3::new(-half.x - t, 0.0, -half.y - t),
        Vec3::new(half.x + t, WALL, -half.y),
        w,
    );
    shell.block(
        Vec3::new(-half.x - t, 0.0, -half.y),
        Vec3::new(-half.x, WALL, half.y),
        w,
    );
    shell.block(
        Vec3::new(half.x, 0.0, -half.y),
        Vec3::new(half.x + t, WALL, half.y),
        w,
    );
    let gap = 0.75;
    shell.block(
        Vec3::new(-half.x - t, 0.0, half.y),
        Vec3::new(-gap, 0.5, half.y + t),
        w,
    );
    shell.block(
        Vec3::new(gap, 0.0, half.y),
        Vec3::new(half.x + t, 0.5, half.y + t),
        w,
    );
    // The doormat.
    shell.floor(
        Vec2::new(-0.6, half.y - 0.5),
        Vec2::new(0.6, half.y),
        0.014,
        rgb(120, 96, 80),
    );
    let mut solids = vec![
        Solid::around(
            Vec2::new(0.0, -half.y - 0.1),
            Vec2::new(half.x + 0.2, 0.1),
            SolidKind::Building,
        ),
        Solid::around(
            Vec2::new(-half.x - 0.1, 0.0),
            Vec2::new(0.1, half.y),
            SolidKind::Building,
        ),
        Solid::around(
            Vec2::new(half.x + 0.1, 0.0),
            Vec2::new(0.1, half.y),
            SolidKind::Building,
        ),
        Solid {
            min: Vec2::new(-half.x, half.y),
            max: Vec2::new(-gap, half.y + 0.2),
            kind: SolidKind::Building,
            model: "",
        },
        Solid {
            min: Vec2::new(gap, half.y),
            max: Vec2::new(half.x, half.y + 0.2),
            kind: SolidKind::Building,
            model: "",
        },
    ];
    let mut m = Maker {
        furniture: Geometry::default(),
        solids: Vec::new(),
    };
    let back = -half.y;
    let host = match space.kind {
        RoomKind::Office => {
            // The desk, the detective's chair, a pin board with the case.
            m.put(
                Vec2::new(0.0, -1.2),
                Vec3::new(2.2, 0.78, 0.95),
                (122, 82, 58),
            );
            m.lay(
                Vec2::new(-0.5, -1.25),
                0.78,
                Vec3::new(0.5, 0.02, 0.35),
                (250, 248, 236),
            );
            m.lay(
                Vec2::new(0.6, -1.3),
                0.78,
                Vec3::new(0.25, 0.3, 0.2),
                (70, 74, 96),
            );
            m.put(
                Vec2::new(0.0, -2.2),
                Vec3::new(0.62, 0.5, 0.6),
                (96, 64, 48),
            );
            m.lay(
                Vec2::new(0.0, -2.48),
                0.5,
                Vec3::new(0.62, 0.6, 0.1),
                (96, 64, 48),
            );
            m.on_wall(back, 0.0, 1.7, Vec2::new(2.0, 1.1), (206, 166, 118));
            for (i, (x, y)) in [
                (-0.6, 1.9),
                (0.1, 1.5),
                (0.7, 1.95),
                (-0.2, 2.05),
                (0.5, 1.4),
            ]
            .iter()
            .enumerate()
            {
                let c = [
                    (250, 248, 236),
                    (246, 224, 160),
                    (250, 248, 236),
                    (220, 236, 250),
                    (250, 248, 236),
                ][i];
                m.lay(
                    Vec2::new(*x, back + 0.04),
                    *y - 0.15,
                    Vec3::new(0.34, 0.3, 0.03),
                    c,
                );
            }
            // Filing cabinets, a bookcase, a plant, a coat stand, a rug.
            m.put(
                Vec2::new(-half.x + 0.45, back + 0.4),
                Vec3::new(0.8, 1.35, 0.6),
                (128, 158, 146),
            );
            m.put(
                Vec2::new(-half.x + 1.3, back + 0.4),
                Vec3::new(0.8, 1.35, 0.6),
                (128, 158, 146),
            );
            m.put(
                Vec2::new(half.x - 0.9, back + 0.25),
                Vec3::new(1.5, 2.1, 0.45),
                (122, 82, 58),
            );
            for shelf in 0..4 {
                for b in 0..6 {
                    let c = FABRICS[(b + shelf) % FABRICS.len()];
                    m.lay(
                        Vec2::new(half.x - 1.5 + b as f32 * 0.22, back + 0.3),
                        0.15 + shelf as f32 * 0.5,
                        Vec3::new(0.16, 0.36, 0.3),
                        c,
                    );
                }
            }
            m.put(
                Vec2::new(-half.x + 0.5, half.y - 0.9),
                Vec3::new(0.5, 0.45, 0.5),
                (200, 110, 80),
            );
            m.lay(
                Vec2::new(-half.x + 0.5, half.y - 0.9),
                0.45,
                Vec3::new(0.7, 0.8, 0.7),
                (72, 160, 90),
            );
            m.put(
                Vec2::new(half.x - 0.5, half.y - 1.0),
                Vec3::new(0.12, 1.8, 0.12),
                (96, 64, 48),
            );
            m.lay(
                Vec2::new(half.x - 0.5, half.y - 1.0),
                1.45,
                Vec3::new(0.45, 0.4, 0.3),
                (110, 120, 150),
            );
            m.rug(Vec2::new(0.0, 0.6), Vec2::new(3.0, 2.0), (178, 72, 80));
            // A window on the back wall, left.
            m.on_wall(back, -2.4, 1.7, Vec2::new(1.1, 1.0), (170, 214, 246));
            // Whoever comes in stands before the desk.
            (Vec2::new(0.9, 0.2), 0.0)
        }
        RoomKind::Home => {
            let sofa = FABRICS[pick(s, 3, FABRICS.len())];
            m.put(
                Vec2::new(-1.6, back + 0.55),
                Vec3::new(2.2, 0.45, 0.9),
                sofa,
            );
            m.lay(
                Vec2::new(-1.6, back + 0.15),
                0.45,
                Vec3::new(2.2, 0.5, 0.2),
                sofa,
            );
            m.put(
                Vec2::new(-1.6, back + 1.6),
                Vec3::new(1.2, 0.4, 0.6),
                (150, 104, 76),
            );
            m.rug(
                Vec2::new(-1.6, back + 1.4),
                Vec2::new(2.8, 1.8),
                FABRICS[pick(s, 4, FABRICS.len())],
            );
            // A table and two chairs, a lamp, a bookcase, a plant.
            m.put(
                Vec2::new(2.0, -0.2),
                Vec3::new(1.2, 0.75, 0.9),
                (196, 146, 104),
            );
            m.put(
                Vec2::new(2.0, -1.0),
                Vec3::new(0.45, 0.45, 0.45),
                (122, 82, 58),
            );
            m.put(
                Vec2::new(2.0, 0.6),
                Vec3::new(0.45, 0.45, 0.45),
                (122, 82, 58),
            );
            m.put(
                Vec2::new(half.x - 0.3, back + 0.3),
                Vec3::new(0.12, 1.6, 0.12),
                (70, 74, 96),
            );
            m.lay(
                Vec2::new(half.x - 0.3, back + 0.3),
                1.5,
                Vec3::new(0.45, 0.35, 0.45),
                (250, 230, 160),
            );
            m.put(
                Vec2::new(0.9, back + 0.25),
                Vec3::new(1.2, 1.9, 0.4),
                (150, 104, 76),
            );
            for shelf in 0..3 {
                for b in 0..5 {
                    m.lay(
                        Vec2::new(0.5 + b as f32 * 0.2, back + 0.3),
                        0.2 + shelf as f32 * 0.55,
                        Vec3::new(0.15, 0.36, 0.3),
                        FABRICS[(b + shelf + pick(s, 5, 5)) % FABRICS.len()],
                    );
                }
            }
            m.put(
                Vec2::new(-half.x + 0.45, half.y - 0.9),
                Vec3::new(0.45, 0.4, 0.45),
                (200, 110, 80),
            );
            m.lay(
                Vec2::new(-half.x + 0.45, half.y - 0.9),
                0.4,
                Vec3::new(0.6, 0.7, 0.6),
                (72, 160, 90),
            );
            m.on_wall(
                back,
                -1.6,
                1.8,
                Vec2::new(1.0, 0.7),
                FABRICS[pick(s, 6, FABRICS.len())],
            );
            m.on_wall(back, 2.4, 1.7, Vec2::new(1.0, 1.0), (170, 214, 246));
            (Vec2::new(0.4, -0.6), 0.0)
        }
        RoomKind::Shop(kind) => {
            // A counter across the back, and the wares of the trade.
            let counter = match kind {
                PlaceKind::Bakery => (226, 170, 120),
                PlaceKind::Florist => (120, 170, 130),
                PlaceKind::Cafe => (122, 82, 58),
                PlaceKind::Grocer => (196, 146, 104),
                _ => (92, 146, 214),
            };
            m.put(
                Vec2::new(0.0, back + 1.4),
                Vec3::new(3.0, 1.0, 0.7),
                counter,
            );
            m.lay(
                Vec2::new(0.0, back + 1.4),
                1.0,
                Vec3::new(3.1, 0.06, 0.8),
                (250, 248, 236),
            );
            let shelf = |m: &mut Maker, colours: &[(u8, u8, u8)], size: Vec3| {
                m.put(
                    Vec2::new(0.0, back + 0.25),
                    Vec3::new(half.x * 2.0 - 0.6, 2.0, 0.45),
                    (196, 160, 120),
                );
                for row in 0..3 {
                    for i in 0..((half.x * 2.0 - 0.9) / (size.x + 0.08)) as usize {
                        let x = -half.x + 0.6 + i as f32 * (size.x + 0.08);
                        m.lay(
                            Vec2::new(x, back + 0.32),
                            0.35 + row as f32 * 0.6,
                            size,
                            colours[(i + row) % colours.len()],
                        );
                    }
                }
            };
            match kind {
                PlaceKind::Bakery => {
                    shelf(
                        &mut m,
                        &[(214, 160, 90), (196, 130, 70), (236, 196, 130)],
                        Vec3::new(0.4, 0.2, 0.25),
                    );
                    for i in 0..5 {
                        m.lay(
                            Vec2::new(-1.2 + i as f32 * 0.6, back + 1.4),
                            1.06,
                            Vec3::new(0.35, 0.12, 0.25),
                            (214, 160, 90),
                        );
                    }
                }
                PlaceKind::Florist => {
                    shelf(
                        &mut m,
                        &[
                            (246, 110, 160),
                            (250, 200, 80),
                            (170, 120, 200),
                            (240, 240, 250),
                        ],
                        Vec3::new(0.3, 0.3, 0.3),
                    );
                    for (i, x) in [-half.x + 0.5, half.x - 0.5].iter().enumerate() {
                        for j in 0..3 {
                            let at = Vec2::new(*x, -0.4 + j as f32 * 0.8);
                            m.put(at, Vec3::new(0.45, 0.4, 0.45), (92, 146, 214));
                            m.lay(
                                at,
                                0.4,
                                Vec3::new(0.5, 0.35, 0.5),
                                FABRICS[(i + j) % FABRICS.len()],
                            );
                        }
                    }
                }
                PlaceKind::Cafe => {
                    shelf(
                        &mut m,
                        &[(250, 248, 236), (120, 84, 60), (240, 180, 70)],
                        Vec3::new(0.18, 0.28, 0.18),
                    );
                    m.lay(
                        Vec2::new(0.9, back + 1.4),
                        1.06,
                        Vec3::new(0.5, 0.5, 0.4),
                        (170, 176, 196),
                    );
                    for (x, z) in [(-2.4, 0.6), (2.4, 0.6), (-2.4, 2.0)] {
                        m.put(Vec2::new(x, z), Vec3::new(0.12, 0.72, 0.12), (70, 74, 96));
                        m.lay(
                            Vec2::new(x, z),
                            0.72,
                            Vec3::new(0.8, 0.05, 0.8),
                            (250, 248, 236),
                        );
                        for side in [-0.6, 0.6] {
                            m.put(
                                Vec2::new(x + side, z),
                                Vec3::new(0.38, 0.45, 0.38),
                                FABRICS[pick(s, 7, 5)],
                            );
                        }
                    }
                }
                PlaceKind::Grocer => {
                    shelf(
                        &mut m,
                        &[
                            (226, 80, 70),
                            (250, 180, 60),
                            (110, 196, 110),
                            (240, 220, 90),
                        ],
                        Vec3::new(0.3, 0.25, 0.3),
                    );
                    for (i, x) in [-2.6, -1.8, 1.8, 2.6].iter().enumerate() {
                        let at = Vec2::new(*x, 0.6);
                        m.put(at, Vec3::new(0.7, 0.5, 0.5), (196, 146, 104));
                        m.lay(
                            at,
                            0.5,
                            Vec3::new(0.6, 0.12, 0.4),
                            [
                                (226, 80, 70),
                                (250, 180, 60),
                                (110, 196, 110),
                                (150, 90, 170),
                            ][i],
                        );
                    }
                }
                _ => {
                    shelf(
                        &mut m,
                        &[
                            (226, 104, 112),
                            (92, 146, 214),
                            (250, 248, 236),
                            (240, 180, 70),
                        ],
                        Vec3::new(0.32, 0.42, 0.06),
                    );
                }
            }
            m.rug(Vec2::new(0.0, 1.3), Vec2::new(1.6, 1.0), (178, 72, 80));
            m.on_wall(
                back,
                -half.x + 0.9,
                2.45,
                Vec2::new(1.2, 0.35),
                (250, 248, 236),
            );
            // Beside the counter, facing the door: in full view of the
            // camera, not hidden behind the character.
            (Vec2::new(-half.x + 1.2, back + 2.3), 0.0)
        }
    };
    solids.extend(m.solids);
    Built {
        shell,
        furniture: m.furniture,
        solids,
        half,
        host,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_kind() -> Vec<RoomKind> {
        let mut kinds = vec![RoomKind::Office, RoomKind::Home];
        kinds.extend(PlaceKind::ALL.iter().map(|k| RoomKind::Shop(*k)));
        kinds
    }

    #[test]
    fn every_room_leaves_a_way_from_the_door_to_its_host() {
        for kind in every_kind() {
            for salt in 0..20 {
                let built = build(Space {
                    kind,
                    door: Vec2::ZERO,
                    out_facing: 0.0,
                    salt,
                });
                let blocked = |p: Vec2| built.solids.iter().any(|s| s.touches(p, 0.38));
                let start = Vec2::new(0.0, built.half.y - 1.3);
                assert!(!blocked(start), "{kind:?}: the way in is blocked");
                assert!(
                    !blocked(Vec2::new(0.0, built.half.y - 0.25)),
                    "{kind:?}: the doormat"
                );
                // Somewhere near the host is free to stand and talk.
                let (host, _) = built.host;
                let near = (0..16).any(|i| {
                    let a = i as f32 / 16.0 * std::f32::consts::TAU;
                    let p = host + Vec2::new(a.cos(), a.sin()) * 1.6;
                    !blocked(p) && p.y < built.half.y
                });
                assert!(near, "{kind:?}: nowhere to stand by the host");
                assert!(built.furniture.triangles() > 50);
            }
        }
    }
}
