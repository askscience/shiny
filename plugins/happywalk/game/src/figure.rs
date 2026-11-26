//! Figures: the character and every passer-by, built from a [`Look`].
//!
//! A look is a handful of small numbers (which skin, which hair and in what
//! colour, a beard or not, glasses, a hat, what it wears). The figure's parts
//! are sculpted from it on the spot, as voxel models of ellipsoids, at 4 cm
//! a voxel: nothing about a figure is drawn by hand or stored in a file. The
//! player's look comes from the wardrobe (`web/wardrobe.js`); a passer-by's
//! comes from the city's seed.
//!
//! To add an option: add its name to the list in [`OPTIONS`] and handle its
//! number where the part is sculpted. The wardrobe reads [`options_json`],
//! so it shows the new choice without being touched.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::vox::VoxModel;

type Rgb = [u8; 3];

pub const SKIN: &[Rgb] = &[
    [255, 226, 200],
    [250, 208, 176],
    [238, 190, 152],
    [216, 162, 122],
    [192, 136, 98],
    [160, 108, 76],
    [124, 82, 58],
    [92, 60, 44],
];
pub const HAIR: &[Rgb] = &[
    [46, 42, 48],
    [84, 60, 52],
    [132, 92, 68],
    [172, 94, 62],
    [232, 196, 120],
    [172, 172, 180],
    [240, 238, 232],
    [240, 150, 180],
    [120, 160, 220],
    [130, 196, 150],
];
/// Clothes, hats, scarves, shoes, backpacks: the game's pastels and a few
/// stronger ones to set them off.
pub const CLOTH: &[Rgb] = &[
    [252, 178, 116],
    [249, 170, 180],
    [236, 120, 128],
    [150, 196, 240],
    [110, 140, 210],
    [160, 220, 196],
    [120, 186, 140],
    [196, 180, 240],
    [253, 238, 200],
    [250, 214, 110],
    [248, 248, 250],
    [84, 88, 104],
];

const INK: Rgb = [52, 52, 68];
const WHITE: Rgb = [255, 255, 255];
const BLUSH: Rgb = [246, 170, 170];
const MOUTH: Rgb = [196, 104, 110];

pub const HAIR_STYLES: &[&str] = &[
    "bald", "short", "fringe", "long", "bob", "ponytail", "bun", "curly", "spiky",
];
pub const BEARDS: &[&str] = &["none", "stubble", "moustache", "goatee", "full"];
pub const GLASSES: &[&str] = &["none", "round", "sun"];
pub const HATS: &[&str] = &["none", "beanie", "pompom", "cap", "sunhat", "bow"];
pub const TOPS: &[&str] = &["hoodie", "tee", "jacket", "stripes", "dress"];
pub const BOTTOMS: &[&str] = &["trousers", "shorts", "skirt"];
pub const HEIGHTS: &[&str] = &["small", "medium", "tall"];

/// What a figure looks like. Every field is an index into its list; `scarf`
/// and `backpack` are 0 for none, otherwise a cloth colour plus one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Look {
    pub skin: u8,
    pub hair: u8,
    pub hair_colour: u8,
    pub beard: u8,
    pub glasses: u8,
    pub hat: u8,
    pub hat_colour: u8,
    pub top: u8,
    pub top_colour: u8,
    pub accent: u8,
    pub scarf: u8,
    pub bottom: u8,
    pub bottom_colour: u8,
    pub shoes: u8,
    pub backpack: u8,
    pub height: u8,
}

impl Default for Look {
    /// The character as it is before anyone dresses it.
    fn default() -> Self {
        Self {
            skin: 1,
            hair: 2,
            hair_colour: 1,
            beard: 0,
            glasses: 0,
            hat: 2,
            hat_colour: 5,
            top: 0,
            top_colour: 0,
            accent: 9,
            scarf: 3,
            bottom: 0,
            bottom_colour: 4,
            shoes: 10,
            backpack: 8,
            height: 1,
        }
    }
}

/// One thing that can be chosen: the field of [`Look`] it sets, what kind of
/// choice it is for the wardrobe, and how many values it has.
pub struct Choice {
    pub key: &'static str,
    /// `"skin"`, `"hair"` or `"cloth"` for a colour from that list; `"style"`
    /// for a named style.
    pub kind: &'static str,
    pub names: &'static [&'static str],
    /// A colour that can also be "none" (the value 0).
    pub optional: bool,
}

const fn style(key: &'static str, names: &'static [&'static str]) -> Choice {
    Choice {
        key,
        kind: "style",
        names,
        optional: false,
    }
}

const fn colour(key: &'static str, kind: &'static str, optional: bool) -> Choice {
    Choice {
        key,
        kind,
        names: &[],
        optional,
    }
}

/// Everything the wardrobe offers, in the order it shows it.
pub const OPTIONS: &[Choice] = &[
    colour("skin", "skin", false),
    style("height", HEIGHTS),
    style("hair", HAIR_STYLES),
    colour("hair_colour", "hair", false),
    style("beard", BEARDS),
    style("glasses", GLASSES),
    style("hat", HATS),
    colour("hat_colour", "cloth", false),
    style("top", TOPS),
    colour("top_colour", "cloth", false),
    colour("accent", "cloth", false),
    colour("scarf", "cloth", true),
    style("bottom", BOTTOMS),
    colour("bottom_colour", "cloth", false),
    colour("shoes", "cloth", false),
    colour("backpack", "cloth", true),
];

fn colours(kind: &str) -> &'static [Rgb] {
    match kind {
        "skin" => SKIN,
        "hair" => HAIR,
        _ => CLOTH,
    }
}

impl Choice {
    /// How many values the field can take.
    pub fn count(&self) -> u8 {
        if self.kind == "style" {
            self.names.len() as u8
        } else {
            colours(self.kind).len() as u8 + self.optional as u8
        }
    }
}

/// The wardrobe's menu, as JSON: for each choice its key, its kind, and its
/// style names or its colours (`#rrggbb`).
pub fn options_json() -> String {
    let hex = |c: &Rgb| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
    let choices: Vec<serde_json::Value> = OPTIONS
        .iter()
        .map(|choice| {
            serde_json::json!({
                "key": choice.key,
                "kind": choice.kind,
                "optional": choice.optional,
                "names": choice.names,
                "colours": if choice.kind == "style" { Vec::new() } else { colours(choice.kind).iter().map(hex).collect() },
            })
        })
        .collect();
    serde_json::Value::Array(choices).to_string()
}

impl Look {
    fn field(&mut self, key: &str) -> &mut u8 {
        match key {
            "skin" => &mut self.skin,
            "height" => &mut self.height,
            "hair" => &mut self.hair,
            "hair_colour" => &mut self.hair_colour,
            "beard" => &mut self.beard,
            "glasses" => &mut self.glasses,
            "hat" => &mut self.hat,
            "hat_colour" => &mut self.hat_colour,
            "top" => &mut self.top,
            "top_colour" => &mut self.top_colour,
            "accent" => &mut self.accent,
            "scarf" => &mut self.scarf,
            "bottom" => &mut self.bottom,
            "bottom_colour" => &mut self.bottom_colour,
            "shoes" => &mut self.shoes,
            _ => &mut self.backpack,
        }
    }

    /// The same look with every field brought into range: a look from
    /// outside (the page, an older save) is never trusted.
    pub fn checked(mut self) -> Self {
        for choice in OPTIONS {
            let field = self.field(choice.key);
            *field %= choice.count().max(1);
        }
        self
    }

    /// A look from a number: the same number, the same person.
    pub fn random(bits: u64) -> Self {
        // SplitMix64: every roll is a full mix of the number, so people made
        // from neighbouring numbers are no more alike than any two others.
        let mut state = bits;
        let mut roll = |n: u8| {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            (z % n.max(1) as u64) as u8
        };
        let mut look = Look::default();
        for choice in OPTIONS {
            let value = roll(choice.count());
            *look.field(choice.key) = value;
        }
        // Most people have natural hair, no beard, no glasses, a bag only
        // sometimes: thin the unusual out so it stays unusual.
        if roll(4) != 0 {
            look.hair_colour = roll(6);
        }
        if roll(3) != 0 {
            look.beard = 0;
        }
        if roll(3) != 0 {
            look.glasses = 0;
        }
        if roll(2) == 0 {
            look.hat = 0;
        }
        if roll(2) == 0 {
            look.scarf = 0;
        }
        if roll(3) != 0 {
            look.backpack = 0;
        }
        look
    }

    /// The look as it shows: choices that cannot be seen (the colour of a
    /// hat nobody wears, the details of a top that has none) set to 0, so
    /// two looks that would be drawn the same compare equal.
    pub fn visible(&self) -> Self {
        let mut look = self.checked();
        if look.hat == 0 {
            look.hat_colour = 0;
        }
        if look.hair == 0 && look.beard == 0 {
            // Shaved and beardless: only the brows show the hair colour, and
            // they are drawn from the skin then.
            look.hair_colour = 0;
        }
        let ponytail = HAIR_STYLES[look.hair as usize] == "ponytail";
        let sunhat = HATS[look.hat as usize] == "sunhat";
        let accent_shows =
            TOPS[look.top as usize] != "tee" || ponytail || sunhat || !look.short_sleeves();
        if !accent_shows {
            look.accent = 0;
        }
        look
    }

    /// Metres to a voxel: how tall the figure stands.
    pub fn voxel(&self) -> f32 {
        [0.036, 0.04, 0.044][self.height.min(2) as usize]
    }

    fn skin_rgb(&self) -> Rgb {
        SKIN[self.skin as usize % SKIN.len()]
    }

    fn hair_rgb(&self) -> Rgb {
        HAIR[self.hair_colour as usize % HAIR.len()]
    }

    fn cloth(index: u8) -> Rgb {
        CLOTH[index as usize % CLOTH.len()]
    }

    /// Bare legs and arms go with some clothes.
    fn bare_legs(&self) -> bool {
        self.top == 4 || self.bottom != 0
    }

    fn short_sleeves(&self) -> bool {
        self.top == 1 || self.top == 4
    }
}

/// A block of voxels being shaped, with its own little palette.
struct Sculpt {
    size: IVec3,
    cells: Vec<u8>,
    palette: Vec<Rgb>,
}

impl Sculpt {
    fn new(x: i32, y: i32, z: i32) -> Self {
        Self {
            size: IVec3::new(x, y, z),
            cells: vec![0; (x * y * z) as usize],
            palette: Vec::new(),
        }
    }

    /// The palette index of a colour, added if it is new.
    fn paint(&mut self, rgb: Rgb) -> u8 {
        match self.palette.iter().position(|c| *c == rgb) {
            Some(index) => index as u8 + 1,
            None => {
                self.palette.push(rgb);
                self.palette.len() as u8
            }
        }
    }

    fn inside(&self, x: i32, y: i32, z: i32) -> bool {
        x >= 0 && y >= 0 && z >= 0 && x < self.size.x && y < self.size.y && z < self.size.z
    }

    fn index(&self, x: i32, y: i32, z: i32) -> usize {
        (x + self.size.x * (y + self.size.y * z)) as usize
    }

    fn get(&self, x: i32, y: i32, z: i32) -> u8 {
        if self.inside(x, y, z) {
            self.cells[self.index(x, y, z)]
        } else {
            0
        }
    }

    fn set(&mut self, x: i32, y: i32, z: i32, rgb: Rgb) {
        if self.inside(x, y, z) {
            let colour = self.paint(rgb);
            let index = self.index(x, y, z);
            self.cells[index] = colour;
        }
    }

    /// Fill the ellipsoid round `centre` where `keep` allows.
    fn ball(&mut self, centre: Vec3, radii: Vec3, rgb: Rgb, keep: impl Fn(i32, i32, i32) -> bool) {
        for z in 0..self.size.z {
            for y in 0..self.size.y {
                for x in 0..self.size.x {
                    let d = (Vec3::new(x as f32, y as f32, z as f32) - centre) / radii;
                    if d.length_squared() <= 1.0 && keep(x, y, z) {
                        self.set(x, y, z, rgb);
                    }
                }
            }
        }
    }

    fn block(&mut self, from: IVec3, to: IVec3, rgb: Rgb) {
        for z in from.z..=to.z {
            for y in from.y..=to.y {
                for x in from.x..=to.x {
                    self.set(x, y, z, rgb);
                }
            }
        }
    }

    /// The front-most filled voxel of a column: the front is low `z`.
    fn front(&self, x: i32, y: i32) -> Option<i32> {
        (0..self.size.z).find(|z| self.get(x, y, *z) != 0)
    }

    fn back(&self, x: i32, y: i32) -> Option<i32> {
        (0..self.size.z).rev().find(|z| self.get(x, y, *z) != 0)
    }

    /// Colour the front surface at (`x`, `y`).
    fn on_front(&mut self, x: i32, y: i32, rgb: Rgb) {
        if let Some(z) = self.front(x, y) {
            self.set(x, y, z, rgb);
        }
    }

    /// Copy another block in, moved by `by`.
    fn paste(&mut self, other: &Sculpt, by: IVec3) {
        for z in 0..other.size.z {
            for y in 0..other.size.y {
                for x in 0..other.size.x {
                    let colour = other.get(x, y, z);
                    if colour != 0 {
                        self.set(
                            x + by.x,
                            y + by.y,
                            z + by.z,
                            other.palette[colour as usize - 1],
                        );
                    }
                }
            }
        }
    }

    fn model(self) -> VoxModel {
        let mut palette = vec![[255, 255, 255]; 256];
        for (index, rgb) in self.palette.iter().enumerate() {
            palette[index + 1] = *rgb;
        }
        VoxModel::from_parts(self.size.as_uvec3(), self.cells, palette)
    }
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [0, 1, 2].map(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8)
}

const ANY: fn(i32, i32, i32) -> bool = |_, _, _| true;

/// The head: 17 wide, 21 high, 19 deep, the face two voxels back from the
/// front so a cap's peak and a beard have room.
fn head(look: &Look) -> Sculpt {
    let mut m = Sculpt::new(17, 21, 19);
    let (skin, hair, hat) = (
        look.skin_rgb(),
        look.hair_rgb(),
        Look::cloth(look.hat_colour),
    );
    let c = Vec3::new(8.0, 7.0, 10.0);
    m.ball(c, Vec3::new(8.4, 7.4, 8.0), skin, ANY);

    // Hair. Every style but "bald" and "curly" starts from a cap of hair over
    // the top and the back of the head.
    let cap = |m: &mut Sculpt| {
        m.ball(
            Vec3::new(8.0, 8.0, 10.8),
            Vec3::new(8.7, 7.3, 8.0),
            hair,
            |x, y, z| z >= 11 || y >= 11 || ((x <= 1 || x >= 15) && y >= 7),
        );
    };
    match HAIR_STYLES[look.hair as usize % HAIR_STYLES.len()] {
        "bald" => {}
        "short" => cap(&mut m),
        "fringe" => {
            cap(&mut m);
            m.ball(
                Vec3::new(8.0, 8.0, 10.0),
                Vec3::new(8.6, 7.3, 8.3),
                hair,
                |x, y, z| z < 11 && y >= 9 + (x % 3 == 0) as i32,
            );
        }
        "long" => {
            cap(&mut m);
            m.ball(
                Vec3::new(8.0, 5.0, 12.0),
                Vec3::new(8.8, 8.5, 6.6),
                hair,
                |x, _, z| z >= 12 || ((x <= 1 || x >= 15) && z >= 9),
            );
        }
        "bob" => {
            cap(&mut m);
            m.ball(
                Vec3::new(8.0, 6.0, 10.6),
                Vec3::new(8.9, 6.5, 8.0),
                hair,
                |x, y, z| y >= 2 && (z >= 12 || ((x <= 2 || x >= 14) && z >= 7)),
            );
        }
        "ponytail" => {
            cap(&mut m);
            m.ball(
                Vec3::new(8.0, 7.0, 17.4),
                Vec3::new(2.4, 5.6, 1.7),
                hair,
                ANY,
            );
            m.ball(
                Vec3::new(8.0, 11.5, 16.6),
                Vec3::new(1.6, 1.4, 1.6),
                Look::cloth(look.accent),
                ANY,
            );
        }
        "bun" => {
            cap(&mut m);
            m.ball(Vec3::new(8.0, 15.6, 12.5), Vec3::splat(2.9), hair, ANY);
        }
        "curly" => {
            m.ball(
                Vec3::new(8.0, 9.6, 10.4),
                Vec3::new(8.9, 8.4, 8.5),
                hair,
                |x, y, z| {
                    !(z < 10 && y < 10 && (x - 8).abs() < 7) && (x * 3 + y * 5 + z * 7) % 11 != 0
                },
            );
        }
        _ => {
            cap(&mut m);
            for x in (2..15).step_by(3) {
                for z in (6..16).step_by(3) {
                    if let Some(top) = (0..m.size.y).rev().find(|y| m.get(x, *y, z) != 0) {
                        m.block(IVec3::new(x, top + 1, z), IVec3::new(x, top + 2, z), hair);
                    }
                }
            }
        }
    }

    // The face, on the front surface: wide-set eyes with a glint, brows,
    // cheeks, a small mouth.
    for eye in [4, 11] {
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            m.on_front(eye + dx, 6 + dy, INK);
        }
        m.on_front(eye, 7, WHITE);
        let brow = if look.hair == 0 {
            mix(skin, INK, 0.45)
        } else {
            hair
        };
        m.on_front(eye, 9, brow);
        m.on_front(eye + 1, 9, brow);
    }
    for x in 7..=9 {
        m.on_front(x, 3, MOUTH);
    }
    for x in [2, 3, 13, 14] {
        m.on_front(x, 4, BLUSH);
    }

    match BEARDS[look.beard as usize % BEARDS.len()] {
        "stubble" => {
            let shade = mix(skin, hair, 0.35);
            for x in 3..=13 {
                for y in 0..=4 {
                    if (x + y) % 2 == 0
                        && !(y == 3 && (7..=9).contains(&x))
                        && !(y == 4 && !(4..=12).contains(&x))
                    {
                        m.on_front(x, y, shade);
                    }
                }
            }
        }
        "moustache" => {
            for x in 6..=10 {
                m.on_front(x, 4, hair);
            }
        }
        "goatee" => {
            for x in 6..=10 {
                m.on_front(x, 4, hair);
            }
            for x in 7..=9 {
                for y in 0..=2 {
                    m.on_front(x, y, hair);
                }
            }
        }
        "full" => {
            // A layer over the jaw, standing out a voxel, with the mouth
            // left showing.
            for x in 2..=14 {
                for y in 0..=4 {
                    if y == 3 && (7..=9).contains(&x) {
                        continue;
                    }
                    if y == 4 && !(4..=12).contains(&x) {
                        continue;
                    }
                    if let Some(z) = m.front(x, y) {
                        if z < 10 {
                            m.set(x, y, z, hair);
                            m.set(x, y, z - 1, hair);
                        }
                    }
                }
            }
        }
        _ => {}
    }

    let glasses = GLASSES[look.glasses as usize % GLASSES.len()];
    if glasses != "none" {
        let frame = if glasses == "sun" {
            INK
        } else {
            [116, 122, 148]
        };
        for eye in [4, 11] {
            for x in eye - 1..=eye + 2 {
                for y in 5..=8 {
                    let edge = x == eye - 1 || x == eye + 2 || y == 5 || y == 8;
                    if glasses == "sun" || edge {
                        if let Some(z) = m.front(x, y) {
                            m.set(x, y, z - 1, frame);
                        }
                    }
                }
            }
        }
        for x in 7..=9 {
            if let Some(z) = m.front(x, 7) {
                m.set(x, 7, z - 1, frame);
            }
        }
    }

    // A hat sits on the head: whatever hair stood above its brim goes, or
    // curls and spikes poke through it.
    if matches!(
        HATS[look.hat as usize % HATS.len()],
        "beanie" | "pompom" | "cap" | "sunhat"
    ) {
        for z in 0..m.size.z {
            for y in 11..m.size.y {
                for x in 0..m.size.x {
                    let index = m.index(x, y, z);
                    m.cells[index] = 0;
                }
            }
        }
    }
    match HATS[look.hat as usize % HATS.len()] {
        "beanie" | "pompom" => {
            m.ball(
                Vec3::new(8.0, 9.0, 10.0),
                Vec3::new(8.9, 7.6, 8.6),
                hat,
                |_, y, _| y >= 11,
            );
            m.ball(
                Vec3::new(8.0, 11.0, 10.0),
                Vec3::new(9.0, 1.6, 8.8),
                WHITE,
                |_, y, _| (10..=11).contains(&y),
            );
            if HATS[look.hat as usize % HATS.len()] == "pompom" {
                m.ball(Vec3::new(8.0, 18.2, 10.0), Vec3::splat(2.6), WHITE, ANY);
            }
        }
        "cap" => {
            m.ball(
                Vec3::new(8.0, 9.5, 10.0),
                Vec3::new(8.8, 6.4, 8.5),
                hat,
                |_, y, _| y >= 11,
            );
            m.block(IVec3::new(3, 11, 0), IVec3::new(13, 11, 3), hat);
            m.block(IVec3::new(7, 15, 9), IVec3::new(9, 16, 11), WHITE);
        }
        "sunhat" => {
            m.ball(
                Vec3::new(8.0, 9.5, 10.0),
                Vec3::new(8.8, 6.8, 8.5),
                hat,
                |_, y, _| y >= 11,
            );
            m.ball(
                Vec3::new(8.0, 11.0, 9.5),
                Vec3::new(9.5, 0.8, 10.0),
                hat,
                |_, y, _| y == 11,
            );
            m.ball(
                Vec3::new(8.0, 12.0, 10.0),
                Vec3::new(8.9, 0.9, 8.6),
                Look::cloth(look.accent),
                |_, y, _| y == 12,
            );
        }
        "bow" => {
            let bow = hat;
            m.ball(
                Vec3::new(4.5, 15.0, 9.0),
                Vec3::new(2.2, 1.8, 1.6),
                bow,
                ANY,
            );
            m.ball(
                Vec3::new(9.5, 15.0, 9.0),
                Vec3::new(2.2, 1.8, 1.6),
                bow,
                ANY,
            );
            m.block(IVec3::new(7, 14, 8), IVec3::new(7, 15, 9), WHITE);
        }
        _ => {}
    }
    m
}

/// The body: 13 wide (room for a skirt to flare), 11 high, 11 deep.
fn torso(look: &Look) -> Sculpt {
    let mut m = Sculpt::new(13, 11, 11);
    let (top, accent, skin) = (
        Look::cloth(look.top_colour),
        Look::cloth(look.accent),
        look.skin_rgb(),
    );
    let c = Vec3::new(6.0, 5.0, 5.0);
    m.ball(c, Vec3::new(5.6, 6.6, 4.4), top, ANY);
    m.ball(Vec3::new(6.0, 0.5, 5.0), Vec3::new(5.4, 2.2, 4.3), top, ANY);
    match TOPS[look.top as usize % TOPS.len()] {
        "hoodie" => {
            for x in 4..=8 {
                m.on_front(x, 3, accent);
            }
            m.ball(
                Vec3::new(6.0, 8.4, 7.6),
                Vec3::new(3.6, 2.4, 2.2),
                accent,
                ANY,
            );
        }
        "tee" => {
            for (x, y) in [(5, 6), (6, 6), (5, 7), (6, 7), (7, 6), (7, 7)] {
                m.on_front(x, y, accent);
            }
            // A round neck.
            for x in 5..=7 {
                m.on_front(x, 10, skin);
            }
        }
        "jacket" => {
            for y in 0..=9 {
                m.on_front(6, y, accent);
                if y >= 6 {
                    m.on_front(5, y, accent);
                    m.on_front(7, y, accent);
                }
            }
            m.on_front(6, 3, WHITE);
            m.on_front(6, 6, WHITE);
        }
        "stripes" => {
            for y in (1..11).step_by(3) {
                for x in 0..13 {
                    for z in 0..11 {
                        if m.get(x, y, z) != 0 {
                            m.set(x, y, z, accent);
                        }
                    }
                }
            }
        }
        _ => {
            // A dress: the skirt flares from the waist.
            m.ball(
                Vec3::new(6.0, 0.8, 5.0),
                Vec3::new(6.4, 3.4, 5.4),
                top,
                |_, y, _| y <= 3,
            );
            for x in 0..13 {
                m.on_front(x, 4, accent);
                if let Some(z) = m.back(x, 4) {
                    m.set(x, 4, z, accent);
                }
            }
        }
    }
    if BOTTOMS[look.bottom as usize % BOTTOMS.len()] == "skirt" && look.top != 4 {
        m.ball(
            Vec3::new(6.0, 0.6, 5.0),
            Vec3::new(6.4, 2.6, 5.4),
            Look::cloth(look.bottom_colour),
            |_, y, _| y <= 2,
        );
    }
    if look.scarf > 0 {
        let scarf = Look::cloth(look.scarf - 1);
        m.ball(
            Vec3::new(6.0, 10.0, 5.0),
            Vec3::new(4.2, 1.3, 3.8),
            scarf,
            |_, y, _| y >= 9,
        );
        for y in 6..=8 {
            m.on_front(8, y, scarf);
        }
    }
    m
}

fn arm(look: &Look) -> Sculpt {
    let mut m = Sculpt::new(4, 9, 4);
    let (top, accent, skin) = (
        Look::cloth(look.top_colour),
        Look::cloth(look.accent),
        look.skin_rgb(),
    );
    let c = Vec3::new(1.5, 5.5, 1.5);
    if look.short_sleeves() {
        m.ball(c, Vec3::new(1.9, 4.2, 1.9), skin, |_, y, _| y >= 2);
        m.ball(c, Vec3::new(2.1, 4.2, 2.1), top, |_, y, _| y >= 6);
    } else {
        m.ball(c, Vec3::new(2.1, 4.2, 2.1), top, |_, y, _| y >= 2);
        m.block(IVec3::new(0, 2, 0), IVec3::new(3, 2, 3), accent);
    }
    // A mitten of a hand.
    m.ball(
        Vec3::new(1.5, 1.0, 1.5),
        Vec3::new(2.0, 1.6, 2.0),
        skin,
        |_, y, _| y <= 1,
    );
    m
}

fn leg(look: &Look) -> Sculpt {
    let mut m = Sculpt::new(4, 8, 7);
    let (cloth, skin) = (Look::cloth(look.bottom_colour), look.skin_rgb());
    let shorts = BOTTOMS[look.bottom as usize % BOTTOMS.len()] == "shorts" && look.top != 4;
    m.ball(
        Vec3::new(1.5, 5.0, 4.0),
        Vec3::new(2.2, 4.0, 2.2),
        if look.bare_legs() { skin } else { cloth },
        |_, y, _| y >= 2,
    );
    if shorts {
        m.ball(
            Vec3::new(1.5, 5.0, 4.0),
            Vec3::new(2.3, 4.0, 2.3),
            cloth,
            |_, y, _| y >= 5,
        );
    }
    // A round-toed shoe, longer at the front, on a dark sole.
    m.ball(
        Vec3::new(1.5, 0.8, 3.2),
        Vec3::new(2.2, 1.5, 3.4),
        Look::cloth(look.shoes),
        |_, y, _| y <= 1,
    );
    for x in 0..4 {
        for z in 0..7 {
            if m.get(x, 0, z) != 0 {
                m.set(x, 0, z, INK);
            }
        }
    }
    m
}

fn backpack(look: &Look) -> Option<Sculpt> {
    if look.backpack == 0 {
        return None;
    }
    let mut m = Sculpt::new(8, 8, 5);
    m.ball(
        Vec3::new(3.5, 3.5, 1.5),
        Vec3::new(4.2, 4.3, 3.4),
        Look::cloth(look.backpack - 1),
        ANY,
    );
    for x in 1..7 {
        if let Some(z) = m.back(x, 2) {
            m.set(x, 2, z, WHITE);
        }
    }
    if let Some(z) = m.back(4, 5) {
        m.set(4, 5, z, Look::cloth(look.accent));
    }
    Some(m)
}

/// Where the shoulders of a [`Look::body`] are, in voxels from the middle of
/// its base: left and right, the arm's top, the middle of its depth.
pub const SHOULDERS: [Vec3; 2] = [Vec3::new(-8.0, 10.0, -2.5), Vec3::new(7.0, 10.0, -2.5)];

/// A figure's parts, each hung from its joint by whoever animates it.
pub struct Parts {
    pub head: VoxModel,
    pub torso: VoxModel,
    pub arm: VoxModel,
    pub leg: VoxModel,
    pub backpack: Option<VoxModel>,
}

impl Look {
    /// The parts of a figure that moves every limb (the character).
    pub fn parts(&self) -> Parts {
        Parts {
            head: head(self).model(),
            torso: torso(self).model(),
            arm: arm(self).model(),
            leg: leg(self).model(),
            backpack: backpack(self).map(Sculpt::model),
        }
    }

    /// Head, body, arms and bag as one model, for a figure that only swings
    /// its legs (a passer-by): one mesh instead of five.
    pub fn body(&self) -> VoxModel {
        self.body_with(true)
    }

    /// [`body`](Self::body) with the arms left off, for a figure whose arms
    /// reach for something (a cyclist's handlebars). [`SHOULDERS`] says where
    /// they go.
    pub fn body_without_arms(&self) -> VoxModel {
        self.body_with(false)
    }

    /// An arm on its own, hanging from its top: the shoulder.
    pub fn arm(&self) -> VoxModel {
        arm(self).model()
    }

    fn body_with(&self, arms: bool) -> VoxModel {
        let mut m = Sculpt::new(19, 31, 24);
        m.paste(&torso(self), IVec3::new(3, 0, 4));
        m.paste(&head(self), IVec3::new(1, 10, 0));
        let arm = arm(self);
        for x in [0, 15] {
            if arms {
                m.paste(&arm, IVec3::new(x, 1, 7));
            }
        }
        if let Some(pack) = backpack(self) {
            m.paste(&pack, IVec3::new(5, 2, 14));
        }
        m.model()
    }

    pub fn leg(&self) -> VoxModel {
        leg(self).model()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_option_builds_a_figure() {
        // One look per value of every choice, each on the default look.
        for choice in OPTIONS {
            for value in 0..choice.count() {
                let mut look = Look::default();
                *look.field(choice.key) = value;
                assert_eq!(look.checked(), look, "{} = {value}", choice.key);
                let parts = look.parts();
                for (name, model) in [
                    ("head", &parts.head),
                    ("torso", &parts.torso),
                    ("arm", &parts.arm),
                    ("leg", &parts.leg),
                ] {
                    assert!(
                        !model.quads().is_empty(),
                        "{} = {value}: empty {name}",
                        choice.key
                    );
                }
                assert_eq!(parts.backpack.is_some(), look.backpack > 0);
                assert!(look.body().filled() > parts.head.filled());
            }
        }
    }

    #[test]
    fn a_choice_changes_what_is_built() {
        let plain = Look::default();
        for (key, value) in [
            ("skin", 6),
            ("hair", 3),
            ("hair_colour", 4),
            ("beard", 4),
            ("glasses", 2),
            ("hat", 0),
            ("hat_colour", 2),
        ] {
            let mut look = plain;
            *look.field(key) = value;
            assert_ne!(look.parts().head, plain.parts().head, "{key}");
        }
        for (key, value) in [("top", 2), ("top_colour", 3), ("scarf", 0)] {
            let mut look = plain;
            *look.field(key) = value;
            assert_ne!(look.parts().torso, plain.parts().torso, "{key}");
        }
        let mut shorts = plain;
        shorts.bottom = 1;
        assert_ne!(shorts.leg(), plain.leg());
    }

    #[test]
    fn hair_does_not_poke_through_a_hat() {
        for hair in 0..HAIR_STYLES.len() as u8 {
            let look = Look {
                hair,
                hat: 1,
                hat_colour: 9,
                hair_colour: 1,
                ..Look::default()
            };
            let head = look.parts().head;
            let hair_rgb = HAIR[1];
            for x in 0..17 {
                for y in 12..21 {
                    for z in 0..19 {
                        assert_ne!(
                            head.colour_at(x, y, z),
                            Some(hair_rgb),
                            "{} at {x},{y},{z}",
                            HAIR_STYLES[hair as usize]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_face_is_on_the_front() {
        let head = Look::default().parts().head;
        let ink = |z: i32| (0..17).any(|x| (0..21).any(|y| head.colour_at(x, y, z) == Some(INK)));
        assert!((0..9).any(ink));
        assert!(!(12..19).any(ink));
    }

    #[test]
    fn a_look_from_outside_is_brought_into_range() {
        let wild = Look {
            skin: 200,
            hair: 99,
            beard: 77,
            scarf: 255,
            backpack: 255,
            height: 9,
            ..default()
        };
        let tame = wild.checked();
        assert!((tame.skin as usize) < SKIN.len());
        assert!((tame.hair as usize) < HAIR_STYLES.len());
        assert!((tame.scarf as usize) <= CLOTH.len());
        assert!(tame.height < 3);
        // It builds.
        assert!(tame.body().filled() > 0);
        // And a look written by an older game, with fields missing, reads.
        let old: Look = serde_json::from_str(r#"{"skin": 4, "hat": 1}"#).unwrap();
        assert_eq!(old.skin, 4);
        assert_eq!(old.top, Look::default().top);
    }

    #[test]
    fn passers_by_are_many_different_people() {
        let looks: std::collections::HashSet<Look> = (1..=200u64)
            .map(|i| Look::random(i.wrapping_mul(0x9E37_79B9_7F4A_7C15)))
            .collect();
        assert!(looks.len() > 190, "{} different looks in 200", looks.len());
        assert_eq!(Look::random(42), Look::random(42));
        for look in &looks {
            assert_eq!(look.checked(), *look);
        }
        // The unusual stays unusual, and everything turns up.
        let with = |f: fn(&Look) -> bool| looks.iter().filter(|l| f(l)).count();
        assert!(with(|l| l.beard > 0) < 100);
        assert!(with(|l| l.beard > 0) > 10);
        assert!(with(|l| l.glasses > 0) > 10);
        assert!(with(|l| l.hat == 0) > 60);
        for skin in 0..SKIN.len() as u8 {
            assert!(
                looks.iter().any(|l| l.skin == skin),
                "nobody with skin {skin}"
            );
        }
    }

    #[test]
    fn the_menu_lists_every_choice() {
        let menu: serde_json::Value = serde_json::from_str(&options_json()).unwrap();
        let menu = menu.as_array().unwrap();
        assert_eq!(menu.len(), OPTIONS.len());
        assert_eq!(menu[0]["key"], "skin");
        assert_eq!(menu[0]["colours"].as_array().unwrap().len(), SKIN.len());
        assert_eq!(menu[0]["colours"][0], "#ffe2c8");
        let hair = menu.iter().find(|c| c["key"] == "hair").unwrap();
        assert_eq!(hair["names"].as_array().unwrap().len(), HAIR_STYLES.len());
        // Every field of a look is on the menu, once.
        let keys: std::collections::HashSet<&str> = OPTIONS.iter().map(|c| c.key).collect();
        assert_eq!(keys.len(), 16);
    }

    #[test]
    fn a_figure_is_cheap_to_draw() {
        let look = Look {
            hair: 7,
            beard: 4,
            glasses: 1,
            hat: 4,
            top: 3,
            ..default()
        };
        let quads = look.body().quads().len() + 2 * look.leg().quads().len();
        assert!(quads < 2600, "{quads} quads");
    }
}
