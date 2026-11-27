//! The city generator: from a seed and a place on the grid to what stands
//! there. Pure functions, no engine: the same seed gives the same city on
//! every machine and in every test.
//!
//! The city is an endless grid of **chunks**, one lot each with the streets
//! along it. A chunk's lot is of a [`LotKind`] (homes, shops, a park, a
//! square), chosen by the **district** it lies in, which changes slowly
//! across the map. What a lot holds is a list of [`Prop`]s; some lots are
//! also **places** with a name the character and the errands can use (the
//! bakery, the park).
//!
//! To add a kind of lot: add it to [`LotKind`], give it a weight per district
//! in [`District::weights`], and write its layout function.

use bevy::math::{IVec2, UVec3, Vec2};
use std::collections::HashMap;

/// Lots are `PITCH` apart with pavements `PAVED` wide, so the streets are
/// `PITCH - PAVED` wide and run along `(k + 0.5) * PITCH`.
pub const PITCH: f32 = 22.0;
pub const PAVED: f32 = 16.8;
/// How high a pavement stands above the street.
pub const PAVEMENT: f32 = 0.12;

/// What kind of thing a solid is (what the character says differs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SolidKind {
    Building,
    Lamp,
    Tree,
    Bench,
    Planter,
    Fountain,
    Kiosk,
    /// A passer-by.
    Person,
    Car,
    Bike,
}

/// A box on the ground, as seen from above (`x`, `z`).
#[derive(Clone, Copy, Debug)]
pub struct Solid {
    pub min: Vec2,
    pub max: Vec2,
    pub kind: SolidKind,
    /// The model it is, for describing it (empty when it is none).
    pub model: &'static str,
}

impl Solid {
    pub fn around(centre: Vec2, half: Vec2, kind: SolidKind) -> Self {
        Self {
            min: centre - half,
            max: centre + half,
            kind,
            model: "",
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

/// A voxel model standing somewhere.
#[derive(Clone, Debug, PartialEq)]
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
    pub fn new(model: &'static str, at: Vec2, voxel: f32) -> Self {
        Self {
            model,
            at,
            lift: PAVEMENT,
            quarter_turns: 0,
            voxel,
            solid: None,
        }
    }

    pub fn turned(mut self, quarter_turns: i32) -> Self {
        self.quarter_turns = quarter_turns;
        self
    }

    /// Solid over its whole footprint.
    pub fn solid(mut self, kind: SolidKind) -> Self {
        self.solid = Some((kind, None));
        self
    }

    /// Solid only around its foot (a tree's trunk, not its crown).
    pub fn solid_foot(mut self, kind: SolidKind, half: f32) -> Self {
        self.solid = Some((kind, Some(Vec2::splat(half))));
        self
    }

    /// Whether it is worth drawing in a chunk seen from a distance.
    pub fn shows_from_afar(&self) -> bool {
        matches!(
            self.solid,
            Some((
                SolidKind::Building | SolidKind::Tree | SolidKind::Lamp | SolidKind::Kiosk,
                _
            ))
        )
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
        Some(Solid {
            model: self.model,
            ..Solid::around(self.at, half.unwrap_or(own), kind)
        })
    }
}

/// A place with a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlaceKind {
    Bakery,
    Florist,
    Cafe,
    Grocer,
    Newsstand,
    Park,
    Square,
    /// Somebody's front door, in a lot of homes. Not a public place: not in
    /// [`PlaceKind::ALL`].
    Home,
}

impl PlaceKind {
    pub const ALL: [PlaceKind; 7] = [
        PlaceKind::Bakery,
        PlaceKind::Florist,
        PlaceKind::Cafe,
        PlaceKind::Grocer,
        PlaceKind::Newsstand,
        PlaceKind::Park,
        PlaceKind::Square,
    ];

    /// What to call it when describing the city in plain words.
    pub fn name(self) -> &'static str {
        match self {
            PlaceKind::Bakery => "the bakery",
            PlaceKind::Florist => "the florist",
            PlaceKind::Cafe => "the café",
            PlaceKind::Grocer => "the greengrocer",
            PlaceKind::Newsstand => "the newsstand",
            PlaceKind::Park => "the park",
            PlaceKind::Square => "the square",
            PlaceKind::Home => "home",
        }
    }

    /// Its name in the player's language (Italian or English).
    pub fn name_in(self, lang: &str) -> &'static str {
        if !lang.starts_with("it") {
            return self.name();
        }
        match self {
            PlaceKind::Bakery => "il forno",
            PlaceKind::Florist => "il fiorista",
            PlaceKind::Cafe => "il bar",
            PlaceKind::Grocer => "il fruttivendolo",
            PlaceKind::Newsstand => "l'edicola",
            PlaceKind::Park => "il parco",
            PlaceKind::Square => "la piazza",
            PlaceKind::Home => "casa",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub kind: PlaceKind,
    /// Where to stand to be "at" it: on the street in front of it.
    pub at: Vec2,
}

/// What a lot is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LotKind {
    Homes,
    Shops,
    Park,
    Square,
}

/// A part of town. Districts are a few chunks across and blend into each
/// other only in what their lots tend to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum District {
    Centre,
    Residential,
    Green,
}

impl District {
    /// Out of 100: homes, shops, park, square.
    fn weights(self) -> [(LotKind, u32); 4] {
        match self {
            District::Centre => [
                (LotKind::Shops, 50),
                (LotKind::Homes, 22),
                (LotKind::Square, 20),
                (LotKind::Park, 8),
            ],
            District::Residential => [
                (LotKind::Homes, 66),
                (LotKind::Shops, 16),
                (LotKind::Park, 13),
                (LotKind::Square, 5),
            ],
            District::Green => [
                (LotKind::Park, 40),
                (LotKind::Homes, 44),
                (LotKind::Shops, 10),
                (LotKind::Square, 6),
            ],
        }
    }
}

impl District {
    /// The houses of this part of town: each district has its own manner.
    fn homes(self) -> &'static [&'static str] {
        match self {
            // Warm walls, red roofs, balconies with pots, shutters.
            District::Centre => &["home_old_a", "home_old_b", "home_old_c"],
            // Cool colours, taller, flower boxes, a climber here and there.
            District::Residential => &["home_town_a", "home_town_b", "home_town_c"],
            // Low houses with plants up the walls and on the roofs.
            District::Green => &["home_garden_a", "home_garden_b", "home_garden_c"],
        }
    }
}

/// The shops: the building, the place it is, and what stands outside it.
const SHOPS: [(&str, PlaceKind, &str); 4] = [
    ("shop_bakery", PlaceKind::Bakery, "sign_board"),
    ("shop_florist", PlaceKind::Florist, "flower_buckets"),
    ("shop_cafe", PlaceKind::Cafe, "cafe_table"),
    ("shop_grocer", PlaceKind::Grocer, "crates"),
];

/// A number from the seed, a chunk and a purpose: the generator's only
/// source of chance.
pub fn chance(seed: u64, chunk: IVec2, salt: u32) -> u32 {
    // Each input is mixed in after the one before, through the full SplitMix64
    // finaliser. (Combining them first with XOR, as this did, made a chunk and
    // its mirror through the origin share numbers: the same people, and the
    // same lots, at (3, -19) and (-3, 19).)
    let mix = |mut z: u64| {
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut h = mix(seed.wrapping_add(0x9E37_79B9_7F4A_7C15));
    for part in [
        chunk.x as u32 as u64,
        chunk.y as u32 as u64 | 1 << 32,
        salt as u64 | 2 << 32,
    ] {
        h = mix(h ^ part.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    }
    (h >> 32) as u32
}

fn pick(seed: u64, chunk: IVec2, salt: u32, modulo: u32) -> u32 {
    chance(seed, chunk, salt) % modulo
}

/// The district a chunk lies in: smooth noise over cells four chunks wide.
pub fn district(seed: u64, chunk: IVec2) -> District {
    const CELL: f32 = 4.0;
    let p = Vec2::new(chunk.x as f32, chunk.y as f32) / CELL;
    let cell = p.floor();
    let f = p - cell;
    let corner = |dx: i32, dy: i32| {
        chance(
            seed,
            IVec2::new(cell.x as i32 + dx, cell.y as i32 + dy),
            900,
        ) as f32
            / u32::MAX as f32
    };
    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    let (u, v) = (smooth(f.x), smooth(f.y));
    let top = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * u;
    let bottom = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * u;
    match top + (bottom - top) * v {
        n if n < 0.38 => District::Green,
        n if n < 0.62 => District::Residential,
        _ => District::Centre,
    }
}

pub fn lot_kind(seed: u64, chunk: IVec2) -> LotKind {
    let mut roll = pick(seed, chunk, 1, 100);
    for (kind, weight) in district(seed, chunk).weights() {
        if roll < weight {
            return kind;
        }
        roll -= weight;
    }
    LotKind::Homes
}

/// The middle of a chunk's lot.
pub fn centre(chunk: IVec2) -> Vec2 {
    Vec2::new(chunk.x as f32, chunk.y as f32) * PITCH
}

/// The chunk a point on the ground is in.
pub fn chunk_of(at: Vec2) -> IVec2 {
    IVec2::new((at.x / PITCH).round() as i32, (at.y / PITCH).round() as i32)
}

/// Everything in one chunk.
#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    pub coord: IVec2,
    pub kind: LotKind,
    pub props: Vec<Prop>,
    pub places: Vec<Place>,
}

/// Generate a chunk.
pub fn chunk(seed: u64, coord: IVec2) -> Chunk {
    let kind = lot_kind(seed, coord);
    let lot = centre(coord);
    let mut out = Chunk {
        coord,
        kind,
        props: Vec::new(),
        places: Vec::new(),
    };
    let front = PAVED / 2.0 + 1.6;
    match kind {
        LotKind::Homes | LotKind::Shops => {
            for (i, corner) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
                .into_iter()
                .enumerate()
            {
                let salt = 10 + i as u32;
                // In a shopping lot most fronts are shops, not all.
                let shop = kind == LotKind::Shops && pick(seed, coord, salt + 20, 4) != 0;
                let at = lot + Vec2::new(corner.0, corner.1) * 3.5;
                // Fronts look away from the middle of the lot.
                let (turns, facing) = if corner.1 < 0.0 { (0, -1.0) } else { (2, 1.0) };
                let model = if shop {
                    let (model, place, outside) =
                        SHOPS[pick(seed, coord, salt, SHOPS.len() as u32) as usize];
                    out.places.push(Place {
                        kind: place,
                        at: Vec2::new(at.x, lot.y + facing * front),
                    });
                    // Its wares on the pavement, beside the door, towards
                    // the corner of the lot.
                    let beside =
                        Vec2::new(at.x + corner.0 * 2.1, lot.y + facing * (PAVED / 2.0 - 0.95));
                    out.props.push(
                        Prop::new(outside, beside, 0.1)
                            .turned(turns)
                            .solid(SolidKind::Bench),
                    );
                    model
                } else {
                    out.places.push(Place {
                        kind: PlaceKind::Home,
                        at: Vec2::new(at.x, lot.y + facing * front),
                    });
                    let homes = district(seed, coord).homes();
                    homes[pick(seed, coord, salt, homes.len() as u32) as usize]
                };
                out.props.push(
                    Prop::new(model, at, 0.25)
                        .turned(turns)
                        .solid(SolidKind::Building),
                );
            }
            street_furniture(seed, coord, &mut out.props);
        }
        LotKind::Park => {
            out.places.push(Place {
                kind: PlaceKind::Park,
                at: lot + Vec2::new(0.0, front),
            });
            // Trees on a loose grid, a fountain or flowers in the middle,
            // benches looking at it.
            for gx in -2i32..=2 {
                for gz in -2i32..=2 {
                    let salt = 100 + ((gx + 2) * 5 + gz + 2) as u32;
                    if (gx == 0 && gz == 0)
                        || pick(seed, coord, salt, 3) == 0
                        || (gx.abs() <= 1 && gz.abs() <= 1)
                    {
                        continue;
                    }
                    let jitter = Vec2::new(
                        pick(seed, coord, salt + 40, 9) as f32,
                        pick(seed, coord, salt + 80, 9) as f32,
                    ) * 0.12
                        - 0.5;
                    let tree = if pick(seed, coord, salt + 120, 2) == 0 {
                        "tree"
                    } else {
                        "tree_dark"
                    };
                    out.props.push(
                        Prop::new(
                            tree,
                            lot + Vec2::new(gx as f32, gz as f32) * 3.3 + jitter,
                            0.1875,
                        )
                        .solid_foot(SolidKind::Tree, 0.3),
                    );
                }
            }
            if pick(seed, coord, 2, 2) == 0 {
                out.props
                    .push(Prop::new("fountain", lot, 0.125).solid(SolidKind::Fountain));
            } else {
                out.props
                    .push(Prop::new("planter", lot, 0.1875).solid(SolidKind::Planter));
            }
            for (side, turns) in [
                (Vec2::X, 1),
                (Vec2::NEG_X, 3),
                (Vec2::Y, 0),
                (Vec2::NEG_Y, 2),
            ] {
                out.props.push(
                    Prop::new("bench", lot + side * 2.6, 0.125)
                        .turned(turns)
                        .solid(SolidKind::Bench),
                );
            }
        }
        LotKind::Square => {
            out.places.push(Place {
                kind: PlaceKind::Square,
                at: lot + Vec2::new(0.0, front),
            });
            out.props
                .push(Prop::new("fountain", lot, 0.1875).solid(SolidKind::Fountain));
            let kiosk = lot + Vec2::new(-4.8, -4.6);
            out.props
                .push(Prop::new("kiosk", kiosk, 0.125).solid(SolidKind::Kiosk));
            out.places.push(Place {
                kind: PlaceKind::Newsstand,
                at: Vec2::new(kiosk.x, lot.y - front),
            });
            for (corner, turns) in [((1.0, -1.0), 2), ((-1.0, 1.0), 0), ((1.0, 1.0), 0)] {
                let at = lot + Vec2::new(corner.0, corner.1) * 5.6;
                out.props
                    .push(Prop::new("planter", at, 0.1875).solid(SolidKind::Planter));
                out.props.push(
                    Prop::new("bench", at - Vec2::new(corner.0 * 2.2, 0.0), 0.125)
                        .turned(turns)
                        .solid(SolidKind::Bench),
                );
            }
            for x in [-1.0, 1.0] {
                out.props.push(
                    Prop::new("tree", lot + Vec2::new(x * 6.6, 0.0), 0.1875)
                        .solid_foot(SolidKind::Tree, 0.3),
                );
            }
        }
    }
    // A lamp post at two corners of every lot.
    // Back from the kerb, where passers-by walk.
    let reach = PAVED / 2.0 - 0.95;
    for corner in [Vec2::new(1.0, 1.0), Vec2::new(-1.0, -1.0)] {
        out.props
            .push(Prop::new("lamp", lot + corner * reach, 0.125).solid_foot(SolidKind::Lamp, 0.16));
    }
    out
}

/// Trees, benches and planters on the strip of pavement around buildings.
fn street_furniture(seed: u64, coord: IVec2, props: &mut Vec<Prop>) {
    let lot = centre(coord);
    // Clear of the kerb, where passers-by walk.
    let strip = PAVED / 2.0 - 0.95;
    for (i, (side, along)) in [
        (Vec2::X, Vec2::Y),
        (Vec2::NEG_X, Vec2::Y),
        (Vec2::Y, Vec2::X),
        (Vec2::NEG_Y, Vec2::X),
    ]
    .into_iter()
    .enumerate()
    {
        let salt = 50 + i as u32 * 4;
        if pick(seed, coord, salt, 5) != 0 {
            let tree = if pick(seed, coord, salt + 1, 2) == 0 {
                "tree"
            } else {
                "tree_dark"
            };
            props
                .push(Prop::new(tree, lot + side * strip, 0.1875).solid_foot(SolidKind::Tree, 0.3));
        }
        let turns = if side.x != 0.0 { 1 } else { 0 };
        match pick(seed, coord, salt + 2, 4) {
            0 => props.push(
                Prop::new("bench", lot + side * strip + along * 3.6, 0.125)
                    .turned(turns)
                    .solid(SolidKind::Bench),
            ),
            1 => props.push(
                Prop::new("planter", lot + side * strip - along * 3.9, 0.1)
                    .solid(SolidKind::Planter),
            ),
            _ => {}
        }
    }
}

/// The solids of a chunk, given the models' sizes.
pub fn chunk_solids(chunk: &Chunk, sizes: &HashMap<&'static str, UVec3>) -> Vec<Solid> {
    chunk
        .props
        .iter()
        .filter_map(|prop| prop.footprint(sizes.get(prop.model).copied().unwrap_or(UVec3::ONE)))
        .collect()
}

/// The nearest place of a kind to `from`, looking `reach` chunks around.
pub fn nearest_place(seed: u64, from: Vec2, kind: PlaceKind, reach: i32) -> Option<Place> {
    let home = chunk_of(from);
    let mut best: Option<Place> = None;
    for dx in -reach..=reach {
        for dz in -reach..=reach {
            for place in chunk(seed, home + IVec2::new(dx, dz)).places {
                if place.kind == kind
                    && best.is_none_or(|b| {
                        place.at.distance_squared(from) < b.at.distance_squared(from)
                    })
                {
                    best = Some(place);
                }
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn area(reach: i32) -> impl Iterator<Item = IVec2> {
        (-reach..=reach).flat_map(move |x| (-reach..=reach).map(move |z| IVec2::new(x, z)))
    }

    #[test]
    fn the_same_seed_is_the_same_city() {
        for coord in area(3) {
            assert_eq!(chunk(7, coord), chunk(7, coord));
        }
    }

    #[test]
    fn two_seeds_are_two_cities() {
        let kinds = |seed| area(4).map(|c| lot_kind(seed, c)).collect::<Vec<_>>();
        assert_ne!(kinds(1), kinds(2));
        let different = area(4)
            .filter(|c| chunk(1, *c).props != chunk(2, *c).props)
            .count();
        assert!(different > 60, "{different}");
    }

    #[test]
    fn a_city_has_every_kind_of_lot_and_place() {
        for seed in [1u64, 2, 2026] {
            let lots: HashSet<_> = area(8).map(|c| lot_kind(seed, c)).collect();
            assert_eq!(lots.len(), 4, "seed {seed}: {lots:?}");
            let districts: HashSet<_> = area(8).map(|c| district(seed, c)).collect();
            assert_eq!(districts.len(), 3, "seed {seed}: {districts:?}");
            for kind in PlaceKind::ALL {
                assert!(
                    nearest_place(seed, Vec2::ZERO, kind, 8).is_some(),
                    "seed {seed}: no {kind:?}"
                );
            }
        }
    }

    #[test]
    fn streets_are_always_clear() {
        let sizes = crate::vox::sizes();
        for seed in [1u64, 99] {
            for coord in area(5) {
                let chunk = chunk(seed, coord);
                let lot = centre(coord);
                for solid in chunk_solids(&chunk, &sizes) {
                    let reach = (solid.max - lot).abs().max((solid.min - lot).abs());
                    assert!(
                        reach.x <= PAVED / 2.0 && reach.y <= PAVED / 2.0,
                        "seed {seed} {coord}: {solid:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn props_use_models_that_exist_and_do_not_stand_in_each_other() {
        let sizes = crate::vox::sizes();
        for coord in area(5) {
            let chunk = chunk(3, coord);
            for prop in &chunk.props {
                assert!(sizes.contains_key(prop.model), "no model {}", prop.model);
            }
            let solids = chunk_solids(&chunk, &sizes);
            for (i, a) in solids.iter().enumerate() {
                for b in &solids[i + 1..] {
                    let apart = a.max.x <= b.min.x
                        || b.max.x <= a.min.x
                        || a.max.y <= b.min.y
                        || b.max.y <= a.min.y;
                    assert!(apart, "{coord} ({:?}): {a:?} in {b:?}", chunk.kind);
                }
            }
        }
    }

    #[test]
    fn places_are_reached_from_the_street() {
        let sizes = crate::vox::sizes();
        for coord in area(6) {
            let chunk = chunk(5, coord);
            let solids = chunk_solids(&chunk, &sizes);
            for place in &chunk.places {
                assert!(
                    !solids.iter().any(|s| s.touches(place.at, 0.4)),
                    "{coord}: {place:?}"
                );
                let off = (place.at - centre(coord)).abs();
                assert!(
                    off.x > PAVED / 2.0 || off.y > PAVED / 2.0,
                    "{place:?} is not on a street"
                );
            }
        }
    }

    #[test]
    fn no_two_places_share_their_numbers() {
        // Over a wide stretch of city, every chunk's number is its own: no
        // mirror images, no repeating rows.
        let mut seen = std::collections::HashMap::new();
        for x in -40..40 {
            for z in -40..40 {
                let coord = IVec2::new(x, z);
                if let Some(other) = seen.insert(chance(7, coord, 713), coord) {
                    panic!("{coord} and {other} share a number");
                }
            }
        }
        assert_ne!(
            chance(7, IVec2::new(3, -19), 1),
            chance(7, IVec2::new(-3, 19), 1)
        );
        assert_ne!(
            chance(7, IVec2::new(2, 5), 1),
            chance(7, IVec2::new(5, 2), 1)
        );
        assert_ne!(chance(7, IVec2::ZERO, 1), chance(8, IVec2::ZERO, 1));
    }

    #[test]
    fn chunks_tile_the_ground() {
        assert_eq!(chunk_of(Vec2::new(0.0, 0.0)), IVec2::ZERO);
        assert_eq!(chunk_of(Vec2::new(PITCH / 2.0 - 0.1, 0.0)), IVec2::ZERO);
        assert_eq!(
            chunk_of(Vec2::new(PITCH / 2.0 + 0.1, -PITCH)),
            IVec2::new(1, -1)
        );
        assert_eq!(centre(IVec2::new(-2, 3)), Vec2::new(-44.0, 66.0));
    }
}
