//! `VoxPlugin`: voxel models. MagicaVoxel `.vox` files become meshes, once,
//! and everything in the game that is made of voxels is placed from them.
//!
//! * **Models are found by name.** Every file in `assets/vox/` is built in
//!   (`build.rs`); `VoxLibrary::mesh("lamp")` is all a module needs.
//! * **Greedy meshing.** Neighbouring faces of one colour and one shading
//!   are merged into a single rectangle, so a building is a few hundred
//!   quads instead of tens of thousands.
//! * **Soft ambient occlusion**, baked into the vertex colours: corners and
//!   creases are a little darker, which is most of the voxel look.
//! * **One material for everything.** Colour comes from the vertices, so all
//!   models share a material, and the same model placed many times shares
//!   its mesh: the renderer batches them.
//!
//! A model's axes follow the game: `x` right, `y` up, `-z` the front. Its
//! origin is the centre of its base, and its unit is the voxel; whoever
//! places it gives the voxel a size through the transform's scale.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

include!(concat!(env!("OUT_DIR"), "/vox_assets.rs"));

/// How dark a vertex is with 0, 1, 2 or 3 of its three neighbours filled.
const SHADE: [f32; 4] = [1.0, 0.88, 0.76, 0.64];

/// A box of voxels. `0` is empty; other values index the palette.
#[derive(Clone, Debug, PartialEq)]
pub struct VoxModel {
    /// Voxels along `x`, `y` (up) and `z`.
    pub size: UVec3,
    cells: Vec<u8>,
    /// sRGB colours; index 0 is unused.
    palette: Vec<[u8; 3]>,
}

impl VoxModel {
    pub fn empty(size: UVec3) -> Self {
        Self {
            size,
            cells: vec![0; (size.x * size.y * size.z) as usize],
            palette: vec![[255, 255, 255]; 256],
        }
    }

    fn index(&self, x: u32, y: u32, z: u32) -> usize {
        (x + self.size.x * (y + self.size.y * z)) as usize
    }

    pub fn set(&mut self, x: u32, y: u32, z: u32, colour: u8) {
        let index = self.index(x, y, z);
        self.cells[index] = colour;
    }

    /// A model from its cells (`x + size.x * (y + size.y * z)`) and its
    /// palette of 256 colours: for models made in the game, not read from a
    /// file.
    pub fn from_parts(size: UVec3, cells: Vec<u8>, palette: Vec<[u8; 3]>) -> Self {
        debug_assert_eq!(cells.len(), (size.x * size.y * size.z) as usize);
        debug_assert_eq!(palette.len(), 256);
        Self {
            size,
            cells,
            palette,
        }
    }

    /// The sRGB colour of the voxel at a position, if there is one.
    pub fn colour_at(&self, x: i32, y: i32, z: i32) -> Option<[u8; 3]> {
        match self.at(x, y, z) {
            0 => None,
            index => Some(self.palette[index as usize]),
        }
    }

    /// How many voxels are filled.
    pub fn filled(&self) -> usize {
        self.cells.iter().filter(|c| **c != 0).count()
    }

    /// The voxel at a position, `0` outside the model.
    pub fn at(&self, x: i32, y: i32, z: i32) -> u8 {
        let inside = |v: i32, size: u32| v >= 0 && (v as u32) < size;
        if inside(x, self.size.x) && inside(y, self.size.y) && inside(z, self.size.z) {
            self.cells[self.index(x as u32, y as u32, z as u32)]
        } else {
            0
        }
    }

    /// Read a MagicaVoxel file: the first model in it and its palette.
    ///
    /// MagicaVoxel's axes are `x` right, `y` into the screen, `z` up; the
    /// model is turned to the game's (`y` up, front towards `-z`) without
    /// mirroring it.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let word = |at: usize| -> Result<u32, String> {
            bytes
                .get(at..at + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .ok_or_else(|| "truncated".to_string())
        };
        if bytes.get(0..4) != Some(b"VOX ") {
            return Err("not a .vox file".into());
        }
        let mut at = 8;
        let mut size = None;
        let mut voxels: Vec<[u8; 4]> = Vec::new();
        let mut palette = vec![[255u8, 255, 255]; 256];
        while at + 12 <= bytes.len() {
            let id = &bytes[at..at + 4];
            let content = word(at + 4)? as usize;
            let body = at + 12;
            if id == b"MAIN" {
                // Its chunks follow as children: read on.
                at = body;
                continue;
            }
            let end = body
                .checked_add(content)
                .filter(|end| *end <= bytes.len())
                .ok_or("truncated chunk")?;
            match id {
                b"SIZE" if size.is_none() => {
                    size = Some(UVec3::new(word(body)?, word(body + 4)?, word(body + 8)?))
                }
                b"XYZI" if voxels.is_empty() => {
                    let count = word(body)? as usize;
                    let data = bytes
                        .get(body + 4..body + 4 + count * 4)
                        .ok_or("truncated voxels")?;
                    voxels = data
                        .chunks_exact(4)
                        .map(|v| [v[0], v[1], v[2], v[3]])
                        .collect();
                }
                b"RGBA" => {
                    // Colour `i` of the file is palette index `i + 1`.
                    for (i, rgba) in bytes[body..end].chunks_exact(4).take(255).enumerate() {
                        palette[i + 1] = [rgba[0], rgba[1], rgba[2]];
                    }
                }
                _ => {}
            }
            at = end;
        }
        let file = size.ok_or("no SIZE chunk")?;
        let mut model = Self::empty(UVec3::new(file.x, file.z, file.y));
        model.palette = palette;
        for [x, y, z, colour] in voxels {
            let (x, y, z) = (x as u32, y as u32, z as u32);
            if x < file.x && y < file.y && z < file.z {
                model.set(file.x - 1 - x, z, y, colour);
            }
        }
        Ok(model)
    }

    /// Linear colour of a palette index.
    fn colour(&self, index: u8) -> [f32; 3] {
        let srgb = self.palette[index as usize];
        let c = Color::srgb_u8(srgb[0], srgb[1], srgb[2]).to_linear();
        [c.red, c.green, c.blue]
    }

    /// The model's faces as merged rectangles.
    pub fn quads(&self) -> Vec<Quad> {
        let size = [self.size.x as i32, self.size.y as i32, self.size.z as i32];
        let mut quads = Vec::new();
        for d in 0..3 {
            let (u, v) = ((d + 1) % 3, (d + 2) % 3);
            for side in [-1i32, 1] {
                for slice in 0..size[d] {
                    // The underside of a model stands on the ground: never seen.
                    if d == 1 && side < 0 && slice == 0 {
                        continue;
                    }
                    // What shows on this slice, facing `side` along `d`.
                    let mut mask: Vec<Option<(u8, [u8; 4])>> =
                        vec![None; (size[u] * size[v]) as usize];
                    let cell = |a: i32, b: i32, layer: i32| {
                        let mut p = [0; 3];
                        p[d] = layer;
                        p[u] = a;
                        p[v] = b;
                        self.at(p[0], p[1], p[2])
                    };
                    for b in 0..size[v] {
                        for a in 0..size[u] {
                            let colour = cell(a, b, slice);
                            if colour == 0 || cell(a, b, slice + side) != 0 {
                                continue;
                            }
                            // For each corner, how many of the three cells
                            // around it, in the layer in front, are filled.
                            let front = slice + side;
                            let filled =
                                |da: i32, db: i32| (cell(a + da, b + db, front) != 0) as u8;
                            let corner = |da: i32, db: i32| {
                                let (s1, s2) = (filled(da, 0), filled(0, db));
                                if s1 + s2 == 2 {
                                    3
                                } else {
                                    s1 + s2 + filled(da, db)
                                }
                            };
                            let shade =
                                [corner(-1, -1), corner(1, -1), corner(1, 1), corner(-1, 1)];
                            mask[(a + b * size[u]) as usize] = Some((colour, shade));
                        }
                    }
                    // Merge equal neighbours into rectangles.
                    for b in 0..size[v] {
                        let mut a = 0;
                        while a < size[u] {
                            let Some(key) = mask[(a + b * size[u]) as usize] else {
                                a += 1;
                                continue;
                            };
                            let mut wide = 1;
                            while a + wide < size[u]
                                && mask[(a + wide + b * size[u]) as usize] == Some(key)
                            {
                                wide += 1;
                            }
                            let mut tall = 1;
                            'grow: while b + tall < size[v] {
                                for k in 0..wide {
                                    if mask[(a + k + (b + tall) * size[u]) as usize] != Some(key) {
                                        break 'grow;
                                    }
                                }
                                tall += 1;
                            }
                            for db in 0..tall {
                                for da in 0..wide {
                                    mask[(a + da + (b + db) * size[u]) as usize] = None;
                                }
                            }
                            let plane = if side > 0 { slice + 1 } else { slice };
                            let point = |a: i32, b: i32| {
                                let mut p = [0.0; 3];
                                p[d] = plane as f32;
                                p[u] = a as f32;
                                p[v] = b as f32;
                                p
                            };
                            let mut normal = [0.0; 3];
                            normal[d] = side as f32;
                            quads.push(Quad {
                                corners: [
                                    point(a, b),
                                    point(a + wide, b),
                                    point(a + wide, b + tall),
                                    point(a, b + tall),
                                ],
                                normal,
                                colour: key.0,
                                shade: key.1,
                                flipped: side < 0,
                            });
                            a += wide;
                        }
                    }
                }
            }
        }
        quads
    }

    /// The model as a mesh: positions in voxels around the centre of the
    /// base, normals, and vertex colours with the occlusion in them.
    pub fn mesh(&self) -> Mesh {
        let quads = self.quads();
        let centre = [self.size.x as f32 / 2.0, 0.0, self.size.z as f32 / 2.0];
        let mut positions = Vec::with_capacity(quads.len() * 4);
        let mut normals = Vec::with_capacity(quads.len() * 4);
        let mut colours = Vec::with_capacity(quads.len() * 4);
        let mut indices: Vec<u32> = Vec::with_capacity(quads.len() * 6);
        for quad in &quads {
            let first = positions.len() as u32;
            let base = self.colour(quad.colour);
            for (corner, shade) in quad.corners.iter().zip(quad.shade) {
                positions.push([
                    corner[0] - centre[0],
                    corner[1] - centre[1],
                    corner[2] - centre[2],
                ]);
                normals.push(quad.normal);
                let s = SHADE[shade as usize];
                colours.push([base[0] * s, base[1] * s, base[2] * s, 1.0]);
            }
            // Split along the diagonal that keeps the shading smooth.
            let split = if quad.shade[0] + quad.shade[2] > quad.shade[1] + quad.shade[3] {
                [1, 2, 3, 1, 3, 0]
            } else {
                [0, 1, 2, 0, 2, 3]
            };
            for i in split.chunks_exact(3) {
                let tri = if quad.flipped {
                    [i[0], i[2], i[1]]
                } else {
                    [i[0], i[1], i[2]]
                };
                indices.extend(tri.map(|k| first + k));
            }
        }
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colours)
        .with_inserted_indices(Indices::U32(indices))
    }
}

/// One merged face.
#[derive(Clone, Debug, PartialEq)]
pub struct Quad {
    pub corners: [[f32; 3]; 4],
    pub normal: [f32; 3],
    pub colour: u8,
    /// Occlusion at each corner, 0 (open) to 3.
    pub shade: [u8; 4],
    flipped: bool,
}

/// Every voxel model, by name, meshed once.
#[derive(Resource)]
pub struct VoxLibrary {
    models: HashMap<&'static str, (UVec3, Handle<Mesh>)>,
    /// The one material all voxel models share.
    pub material: Handle<StandardMaterial>,
}

impl VoxLibrary {
    /// The mesh of a model. A missing model is a programming error: it is
    /// logged once and drawn as nothing.
    pub fn mesh(&self, name: &str) -> Handle<Mesh> {
        match self.models.get(name) {
            Some((_, mesh)) => mesh.clone(),
            None => {
                error!("no voxel model named {name:?} in assets/vox");
                Handle::default()
            }
        }
    }

    /// The model's size in voxels (`x`, `y` up, `z`).
    pub fn size(&self, name: &str) -> UVec3 {
        self.models
            .get(name)
            .map(|(size, _)| *size)
            .unwrap_or(UVec3::ONE)
    }

    /// What to spawn to show `name` with voxels `voxel` metres big, standing
    /// on `at`, turned `quarter_turns` quarter turns.
    pub fn place(&self, name: &str, at: Vec3, voxel: f32, quarter_turns: i32) -> impl Bundle {
        (
            Mesh3d(self.mesh(name)),
            MeshMaterial3d(self.material.clone()),
            Transform::from_translation(at)
                .with_rotation(Quat::from_rotation_y(
                    quarter_turns as f32 * std::f32::consts::FRAC_PI_2,
                ))
                .with_scale(Vec3::splat(voxel)),
        )
    }
}

/// Every built-in model, by name.
pub fn models() -> impl Iterator<Item = (&'static str, VoxModel)> {
    VOX_ASSETS
        .iter()
        .filter_map(|(name, bytes)| VoxModel::parse(bytes).ok().map(|model| (*name, model)))
}

/// The size in voxels of every built-in model, without a renderer: for code
/// that lays things out (and its tests).
pub fn sizes() -> HashMap<&'static str, UVec3> {
    VOX_ASSETS
        .iter()
        .filter_map(|(name, bytes)| VoxModel::parse(bytes).ok().map(|model| (*name, model.size)))
        .collect()
}

pub struct VoxPlugin;

impl Plugin for VoxPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, load);
    }
}

fn load(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut models = HashMap::new();
    let mut quads = 0;
    for (name, bytes) in VOX_ASSETS {
        match VoxModel::parse(bytes) {
            Ok(model) => {
                let mesh = model.mesh();
                quads += mesh.indices().map(|i| i.len() / 6).unwrap_or(0);
                models.insert(*name, (model.size, meshes.add(mesh)));
            }
            Err(e) => error!("voxel model {name}: {e}"),
        }
    }
    info!("voxel models: {} meshed into {quads} quads", models.len());
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 1.0,
        reflectance: 0.05,
        ..default()
    });
    commands.insert_resource(VoxLibrary { models, material });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(size: UVec3) -> VoxModel {
        let mut model = VoxModel::empty(size);
        for x in 0..size.x {
            for y in 0..size.y {
                for z in 0..size.z {
                    model.set(x, y, z, 1);
                }
            }
        }
        model
    }

    #[test]
    fn a_solid_block_is_five_quads() {
        // Four sides and the top; the underside stands on the ground.
        let quads = solid(UVec3::new(8, 5, 3)).quads();
        assert_eq!(quads.len(), 5);
        assert!(!quads.iter().any(|q| q.normal == [0.0, -1.0, 0.0]));
        assert!(quads.iter().all(|q| q.shade == [0; 4]));
    }

    #[test]
    fn faces_point_outwards() {
        // Each triangle's winding agrees with its quad's normal.
        let model = solid(UVec3::new(2, 3, 4));
        let mesh = model.mesh();
        let positions = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .unwrap()
            .as_float3()
            .unwrap()
            .to_vec();
        let normals = mesh
            .attribute(Mesh::ATTRIBUTE_NORMAL)
            .unwrap()
            .as_float3()
            .unwrap()
            .to_vec();
        let Some(Indices::U32(indices)) = mesh.indices() else {
            panic!("no indices")
        };
        for tri in indices.chunks_exact(3) {
            let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| Vec3::from(positions[i as usize]));
            let facing = (b - a).cross(c - a).normalize();
            assert!(facing.dot(Vec3::from(normals[tri[0] as usize])) > 0.99);
        }
    }

    #[test]
    fn two_colours_do_not_merge_and_a_step_casts_shade() {
        let mut model = solid(UVec3::new(4, 1, 1));
        model.set(3, 0, 0, 2);
        // Top face: one quad per colour.
        let tops: Vec<_> = model
            .quads()
            .into_iter()
            .filter(|q| q.normal == [0.0, 1.0, 0.0])
            .collect();
        assert_eq!(tops.len(), 2);

        // A floor with a block standing on it: the floor around the block
        // is shaded where it meets it.
        let mut step = VoxModel::empty(UVec3::new(3, 2, 3));
        for x in 0..3 {
            for z in 0..3 {
                step.set(x, 0, z, 1);
            }
        }
        step.set(1, 1, 1, 1);
        let floor: Vec<_> = step
            .quads()
            .into_iter()
            .filter(|q| q.normal == [0.0, 1.0, 0.0] && q.corners[0][1] == 1.0)
            .collect();
        assert!(floor.iter().any(|q| q.shade.iter().any(|s| *s > 0)));
    }

    #[test]
    fn the_built_in_models_parse_and_stand_the_right_way_up() {
        let sizes = sizes();
        assert!(sizes.len() >= 10);
        // A lamp post is tall, not long.
        let lamp = sizes["lamp"];
        assert!(lamp.y > lamp.x * 3 && lamp.y > lamp.z * 3, "{lamp:?}");
        for (name, bytes) in VOX_ASSETS {
            let model = VoxModel::parse(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!model.quads().is_empty(), "{name}");
        }
    }

    #[test]
    fn the_front_of_a_model_faces_minus_z() {
        // A car's headlights are on MagicaVoxel's low-y side, its tail
        // lights on the other.
        let (_, car) = models().find(|(name, _)| *name == "car_small_red").unwrap();
        let half = car.size.z as i32 / 2;
        let has = |rgb: [u8; 3], depths: std::ops::Range<i32>| {
            depths
                .flat_map(|z| {
                    (0..car.size.x as i32)
                        .flat_map(move |x| (0..car.size.y as i32).map(move |y| (x, y, z)))
                })
                .any(|(x, y, z)| car.colour_at(x, y, z) == Some(rgb))
        };
        let headlight = [255, 246, 200];
        assert!(has(headlight, 0..half));
        assert!(!has(headlight, half..car.size.z as i32));
    }

    #[test]
    fn greedy_meshing_keeps_buildings_small() {
        // A building is tens of thousands of voxels and about a thousand
        // quads: what the renderer draws is what shows, not what is there.
        let mut seen = 0;
        for (name, model) in
            models().filter(|(name, _)| name.starts_with("home_") || name.starts_with("shop_"))
        {
            let quads = model.quads().len();
            assert!(quads < 1500, "{name}: {quads} quads");
            assert!(
                quads * 8 < model.filled(),
                "{name}: {quads} quads for {} voxels",
                model.filled()
            );
            seen += 1;
        }
        assert!(seen >= 10);
    }

    #[test]
    fn rubbish_is_refused() {
        assert!(VoxModel::parse(b"nope").is_err());
        assert!(VoxModel::parse(b"VOX \x96\0\0\0").is_err());
    }
}
