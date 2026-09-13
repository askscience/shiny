//! Drives the *real* host loader (`PluginManager::discover_and_install`) over
//! a scratch plugins directory, so a plugin-install crash can be reproduced
//! outside the running server and its panic message read directly.
//!
//! Run with:
//!     cargo test -p shiny --test load_plugin -- --nocapture

use std::sync::Arc;

use shiny::api::AppState;
use shiny::config::Config;
use shiny::services::diary_gen::DiaryGenerator;
use shiny::services::gpsd::GpsdService;
use shiny::services::ollama::OllamaClient;
use shiny::services::osm::OsmService;
use shiny::services::supertonic::SupertonicClient;
use shiny::services::web_search::SearchService;
use shiny::services::whisper::WhisperClient;

/// Build an `AppState` pointed at a scratch plugins dir and database.
async fn state_for(plugins_dir: &str, db: &str) -> AppState {
    let db_url = format!("sqlite://{db}?mode=rwc");
    let pool = shiny::db::init_pool(&db_url).await.expect("pool");
    shiny::db::run_migrations(&pool).await.expect("migrations");

    let mut config = Config::from_env();
    config.plugins_dir = plugins_dir.to_string();
    config.database_url = db_url;

    AppState {
        pool: pool.clone(),
        ollama: OllamaClient::new(config.ollama_url.clone(), config.ollama_model.clone()),
        search: SearchService::new(),
        osm: OsmService::new(),
        gpsd: GpsdService::new(config.gpsd_host.clone(), config.gpsd_port),
        diary_gen: Arc::new(DiaryGenerator::new(
            pool.clone(),
            OllamaClient::new(config.ollama_url.clone(), config.ollama_model.clone()),
            OsmService::new(),
        )),
        agent_turns: Default::default(),
        supertonic: SupertonicClient::new(config.supertonic_url.clone(), config.supertonic_voice.clone()),
        whisper: WhisperClient::new(config.whisper_url.clone(), config.whisper_models_dir.clone()),
        plugins: shiny::plugins::PluginManager::new(
            std::path::PathBuf::from(&config.plugins_dir),
            pool.clone(),
        ),
        router_rebuild: None,
        config,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn loads_the_peakd_plugin() {
    // The plugin's source tree is the installed layout, minus the cdylib,
    // which we point at from target/.
    let dir = std::path::PathBuf::from("/tmp/dllprobe-plugins");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let plugin_dir = dir.join("peakd");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    for file in ["plugin.toml"] {
        std::fs::copy(format!("plugins/peakd/{file}"), plugin_dir.join(file)).unwrap();
    }
    for (src, dst) in [
        ("plugins/peakd/migrations", "migrations"),
        ("plugins/peakd/web", "web"),
        ("plugins/peakd/skills", "skills"),
    ] {
        copy_dir(src, &plugin_dir.join(dst));
    }
    std::fs::copy(
        "target/release/libshiny_peakd_plugin.dylib",
        plugin_dir.join("libshiny_peakd_plugin.dylib"),
    )
    .expect("cdylib — build it first: cargo build --release -p shiny-peakd-plugin");

    let db = "/tmp/dllprobe-plugins.db";
    let _ = std::fs::remove_file(db);

    println!("--- building state ---");
    let state = state_for(dir.to_str().unwrap(), db).await;
    println!("--- discover_and_install ---");
    let installed = state.plugins.discover_and_install(state.plugin_ctx()).await;
    println!("installed: {installed:?}");

    println!("--- list ---");
    for m in state.plugins.list() {
        println!("  {} v{}", m.name, m.version);
    }
    println!("--- registry tool for browser ---");
    println!("  registered tools: {:?}", state.plugins.tools().list());
    println!("SURVIVED");
}

fn copy_dir(src: &str, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(entry.path().to_str().unwrap(), &to);
        } else {
            std::fs::copy(entry.path(), to).unwrap();
        }
    }
}
