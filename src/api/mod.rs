pub mod artifacts;
pub mod auth;
pub mod background;
pub mod fonts;
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
pub mod battery;
pub mod bluetooth;
pub mod power;
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
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::auth::auth_middleware;
use crate::config::Config;
use crate::plugins::PluginManager;use crate::services::audio::AudioService;
use crate::services::battery::BatteryService;
use crate::services::bluetooth::BluetoothService;
use crate::services::diary_gen::DiaryGenerator;
use crate::services::display::DisplayService;
use crate::services::gpsd::GpsdService;
use crate::services::network::NetworkService;
use crate::services::ollama::OllamaClient;
use crate::services::power::PowerService;
use crate::services::osm::OsmService;
use crate::services::supertonic::SupertonicClient;
use crate::services::touchbar::TouchBarService;
use crate::services::keyboard_backlight::KeyboardBacklightService;
use crate::services::screen_brightness::ScreenBrightnessService;
use crate::services::web_search::SearchService;
use crate::services::whisper::WhisperClient;
use crate::services::qwen_tts::QwenClient;

/// Content-Security-Policy for the whole app.
///
/// The UI is fully self-hosted (Leaflet, marked and Vosk are vendored under
/// `/vendor`), so `script-src 'self'` is enough — inline `<script>` and inline
/// event handlers are forbidden, which is the backstop for the sanitized
/// markdown path. `'wasm-unsafe-eval'` is required by the in-browser Vosk
/// recognizer. Styles stay `'unsafe-inline'` for the many `style` attributes
/// the UI sets; images may come from anywhere (map tiles, article images).
///
/// Three app features need explicit allowances:
/// - The top bar's weather chip and the clock read Open-Meteo directly from the
///   browser (free, no API key) for the forecast and the located place's
///   timezone, so `api.open-meteo.com` is in `connect-src`.
/// - The Browser plugin's start page is a sandboxed `srcdoc` iframe whose
///   click/refresh handler is a **static** inline `<script>`; CSP hashes are the
///   only way to allow an inline script in a sandboxed (opaque-origin) frame, so
///   the script's exact SHA-256 is allowlisted below. Editing that script in
///   `plugins/browser/web/plugin.js` requires updating the hash here.
/// - Frames are used across the app (the sandboxed `srcdoc` shelves, mail
///   bodies, the YouTube embed), so `frame-src` is left permissive. Note this
///   only governs pages the app itself frames; ordinary browsing happens in
///   native child webviews at the site's real origin, which our CSP never sees.
///   `frame-ancestors 'none'` still stops other sites from framing the app.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; \
script-src 'self' 'wasm-unsafe-eval' 'sha256-Sz9x6nnEJCuJi8kFyc0Cy6JglT8TyTnBSc7jFTweAIY='; \
style-src 'self' 'unsafe-inline'; \
img-src 'self' data: blob: http: https:; \
connect-src 'self' https://router.project-osrm.org https://api.open-meteo.com; \
font-src 'self' data:; \
media-src 'self' blob:; \
worker-src 'self' blob:; \
object-src 'none'; \
frame-src 'self' https: http: data: blob:; \
base-uri 'none'; \
form-action 'self'; \
frame-ancestors 'none'";

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
    /// Host battery/supply state (the kernel's sysfs power-supply class).
    pub battery: BatteryService,
    /// Host Bluetooth adapter and devices (BlueZ over D-Bus).
    pub bluetooth: BluetoothService,
    /// Host power actions (reboot / power off / suspend) via freedesktop logind.
    pub power: PowerService,
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
    /// Qwen3-TTS sidecar (qwentts.cpp; optional high-quality TTS engine).
    pub qwen_tts: QwenClient,
    /// Plugin manager: hosts the ToolRegistry + loaded cdylibs.
    pub plugins: PluginManager,
    /// Iroh remote-access service.
    pub iroh: crate::services::iroh_remote::IrohRemote,
    /// Tailscale Serve/Funnel access (a public `*.ts.net` HTTPS URL).
    pub tailscale: crate::services::tailscale::TailscaleService,
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
        // Never map a Shiny account to a system/non-human OS account. `root`
        // is the dangerous case: an account named "root" (possible while
        // linux_users is off) must not inherit /root as its Files sandbox once
        // the flag is enabled.
        crate::services::unix_user::lookup_name(name).filter(|u| u.is_human())
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
    // A client must never be able to smuggle path parameters onto a route the
    // core did not capture them for. Drop any inbound header first, then
    // re-add the real path params (if this route has any).
    parts
        .headers
        .remove(shiny_plugin_sdk::routes::PATH_PARAMS_HEADER);
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
        // A public route never sees core-injected identity. Strip any
        // client-supplied identity/OS headers so a handler reading them cannot
        // be spoofed — the same hardening `auth_middleware` applies on
        // authenticated routes.
        router.layer(axum::middleware::from_fn(strip_identity_headers))
    } else {
        router.layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
    }
}

/// Remove every core-injected identity header from a request. Used on plugin
/// routes that opt out of auth, where `auth_middleware` never runs.
async fn strip_identity_headers(
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    for name in [
        shiny_plugin_sdk::routes::USER_ID_HEADER,
        shiny_plugin_sdk::routes::TRAVELER_ID_HEADER,
        shiny_plugin_sdk::routes::OS_USER_HEADER,
        shiny_plugin_sdk::routes::OS_HOME_HEADER,
        shiny_plugin_sdk::routes::OS_UID_HEADER,
        shiny_plugin_sdk::routes::PATH_PARAMS_HEADER,
    ] {
        req.headers_mut().remove(name);
    }
    next.run(req).await
}

/// Build the plugin-contributed portion of the router: every installed
/// plugin's `RouteSpec` routes plus its served `web/` assets.
fn build_plugin_routes(state: &AppState) -> Router<AppState> {
    let mut router: Router<AppState> = Router::new();
    for (_name, spec, handler) in state.plugins.routes() {
        router = router.merge(plugin_route(state, spec, handler));
    }
    // Plugin upload routes (multipart archives/documents/images) routinely
    // exceed axum's 2MB default body limit. Rather than disabling the limit
    // entirely, raise it to a generous ceiling; plugins still clamp their own
    // uploads (raw-Request handlers read the body themselves, so this is a
    // backstop for extractor-based handlers).
    router = router.layer(axum::extract::DefaultBodyLimit::max(160 * 1024 * 1024));

    // Serve each installed plugin's web assets at /plugins/<name>/.
    // Register the ServeDir for every plugin unconditionally: ServeDir reads
    // from disk per request, so a plugin whose web/ dir (or icon.svg) is added
    // after startup is still served — no restart needed.
    for manifest in state.plugins.list() {
        // `web_dir` is attacker-controlled in plugin.toml; the loader already
        // rejects absolute/`..` values, and this second check keeps the router
        // safe even if a manifest was produced by an older build.
        if let Err(e) =
            crate::plugins::loader::validate_relative_path("web_dir", &manifest.web_dir)
        {
            tracing::warn!(
                "plugin '{}' has an invalid web_dir, not serving it: {e}",
                manifest.name
            );
            continue;
        }
        // A user override lives in the writable plugins dir, a system plugin
        // in the read-only baseline; serve whichever one won the load.
        let Some(install_dir) = state.plugins.plugin_dir_for(&manifest.name) else {
            continue;
        };
        let web_path = install_dir.join(&manifest.web_dir);
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
        // Installed font families — the appearance "Global font" picker and the
        // Writer's font menu read the live fontconfig list (see fonts.rs).
        .route("/api/fonts", get(fonts::list))
        // Host panels (network/audio/display/backlight/brightness/touchbar) are
        // registered separately in `host_routes` below so the remote-client gate
        // can wrap them without wrapping the rest of the API.
        .route("/api/remote/status", get(remote::status))
        .route("/api/remote/enable", post(remote::enable))
        .route("/api/remote/rotate", post(remote::rotate))
        .route("/api/remote/qr", get(remote::qr))
        .route("/api/remote/pair", post(remote::pair))
        .route("/api/remote/unpair", post(remote::unpair))
        // Tailscale Funnel: a public `*.ts.net` HTTPS URL (kept alongside Iroh).
        .route("/api/remote/tailscale/enable", post(remote::tailscale_enable))
        .route("/api/remote/tailscale/disable", post(remote::tailscale_disable))
        .route("/api/remote/tailscale/qr", get(remote::tailscale_qr))
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
        // Qwen3-TTS (optional TTS engine): GGUF model downloads.
        .route("/api/voice/qwen/download", post(voice::voice_qwen_download))
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
        .route("/api/battery/status", get(battery::status))
        .route("/api/battery/events", get(battery::events))
        .route("/api/bluetooth/status", get(bluetooth::status))
        .route("/api/bluetooth/events", get(bluetooth::events))
        .route("/api/bluetooth/power", post(bluetooth::power))
        .route("/api/bluetooth/scan", post(bluetooth::scan))
        .route("/api/bluetooth/pair", post(bluetooth::pair))
        .route("/api/bluetooth/connect", post(bluetooth::connect))
        .route("/api/bluetooth/disconnect", post(bluetooth::disconnect))
        .route("/api/bluetooth/forget", post(bluetooth::forget))
        .route("/api/bluetooth/trust", post(bluetooth::trust))
        .route("/api/power/status", get(power::status))
        .route("/api/power/reboot", post(power::reboot))
        .route("/api/power/off", post(power::power_off))
        .route("/api/power/suspend", post(power::suspend))
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
        // The UI is same-origin and never needs CORS. The previous
        // `CorsLayer::permissive()` emitted `Access-Control-Allow-Origin: *`,
        // letting any website read this server's responses (and turning a
        // token leak into a durable cross-origin capability). There is no
        // legitimate cross-origin caller, so no CORS layer is added at all.
        //
        // Never cache HTML/JS/CSS — the frontend must always re-fetch, so a
        // server restart or a source edit is picked up on the next reload
        // without stale JS lingering in the browser.
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-store"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::HeaderName::from_static("content-security-policy"),
            axum::http::HeaderValue::from_static(CONTENT_SECURITY_POLICY),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::HeaderName::from_static("x-content-type-options"),
            axum::http::HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::HeaderName::from_static("x-frame-options"),
            axum::http::HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::HeaderName::from_static("referrer-policy"),
            axum::http::HeaderValue::from_static("no-referrer"),
        ))
        .with_state(state)
}
