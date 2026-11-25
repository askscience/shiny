//! Print how heavy each voxel model is once meshed.
//!
//!     cargo run --example stats
//!
//! Quads are what the renderer draws (two triangles each); the ratio to
//! voxels shows how well greedy meshing does on a model. A model that is
//! placed many times (trees, lamps) is worth keeping light.

use happywalk_game::vox;

fn main() {
    let mut rows: Vec<(&str, usize, usize)> = vox::models()
        .map(|(name, model)| (name, model.filled(), model.quads().len()))
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.2));
    println!("{:<22} {:>8} {:>8}", "model", "voxels", "quads");
    for (name, voxels, quads) in &rows {
        println!("{name:<22} {voxels:>8} {quads:>8}");
    }
    println!(
        "{:<22} {:>8} {:>8}",
        "total",
        rows.iter().map(|r| r.1).sum::<usize>(),
        rows.iter().map(|r| r.2).sum::<usize>()
    );
}
