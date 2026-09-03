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
pub mod ollama;

use axum::Router;
use axum::routing::{delete, get, patch, post, put};
use shiny_plugin_sdk::routes::{HttpMethod, RouteHandler, RouteSpec};
use sqlx::SqlitePool;
use std::sync::Arc;
use std::path::PathBuf;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::auth::auth_middleware;
use crate::config::Config;
use crate::plugins::PluginManager;
use crate::services::diary_gen::DiaryGenerator;
use crate::services::gpsd::GpsdService;
use crate::services::ollama::OllamaClient;
use crate::services::osm::OsmService;
use crate::services::supertonic::SupertonicClient;
use crate::services::web_search::SearchService;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Config,
    pub ollama: OllamaClient,
    pub search: SearchService,
    pub osm: OsmService,
    pub gpsd: GpsdService,
    pub diary_gen: Arc<DiaryGenerator>,
    pub supertonic: SupertonicClient,
    /// Plugin manager: hosts the ToolRegistry + loaded cdylibs.
    pub plugins: PluginManager,
    /// Admin-supplied router-rebuild trigger (set by `main.rs` once the live
    /// router swap is wired up).
    pub router_rebuild: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl AppState {
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
    let plugins_dir = std::path::Path::new(&state.config.plugins_dir);
    for manifest in state.plugins.list() {
        let web_path = plugins_dir.join(&manifest.name).join(&manifest.web_dir);
        if web_path.is_dir() {
            router = router.nest_service(
                &format!("/plugins/{}", manifest.name),
                ServeDir::new(web_path),
            );
        }
    }
    router
}

pub fn build_router(state: AppState) -> Router {
    let web_dir = state.config.web_dir.clone();
    let vosk_models_dir = state.config.vosk_models_dir.clone();

    let public_routes = Router::new()
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
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
        .route("/api/preferences", get(preferences::get_preferences).put(preferences::put_preferences))
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
        .route("/api/ollama/models", get(ollama::list_models))
        .route("/api/insights/context", get(insights::context))
        .route("/api/artifacts", get(artifacts::list).post(artifacts::create))
        .route("/api/artifacts/:id", get(artifacts::get_one).put(artifacts::update))
        .route("/api/tts", post(voice::tts))
        .route("/api/voice/status", get(voice::voice_status))
        .route("/api/voice/download", post(voice::voice_download))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    let static_files = ServeDir::new(&web_dir)
        .not_found_service(ServeFile::new(format!("{}/index.html", web_dir)));

    // Explicit HTML routes for the standalone pages. ServeDir's fallback would
    // otherwise return index.html for them, which we don't want — these pages
    // are separate documents.
    let plugins_page = ServeFile::new(format!("{}/plugins.html", web_dir));
    let settings_page = ServeFile::new(format!("{}/settings.html", web_dir));

    Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .merge(build_plugin_routes(&state))
        .route_service("/plugins", plugins_page)
        .route_service("/settings", settings_page)
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
