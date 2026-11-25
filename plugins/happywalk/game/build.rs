//! Every `.vox` file in `assets/vox/` is built into the game and found by its
//! file name: adding a model is dropping a file there.

use std::{env, fs, path::Path};

fn main() {
    let dir = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets/vox");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut names: Vec<String> = fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .filter(|path| path.extension().is_some_and(|ext| ext == "vox"))
                .filter_map(|path| {
                    path.file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    let mut out = String::from("pub const VOX_ASSETS: &[(&str, &[u8])] = &[\n");
    for name in &names {
        let path = dir.join(format!("{name}.vox"));
        println!("cargo:rerun-if-changed={}", path.display());
        out.push_str(&format!(
            "    ({name:?}, include_bytes!({:?})),\n",
            path.display().to_string()
        ));
    }
    out.push_str("];\n");
    fs::write(
        Path::new(&env::var("OUT_DIR").unwrap()).join("vox_assets.rs"),
        out,
    )
    .unwrap();
}
