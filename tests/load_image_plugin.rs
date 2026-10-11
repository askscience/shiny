//! Load the built `image` plugin the way the host does: over a copied install
//! tree with the release cdylib, against a scratch database. This exercises
//! dlopen, the manifest, migrations (including the selection migration) and
//! tool/route registration without starting the full server.
//!
//! Run with:
//!     cargo test -p shiny --test load_image_plugin -- --nocapture

use std::sync::Arc;

use shiny::api::AppState;
use shiny::config::Config;
use shiny::services::audio::AudioService;
use shiny::services::bluetooth::BluetoothService;
use shiny::services::diary_gen::DiaryGenerator;
use shiny::services::display::DisplayService;
use shiny::services::gpsd::GpsdService;
use shiny::services::keyboard_backlight::KeyboardBacklightService;
use shiny::services::network::NetworkService;
use shiny::services::ollama::OllamaClient;
use shiny::services::osm::OsmService;
use shiny::services::power::PowerService;
use shiny::services::screen_brightness::ScreenBrightnessService;
use shiny::services::supertonic::SupertonicClient;
use shiny::services::touchbar::TouchBarService;
use shiny::services::web_search::SearchService;
use shiny::services::whisper::WhisperClient;

async fn state_for(plugins_dir: &str, db: &str) -> AppState {
    let db_url = format!("sqlite://{db}?mode=rwc");
    let mut conn = shiny::db::connect(&db_url).await.expect("connection");
    shiny::db::run_migrations(&mut conn).await.expect("migrations");
    drop(conn);
    let pool = shiny::db::init_pool(&db_url).await.expect("pool");

    let mut config = Config::from_env();
    config.plugins_dir = plugins_dir.to_string();
    config.database_url = db_url;

    AppState {
        pool: pool.clone(),
        ollama: OllamaClient::new(config.ollama_url.clone(), config.ollama_model.clone()),
        search: SearchService::new(),
        osm: OsmService::new(),
        gpsd: GpsdService::new(config.gpsd_host.clone(), config.gpsd_port),
        network: NetworkService::new(),
        audio: AudioService::new(),
        battery: shiny::services::battery::BatteryService::new(),
        bluetooth: BluetoothService::new(),
        power: PowerService::new(),
        display: DisplayService::new(),
        touchbar: TouchBarService::new(),
        keyboard_backlight: KeyboardBacklightService::new(),
        screen_brightness: ScreenBrightnessService::new(),
        diary_gen: Arc::new(DiaryGenerator::new(
            pool.clone(),
            OllamaClient::new(config.ollama_url.clone(), config.ollama_model.clone()),
            OsmService::new(),
        )),
        agent_turns: Default::default(),
        supertonic: SupertonicClient::new(config.supertonic_url.clone(), config.supertonic_voice.clone()),
        whisper: WhisperClient::new(config.whisper_url.clone(), config.whisper_models_dir.clone()),
        qwen_tts: shiny::services::qwen_tts::QwenClient::new(
            config.qwen_tts_url.clone(),
            std::path::PathBuf::from(&config.qwen_tts_models_dir),
        ),
        image_gen: shiny::services::image_gen::ImageGenClient::new(
            vec![
                (
                    shiny::services::image_gen::Z_IMAGE.id,
                    std::path::PathBuf::from(&config.z_image_models_dir),
                ),
                (
                    shiny::services::image_gen::QWEN_IMAGE.id,
                    std::path::PathBuf::from(&config.qwen_image_models_dir),
                ),
            ],
            config.z_image_sd_cli.clone(),
        ),
        plugins: shiny::plugins::PluginManager::new(
            std::path::PathBuf::from(&config.plugins_dir),
            pool.clone(),
        ),
        iroh: shiny::services::iroh_remote::IrohRemote::new(),
        tailscale: shiny::services::tailscale::TailscaleService::new(config.server_port),
        session: Default::default(),
        router_rebuild: None,
        config,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn loads_the_image_plugin() {
    let dir = std::path::PathBuf::from("/tmp/image-plugin-probe");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let plugin_dir = dir.join("image");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::copy("plugins/image/plugin.toml", plugin_dir.join("plugin.toml")).unwrap();
    for (src, dst) in [
        ("plugins/image/migrations", "migrations"),
        ("plugins/image/web", "web"),
        ("plugins/image/skills", "skills"),
    ] {
        copy_dir(src, &plugin_dir.join(dst));
    }
    let cdylib = "target/release/libshiny_image_plugin.so";
    if !std::path::Path::new(cdylib).exists() {
        eprintln!("skipping: build it first with `cargo build --release -p shiny-image-plugin`");
        return;
    }
    std::fs::copy(cdylib, plugin_dir.join("libshiny_image_plugin.so"))
        .expect("copy cdylib");

    let db = "/tmp/image-plugin-probe.db";
    let _ = std::fs::remove_file(db);

    let state = state_for(dir.to_str().unwrap(), db).await;
    let installed = state.plugins.discover_and_install(state.plugin_ctx()).await;
    println!("installed: {installed:?}");

    let names = state.plugins.tools().list();
    for tool in ["image_list", "image_get", "image_edit", "image_delete"] {
        assert!(names.iter().any(|t| t == tool), "{tool} registered; got {names:?}");
    }
    // The layers era is gone: no layer tools, no flatten, and no
    // AI-edit-sounding aliases (`edit_image` used to swallow intent meant for
    // core `generate_image`).
    for gone in [
        "image_layer_list", "image_layer_add", "image_layer_delete", "image_flatten",
        "add_layer", "flatten_image", "edit_image", "transform_image",
    ] {
        assert!(!names.iter().any(|t| t == gone), "{gone} should be gone; got {names:?}");
    }
    // Reaching here means the manifest, cdylib and *all* migrations (including
    // 005_minimal_layers and 006_drop_layers) applied cleanly, since a
    // migration failure aborts install before tools are registered.
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
