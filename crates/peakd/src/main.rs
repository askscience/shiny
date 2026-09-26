//! `peakd` — the PEAK'D! kiosk shell on Qt 6 + QtWebEngine.
//!
//! The app stays the website it is; this binary is the engine host that
//! replaces WebKitGTK (see `PLAN-qt6-webengine.md`). Rust owns the policy:
//! config, the Browser plugin's command protocol and queues, interface scale,
//! trackpad gestures and exit semantics. Qt lives behind `shim.rs`.
//!
//! Layout:
//! * [`config`] — launch flags/environment.
//! * [`browse`] — the `peakd:view:*` protocol, queues and the view state
//!   machine; [`host`] binds it to the Qt child views.
//! * [`display`] — interface scale files and the DPI-derived `auto` choice.
//! * [`gestures`] — evdev three-finger swipes.
//! * [`shim`] — the C ABI in `shim/peakd_qt.h`.

mod bench;
mod browse;
mod config;
mod display;
mod gestures;
mod host;
mod shim;

use std::ffi::{c_char, c_void, CStr};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime};

use parking_lot::Mutex;

use browse::{ViewBus, Views};
use config::PeakdConfig;
use host::QtHost;

/// The IPC message the page sends when the user asks to leave the kiosk.
const EXIT_MESSAGE: &str = "peakd:exit";

/// The page posts this when the user turns on Server mode. The shell exits
/// with [`SERVER_MODE_EXIT`] so the session supervisor can relaunch into the
/// server-mode window instead of ending the session.
const SERVER_MODE_MESSAGE: &str = "peakd:server-mode";

/// Exit status meaning "switch to server mode".
const SERVER_MODE_EXIT: i32 = 42;

/// Set by the IPC handler; read once the Qt event loop returns.
static EXIT_CODE: AtomicI32 = AtomicI32::new(0);

/// Leaving the kiosk: Cmd+Q and Alt+Q.
///
/// On Linux Chromium delivers the Super/Command modifier to the DOM (unlike
/// WebKitGTK, which stripped it before the page could see it), so this single
/// script works on every platform the Qt shell runs on. `event.code` rather
/// than `event.key`: with Alt held on ISO layouts the key *is* `œ`, while the
/// physical key is still Q. Capture phase, so a page's own handler cannot
/// swallow the way out first. Injected into every page and future page.
const EXIT_SHORTCUT_JS: &str = r#"(function () {
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

static BUS: OnceLock<ViewBus> = OnceLock::new();
static VIEWS: OnceLock<Mutex<Views<QtHost>>> = OnceLock::new();
static GESTURES: OnceLock<gestures::GestureBridge> = OnceLock::new();
static DISPLAY: OnceLock<Mutex<DisplayState>> = OnceLock::new();
static BENCH: OnceLock<Mutex<bench::BenchState>> = OnceLock::new();
static START_URL: OnceLock<String> = OnceLock::new();

/// Interface-scale state, polled on the pump (a stat every 100 ms).
struct DisplayState {
    choice_path: PathBuf,
    runtime_path: PathBuf,
    /// `--scale`, applied at startup; a later change to the choice file wins,
    /// exactly like the GTK shell.
    override_choice: Option<display::Choice>,
    /// The panel-derived factor `auto` resolves to.
    auto: f64,
    /// What was applied; `None` until the first tick.
    applied: Option<f64>,
    last_mtime: Option<SystemTime>,
    info: Option<(f64, i32, i32)>,
}

fn main() {
    #[cfg(target_os = "linux")]
    std::process::exit(linux_main());

    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("peakd is Linux-only; macOS keeps the wry/WKWebView shell");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
fn linux_main() -> i32 {
    let cfg = PeakdConfig::from_env_and_args();

    // Force Qt's own scale factor to 1 before Qt initialises: the interface
    // scale is the app's page zoom (display.rs), exactly as with WebKitGTK.
    // Without this, a HiDPI panel would scale twice.
    std::env::set_var("QT_SCALE_FACTOR", "1");
    std::env::set_var("QT_AUTO_SCREEN_SCALE_FACTOR", "0");

    if cfg.devtools {
        // QtWebEngine's devtools live in Chromium; expose them on loopback.
        std::env::set_var("QTWEBENGINE_REMOTE_DEBUGGING", "127.0.0.1:9222");
        println!("peakd: devtools on http://127.0.0.1:9222");
    }

    match run(cfg) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("peakd: {err}");
            1
        }
    }
}

/// One IPC message from any page. Browser-view commands go to the queue;
/// everything else is the exit contract.
extern "C" fn on_ipc(_userdata: *mut c_void, body: *const c_char) {
    let body = unsafe { CStr::from_ptr(body) }.to_string_lossy().into_owned();
    if let Some(bus) = BUS.get() {
        if bus.handle_ipc(&body) {
            // Apply the view command now, not on the next 100 ms pump tick. A
            // `setBounds` during a window drag has to land in the same frame as
            // the DOM write, otherwise the native Browser page trails its own
            // window frame by up to a tick.
            pump_views();
            return;
        }
    }
    match body.as_str() {
        EXIT_MESSAGE => {
            println!("peakd: leaving the kiosk");
            EXIT_CODE.store(0, Ordering::SeqCst);
            shim::quit();
        }
        SERVER_MODE_MESSAGE => {
            EXIT_CODE.store(SERVER_MODE_EXIT, Ordering::SeqCst);
            shim::quit();
        }
        _ => {}
    }
}

/// View lifecycle from the shim. Events are queued; the pump reports them to
/// the plugin with `window.__peakdViewEvent`.
extern "C" fn on_view(_userdata: *mut c_void, id: *const c_char, kind: *const c_char, payload: *const c_char) {
    let id = unsafe { CStr::from_ptr(id) }.to_string_lossy().into_owned();
    let kind = unsafe { CStr::from_ptr(kind) }.to_string_lossy().into_owned();
    let payload = unsafe { CStr::from_ptr(payload) }.to_string_lossy().into_owned();

    // The top-level window's move is the benchmark's drag signal.
    if id == "window" && kind == "move" {
        if let Some(bench) = BENCH.get() {
            bench.lock().observe_window_move();
        }
        return;
    }

    let Some(bus) = BUS.get() else { return };
    match kind.as_str() {
        "load" | "title" | "url" | "new-window" => {
            bus.push_event(ViewBus::view_event(&id, &kind, &payload));
        }
        "crashed" => {
            eprintln!("peakd: view {id} crashed ({payload})");
            if id == "main" {
                // The app's own renderer died: bring it back rather than
                // leaving a dead white window in the kiosk.
                if let Some(url) = START_URL.get() {
                    shim::main_load(url);
                }
            }
        }
        _ => {}
    }
}

/// The shell's pump (Qt timer, ~100 ms): drain view commands/events, dispatch
/// gestures, apply scale changes.
extern "C" fn on_pump(_userdata: *mut c_void) {
    pump_views();
    if let Some(bridge) = GESTURES.get() {
        for direction in bridge.drain() {
            shim::main_run_js(&gestures::dispatch_script(direction), 0);
        }
    }
    display_tick();
    bench_tick();
}

/// Drain the browser-view command and event queues. Called on the pump and,
/// for commands, immediately from `on_ipc` so a moving native view keeps up.
fn pump_views() {
    if let Some(views) = VIEWS.get() {
        views.lock().pump(|script| shim::main_run_js(script, 0));
    }
}

/// The benchmark's schedule, advanced on the pump.
fn bench_tick() {
    let Some(lock) = BENCH.get() else { return };
    let mut state = lock.lock();
    let Some(step) = state.tick() else { return };
    match step {
        bench::Step::Inject => {
            shim::main_run_js(bench::BENCH_JS, 0);
            if state.drag_tile() {
                shim::main_run_js(bench::DRAG_TILE_JS, 0);
                println!("benchmark: dragging a plugin window");
            }
            println!("benchmark: measuring...");
        }
        bench::Step::Collect => {
            if state.drag_tile() {
                shim::main_run_js(
                    "window.__peakdTileDragStop && window.__peakdTileDragStop()",
                    0,
                );
            }
            shim::main_run_js(
                "JSON.stringify(window.__peakdBenchReport ? window.__peakdBenchReport() : null)",
                1,
            );
        }
    }
}

/// Results of `run_js`: callback id 1 is the benchmark report.
extern "C" fn on_js(_userdata: *mut c_void, id: i32, value: *const c_char) {
    if id != 1 {
        return;
    }
    let raw = unsafe { CStr::from_ptr(value) }.to_string_lossy().into_owned();
    if let Some(bench) = BENCH.get() {
        bench.lock().finish(&raw);
    }
    shim::quit();
}

fn display_tick() {
    let Some(lock) = DISPLAY.get() else { return };
    let mut state = lock.lock();

    let mtime = display::modified(&state.choice_path);
    let changed = mtime != state.last_mtime;
    state.last_mtime = mtime;
    if !changed && state.applied.is_some() {
        return;
    }

    let resolved = if state.applied.is_none() {
        // First application: `--scale` wins over the stored choice.
        state
            .override_choice
            .unwrap_or_else(|| display::read_choice(&state.choice_path))
            .resolve(state.auto)
    } else {
        display::read_choice(&state.choice_path).resolve(state.auto)
    };
    if state.applied == Some(resolved) {
        return;
    }
    shim::main_zoom(resolved);
    display::write_runtime(&state.runtime_path, resolved, state.info);
    state.applied = Some(resolved);
    println!("peakd: interface scale {:.0}%", resolved * 100.0);
}

/// Wait until a loopback start URL actually accepts connections.
///
/// The app is served by `shiny.service`, which loads every installed plugin
/// before it starts listening (several seconds on a cold boot). `After=` only
/// orders the units, not readiness, so peakd can start first — and a
/// `connection refused` page is never retried by the webview, which is exactly
/// how the kiosk ended up white. Non-loopback URLs are not waited for.
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

/// When `--iroh <ticket>` is set, start a local proxy that tunnels to the
/// remote server and point the shell at it. Refuse without the feature.
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

fn run(cfg: PeakdConfig) -> Result<i32, String> {
    let cfg = resolve_iroh(cfg)?;

    wait_for_loopback_origin(&cfg.start_url, Duration::from_secs(30));

    println!("peakd: opening {}", cfg.start_url);
    println!(
        "peakd: app origin {}, app mode {}",
        cfg.app_origin,
        if cfg.app_mode { "on" } else { "off" }
    );
    if let Some(benchmark) = cfg.benchmark.clone() {
        bench::announce(&benchmark);
        BENCH
            .set(Mutex::new(bench::BenchState::new(
                benchmark,
                cfg.benchmark_drag_tile,
            )))
            .ok();
    }

    // The profile lives beside the shell's other state.
    let profile_dir = format!("{}/qtwebengine", cfg.data_dir);

    START_URL.set(cfg.start_url.clone()).ok();
    let bus = ViewBus::new();
    BUS.set(bus.clone()).ok();
    VIEWS
        .set(Mutex::new(Views::new(bus, QtHost)))
        .ok();
    GESTURES.set(gestures::spawn()).ok();

    let dpi = shim::screen_dpi();
    let (width, height) = shim::screen_size();
    let auto = display::auto_scale(dpi);
    DISPLAY
        .set(Mutex::new(DisplayState {
            choice_path: cfg.display_file.clone(),
            runtime_path: cfg.display_runtime_file.clone(),
            override_choice: cfg.scale_override.map(display::Choice::Factor),
            auto,
            applied: None,
            last_mtime: None,
            info: (dpi > 0.0).then_some((dpi, width, height)),
        }))
        .ok();

    shim::inject_script("peakd:exit", EXIT_SHORTCUT_JS);
    shim::set_window(&cfg.window_title, cfg.width, cfg.height);

    let url = std::ffi::CString::new(cfg.start_url.clone()).expect("URLs carry no NUL");
    let data_dir =
        std::ffi::CString::new(profile_dir.clone()).expect("paths carry no NUL");
    let rc = unsafe {
        shim::peakd_qt_run(
            url.as_ptr(),
            data_dir.as_ptr(),
            0,
            on_ipc,
            on_view,
            on_pump,
            on_js,
            std::ptr::null_mut(),
        )
    };

    let code = EXIT_CODE.load(Ordering::SeqCst);
    Ok(if code != 0 { code } else { rc })
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
