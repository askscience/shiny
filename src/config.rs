use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub server_host: String,
    pub server_port: u16,
    pub database_url: String,
    pub ollama_url: String,
    pub ollama_model: String,
    pub gpsd_host: String,
    pub gpsd_port: u16,
    pub diary_auto_generate: bool,
    pub diary_generate_time: String,
    pub log_level: String,
    /// File the tracer also writes to (tee'd alongside stdout).
    pub log_file: String,
    pub supertonic_url: String,
    pub supertonic_voice: String,
    pub vosk_models_dir: String,
    /// URL of the faster-whisper streaming STT sidecar.
    pub whisper_url: String,
    /// Directory holding faster-whisper (CTranslate2) model folders.
    pub whisper_models_dir: String,
    /// Start the faster-whisper sidecar together with the server.
    pub auto_start_whisper: bool,
    /// Let the launcher build `.venv-whisper` and pip-install faster-whisper
    /// when no interpreter on the machine already provides it.
    pub auto_install_whisper: bool,
    /// Explicit interpreter for the sidecar (must have faster-whisper).
    pub whisper_python: Option<String>,
    /// Start the Supertonic TTS sidecar together with the server.
    pub auto_start_supertonic: bool,
    /// Let the launcher build `.venv-supertonic` and pip-install the package
    /// when no interpreter on the machine already provides it.
    pub auto_install_supertonic: bool,
    /// Explicit interpreter for the sidecar (must have supertonic[serve]).
    pub supertonic_python: Option<String>,
    /// URL of the Qwen3-TTS sidecar (qwentts.cpp `tts-server`).
    pub qwen_tts_url: String,
    /// Directory holding the Qwen3-TTS GGUF weights.
    pub qwen_tts_models_dir: String,
    /// Start the Qwen3-TTS sidecar together with the server.
    pub auto_start_qwen_tts: bool,
    pub web_dir: String,
    /// Writable per-user plugin directory: uploads land here.
    pub plugins_dir: String,
    /// Read-only system plugin baseline, if any. Plugins found here load for
    /// every user; a plugin with the same name in `plugins_dir` overrides it.
    pub system_plugins_dir: Option<String>,
    /// Directory holding per-user desktop background images.
    pub backgrounds_dir: String,
    pub admin_token: Option<String>,
    /// Bind Shiny accounts to real Linux accounts (NSS lookup + PAM login).
    /// Off by default: without it nothing changes.
    pub linux_users: bool,
    /// Files-plugin home mode: `"real"` uses the account's OS home directory,
    /// `"virtual"` (default) keeps `~/.shiny/home/<id>`.
    pub home_mode: String,
    /// Verify the real Linux password through the privileged `shiny-auth`
    /// helper instead of the local Argon2 hash (falls back when unavailable).
    pub auth_enabled: bool,
    /// Unix socket the `shiny-auth` helper listens on.
    pub auth_sock: String,
    /// Greeter mode: this server *is* the login screen. `POST /api/auth/login`
    /// verifies through the helper and starts that user's own kiosk session —
    /// no Shiny account is created and no cookie is issued here.
    pub greeter_mode: bool,
    /// User sessions: only accept a PAM login for the account the server runs
    /// as. Every plugin (Terminal included) executes as that user, so another
    /// account's credentials must not be accepted by this process. Defaults to
    /// on when Linux-user mode is on; `SHINY_LOGIN_SELF_ONLY=false` opts out.
    pub login_self_only: bool,
    /// Where the loopback-only session token is written (default
    /// `$XDG_RUNTIME_DIR/shiny-session-token`).
    pub session_token_file: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        let linux_users = env::var("SHINY_LINUX_USERS")
            .unwrap_or_else(|_| "false".into())
            .parse()
            .unwrap_or(false);
        // Another account's password must never be accepted by a process that
        // runs as the session user — every plugin (the Terminal's PTY too)
        // executes as this process. On by default in Linux-user mode.
        let login_self_only = match env::var("SHINY_LOGIN_SELF_ONLY") {
            Ok(v) => v.trim() != "0" && !v.trim().eq_ignore_ascii_case("false"),
            Err(_) => linux_users,
        };
        Self {
            server_host: env::var("SERVER_HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            server_port: env::var("SERVER_PORT")
                .unwrap_or_else(|_| "8080".into())
                .parse()
                .unwrap_or(8080),
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite://data/traveler.db".into()),
            ollama_url: env::var("OLLAMA_URL")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "http://127.0.0.1:11434".into()),
            ollama_model: env::var("OLLAMA_MODEL").unwrap_or_else(|_| "gemma4:31b-cloud".into()),
            gpsd_host: env::var("GPSD_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            gpsd_port: env::var("GPSD_PORT")
                .unwrap_or_else(|_| "2947".into())
                .parse()
                .unwrap_or(2947),
            diary_auto_generate: env::var("DIARY_AUTO_GENERATE")
                .unwrap_or_else(|_| "true".into())
                .parse()
                .unwrap_or(true),
            diary_generate_time: env::var("DIARY_GENERATE_TIME").unwrap_or_else(|_| "21:00".into()),
            log_level: env::var("LOG_LEVEL").unwrap_or_else(|_| "info".into()),
            log_file: env::var("LOG_FILE").unwrap_or_else(|_| "data/shiny.log".into()),
            supertonic_url: env::var("SUPERTONIC_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:7788".into()),
            supertonic_voice: env::var("SUPERTONIC_VOICE").unwrap_or_else(|_| "M1".into()),
            vosk_models_dir: env::var("VOSK_MODELS_DIR")
                .unwrap_or_else(|_| "data/vosk-models".into()),
            whisper_url: env::var("WHISPER_URL")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "http://127.0.0.1:7789".into()),
            whisper_models_dir: env::var("WHISPER_MODELS_DIR")
                .unwrap_or_else(|_| "data/whisper-models".into()),
            auto_start_whisper: env::var("AUTO_START_WHISPER")
                .unwrap_or_else(|_| "true".into())
                .parse()
                .unwrap_or(true),
            auto_install_whisper: env::var("WHISPER_AUTO_INSTALL")
                .unwrap_or_else(|_| "false".into())
                .parse()
                .unwrap_or(false),
            whisper_python: env::var("WHISPER_PYTHON")
                .ok()
                .filter(|v| !v.trim().is_empty()),
            // TTS is otherwise unavailable out of the box, so this defaults on:
            // the launcher probes for an installed interpreter, and (unless
            // disabled) provisions `.venv-supertonic` on first run.
            auto_start_supertonic: env::var("AUTO_START_SUPERTONIC")
                .unwrap_or_else(|_| "true".into())
                .parse()
                .unwrap_or(true),
            auto_install_supertonic: env::var("SUPERTONIC_AUTO_INSTALL")
                .unwrap_or_else(|_| "true".into())
                .parse()
                .unwrap_or(true),
            supertonic_python: env::var("SUPERTONIC_PYTHON")
                .ok()
                .filter(|v| !v.trim().is_empty()),
            qwen_tts_url: env::var("QWEN_TTS_URL")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "http://127.0.0.1:7787".into()),
            qwen_tts_models_dir: env::var("QWEN_TTS_MODELS_DIR")
                .unwrap_or_else(|_| "data/qwen-tts-models".into()),
            // Opt-in, unlike Supertonic: the first start may build qwentts.cpp
            // (several minutes) and the 0.6B weights are a ~600 MB download.
            // Settings offers both; nothing heavy happens behind the user's back.
            auto_start_qwen_tts: env::var("AUTO_START_QWEN_TTS")
                .unwrap_or_else(|_| "false".into())
                .parse()
                .unwrap_or(false),
            web_dir: env::var("WEB_DIR").unwrap_or_else(|_| "web".into()),
            plugins_dir: env::var("PLUGINS_DIR").unwrap_or_else(|_| "data/plugins".into()),
            system_plugins_dir: env::var("SYSTEM_PLUGINS_DIR")
                .ok()
                .filter(|v| !v.trim().is_empty()),
            backgrounds_dir: env::var("BACKGROUNDS_DIR").unwrap_or_else(|_| "data/backgrounds".into()),
            admin_token: env::var("ADMIN_TOKEN").ok().filter(|v| !v.trim().is_empty()),
            linux_users,
            home_mode: env::var("SHINY_HOME_MODE")
                .unwrap_or_else(|_| "virtual".into())
                .trim()
                .to_lowercase(),
            auth_enabled: env::var("SHINY_AUTH_ENABLED")
                .unwrap_or_else(|_| "false".into())
                .parse()
                .unwrap_or(false),
            auth_sock: env::var("SHINY_AUTH_SOCK")
                .unwrap_or_else(|_| "/run/shiny/auth.sock".into()),
            greeter_mode: env::var("SHINY_GREETER")
                .map(|v| v.trim() == "1" || v.trim().eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            login_self_only,
            session_token_file: env::var("SHINY_SESSION_TOKEN_FILE")
                .ok()
                .filter(|v| !v.trim().is_empty()),
        }
    }

    /// Whether the Files plugin should operate on the account's real OS home.
    /// Requires Linux-user binding; otherwise the virtual home is used.
    pub fn real_home_mode(&self) -> bool {
        self.linux_users && self.home_mode == "real"
    }

    /// Build a `ConfigSnapshot` for plugin `PluginCtx` construction.
    pub fn snapshot(&self) -> shiny_plugin_sdk::services::ConfigSnapshot {
        shiny_plugin_sdk::services::ConfigSnapshot {
            server_host: self.server_host.clone(),
            server_port: self.server_port,
            database_url: self.database_url.clone(),
            ollama_url: self.ollama_url.clone(),
            ollama_model: self.ollama_model.clone(),
            supertonic_url: self.supertonic_url.clone(),
            supertonic_voice: self.supertonic_voice.clone(),
            web_dir: self.web_dir.clone(),
            vosk_models_dir: self.vosk_models_dir.clone(),
            auto_start_supertonic: self.auto_start_supertonic,
            log_level: self.log_level.clone(),
            plugins_dir: self.plugins_dir.clone(),
            system_plugins_dir: self.system_plugins_dir.clone(),
            admin_token: self.admin_token.clone(),
        }
    }
}