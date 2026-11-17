//! A plugin in the user directory overrides the system copy of the same name.
//! At start-up only the override loads: loading the system copy first ran its
//! `on_load` and, for a process plugin, started a process only to kill it.
//! When the override does not load, the system copy is the fallback.
mod support;

use shiny::plugins::manager::PluginManager;
use std::path::{Path, PathBuf};

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

#[tokio::test]
async fn an_override_starts_once_and_a_broken_one_falls_back() {
    let app = support::TestApp::new().await;
    app.stage_plugin("hello");
    app.stage_plugin("calculator");
    let user = PathBuf::from(&app.state.config.plugins_dir);
    let system = app.root.path().join("system-plugins");
    copy_dir(&user.join("hello"), &system.join("hello"));
    copy_dir(&user.join("calculator"), &system.join("calculator"));

    // hello: overridden by a good user copy. calculator: the user copy has no
    // executable, so it cannot load.
    std::fs::remove_file(user.join("calculator/bin/calculator")).unwrap();

    let manager =
        PluginManager::with_system_dir(user.clone(), Some(system.clone()), app.state.pool.clone());
    let before = shiny::plugins::process::started_processes();
    let mut loaded = manager.discover_and_install(app.state.plugin_ctx()).await;
    loaded.sort();
    assert_eq!(loaded, vec!["calculator".to_string(), "hello".to_string()]);
    // One process each: no system hello started and replaced.
    assert_eq!(shiny::plugins::process::started_processes() - before, 2);
    assert_eq!(
        manager.loader().install_dir_of("hello").unwrap(),
        user.join("hello")
    );
    assert_eq!(
        manager.loader().install_dir_of("calculator").unwrap(),
        system.join("calculator")
    );
    manager.stop_processes().await;
}
