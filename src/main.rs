use shiny::application::{logging::TeeMakeWriter, router::RouterHandle, sidecars::*};
use shiny::services::diary_gen::DiaryGenerator;
use shiny::{api, api::AppState, config::Config, db};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    // A system installed under the former name `peakd` still sets `PEAKD_*`.
    liteaura_core::legacy::adopt_legacy_env();

    let config = Config::from_env();
    config.validate()?;

    let log_writer = TeeMakeWriter::new(&config.log_file)?;

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log_level)),
        )
        .with_writer(log_writer)
        .init();

    tracing::info!("Starting Shiny AI assistant…");
    tracing::info!("Logging to {}", config.log_file);

    shiny::services::ai_switch::init();

    // "No slop": with the AI switched off no speech sidecar is started; the
    // watchdog starts them later if the user turns the AI back on. They start
    // now, in parallel with the session, at a low priority (`launcher`).
    let ai_on = shiny::services::ai_switch::machine_on();
    if config.auto_start_supertonic && ai_on {
        spawn_supertonic_sidecar(&config);
    }
    // faster-whisper is the default speech engine, so bring its sidecar up with
    // the server. The launcher is idempotent (it probes /health first) and
    // exits quietly when the package is not installed; Vosk still works.
    if config.auto_start_whisper && ai_on {
        spawn_whisper_sidecar(&config);
    }
    // Qwen3-TTS is opt-in: the first start may build qwentts.cpp and fetch a
    // ~600 MB model, which must never happen behind the user's back.
    if config.auto_start_qwen_tts && ai_on {
        spawn_qwen_tts_sidecar(&config);
    }

    // Migrate on a throwaway connection first, then open the pool: a pooled
    // connection must never see a schema change after it has cached a
    // prepared statement (stale `SELECT *` column metadata panics on read).
    let mut migration_conn = db::connect(&config.database_url).await?;
    db::run_migrations(&mut migration_conn).await?;
    drop(migration_conn);

    let pool = db::init_pool(&config.database_url).await?;

    let mut state = AppState::new(config.clone(), pool.clone());
    // A per-user server's first start: its account must exist before the
    // session token is bound to it (see `provision_session_account`).
    let provisioned = shiny::api::auth::provision_session_account(&state).await;
    state.session = shiny::auth::init_session(&pool, &config).await;
    let mut workers = shiny::application::lifecycle::Workers::default();
    workers.track(Some(shiny::application::voice_watchdog::start(
        state.clone(),
    )));
    workers.track(state.gpsd.start_tracked().await);
    workers.track(state.network.start_tracked().await);
    workers.track(state.audio.start_tracked().await);
    workers.track(state.bluetooth.start_tracked().await);
    workers.track(state.battery.start_tracked().await);
    // Optional services never prevent the desktop from becoming available.
    tracing::info!("Optional providers will be probed on use; unavailable services retain their existing fallbacks");

    // Server mode at startup, when the session user asked for it.
    state.autostart_remote().await;

    // Scan plugins directory and load installed cdylib plugins (hot-reload
    // registration on startup; subsequent installs use the admin API).
    std::fs::create_dir_all(&config.plugins_dir)?;
    let base_ctx = state.plugin_ctx();
    let installed = state.plugins.discover_and_install(base_ctx).await;
    // Plugins provision per-user state (the Files home) for a new account;
    // the account above was created before they were loaded.
    if let Some(id) = &provisioned {
        state.plugins.notify_user_registered(id).await;
    }
    if !installed.is_empty() {
        tracing::info!("Loaded plugins: {}", installed.join(", "));
    } else {
        tracing::info!("No plugins installed. Core runs as pure AI assistant.");
    }

    if config.diary_auto_generate {
        workers.track(Some(spawn_diary_cron(
            state.diary_gen.clone(),
            config.diary_generate_time.clone(),
        )));
    }

    // Wire live router rebuild: the served router is swappable, and every
    // plugin install/uninstall re-builds it (hot router swap, roadmap #2).
    // The rebuild closure must survive into every rebuilt router's own state,
    // so we route it through a small dispatch cell that the closure reads.
    let router_handle = RouterHandle::new(axum::Router::new());
    let rebuild_cell: Arc<std::sync::Mutex<Option<Arc<dyn Fn() + Send + Sync>>>> =
        Arc::new(std::sync::Mutex::new(None));

    let dispatch = |cell: &Arc<std::sync::Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>| {
        let cell = Arc::downgrade(cell);
        Arc::new(move || {
            if let Some(cb) = cell.upgrade().and_then(|cell| cell.lock().unwrap().clone()) {
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

    // Started by a previous server swapping itself for this binary: take its
    // socket instead of binding, so the port never closed (application::reexec).
    let std_listener = match shiny::application::reexec::inherited_listener() {
        Some(inherited) => {
            tracing::info!("Continuing on the inherited listener (server swap)");
            inherited
        }
        None => std::net::TcpListener::bind(&addr)?,
    };
    std_listener.set_nonblocking(true)?;
    // A second handle to the same socket, kept for the next swap.
    let swap_listener = std_listener.try_clone()?;
    let listener = tokio::net::TcpListener::from_std(std_listener)?;
    // Plugins replaced on disk are loaded again without a restart.
    workers.track(shiny::application::plugin_watch::start(
        state.clone(),
        rebuild.clone(),
    ));
    let mut reexec_rx = state.reexec.subscribe();
    let mut swap_requested = false;
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let server = axum::serve(listener, router_handle)
        .tcp_nodelay(true)
        .with_graceful_shutdown(async move {
            let mut shutdown_rx = shutdown_rx;
            let _ = shutdown_rx.changed().await;
        });
    let server = std::future::IntoFuture::into_future(server);
    tokio::pin!(server);
    let result = tokio::select! {
        result = &mut server => result,
        _ = wait_for_swap(&mut reexec_rx, &state) => {
            swap_requested = true;
            tracing::info!("Swapping to the new server binary: draining requests for up to 10 seconds");
            state.agent_turns.cancel_all();
            let _ = shutdown_tx.send(true);
            match tokio::time::timeout(std::time::Duration::from_secs(10), &mut server).await {
                Ok(result) => result,
                Err(_) => {
                    tracing::warn!("Request drain deadline reached; closing remaining streams");
                    Ok(())
                }
            }
        }
        _ = shiny::application::lifecycle::shutdown_signal() => {
            tracing::info!("Shutting down: draining requests for up to 10 seconds");
            state.agent_turns.cancel_all();
            let _ = shutdown_tx.send(true);
            match tokio::time::timeout(std::time::Duration::from_secs(10), &mut server).await {
                Ok(result) => result,
                Err(_) => {
                    tracing::warn!("Request drain deadline reached; closing remaining streams");
                    Ok(())
                }
            }
        }
    };
    state.agent_turns.cancel_all();
    workers.shutdown().await;
    // Long-lived SSE streams may hold a pool clone; never wait indefinitely.
    if tokio::time::timeout(std::time::Duration::from_secs(2), pool.close())
        .await
        .is_err()
    {
        tracing::warn!("Database close deadline reached");
    }
    result?;

    if swap_requested {
        // Only returns on failure; the new binary takes over this process.
        let error = shiny::application::reexec::exec_with_listener(&swap_listener);
        tracing::error!("Could not swap to the new server binary: {error}");
        return Err(error.into());
    }

    Ok(())
}

/// Resolves once a server swap was requested and no assistant turn is
/// running (an answer half-way through would be cut). Waits at most ten
/// minutes for the assistant, then goes ahead.
async fn wait_for_swap(rx: &mut tokio::sync::watch::Receiver<bool>, state: &shiny::api::AppState) {
    while !*rx.borrow_and_update() {
        if rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(600);
    while state.agent_turns.running() > 0 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

fn spawn_diary_cron(
    diary_gen: Arc<DiaryGenerator>,
    generate_time: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Parse the configured HH:MM once. The old implementation ticked
        // every hour *from process start*, so the "%H:%M == target" check
        // only ever matched if the server happened to start during the
        // target minute. Instead, sleep until the next actual occurrence.
        let (h, m) = match generate_time.split_once(':') {
            Some((h, m)) => match (h.trim().parse::<u32>(), m.trim().parse::<u32>()) {
                (Ok(h), Ok(m)) if h < 24 && m < 60 => (h, m),
                _ => {
                    tracing::warn!(
                        "Invalid DIARY_GENERATE_TIME '{generate_time}', falling back to 21:00"
                    );
                    (21, 0)
                }
            },
            None => {
                tracing::warn!(
                    "Invalid DIARY_GENERATE_TIME '{generate_time}', falling back to 21:00"
                );
                (21, 0)
            }
        };
        loop {
            let now = chrono::Local::now().naive_local();
            let today_target = now
                .date()
                .and_hms_opt(h, m, 0)
                .expect("h<24 and m<60 checked above");
            let next = if now < today_target {
                today_target
            } else {
                today_target + chrono::Duration::days(1)
            };
            let wait = (next - now)
                .to_std()
                .unwrap_or(std::time::Duration::from_secs(60));
            tokio::time::sleep(wait).await;
            tracing::info!("Auto-generating daily diary entries...");
            diary_gen.auto_generate_daily().await;
        }
    })
}
