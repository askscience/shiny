//! The plugin catalogue installs process plugins from release archives: it
//! checks the archive against the catalogue's SHA-256, installs a default
//! plugin once (and not again after the user removed it), and refuses an
//! unsigned catalogue unless it is a developer's local one.
mod support;

use sha2::Digest;
use shiny::plugins::catalog::{host_arch, Catalog, Settings};
use std::path::{Path, PathBuf};

/// A release archive of the hello plugin, as build-plugin-archives.sh makes it.
fn hello_archive(dir: &Path) -> (PathBuf, String) {
    let staged = dir.join("stage/hello");
    std::fs::create_dir_all(staged.join("bin")).unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    std::fs::copy(
        repo.join("plugins/hello/plugin.toml"),
        staged.join("plugin.toml"),
    )
    .unwrap();
    let fixture_dir = std::env::var_os("SHINY_TEST_LIB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join("target/debug"));
    std::fs::copy(
        fixture_dir.join("shiny-plugin-hello"),
        staged.join("bin/hello"),
    )
    .expect("hello executable (run scripts/test-rust.sh)");
    let archive = dir.join("hello.tar.gz");
    let file = std::fs::File::create(&archive).unwrap();
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        file,
        flate2::Compression::fast(),
    ));
    builder.append_dir_all("hello", &staged).unwrap();
    builder.into_inner().unwrap().finish().unwrap();
    let sha = hex::encode(sha2::Sha256::digest(std::fs::read(&archive).unwrap()));
    (archive, sha)
}

fn write_index(dir: &Path, archive: &Path, sha: &str) -> PathBuf {
    let index = serde_json::json!({
        "format": 1,
        "release": "test",
        "plugins": [{
            "name": "hello",
            "version": "9.0.0",
            "description": "Demo",
            "arch": host_arch(),
            "protocol": { "min": 1, "max": 1 },
            "url": format!("file://{}", archive.display()),
            "size": std::fs::metadata(archive).unwrap().len(),
            "sha256": sha,
        }],
    });
    let path = dir.join("index.json");
    std::fs::write(&path, serde_json::to_vec(&index).unwrap()).unwrap();
    path
}

fn local_catalog(index: &Path, defaults: &Path) -> Catalog {
    Catalog::new(Settings {
        index_url: format!("file://{}", index.display()),
        keyring: "/nonexistent".into(),
        allow_unsigned: true,
        allow_any_host: true,
        defaults_file: defaults.to_path_buf(),
        auto_maintain: false,
    })
}

#[tokio::test]
async fn catalogue_installs_defaults_once_and_checks_archives() {
    let app = support::TestApp::new().await;
    let work = tempfile::tempdir().unwrap();
    let (archive, sha) = hello_archive(work.path());
    let defaults = work.path().join("default-plugins");
    std::fs::write(&defaults, "# defaults\nhello\n").unwrap();

    // A wrong checksum is refused and installs nothing.
    let bad = write_index(work.path(), &archive, &"0".repeat(64));
    let catalog = local_catalog(&bad, &defaults);
    let entry = catalog.index(true).await.unwrap().plugins.remove(0);
    let error = catalog.install(&app.state, &entry).await.unwrap_err();
    assert!(error.to_string().contains("checksum mismatch"), "{error}");
    assert!(!app.state.plugins.is_installed("hello"));

    // The default is installed by maintenance, and runs as a process.
    let index = write_index(work.path(), &archive, &sha);
    let catalog = local_catalog(&index, &defaults);
    assert_eq!(
        catalog.maintain(&app.state).await.unwrap(),
        vec!["hello".to_string()]
    );
    assert!(app.state.plugins.is_installed("hello"));
    assert!(app
        .state
        .plugins
        .loader()
        .supervisor_for("hello")
        .unwrap()
        .pid()
        .is_some());
    let snapshot = catalog.snapshot(&app.state);
    assert_eq!(snapshot["plugins"][0]["installed_version"], "0.1.1");
    assert_eq!(snapshot["plugins"][0]["update_available"], true);

    // Removed by the user: maintenance leaves it removed.
    assert!(app.state.plugins.uninstall("hello").await);
    std::fs::remove_dir_all(Path::new(&app.state.config.plugins_dir).join("hello")).unwrap();
    assert!(catalog.maintain(&app.state).await.unwrap().is_empty());
    assert!(!app.state.plugins.is_installed("hello"));
}

#[tokio::test]
async fn a_published_catalogue_must_be_signed() {
    let work = tempfile::tempdir().unwrap();
    let index = work.path().join("index.json");
    std::fs::write(&index, r#"{"format":1,"plugins":[]}"#).unwrap();
    let catalog = Catalog::new(Settings {
        index_url: format!("file://{}", index.display()),
        keyring: "/nonexistent".into(),
        allow_unsigned: false,
        allow_any_host: true,
        defaults_file: "/nonexistent".into(),
        auto_maintain: false,
    });
    let error = catalog.index(true).await.unwrap_err();
    assert!(error.contains("signature"), "{error}");

    // A signature that is there but cannot be checked is refused too.
    std::fs::write(work.path().join("index.json.sig"), b"not a signature").unwrap();
    let error = catalog.index(true).await.unwrap_err();
    assert!(
        error.contains("keyring") || error.contains("signature"),
        "{error}"
    );
}
