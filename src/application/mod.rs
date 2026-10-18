//! Application composition. Construction performs no probes, starts no workers,
//! loads no plugins and never writes a session token. Production and isolated
//! tests use the same factory; startup owns all side effects explicitly.
pub mod lifecycle;
pub mod logging;
pub mod router;
pub mod sidecars;
pub mod voice_watchdog;

use crate::services::audio::AudioService;
use crate::services::bluetooth::BluetoothService;
use crate::services::diary_gen::DiaryGenerator;
use crate::services::display::DisplayService;
use crate::services::gpsd::GpsdService;
use crate::services::keyboard_backlight::KeyboardBacklightService;
use crate::services::network::NetworkService;
use crate::services::ollama::OllamaClient;
use crate::services::osm::OsmService;
use crate::services::power::PowerService;
use crate::services::screen_brightness::ScreenBrightnessService;
use crate::services::supertonic::SupertonicClient;
use crate::services::touchbar::TouchBarService;
use crate::services::web_search::SearchService;
use crate::services::whisper::WhisperClient;
use crate::{api::AppState, config::Config};
use sqlx::SqlitePool;
use std::sync::Arc;

impl AppState {
    pub fn new(config: Config, pool: SqlitePool) -> Self {
        let ollama = OllamaClient::new(config.ollama_url.clone(), config.ollama_model.clone());
        let osm = OsmService::new();
        let diary_gen = Arc::new(DiaryGenerator::new(
            pool.clone(),
            ollama.clone(),
            osm.clone(),
        ));
        AppState {
            pool: pool.clone(),
            ollama,
            search: SearchService::new(),
            osm,
            gpsd: GpsdService::new(config.gpsd_host.clone(), config.gpsd_port),
            network: NetworkService::new(),
            audio: AudioService::new(),
            battery: crate::services::battery::BatteryService::new(),
            bluetooth: BluetoothService::new(),
            power: PowerService::new(),
            display: DisplayService::new(),
            touchbar: TouchBarService::new(),
            keyboard_backlight: KeyboardBacklightService::new(),
            screen_brightness: ScreenBrightnessService::new(),
            diary_gen,
            agent_turns: Default::default(),
            supertonic: SupertonicClient::new(
                config.supertonic_url.clone(),
                config.supertonic_voice.clone(),
            ),
            whisper: WhisperClient::new(
                config.whisper_url.clone(),
                config.whisper_models_dir.clone(),
            ),
            qwen_tts: crate::services::qwen_tts::QwenClient::new(
                config.qwen_tts_url.clone(),
                std::path::PathBuf::from(&config.qwen_tts_models_dir),
            ),
            image_gen: crate::services::image_gen::ImageGenClient::new(
                vec![
                    (
                        crate::services::image_gen::Z_IMAGE.id,
                        std::path::PathBuf::from(&config.z_image_models_dir),
                    ),
                    (
                        crate::services::image_gen::QWEN_IMAGE.id,
                        std::path::PathBuf::from(&config.qwen_image_models_dir),
                    ),
                ],
                config.z_image_sd_cli.clone(),
            ),
            plugins: crate::plugins::PluginManager::with_system_dir(
                std::path::PathBuf::from(&config.plugins_dir),
                config
                    .system_plugins_dir
                    .as_ref()
                    .map(std::path::PathBuf::from),
                pool.clone(),
            ),
            song_gen: crate::services::song_gen::SongGenClient::new(
                std::path::PathBuf::from(&config.yue2_models_dir),
                config.yue2_engine.clone(),
            ),
            store: crate::plugins::store::Store::new(),
            craft_mcp: crate::plugins::craft_mcp::CraftMcp::new(),
            iroh: crate::services::iroh_remote::IrohRemote::new(),
            tailscale: crate::services::tailscale::TailscaleService::new(config.server_port),
            session: Default::default(),
            router_rebuild: None,
            config,
        }
    }
}
