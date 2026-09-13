//! `peakd` — the browser whose reason to exist is the PEAK'D! app.
//! It serves the app and nothing it does not need.
//!
//! It serves the app from `127.0.0.1:8080` by default and routes every request
//! through the shared [`shiny_filter`] engine (Brave's adblock-rust), so the
//! app itself and everything browsed inside it are filtered by the same code
//! the in-app `peakd` plugin uses.
//!
//! Layout:
//! * [`config`] — launch flags/environment.
//! * [`filter`] — the filtering proxy on its own runtime.

mod bench;
mod config;
mod filter;

use std::process::ExitCode;

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::ControlFlow;
use tao::window::WindowBuilder;

use wry::WebViewBuilder;

use config::PeakdConfig;

#[cfg(target_os = "macos")]
use wry::ProxyConfig;

/// What the in-process engine can actually filter on this platform.
///
/// This is stated rather than implied because the difference is real and
/// measurable, and a status line that said "filtering: on" on every platform
/// would be a lie. See `apply_proxy` for the mechanism.
#[cfg(target_os = "macos")]
const FILTERING_SCOPE: &str = "not wired into the native webview on macOS (see apply_proxy)";
#[cfg(not(target_os = "macos"))]
const FILTERING_SCOPE: &str = "webview proxied via the http_proxy environment";

fn main() -> ExitCode {
    let cfg = PeakdConfig::from_env_and_args();

    match run(cfg) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("peakd: {err}");
            ExitCode::FAILURE
        }
    }
}

/// A bare tao/WKWebView process launches as an accessory (background) app, so
/// its window opens *behind* whatever is already on screen. Promote it to a
/// regular, activating app — this is both what a user expects of a browser and
/// what makes automated window screenshots work.
#[cfg(target_os = "macos")]
fn activate_app() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
}

#[cfg(not(target_os = "macos"))]
fn activate_app() {}

/// OPT-IN diagnostics: `RUST_LOG=shiny_filter=debug peakd` shows every
/// request the proxy sees (and every one the filter blocks). Off by default so
/// a normal launch stays quiet.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}

struct Shell {
    /// Held so the filter engine outlives the event loop: dropping it would
    /// tear down the proxy the webview is still pointed at.
    _cfg: PeakdConfig,
    filtering: Option<filter::BackgroundFilter>,
}

fn run(cfg: PeakdConfig) -> Result<(), String> {
    init_tracing();
    activate_app();

    // Typed with the benchmark's step enum so one loop serves both a normal
    // session (where the variant is never sent) and a benchmark run.
    let event_loop = bench::event_loop();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title(cfg.window_title.clone())
        .with_inner_size(LogicalSize::new(cfg.width, cfg.height))
        .with_min_inner_size(LogicalSize::new(480.0, 360.0))
        .build(&event_loop)
        .map_err(|e| format!("could not create the browser window: {e}"))?;

    // Start the filter engine in the background rather than before the webview.
    //
    // It used to run inline here, which delayed the first paint by however long
    // the engine takes to deserialise its compiled rule set — on every launch,
    // including on macOS where the platform webview ignores the proxy. The page
    // now starts loading immediately.
    let filtering = if cfg.proxy.is_some() {
        None
    } else {
        Some(filter::start_background(&cfg))
    };

    println!("peakd: opening {}", cfg.start_url);
    println!(
        "peakd: app origin {}, app mode {}",
        cfg.app_origin,
        if cfg.app_mode { "on" } else { "off" }
    );
    match filtering.as_ref() {
        Some(_) => println!("peakd: filter engine starting in the background — {FILTERING_SCOPE}"),
        None if cfg.proxy.is_some() => println!(
            "peakd: delegating to the external proxy {}",
            cfg.proxy.as_deref().unwrap_or("")
        ),
        None => println!("peakd: filtering engine unavailable; traffic is direct"),
    }

    let builder = WebViewBuilder::new()
        .with_url(cfg.start_url.clone())
        .with_devtools(cfg.devtools)
        // A browser needs a real UA; the platform default already is one, so
        // this only pins the version string the app sees.
        .with_user_agent(concat!(
            "Peakd/0.1 ",
            "AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15"
        ))
        // Keep navigation inside the shell: rather than handing `target=_blank`
        // to the OS browser, load in place. A real tab layer replaces this.
        .with_navigation_handler(|_url| true)
        // Grant microphone/camera to the app's own pages. wry already defaults
        // to Grant, but relying on a default for something the user experiences
        // as "the mic does not work" is not good enough — and stating it means
        // a future wry default change cannot silently break voice input.
        //
        // This is only the *webview's* decision. macOS separately requires the
        // host app to be a bundle declaring NSMicrophoneUsageDescription;
        // `scripts/bundle-app.sh` builds that. Without it the request is
        // refused by the OS with no prompt. On Linux the same grant is what the
        // WebKitGTK permission request needs.
        .with_permission_handler(|kind| match kind {
            wry::PermissionKind::Microphone | wry::PermissionKind::Camera => {
                wry::PermissionResponse::Allow
            }
            // Everything else keeps the engine's normal prompting behaviour.
            _ => wry::PermissionResponse::Default,
        });

    let builder = apply_proxy(builder, &cfg, filtering.as_ref());

    let webview = builder
        .build(&window)
        .map_err(|e| format!("could not create the webview: {e}"))?;

    // The benchmark takes over the process: it measures, prints and exits.
    if let Some(bench_config) = cfg.benchmark.clone() {
        bench::announce(&bench_config);
        let drag_tile = cfg.benchmark_drag_tile;
        bench::run(
            webview,
            bench::BenchState::new(bench_config, drag_tile),
            proxy,
            event_loop,
        );
    }

    let shell = Shell { _cfg: cfg, filtering };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
            }
            Event::LoopDestroyed => {
                if let Some(rt) = shell.filtering.as_ref() {
                    println!("peakd: filter engine ready: {}", rt.ready());
                }
            }
            _ => {}
        }
    });
}

/// Route the webview through the filtering proxy where the platform supports it.
///
/// macOS can be configured in-process through wry's NetworkExtension-backed
/// `ProxyConfig` (the `mac-proxy` feature). Linux/GTK has no equivalent hook in
/// wry, so the shell falls back to the environment (`http_proxy`/`https_proxy`,
/// read by libsoup) and reports honestly when filtering is unavailable rather
/// than pretending the traffic was filtered.
fn apply_proxy<'a>(
    builder: WebViewBuilder<'a>,
    cfg: &PeakdConfig,
    own_engine: Option<&filter::BackgroundFilter>,
) -> WebViewBuilder<'a> {
    // An explicit `--proxy` wins and is known immediately.
    if let Some((host, port)) = cfg.proxy_endpoint() {
        return route_through(builder, host, port);
    }

    let Some(engine) = own_engine else {
        return builder;
    };

    // The engine is starting in the background, so its port is not known yet
    // and cannot be waited for without giving back the startup time this change
    // exists to save.
    //
    // On macOS that costs nothing: the platform webview ignores `ProxyConfig`
    // anyway (see the note above), so routing it was never going to filter
    // anything. Blocking startup to configure a proxy that has no effect was
    // pure loss.
    //
    // On Linux the environment variables *are* the mechanism and are read when
    // the webview's network process starts, so there the port genuinely has to
    // be known first — waiting is the correct trade.
    #[cfg(target_os = "macos")]
    {
        let _ = engine;
        println!(
            "peakd: webview filtering is not wired on macOS, so the engine's port is not \
             needed at startup"
        );
        builder
    }

    #[cfg(not(target_os = "macos"))]
    {
        match engine.wait_for_endpoint(std::time::Duration::from_secs(10)) {
            Some((host, port)) => route_through(builder, host, port),
            None => {
                eprintln!(
                    "peakd: filter engine did not come up in time; browsing without it"
                );
                builder
            }
        }
    }
}

/// Point the webview at a proxy, per platform.
fn route_through<'a>(
    builder: WebViewBuilder<'a>,
    host: String,
    port: String,
) -> WebViewBuilder<'a> {
    #[cfg(target_os = "macos")]
    {
        println!("peakd: routing webview traffic through {host}:{port}");
        builder.with_proxy_config(ProxyConfig::Http(wry::ProxyEndpoint { host, port }))
    }

    #[cfg(not(target_os = "macos"))]
    {
        // Best effort: give libsoup/WebKitGTK the proxy through the
        // environment. This runs before the webview (and therefore any WebKit
        // thread) exists, so mutating the environment here is not a data race.
        let addr = format!("http://{host}:{port}");
        std::env::set_var("http_proxy", &addr);
        std::env::set_var("https_proxy", &addr);
        std::env::set_var("HTTP_PROXY", &addr);
        std::env::set_var("HTTPS_PROXY", &addr);
        println!(
            "peakd: webview proxy via environment ({addr}); \
             on this platform filtering depends on the system webview honouring it"
        );
        builder
    }
}
