pub mod auth;
pub mod travelers;
pub mod trips;
pub mod locations;
pub mod diary;
pub mod chat;
pub mod search;

use axum::Router;
use axum::routing::{get, post};
use sqlx::SqlitePool;
use std::sync::Arc;
use tower_http::cors::CorsLayer;

use crate::auth::auth_middleware;
use crate::config::Config;
use crate::services::diary_gen::DiaryGenerator;
use crate::services::gpsd::GpsdService;
use crate::services::ollama::OllamaClient;
use crate::services::osm::OsmService;
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
}

pub fn build_router(state: AppState) -> Router {
    let public_routes = Router::new()
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login));

    let protected_routes = Router::new()
        .route("/api/travelers/me", get(travelers::get_me).put(travelers::update_me))
        .route("/api/trips", get(trips::list).post(trips::create))
        .route("/api/trips/{id}", get(trips::get_one).put(trips::update))
        .route("/api/trips/{id}/start", post(trips::start_trip))
        .route("/api/trips/{id}/end", post(trips::end_trip))
        .route("/api/trips/{id}/stats", get(trips::stats))
        .route("/api/locations", post(locations::submit).get(locations::list))
        .route("/api/trips/{id}/route", get(locations::route))
        .route("/api/map/search", get(trips::map_search))
        .route("/api/map/reverse", get(trips::map_reverse))
        .route("/api/map/route", get(trips::map_route))
        .route("/api/map/poi", get(trips::map_poi))
        .route("/api/diary", get(diary::list))
        .route("/api/diary/{date}", get(diary::get_by_date))
        .route("/api/diary/search", get(diary::search))
        .route("/api/diary/generate", post(diary::generate))
        .route("/api/chat", post(chat::send_message))
        .route("/api/chat/history", get(chat::history))
        .route("/api/search", post(search::search_web))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .layer(CorsLayer::permissive())
        .with_state(state)
}
