pub mod artifacts;
pub mod auth;
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
pub mod radio;
pub mod youtube;
pub mod documents;
pub mod spreadsheets;

use axum::Router;
use axum::routing::{delete, get, post, put};
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
        .route("/api/search", post(search::search_web))
        .route("/api/agent", post(agent::handle_agent_dispatch))
        .route("/api/ollama/models", get(ollama::list_models))
        .route("/api/insights/context", get(insights::context))
        .route("/api/radio/nowplaying", get(radio::now_playing))
        .route("/api/youtube/search", get(youtube::search))
        .route("/api/documents", get(documents::list).post(documents::create))
        // .odt imports are multi-MB uploads — lift the body limit on this route.
        .route("/api/documents/import", post(documents::import_odt)
            .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024)))
        .route(
            "/api/documents/:id",
            get(documents::get_one).put(documents::save).delete(documents::delete),
        )
        .route("/api/documents/:id/export", get(documents::export_odt))
        .route("/api/spreadsheets", get(spreadsheets::list).post(spreadsheets::create))
        .route(
            "/api/spreadsheets/:id",
            get(spreadsheets::get_one).put(spreadsheets::save).delete(spreadsheets::delete),
        )
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
        .route_service("/plugins", plugins_page)
        .route_service("/settings", settings_page)
        .fallback_service(static_files)
        .layer(CorsLayer::permissive())
        // No heuristic caching for HTML/JS/CSS — browsers must revalidate
        // (304 via Last-Modified) so a server restart never serves stale UI.
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-cache"),
        ))
        .with_state(state)
}
