use std::fs::OpenOptions;
use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::task::{Context, Poll};

use arc_swap::ArcSwap;
use tower::Service;

use tracing_subscriber::EnvFilter;

use shiny::api;
use shiny::api::AppState;
use shiny::config::Config;
use shiny::db;
use shiny::services::diary_gen::DiaryGenerator;
use shiny::services::gpsd::GpsdService;
use shiny::services::ollama::OllamaClient;
use shiny::services::osm::OsmService;
use shiny::services::supertonic::SupertonicClient;
use shiny::services::web_search::SearchService;

/// Swappable router handle: implements `tower::Service<IncomingStream>` by
/// delegating to the currently-loaded `Router`, so plugin installs/uninstalls
/// can hot-swap the live router without a server restart.
#[derive(Clone)]
struct RouterHandle {
    inner: Arc<ArcSwap<axum::Router>>,
}

impl RouterHandle {
    fn new(router: axum::Router) -> Self {
        Self { inner: Arc::new(ArcSwap::from_pointee(router)) }
    }

    fn swap(&self, router: axum::Router) {
        self.inner.store(Arc::new(router));
    }
}

impl<'a> Service<axum::serve::IncomingStream<'a>> for RouterHandle {
    type Response = axum::Router;
    type Error = std::convert::Infallible;
    type Future = std::future::Ready<Result<axum::Router, std::convert::Infallible>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _stream: axum::serve::IncomingStream<'a>) -> Self::Future {
        std::future::ready(Ok(self.inner.load_full().as_ref().clone()))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let config = Config::from_env();

    // Full logs: the tracer tees to stdout AND a file (data/shiny.log by
    // default; override with LOG_FILE). Parent dirs are created eagerly.
    if let Some(parent) = std::path::Path::new(&config.log_file).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config.log_file)?;

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&config.log_level)),
        )
        .with_writer(TeeMakeWriter { file: log_file })
        .init();

    tracing::info!("Starting Shiny AI sphere…");
    tracing::info!("Logging to {}", config.log_file);

    std::fs::create_dir_all("data").ok();

    if config.auto_start_supertonic {
        spawn_supertonic_sidecar(&config.supertonic_url);
    }

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

    let supertonic = SupertonicClient::new(
        config.supertonic_url.clone(),
        config.supertonic_voice.clone(),
    );

    if supertonic.is_available().await {
        tracing::info!("Supertonic TTS available at {}", config.supertonic_url);
    } else {
        tracing::warn!(
            "Supertonic not available at {}. TTS will fail until sidecar is started.",
            config.supertonic_url
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
        supertonic,
        plugins: shiny::plugins::PluginManager::new(std::path::PathBuf::from(&config.plugins_dir), pool.clone()),
        router_rebuild: None,
    };

    // Scan plugins directory and load installed cdylib plugins (hot-reload
    // registration on startup; subsequent installs use the admin API).
    std::fs::create_dir_all(&config.plugins_dir).ok();
    let base_ctx = state.plugin_ctx();
    let installed = state.plugins.discover_and_install(base_ctx).await;
    if !installed.is_empty() {
        tracing::info!("Loaded plugins: {}", installed.join(", "));
    } else {
        tracing::info!("No plugins installed. Core runs as pure AI sphere.");
    }

    if config.diary_auto_generate {
        spawn_diary_cron(diary_gen, config.diary_generate_time.clone());
    }

    // Wire live router rebuild: the served router is swappable, and every
    // plugin install/uninstall re-builds it (hot router swap, roadmap #2).
    // The rebuild closure must survive into every rebuilt router's own state,
    // so we route it through a small dispatch cell that the closure reads.
    let router_handle = RouterHandle::new(axum::Router::new());
    let rebuild_cell: Arc<std::sync::Mutex<Option<Arc<dyn Fn() + Send + Sync>>>> =
        Arc::new(std::sync::Mutex::new(None));

    let dispatch = |cell: &Arc<std::sync::Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>| {
        let cell = cell.clone();
        Arc::new(move || {
            if let Some(cb) = cell.lock().unwrap().clone() {
                cb();
            }
        }) as Arc<dyn Fn() + Send + Sync>
    };

    // `router_state` is the AppState embedded in every (re)built router — its
    // `router_rebuild` always points at the dispatch cell.
    let mut router_state = state.clone();
    router_state.router_rebuild = Some(dispatch(&rebuild_cell));

    let rebuild: Arc<dyn Fn() + Send + Sync> = {
        let handle = router_handle.clone();
        let rs = router_state.clone();
        Arc::new(move || handle.swap(api::build_router(rs.clone())))
    };
    *rebuild_cell.lock().unwrap() = Some(rebuild.clone());

    // Initial router.
    router_handle.swap(api::build_router(router_state.clone()));

    let addr = format!("{}:{}", config.server_host, config.server_port);
    tracing::info!("Server listening on http://{}", addr);
    tracing::info!("Web UI served from / (static files in {})", config.web_dir);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, router_handle).await?;

    Ok(())
}

/// Writes each log record to both stdout and the log file.
struct TeeWriter {
    file: std::fs::File,
}

impl Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = io::stdout().write_all(buf);
        self.file.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = io::stdout().flush();
        self.file.flush()
    }
}

/// `MakeWriter` that hands each log line an independent file handle (opened
/// with O_APPEND, so concurrent writes land at the end without interleaving).
struct TeeMakeWriter {
    file: std::fs::File,
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TeeMakeWriter {
    type Writer = TeeWriter;

    fn make_writer(&'a self) -> Self::Writer {
        TeeWriter {
            file: self
                .file
                .try_clone()
                .expect("failed to clone log file handle"),
        }
    }
}

fn spawn_supertonic_sidecar(supertonic_url: &str) {
    let port = supertonic_url
        .rsplit(':')
        .next()
        .and_then(|p| p.trim_end_matches('/').parse::<u16>().ok())
        .unwrap_or(7788);

    match Command::new("supertonic")
        .args(["serve", "--host", "127.0.0.1", "--port", &port.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(_) => tracing::info!("Started Supertonic sidecar on port {}", port),
        Err(e) => tracing::warn!("Could not auto-start Supertonic: {}", e),
    }
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
