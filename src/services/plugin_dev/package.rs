//! From a checked, built workspace to an installed plugin: the archive, and
//! its installation through the ordinary installer.
//!
//! Each step refuses to run on sources the step before it did not see:
//! packing needs the checks to have passed on the current sources (and, for a
//! compiled plugin, the executable to be built from them), and installing
//! takes only the archive the last pack made, checked against its SHA-256.

use std::io::Write;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::workspace::{Packed, Workspace};
use crate::errors::AppError;

/// What goes in an archive besides `plugin.toml` and the executable.
const CONTENT_DIRS: &[&str] = &["web", "skills", "migrations"];

fn refused(message: impl Into<String>) -> AppError {
    AppError::BadRequest(message.into())
}

/// The version in the workspace's manifest.
pub fn manifest_version(workspace: &Workspace) -> Result<String, AppError> {
    let text = std::fs::read_to_string(workspace.dir.join("plugin.toml"))
        .map_err(|_| refused("plugin.toml is missing"))?;
    let table: toml::Value = toml::from_str(&text).map_err(|e| refused(format!("plugin.toml: {}", e.message())))?;
    let version = table.get("version").and_then(toml::Value::as_str).unwrap_or_default();
    semver::Version::parse(version)
        .map(|version| version.to_string())
        .map_err(|_| refused("plugin.toml has no valid version"))
}

/// Make `dist/<name>-<version>.tar.gz` from the workspace.
pub fn pack(workspace: &mut Workspace) -> Result<Packed, AppError> {
    let sources = workspace.sources_digest();
    if workspace.meta.checked.as_deref() != Some(sources.as_str()) {
        return Err(refused(
            "the checks have not passed on these sources. Run plugin_dev_check (all of it) and fix what it \
             reports, then pack.",
        ));
    }
    let executable = crate::plugins::process::find_executable(&workspace.dir, &workspace.name);
    let Some(executable) = executable.filter(|_| workspace.meta.built.as_deref() == Some(sources.as_str())) else {
        return Err(refused(
            "the plugin's engine is not built from these sources. Build it, then pack.",
        ));
    };
    let version = manifest_version(workspace)?;

    let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default()));
    let top = Path::new(&workspace.name);
    let mut add = |relative: &Path, mode: u32| -> Result<(), AppError> {
        let bytes = std::fs::read(workspace.dir.join(relative))?;
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(mode);
        // A fixed time: the same sources give the same archive.
        header.set_mtime(0);
        header.set_cksum();
        archive.append_data(&mut header, top.join(relative), bytes.as_slice())?;
        Ok(())
    };
    add(Path::new("plugin.toml"), 0o644)?;
    add(executable.strip_prefix(&workspace.dir).unwrap_or(&executable), 0o755)?;
    for entry in workspace.files() {
        let in_content = CONTENT_DIRS.iter().any(|dir| entry.path.starts_with(&format!("{dir}/")));
        if in_content {
            add(Path::new(&entry.path), 0o644)?;
        }
    }
    let bytes = archive
        .into_inner()
        .and_then(|encoder| encoder.finish())
        .map_err(|e| AppError::Internal(format!("could not write the archive: {e}")))?;

    let file = format!("dist/{}-{version}.tar.gz", workspace.name);
    let path = workspace.dir.join(&file);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = std::fs::File::create(&path)?;
    out.write_all(&bytes)?;
    out.sync_all()?;

    let packed = Packed { sources, file, sha256: hex::encode(Sha256::digest(&bytes)), version };
    workspace.meta.packed = Some(packed.clone());
    workspace.save_meta()?;
    Ok(packed)
}

/// The bytes of the archive the last pack made, when it is still the pack of
/// the current sources and has not been touched since.
pub fn packed_archive(workspace: &Workspace) -> Result<(Packed, Vec<u8>), AppError> {
    let Some(packed) = workspace.meta.packed.clone() else {
        return Err(refused("this plugin has not been packed. Run plugin_dev_pack first."));
    };
    if packed.sources != workspace.sources_digest() {
        return Err(refused(
            "the sources changed since the plugin was packed. Check, build and pack again, then install.",
        ));
    }
    let bytes = std::fs::read(workspace.dir.join(&packed.file))
        .map_err(|_| refused("the packed archive is missing. Run plugin_dev_pack again."))?;
    if hex::encode(Sha256::digest(&bytes)) != packed.sha256 {
        return Err(refused(
            "the archive on disk is not the one plugin_dev_pack made. Run plugin_dev_pack again.",
        ));
    }
    Ok((packed, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::plugin_dev::workspace::{scaffold_common, NewPlugin, TemplateValues, Workspaces};
    use crate::services::plugin_dev::Language;

    /// A scaffolded workspace with a stand-in executable, checked and built.
    fn ready() -> (tempfile::TempDir, Workspace) {
        let root = tempfile::tempdir().unwrap();
        let mut workspace = Workspaces::at(root.path().join("plugin-dev")).create("notes", Language::Rust).unwrap();
        let values = TemplateValues::of(&NewPlugin {
            name: "notes".into(),
            title: "Notes".into(),
            description: "Quick notes".into(),
            category: "Office".into(),
        });
        scaffold_common(&workspace, &values).unwrap();
        std::fs::create_dir_all(workspace.dir.join("bin")).unwrap();
        std::fs::write(workspace.dir.join("bin/notes"), b"#!/bin/sh\n").unwrap();
        let digest = workspace.sources_digest();
        workspace.meta.checked = Some(digest.clone());
        workspace.meta.built = Some(digest);
        (root, workspace)
    }

    fn entries(bytes: &[u8]) -> Vec<(String, u32)> {
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
        let mut found: Vec<(String, u32)> = archive
            .entries()
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (entry.path().unwrap().to_string_lossy().into_owned(), entry.header().mode().unwrap())
            })
            .collect();
        found.sort();
        found
    }

    #[test]
    fn the_archive_holds_the_plugin_and_nothing_else() {
        let (_root, mut workspace) = ready();
        // Sources of the engine are not part of what gets installed.
        std::fs::create_dir_all(workspace.dir.join("target")).unwrap();
        std::fs::write(workspace.dir.join("target/junk"), "x").unwrap();
        let packed = pack(&mut workspace).unwrap();
        assert_eq!(packed.file, "dist/notes-0.1.0.tar.gz");
        assert_eq!(packed.version, "0.1.0");

        let (again, bytes) = packed_archive(&workspace).unwrap();
        assert_eq!(again, packed);
        assert_eq!(
            entries(&bytes),
            [
                ("notes/bin/notes".to_string(), 0o755),
                ("notes/migrations/001_init.sql".to_string(), 0o644),
                ("notes/plugin.toml".to_string(), 0o644),
                ("notes/skills/notes.md".to_string(), 0o644),
                ("notes/web/i18n/en.json".to_string(), 0o644),
                ("notes/web/icon.svg".to_string(), 0o644),
                ("notes/web/plugin.css".to_string(), 0o644),
                ("notes/web/plugin.js".to_string(), 0o644),
            ]
        );
        // The same sources pack to the same bytes.
        assert_eq!(pack(&mut workspace).unwrap().sha256, packed.sha256);
    }

    #[test]
    fn packing_needs_checked_and_built_sources() {
        let (_root, mut workspace) = ready();
        workspace.write("web/plugin.css", "/* changed */").unwrap();
        let error = pack(&mut workspace).unwrap_err().to_string();
        assert!(error.contains("checks have not passed"), "{error}");

        workspace.meta.checked = Some(workspace.sources_digest());
        let error = pack(&mut workspace).unwrap_err().to_string();
        assert!(error.contains("not built from these sources"), "{error}");

        workspace.meta.built = Some(workspace.sources_digest());
        std::fs::remove_file(workspace.dir.join("bin/notes")).unwrap();
        assert!(pack(&mut workspace).unwrap_err().to_string().contains("not built"));
    }

    #[test]
    fn only_the_archive_of_the_current_sources_installs() {
        let (_root, mut workspace) = ready();
        assert!(packed_archive(&workspace).unwrap_err().to_string().contains("has not been packed"));
        let packed = pack(&mut workspace).unwrap();

        // The archive was replaced on disk.
        std::fs::write(workspace.dir.join(&packed.file), b"something else").unwrap();
        assert!(packed_archive(&workspace).unwrap_err().to_string().contains("not the one plugin_dev_pack made"));
        std::fs::remove_file(workspace.dir.join(&packed.file)).unwrap();
        assert!(packed_archive(&workspace).unwrap_err().to_string().contains("missing"));

        // The sources moved on after the pack.
        pack(&mut workspace).unwrap();
        workspace.write("skills/notes.md", "# changed").unwrap();
        assert!(packed_archive(&workspace).unwrap_err().to_string().contains("sources changed"));
    }
}
