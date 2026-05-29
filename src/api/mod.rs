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

use axum::Router;
use axum::routing::{get, post};
use sqlx::SqlitePool;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

use crate::auth::auth_middleware;
use crate::config::Config;
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
        .route("/api/agent", post(agent::handle_agent))
        .route("/api/insights/context", get(insights::context))
        .route("/api/artifacts", get(artifacts::list).post(artifacts::create))
        .route("/api/artifacts/:id", get(artifacts::get_one).put(artifacts::update))
        .route("/api/tts", post(voice::tts))
        .route("/api/voice/status", get(voice::voice_status))
        .route("/api/voice/download", post(voice::voice_download))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    let static_files = ServeDir::new(&web_dir)
        .not_found_service(ServeFile::new(format!("{}/index.html", web_dir)));

    Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .fallback_service(static_files)
        .layer(CorsLayer::permissive())
        .with_state(state)
}
