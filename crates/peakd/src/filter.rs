//! The browser's filtering runtime.
//!
//! The webview runs on the main thread inside tao's event loop, which is not a
//! tokio runtime and must not be blocked. The proxy therefore gets its own
//! tokio runtime on a dedicated thread; the shell talks to it through the
//! small, synchronous handle built here.
//!
//! Both the `peakd` shell and the `browser` plugin construct the proxy from
//! [`shiny_filter`], so the filter engine, request classifier, rewriter and
//! injection pipeline are literally the same code in both surfaces.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use shiny_filter::engine::{AdFilter, FilterConfig};
use shiny_filter::metrics::{Metrics, MetricsSnapshot};
use shiny_filter::proxy::{run_proxy, ProxyConfig};

use crate::config::PeakdConfig;

/// A proxy running on its own runtime, owned by the browser shell.
/// Start the filter engine on a background thread.
///
/// The engine takes real time to come up: it deserialises a ~6 MB compiled
/// rule set, builds a tokio runtime and binds a listener. Doing that inline
/// before the webview exists meant every launch showed a **blank window** for
/// the duration — which is part of why the app felt slower to open than Chrome,
/// and it is paid even on macOS where the platform webview ignores the proxy
/// entirely.
///
/// Returns immediately. The page starts loading at once; anything that needs
/// the proxy finds it ready by the time it asks.
pub fn start_background(cfg: &PeakdConfig) -> BackgroundFilter {
    use std::sync::mpsc;
    let (tx, rx) = mpsc::channel();
    let cfg = cfg.clone();

    std::thread::Builder::new()
        .name("peakd-filter-start".into())
        .spawn(move || {
            let started = std::time::Instant::now();
            let result: anyhow::Result<FilterRuntime> = FilterRuntime::start(&cfg);
            match &result {
                Ok(rt) => println!(
                    "peakd: filter engine ready on {} in {:?}",
                    rt.base(),
                    started.elapsed()
                ),
                Err(err) => eprintln!("peakd: filter engine unavailable ({err})"),
            }
            let _ = tx.send(result);
        })
        .expect("failed to spawn the filter startup thread");

    BackgroundFilter { rx: Some(rx) }
}

/// Handle to a filter engine that is starting up. Dropping it keeps whatever
/// has already been started alive for the life of the process.
pub struct BackgroundFilter {
    rx: Option<std::sync::mpsc::Receiver<anyhow::Result<FilterRuntime>>>,
}

impl BackgroundFilter {
    /// Wait for the engine's address, for platforms that need it before the
    /// webview is created (Linux: the proxy is passed through the environment
    /// and read when WebKit's network process starts).
    #[cfg(not(target_os = "macos"))]
    pub fn wait_for_endpoint(&self, timeout: std::time::Duration) -> Option<(String, String)> {
        let rx = self.rx.as_ref()?;
        rx.recv_timeout(timeout).ok()?.ok().map(|rt| rt.endpoint())
    }

    /// Whether the engine has finished starting. Never blocks.
    pub fn ready(&self) -> bool {
        self.rx
            .as_ref()
            .map(|rx| !matches!(rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)))
            .unwrap_or(true)
    }
}

pub struct FilterRuntime {
    addr: SocketAddr,
    base: String,
    /// Read only where the shell has to hand the proxy to the webview, which on
    /// macOS it never does (see `apply_proxy`). Kept on every platform because
    /// the struct is constructed the same way everywhere.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    metrics: Metrics,
    /// Wrapped in an `Option` so `Drop` can consume it: `shutdown_timeout`
    /// takes the runtime by value and `Drop` only has `&mut self`.
    runtime: Option<tokio::runtime::Runtime>,
}

impl FilterRuntime {
    /// Start filtering on an ephemeral loopback port.
    ///
    /// Blocking on purpose: the caller has not created the webview yet, and
    /// the webview must be pointed at a proxy that is already listening.
    pub fn start(cfg: &PeakdConfig) -> anyhow::Result<Self> {
        let cache_dir: PathBuf = PathBuf::from(&cfg.data_dir).join("adfilter");

        let mut filter_config = if cfg.offline {
            FilterConfig::offline(cache_dir)
        } else {
            FilterConfig::new(cache_dir)
        };
        // Pin a couple of rules that protect the proxy's own invariants in
        // every profile, and keep them visible rather than magic.
        filter_config.extra_filters.push(
            "! pinned: never filter the app's own origin\n".to_string(),
        );

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("shiny-filter")
            .build()?;

        let (addr, base, metrics) = runtime.block_on(async {
            let filter = AdFilter::load(filter_config).await;
            let handle = run_proxy(ProxyConfig::default(), filter)
                .await
                .map_err(|e| anyhow::anyhow!("could not start the filter proxy: {e}"))?;
            let addr = handle.addr();
            let base = handle.base().to_string();
            let metrics = handle.metrics().clone();
            // The handle owns the accept loop task on this runtime; keep the
            // runtime alive for as long as this struct lives.
            std::mem::forget(handle);
            Ok::<_, anyhow::Error>((addr, base, metrics))
        })?;

        let thread = runtime
            .handle()
            .spawn(async {
                // Keep the runtime's thread alive until the process exits; the
                // accept loop itself lives on a spawned task above.
                std::future::pending::<()>().await
            });

        // `spawn` returns a JoinHandle whose task never completes; we only
        // need the thread to stay parked, so we drop the handle.
        drop(thread);

        Ok(Self {
            addr,
            base,
            metrics,
            runtime: Some(runtime),
        })
    }

    /// The bound address. Kept for diagnostics and tests; the shell itself
    /// only needs [`FilterRuntime::endpoint`].
    #[allow(dead_code)]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Scheme+authority the webview must be pointed at.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// `host:port` for `ProxyConfig`, which wants them separately.
    ///
    /// Linux-only in practice: macOS builds the webview without a proxy, so the
    /// shell never asks for the port there.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn endpoint(&self) -> (String, String) {
        ("127.0.0.1".to_string(), self.addr.port().to_string())
    }

    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn snapshot(&self) -> MetricsSnapshot {
        self.metrics.snapshot()
    }
}

impl Drop for FilterRuntime {
    fn drop(&mut self) {
        // Give in-flight requests a moment, then let the runtime go. The
        // accept-loop task is dropped with it.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_millis(250));
        }
    }
}
