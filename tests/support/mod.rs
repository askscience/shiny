#![allow(dead_code)]
//! Hermetic application and native plugin fixtures. No server workers start,
//! no live plugin directory is read, and every writable path is temporary.
use shiny::{api::AppState, config::Config};
use std::path::{Path, PathBuf};

pub struct TestApp {
    pub state: AppState,
    pub root: tempfile::TempDir,
}

impl TestApp {
    pub async fn new() -> Self {
        let root = tempfile::tempdir().expect("temporary application directory");
        let mut config = Config::from_env();
        let path = |name: &str| root.path().join(name).to_string_lossy().into_owned();
        config.database_url = format!("sqlite://{}", path("state.db"));
        config.plugins_dir = path("plugins");
        config.system_plugins_dir = None;
        config.backgrounds_dir = path("backgrounds");
        config.log_file = path("shiny.log");
        config.vosk_models_dir = path("vosk");
        config.whisper_models_dir = path("whisper");
        config.qwen_tts_models_dir = path("qwen");
        config.z_image_models_dir = path("images");
        config.qwen_image_models_dir = path("qwen-images");
        config.yue2_models_dir = path("songs");
        config.z_image_sd_cli = None;
        config.yue2_engine = None;
        config.auto_start_supertonic = false;
        config.auto_start_whisper = false;
        config.auto_start_qwen_tts = false;
        config.auto_install_supertonic = false;
        config.auto_install_whisper = false;
        config.diary_auto_generate = false;
        config.linux_users = false;
        config.home_mode = "virtual".into();
        config.auth_enabled = false;
        config.greeter_mode = false;
        config.login_self_only = false;
        config.session_token_file = None;
        config.admin_token = None;
        // No provider should be contacted by a fixture. Closed loopback ports
        // make accidental calls fail without reaching the user's AI services.
        for url in [
            &mut config.ollama_url,
            &mut config.whisper_url,
            &mut config.supertonic_url,
            &mut config.qwen_tts_url,
        ] {
            *url = "http://127.0.0.1:1".into();
        }
        std::fs::create_dir_all(&config.plugins_dir).unwrap();
        let mut conn = shiny::db::connect(&config.database_url).await.unwrap();
        shiny::db::run_migrations(&mut conn).await.unwrap();
        drop(conn);
        let pool = shiny::db::init_pool(&config.database_url).await.unwrap();
        Self {
            state: AppState::new(config, pool),
            root,
        }
    }

    pub async fn load_plugins(&self) -> Vec<String> {
        let mut dirs: Vec<_> = std::fs::read_dir(&self.state.config.plugins_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.join("plugin.toml").is_file())
            .collect();
        dirs.sort();
        let mut loaded = Vec::new();
        for dir in dirs {
            loaded.push(
                self.state
                    .plugins
                    .register_dir_static(&dir, self.state.plugin_ctx())
                    .await
                    .unwrap_or_else(|error| panic!("Fixture {} failed: {error}", dir.display())),
            );
        }
        loaded
    }

    pub fn stage_plugin(&self, name: &str) {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("plugins")
            .join(name);
        let destination = Path::new(&self.state.config.plugins_dir).join(name);
        copy_dir(&source, &destination);
        let library = format!(
            "{}shiny_{}_plugin{}",
            std::env::consts::DLL_PREFIX,
            name.replace('-', "_"),
            std::env::consts::DLL_SUFFIX
        );
        let fixture_dir = std::env::var_os("SHINY_TEST_LIB_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug"));
        let artifact = fixture_dir.join(&library);
        assert!(artifact.is_file(), "Missing fixture {}. Run scripts/test-rust.sh first; tests never use installed plugins.", artifact.display());
        std::fs::copy(artifact, destination.join(library)).expect("copy built plugin fixture");
    }

    pub fn stage_all_plugins(&self) {
        for entry in
            std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins")).unwrap()
        {
            let entry = entry.unwrap();
            if entry.path().join("plugin.toml").is_file() {
                self.stage_plugin(entry.file_name().to_str().unwrap());
            }
        }
    }
}

fn copy_dir(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &destination.join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
        }
    }
}
