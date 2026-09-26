pub mod artifacts;
pub mod auth;
pub mod background;
pub mod preferences;
pub mod travelers;
pub mod trips;
pub mod locations;
pub mod diary;
pub mod chat;
pub mod search;
pub mod agent;
pub mod voice;
pub mod insights;
pub mod ai;
pub mod ollama;
pub mod network;
pub mod audio;
pub mod display;
pub mod touchbar;
pub mod keyboard_backlight;
pub mod screen_brightness;
pub mod remote;

use axum::response::IntoResponse;
use axum::Router;
use axum::routing::{delete, get, patch, post, put};
use shiny_plugin_sdk::routes::{HttpMethod, RouteHandler, RouteSpec};
use sqlx::SqlitePool;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::auth::auth_middleware;
use crate::config::Config;
use crate::plugins::PluginManager;
use crate::services::audio::AudioService;
use crate::services::diary_gen::DiaryGenerator;
use crate::services::display::DisplayService;
use crate::services::gpsd::GpsdService;
use crate::services::network::NetworkService;
use crate::services::ollama::OllamaClient;
use crate::services::osm::OsmService;
use crate::services::supertonic::SupertonicClient;
use crate::services::touchbar::TouchBarService;
use crate::services::keyboard_backlight::KeyboardBacklightService;
use crate::services::screen_brightness::ScreenBrightnessService;
use crate::services::web_search::SearchService;
use crate::services::whisper::WhisperClient;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Config,
    pub ollama: OllamaClient,
    pub search: SearchService,
    pub osm: OsmService,
    pub gpsd: GpsdService,
    /// Host Wi-Fi/Ethernet state (NetworkManager over D-Bus).
    pub network: NetworkService,
    /// Host volume/mute/default devices (PipeWire via the Pulse socket).
    pub audio: AudioService,
    /// Host interface scale (webview page zoom); the kiosk shell applies it.
    pub display: DisplayService,
    /// Host Touch Bar capability (T2 MacBook); the web UI listens for its keys.
    pub touchbar: TouchBarService,
    /// Host keyboard backlight LED (the Mac's `kbd_backlight`).
    pub keyboard_backlight: KeyboardBacklightService,
    /// Host panel brightness (the Mac's `gmux_backlight`).
    pub screen_brightness: ScreenBrightnessService,
    pub diary_gen: Arc<DiaryGenerator>,
    /// In-flight agent turns, so a stop request can abort one mid-answer.
    pub agent_turns: crate::services::agent_cancel::TurnRegistry,
    pub supertonic: SupertonicClient,
    /// faster-whisper streaming STT sidecar (optional; Vosk is the fallback).
    pub whisper: WhisperClient,
    /// Plugin manager: hosts the ToolRegistry + loaded cdylibs.
    pub plugins: PluginManager,
    /// Iroh remote-access service (`PLAN-iroh-remote.md`).
    pub iroh: crate::services::iroh_remote::IrohRemote,
    /// Loopback-only session token for the local kiosk / server-mode window.
    pub session: crate::auth::SessionAuth,
    /// Admin-supplied router-rebuild trigger (set by `main.rs` once the live
    /// router swap is wired up).
    pub router_rebuild: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl AppState {
    /// Resolve the AI provider for one traveler: the shared Ollama client by
    /// default, or an OpenAI-compatible client when that provider is configured
    /// in the user's Assistant settings.
    pub async fn resolve_ai(&self, traveler_id: &str) -> crate::services::ai::ResolvedAi {
        crate::services::ai::resolve_for_user(&self.pool, &self.ollama, traveler_id).await
    }

    /// Resolve the real Linux account a Shiny traveler is bound to, when
    /// Linux-user mode is on. Prefers the stored `unix_user` and falls back to
    /// the account's username, so existing accounts are mapped on first use.
    pub fn os_identity_for(
        &self,
        traveler: &crate::models::Traveler,
    ) -> Option<crate::services::unix_user::UnixUser> {
        if !self.config.linux_users {
            return None;
        }
        let name = traveler
            .unix_user
            .as_deref()
            .or(traveler.username.as_deref())?;
        crate::services::unix_user::lookup_name(name)
    }

    /// Enter server mode at startup when the session user asked for it (their
    /// `remote.autostart` preference). Always writes the supervisor's state
    /// file, so `shiny-session` knows which window to open.
    pub async fn autostart_remote(&self) {
        let Some(user_id) = self.session.user_id.as_deref() else {
            crate::api::remote::write_state(false);
            return;
        };
        let autostart = sqlx::query_scalar::<_, String>(
            "SELECT value FROM user_preferences WHERE user_id = ?1 AND key = 'remote.autostart'",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

        if autostart {
            match self.iroh.start(self.config.server_port).await {
                Ok(_) => {
                    crate::api::remote::write_state(true);
                    tracing::info!("iroh: remote access autostarted");
                }
                Err(e) => tracing::warn!("iroh autostart failed: {e:?}"),
            }
        } else {
            crate::api::remote::write_state(false);
        }
    }

    /// Build an `Arc<PluginCtx>` for handing to plugins at install/on_load time.
    pub fn plugin_ctx(&self) -> Arc<shiny_plugin_sdk::services::PluginCtx> {
        // A neutral manifest is used when constructing the base ctx; the loader
        // replaces it with the plugin's real manifest at install time.
        static EMPTY: std::sync::OnceLock<shiny_plugin_sdk::manifest::Manifest> = std::sync::OnceLock::new();
        let empty = EMPTY.get_or_init(|| shiny_plugin_sdk::manifest::Manifest {
            name: String::new(),
            version: semver::Version::new(0, 0, 0),
            api_level: shiny_plugin_sdk::CORE_API_LEVEL,
            entry_symbol: String::new(),
            target_triple: None,
            description: None,
            author: None,
            summary: None,
            migrations_dir: "migrations".into(),
            skills_dir: "skills".into(),
            web_dir: "web".into(),
            signature: None,
        });
        shiny_plugin_sdk::services::PluginCtx::new(
            self.config.snapshot(),
            empty.clone(),
        )
    }
}

/// Re-serialize a request's captured path params into a header the plugin can
/// read. axum stores path params in request extensions as its private
/// `UrlParams` type; a plugin's own axum copy has a different `TypeId` for that
/// type, so a plugin's `Path` extractor can never see them. We extract them
/// here (core-side, same axum as the router that captured them) and re-encode
/// them as a plain header, which crosses the dlopen boundary safely.
async fn inject_path_params(req: axum::extract::Request) -> axum::extract::Request {
    use axum::extract::{FromRequestParts, RawPathParams};
    let (mut parts, body) = req.into_parts();
    let params: Vec<(String, String)> = RawPathParams::from_request_parts(&mut parts, &())
        .await
        .map(|p| {
            p.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
        .unwrap_or_default();
    if !params.is_empty() {
        if let Ok(json) = serde_json::to_string(&params) {
            if let Ok(value) = axum::http::HeaderValue::from_bytes(json.as_bytes()) {
                parts
                    .headers
                    .insert(shiny_plugin_sdk::routes::PATH_PARAMS_HEADER, value);
            }
        }
    }
    axum::extract::Request::from_parts(parts, body)
}

/// Mount one plugin `RouteSpec` onto a fresh router, applying auth middleware
/// unless the spec declares `public` (or `admin`, which is treated as `auth`
/// since core has no admin role).
fn plugin_route(state: &AppState, spec: RouteSpec, handler: RouteHandler) -> Router<AppState> {
    let path = spec.path.clone();
    let method_router = match spec.method {
        HttpMethod::Get => {
            let h = handler.clone();
            get(move |req: axum::extract::Request| {
                let h = h.clone();
                async move {
                    let req = inject_path_params(req).await;
                    h(req).await
                }
            })
        }
        HttpMethod::Post => {
            let h = handler.clone();
            post(move |req: axum::extract::Request| {
                let h = h.clone();
                async move {
                    let req = inject_path_params(req).await;
                    h(req).await
                }
            })
        }
        HttpMethod::Put => {
            let h = handler.clone();
            put(move |req: axum::extract::Request| {
                let h = h.clone();
                async move {
                    let req = inject_path_params(req).await;
                    h(req).await
                }
            })
        }
        HttpMethod::Delete => {
            let h = handler.clone();
            delete(move |req: axum::extract::Request| {
                let h = h.clone();
                async move {
                    let req = inject_path_params(req).await;
                    h(req).await
                }
            })
        }
        HttpMethod::Patch => {
            let h = handler.clone();
            patch(move |req: axum::extract::Request| {
                let h = h.clone();
                async move {
                    let req = inject_path_params(req).await;
                    h(req).await
                }
            })
        }
    };

    let router = Router::new().route(&path, method_router);
    if spec.auth == "public" {
        router
    } else {
        router.layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
    }
}

/// Build the plugin-contributed portion of the router: every installed
/// plugin's `RouteSpec` routes plus its served `web/` assets.
fn build_plugin_routes(state: &AppState) -> Router<AppState> {
    let mut router: Router<AppState> = Router::new();
    for (_name, spec, handler) in state.plugins.routes() {
        router = router.merge(plugin_route(state, spec, handler));
    }

    // Serve each installed plugin's web assets at /plugins/<name>/ (roadmap #4).
    // Register the ServeDir for every plugin unconditionally: ServeDir reads
    // from disk per request, so a plugin whose web/ dir (or icon.svg) is added
    // after startup is still served — no restart needed.
    let plugins_dir = std::path::Path::new(&state.config.plugins_dir);
    for manifest in state.plugins.list() {
        let web_path = plugins_dir.join(&manifest.name).join(&manifest.web_dir);
        router = router.nest_service(
            &format!("/plugins/{}", manifest.name),
            ServeDir::new(web_path),
        );
    }
    router
}

/// Reject host-capability mutations from a remote (Iroh) client. The transparent
/// proxy makes the request's TCP peer loopback, so `require_local` alone would
/// let a remote client change the machine's audio, network or display.
async fn host_remote_gate(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if remote::is_remote(req.headers()) && req.method() != axum::http::Method::GET {
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    next.run(req).await
}

pub fn build_router(state: AppState) -> Router {
    let web_dir = state.config.web_dir.clone();
    let vosk_models_dir = state.config.vosk_models_dir.clone();

    let public_routes = Router::new()
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/unix-users", get(auth::unix_users))
        .route("/api/auth/session", get(auth::session_bootstrap))
        .route("/api/voice/languages", get(voice::voice_languages))
        .nest_service(
            "/api/voice/models/vosk",
            ServeDir::new(vosk_models_dir),
        );

    let protected_routes = Router::new()
        .route("/api/plugins", get(crate::plugins::admin_api::list))
        .route("/api/plugins/active", get(crate::plugins::admin_api::active))
        // Plugin archives are multi-MB zip/tar.gz uploads (cdylib inside) —
        // lift axum's 2MB default body limit on this route only.
        .route("/api/plugins/install", post(crate::plugins::admin_api::install)
            .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024)))
        .route("/api/plugins/uninstall", post(crate::plugins::admin_api::uninstall))
        .route("/api/plugins/activate", post(crate::plugins::admin_api::activate))
        .route("/api/plugins/deactivate", post(crate::plugins::admin_api::deactivate))
        .route("/api/plugins/install.log", get(crate::plugins::admin_api::install_log))
        .route("/api/travelers/me", get(travelers::get_me).put(travelers::update_me))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/preferences", get(preferences::get_preferences).put(preferences::put_preferences))
        // Host panels (network/audio/display/backlight/brightness/touchbar) are
        // registered separately in `host_routes` below so the remote-client gate
        // can wrap them without wrapping the rest of the API.
        .route("/api/remote/status", get(remote::status))
        .route("/api/remote/enable", post(remote::enable))
        .route("/api/remote/rotate", post(remote::rotate))
        .route("/api/remote/qr", get(remote::qr))
        .route("/api/remote/pair", post(remote::pair))
        .route("/api/remote/unpair", post(remote::unpair))
        // Desktop background image: upload/serve/remove the caller's file.
        .route("/api/background", get(background::serve).post(background::upload)
            .layer(axum::extract::DefaultBodyLimit::max(16 * 1024 * 1024))
            .delete(background::remove))
        .route("/api/trips", get(trips::list).post(trips::create))
        .route("/api/trips/active", get(trips::get_active))
        .route("/api/trips/:id", get(trips::get_one).put(trips::update))
        .route("/api/trips/:id/start", post(trips::start_trip))
        .route("/api/trips/:id/end", post(trips::end_trip))
        .route("/api/trips/:id/stats", get(trips::stats))
        .route("/api/locations", post(locations::submit).get(locations::list))
        .route("/api/trips/:id/route", get(locations::route))
        .route("/api/map/search", get(trips::map_search))
        .route("/api/map/reverse", get(trips::map_reverse))
        .route("/api/map/route", get(trips::map_route))
        .route("/api/map/poi", get(trips::map_poi))
        .route("/api/navigate/start", get(trips::navigate_start))
        .route("/api/diary", get(diary::list))
        .route("/api/diary/:date", get(diary::get_by_date))
        .route("/api/diary/search", get(diary::search))
        .route("/api/diary/generate", post(diary::generate))
        .route("/api/chat", post(chat::send_message))
        .route("/api/chat/history", get(chat::history))
        .route("/api/chat/conversations", get(chat::list_conversations).post(chat::create_conversation))
        .route("/api/chat/conversations/:id", get(chat::conversation_messages).delete(chat::delete_conversation))
        .route("/api/search", post(search::search_web))
        .route("/api/agent", post(agent::handle_agent_dispatch))
        // Stop the answer currently being generated (voice barge-in, or the
        // stop button in text mode).
        .route("/api/agent/stop", post(agent::handle_agent_stop))
        .route("/api/ai/models", get(ai::list_models))
        .route("/api/ollama/models", get(ollama::list_models))
        .route("/api/insights/context", get(insights::context))
        .route("/api/artifacts", get(artifacts::list).post(artifacts::create))
        .route("/api/artifacts/:id", get(artifacts::get_one).put(artifacts::update))
        .route("/api/tts", post(voice::tts))
        .route("/api/voice/status", get(voice::voice_status))
        .route("/api/voice/download", post(voice::voice_download))
        // faster-whisper: model downloads + streaming STT proxy. Audio chunks
        // are raw PCM up to ~64 KB each; the default 2 MB body cap is plenty.
        .route("/api/voice/whisper/download", post(voice::voice_whisper_download))
        .route("/api/voice/stt/chunk", post(voice::voice_stt_chunk))
        .route("/api/voice/stt/close", post(voice::voice_stt_close));

    // Host-capability routes. A remote (Iroh) client may read the host panels
    // (the handlers report `available:false`, so the UI hides them) but must
    // never mutate the machine — even though the transparent proxy makes the
    // request's TCP peer loopback.
    let host_routes = Router::new()
        .route("/api/network/status", get(network::status))
        .route("/api/network/events", get(network::events))
        .route("/api/network/scan", post(network::scan))
        .route("/api/network/connect", post(network::connect))
        .route("/api/network/disconnect", post(network::disconnect))
        .route("/api/network/forget", post(network::forget))
        .route("/api/network/wifi-power", post(network::wifi_power))
        .route("/api/audio/status", get(audio::status))
        .route("/api/audio/events", get(audio::events))
        .route("/api/audio/volume", post(audio::volume))
        .route("/api/audio/mute", post(audio::mute))
        .route("/api/audio/default", post(audio::default_device))
        .route("/api/display", get(display::status).put(display::set_scale))
        .route("/api/touchbar", get(touchbar::status))
        .route(
            "/api/keyboard/backlight",
            get(keyboard_backlight::status).post(keyboard_backlight::set),
        )
        .route(
            "/api/screen/brightness",
            get(screen_brightness::status).post(screen_brightness::set),
        )
        .layer(axum::middleware::from_fn(host_remote_gate));

    let protected_routes = protected_routes
        .merge(host_routes)
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    let static_files = ServeDir::new(&web_dir)
        .not_found_service(ServeFile::new(format!("{}/index.html", web_dir)));

    Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .merge(build_plugin_routes(&state))
        .fallback_service(static_files)
        .layer(CorsLayer::permissive())
        // Never cache HTML/JS/CSS — the frontend must always re-fetch, so a
        // server restart or a source edit is picked up on the next reload
        // without stale JS lingering in the browser.
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-store"),
        ))
        .with_state(state)
}
