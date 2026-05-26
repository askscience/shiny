use std::sync::Arc;

use tracing_subscriber::EnvFilter;

use traveler_app::api;
use traveler_app::api::AppState;
use traveler_app::config::Config;
use traveler_app::db;
use traveler_app::services::diary_gen::DiaryGenerator;
use traveler_app::services::gpsd::GpsdService;
use traveler_app::services::ollama::OllamaClient;
use traveler_app::services::osm::OsmService;
use traveler_app::services::web_search::SearchService;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let config = Config::from_env();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&config.log_level)),
        )
        .init();

    tracing::info!("Starting Traveler REST API server...");

    let pool = db::init_pool(&config.database_url).await?;
    db::run_migrations(&pool).await?;

    let ollama = OllamaClient::new(config.ollama_url.clone(), config.ollama_model.clone());

    if ollama.is_available().await {
        tracing::info!("Ollama is available at {}", config.ollama_url);
    } else {
        tracing::warn!(
            "Ollama not available at {}. AI features (chat, diary gen) will fail.",
            config.ollama_url
        );
    }

    let search = SearchService::new();
    let osm = OsmService::new();
    let gpsd = GpsdService::new(config.gpsd_host.clone(), config.gpsd_port);

    gpsd.start().await;

    if gpsd.is_connected().await {
        tracing::info!("GPSD connected at {}:{}", config.gpsd_host, config.gpsd_port);
    } else {
        tracing::warn!(
            "GPSD not available. Using mock GPS data for position tracking."
        );
    }

    let diary_gen = Arc::new(DiaryGenerator::new(pool.clone(), ollama.clone(), osm.clone()));

    let state = AppState {
        pool: pool.clone(),
        config: config.clone(),
        ollama,
        search,
        osm,
        gpsd,
        diary_gen: diary_gen.clone(),
    };

    if config.diary_auto_generate {
        spawn_diary_cron(diary_gen, config.diary_generate_time.clone());
    }

    let app = api::build_router(state);

    let addr = format!("{}:{}", config.server_host, config.server_port);
    tracing::info!("Server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn spawn_diary_cron(diary_gen: Arc<DiaryGenerator>, generate_time: String) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            let now = chrono::Local::now().format("%H:%M").to_string();
            if now == generate_time {
                tracing::info!("Auto-generating daily diary entries...");
                diary_gen.auto_generate_daily().await;
            }
        }
    });
}
