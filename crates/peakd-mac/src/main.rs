//! `peakd` — the browser whose reason to exist is the PEAK'D! app.
//!
//! It serves the app from `127.0.0.1:8080` (loopback, loaded directly) and
//! renders the open web in native child webviews owned by [`browse`]. It no
//! longer runs a filtering proxy: over HTTPS the proxy only saw an opaque
//! tunnel, and the filter engine now runs server-side only (the in-app
//! Browser's news shelf and link previews).
//!
//! Layout:
//! * [`config`] — launch flags/environment.
//! * [`browse`] — native child webviews for the in-app Browser plugin.

// This binary is the macOS shell. On Linux the kiosk runs `peakd`
// (crates/peakd), so everything below is unreachable there — keep the build
// quiet instead of warning on every item.
#![cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]

mod bench;
mod browse;
mod config;
mod display;
mod touchbar;

use std::net::{TcpStream, ToSocketAddrs};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::ControlFlow;
use tao::window::WindowBuilder;

use wry::WebViewBuilder;

use config::PeakdConfig;

#[cfg(target_os = "macos")]
use wry::ProxyConfig;

/// The IPC message the page sends when the user asks to leave the kiosk.
pub(crate) const EXIT_MESSAGE: &str = "peakd:exit";

/// The page posts this when the user turns on Server mode. The shell exits with
/// [`SERVER_MODE_EXIT`] so `shiny-session` can relaunch into the server-mode
/// window instead of ending the session.
pub(crate) const SERVER_MODE_MESSAGE: &str = "peakd:server-mode";

/// Exit status meaning "switch to server mode" (see [`SERVER_MODE_MESSAGE`]).
pub(crate) const SERVER_MODE_EXIT: i32 = 42;

/// Set by the IPC handler; read once as the event loop tears down.
pub(crate) static EXIT_CODE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Leaving the kiosk: Cmd+Q and Alt+Q.
///
/// The webview keeps the keyboard to itself, so tao never sees these keys and
/// a kiosk that took the console had no way back to a terminal short of a
/// reboot. The page reports the gesture over the IPC bridge instead; the shell
/// then stops the event loop and the session tears down.
///
/// WKWebView reports the Command modifier as `metaKey`, so this one script
/// carries macOS; the Linux kiosk is `peakd`, where Chromium does the same
/// and the script is used there too.
///
/// `event.code` rather than `event.key`: with Alt held on ISO layouts the key
/// *is* `œ`, while the physical key is still Q. Capture phase, so a page's own
/// handler cannot swallow the way out first. Injected into the main frame on
/// every navigation, so it also works once the user browses away from the app.
pub(crate) const EXIT_SHORTCUT_JS: &str = r#"(function () {
  if (window.__peakdExitShortcut) return;
  window.__peakdExitShortcut = true;
  window.addEventListener('keydown', function (event) {
    if (event.code !== 'KeyQ' && String(event.key).toLowerCase() !== 'q') return;
    if (!(event.metaKey || event.altKey)) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    try {
      window.ipc.postMessage('peakd:exit');
    } catch (err) {
      // No IPC bridge (a plain page in a test, say): nothing to report to.
    }
  }, true);
})();"#;

fn main() -> ExitCode {
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("peakd-mac is the macOS shell; the Linux kiosk runs peakd");
        return ExitCode::FAILURE;
    }

    #[cfg(target_os = "macos")]
    {
        let cfg = PeakdConfig::from_env_and_args();

        match run(cfg) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("peakd: {err}");
                ExitCode::FAILURE
            }
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

/// OPT-IN diagnostics: `RUST_LOG=peakd=debug` (or any target) turns up the
/// shell's own tracing. Off by default so a normal launch stays quiet.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}

struct Shell {
    /// Held for the life of the process; nothing in it is read after startup.
    _cfg: PeakdConfig,
    /// Owns the native macOS Touch Bar (if any) so its buttons stay live.
    _touchbar: Option<touchbar::TouchBarHandle>,
}

/// Wait until a loopback start URL actually accepts connections.
///
/// The app is served by `shiny.service`, which loads every installed plugin
/// before it starts listening (several seconds on a cold boot). `After=` only
/// orders the units, not readiness, so peakd can start first — and a
/// `connection refused` page is not retried by the webview, which is exactly
/// how the kiosk ended up white. Non-loopback URLs are not waited for: a real
/// site's availability is not this shell's business.
fn wait_for_loopback_origin(url: &str, timeout: Duration) {
    let Some((host, port)) = loopback_endpoint(url) else {
        return;
    };
    let Ok(mut addrs) = (host.as_str(), port).to_socket_addrs() else {
        return;
    };
    let Some(addr) = addrs.next() else {
        return;
    };

    let deadline = Instant::now() + timeout;
    let mut announced = false;
    loop {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok() {
            return;
        }
        if !announced {
            println!("peakd: waiting for the app at {addr}");
            announced = true;
        }
        if Instant::now() >= deadline {
            eprintln!(
                "peakd: the app at {addr} did not answer within {timeout:?}; opening anyway"
            );
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// `(host, port)` when the URL points at loopback, `None` otherwise.
fn loopback_endpoint(url: &str) -> Option<(String, u16)> {
    let authority = url.split_once("://")?.1;
    let authority = authority.split(['/', '?', '#']).next()?;
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, port.parse().ok()?),
        None => (authority, 80),
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let loopback = host == "localhost" || host == "127.0.0.1" || host == "::1";
    loopback.then(|| (host.to_string(), port))
}

/// When `--iroh <ticket>` is set, start a local proxy that tunnels to the remote
/// server and point the shell at it. Refuse without the `iroh` feature.
#[cfg(feature = "iroh")]
fn resolve_iroh(mut cfg: PeakdConfig) -> Result<PeakdConfig, String> {
    let Some(ticket) = cfg.iroh.clone() else {
        return Ok(cfg);
    };
    let listen: std::net::SocketAddr = "127.0.0.1:0".parse().expect("valid loopback");
    let addr = shiny_iroh_client::spawn(&ticket, listen)
        .map_err(|e| format!("could not start the Iroh proxy: {e}"))?;
    let origin = format!("http://{addr}");
    println!("peakd: iroh proxy on {origin}");
    cfg.start_url = format!("{origin}/");
    cfg.app_origin = origin;
    cfg.app_mode = true;
    Ok(cfg)
}

#[cfg(not(feature = "iroh"))]
fn resolve_iroh(cfg: PeakdConfig) -> Result<PeakdConfig, String> {
    if cfg.iroh.is_some() {
        return Err("this peakd build has no Iroh support (rebuild with --features iroh)".into());
    }
    Ok(cfg)
}

fn run(cfg: PeakdConfig) -> Result<(), String> {
    init_tracing();
    activate_app();

    // `--iroh <ticket>`: serve the remote app through a local Iroh proxy.
    let cfg = resolve_iroh(cfg)?;
    // Typed with the benchmark's step enum so one loop serves both a normal
    // session (where the variant is never sent) and a benchmark run.
    let event_loop = bench::event_loop();
    let proxy = event_loop.create_proxy();

    // The app server loads its plugins for several seconds after its unit is
    // up, and only then listens. peakd can win that race, and a connection
    // refused page never retries — the kiosk would sit on a white screen
    // forever. Wait for the app to accept connections first.
    wait_for_loopback_origin(&cfg.start_url, Duration::from_secs(30));

    let window = WindowBuilder::new()
        .with_title(cfg.window_title.clone())
        .with_inner_size(LogicalSize::new(cfg.width, cfg.height))
        .with_min_inner_size(LogicalSize::new(480.0, 360.0))
        .build(&event_loop)
        .map_err(|e| format!("could not create the browser window: {e}"))?;

    println!("peakd: opening {}", cfg.start_url);
    println!(
        "peakd: app origin {}, app mode {}",
        cfg.app_origin,
        if cfg.app_mode { "on" } else { "off" }
    );
    match cfg.proxy.as_deref() {
        Some(addr) if !addr.is_empty() => {
            println!("peakd: delegating to the external proxy {addr}")
        }
        _ => println!(
            "peakd: no filtering proxy; the open web loads directly (filtering is \
             server-side only)"
        ),
    }

    // The Browser plugin drives native child webviews over this bridge (see
    // `browse`). Built before the webview so the IPC handler can own a clone;
    // the queue is drained on the event loop, the only thread allowed to touch
    // a `WebView`.
    let bus = browse::ViewBus::new(proxy.clone());

    // Touch Bar actions are queued by the native bar (macOS) and drained on
    // the event loop, which is the only thread allowed to touch the webview.
    let touchbar_bridge = touchbar::TouchBarBridge::new();

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
        })
        // The kiosk's way out, in the page where the keyboard actually is.
        .with_initialization_script(EXIT_SHORTCUT_JS)
        .with_ipc_handler({
            let bus = bus.clone();
            move |request: wry::http::Request<String>| {
                // Browser-plugin view commands first; the exit shortcut is the
                // only other message the page sends.
                if bus.handle_ipc(request.body()) {
                    return;
                }
                if request.body() == EXIT_MESSAGE {
                    bus.exit();
                    return;
                }
                if request.body() == SERVER_MODE_MESSAGE {
                    EXIT_CODE.store(SERVER_MODE_EXIT, std::sync::atomic::Ordering::SeqCst);
                    bus.exit();
                }
            }
        });

    #[cfg(target_os = "macos")]
    let builder = apply_proxy(builder, &cfg);

    // Tell the page whether this machine has a Touch Bar before it boots. The
    // flag is what the web layer's `auto` mode follows; absent, it stays
    // dormant and a normal PC is untouched.
    let builder = match cfg.touchbar.then(touchbar::init_script).flatten() {
        Some(script) => builder.with_initialization_script(script),
        None => builder,
    };

    let webview = builder
        .build(&window)
        .map_err(|e| format!("could not create the webview: {e}"))?;

    // Put a native Touch Bar on the kiosk window (macOS; a no-op elsewhere).
    let touchbar_handle = if cfg.touchbar {
        touchbar::install(&window, touchbar_bridge.clone())
    } else {
        None
    };

    // Native child webviews for the Browser plugin, drained on the event loop.
    let mut views = browse::Views::new(bus);

    // Interface scale. Settings stores the choice; apply it as webview page
    // zoom before the first paint. `auto` follows the panel DPI (≈226 DPI on a
    // Retina laptop → 200%), which is also the biggest rendering-cost lever:
    // a smaller logical viewport is far less text for WebKit to shape.
    let panel = display::panel_info();
    let auto = panel.map(|(dpi, _, _)| display::auto_scale(dpi)).unwrap_or(1.0);
    let choice = cfg
        .scale_override
        .map(display::Choice::Factor)
        .unwrap_or_else(|| display::read_choice(&cfg.display_file));
    let applied = choice.resolve(auto);
    if let Err(err) = webview.zoom(applied) {
        eprintln!("peakd: could not set the interface scale: {err}");
    }
    let display_runtime = cfg.display_runtime_file.clone();
    display::write_runtime(&display_runtime, applied, panel);
    println!("peakd: interface scale {:.0}%", applied * 100.0);

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

    // Watch for scale changes from Settings. Placed after the benchmark branch
    // (which consumes `proxy` and never returns), so only a normal session
    // watches.
    display::watch(cfg.display_file.clone(), proxy);

    let shell = Shell { _cfg: cfg, _touchbar: touchbar_handle };

    event_loop.run(move |event, _, control_flow| {
        // Poll as well as wait: a user event wakes us at once, but draining the
        // browser-view queues on a timer keeps native views responsive even if a
        // wake-up is coalesced or missed (the child-webview callbacks only
        // queue). 100 ms is below what a person notices and costs nothing when
        // the queues are empty.
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(100));

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
            }
            // Cmd/Alt+Q in the page (see EXIT_SHORTCUT_JS). Exiting cleanly is
            // what lets the service stay down and give tty1 back to a login
            // prompt instead of restarting the kiosk.
            Event::UserEvent(bench::BenchStep::Exit) => {
                println!("peakd: leaving the kiosk");
                *control_flow = ControlFlow::Exit;
            }
            // Settings changed the interface scale: apply it live and record
            // what we applied for the Settings hint.
            Event::UserEvent(bench::BenchStep::SetScale(percent)) => {
                let scale = display::Choice::from_percent(percent).resolve(auto);
                if let Err(err) = webview.zoom(scale) {
                    eprintln!("peakd: could not set the interface scale: {err}");
                }
                display::write_runtime(&display_runtime, scale, panel);
                println!("peakd: interface scale changed to {:.0}%", scale * 100.0);
            }
            Event::LoopDestroyed => {
                // A mode switch (server mode) exits with a distinct code so the
                // session supervisor can relaunch the other window.
                let code = EXIT_CODE.load(std::sync::atomic::Ordering::SeqCst);
                if code != 0 {
                    std::process::exit(code);
                }
                let _ = &shell;
            }
            _ => {}
        }

        // Apply any queued native-view commands/events here, on the event-loop
        // thread — the only thread allowed to touch a `WebView`.
        views.pump(&window, &webview);

        // Dispatch any Touch Bar presses to the page (empty off macOS).
        touchbar::pump(&touchbar_bridge, &webview);
    });
}

/// Point the webview at an explicit proxy when one was requested.
///
/// The shell no longer runs its own filtering proxy. The kiosk's own pages are
/// served from loopback (which the proxy exempted anyway), and the Browser
/// plugin renders the open web in native child webviews, where HTTPS is an
/// opaque tunnel the proxy could not filter. The filter engine still runs
/// server-side for the plugin's news shelf and link previews. An explicit
/// `--proxy` is still honoured for debugging.
#[cfg(target_os = "macos")]
fn apply_proxy<'a>(builder: WebViewBuilder<'a>, cfg: &PeakdConfig) -> WebViewBuilder<'a> {
    match cfg.proxy_endpoint() {
        Some((host, port)) => route_through(builder, host, port),
        None => builder,
    }
}

/// Point the webview at a proxy, macOS only.
#[cfg(target_os = "macos")]
fn route_through<'a>(
    builder: WebViewBuilder<'a>,
    host: String,
    port: String,
) -> WebViewBuilder<'a> {
    println!("peakd: routing webview traffic through {host}:{port}");
    builder.with_proxy_config(ProxyConfig::Http(wry::ProxyEndpoint { host, port }))
}

#[cfg(test)]
mod tests {
    use super::loopback_endpoint;

    #[test]
    fn loopback_detection() {
        assert_eq!(
            loopback_endpoint("http://127.0.0.1:8080"),
            Some(("127.0.0.1".to_string(), 8080))
        );
        assert_eq!(
            loopback_endpoint("http://localhost:8080/x?y#z"),
            Some(("localhost".to_string(), 8080))
        );
        assert_eq!(
            loopback_endpoint("http://[::1]:8080/"),
            Some(("::1".to_string(), 8080))
        );
        assert_eq!(
            loopback_endpoint("http://127.0.0.1/"),
            Some(("127.0.0.1".to_string(), 80))
        );
        assert_eq!(loopback_endpoint("https://example.com/a"), None);
        assert_eq!(loopback_endpoint("https://example.com:8443/a"), None);
        assert_eq!(loopback_endpoint("about:blank"), None);
    }
}
