//! Environment probe: what can this webview actually do?
//!
//! Opens the PEAK'D! app in a real `wry`/WKWebView window, asks the page a set
//! of capability questions, prints the answers as JSON, and exits. This exists
//! because a `WKWebView` has no remote inspector, so "does it support WebGL /
//! how fast does it paint" cannot be answered by inspection — it has to be
//! measured in the engine itself.
//!
//! It reports capabilities, a live paint audit and a frame rate. The frame
//! rate from here is only trustworthy when the window is genuinely frontmost:
//! an occluded WKWebView has `requestAnimationFrame` throttled, and this probe
//! cannot guarantee visibility. For a reliable timing measurement use the
//! browser's own benchmark (below), which runs in the window the user is
//! actually looking at.
//!
//! ```text
//! cargo run -p peakd --example probe -- --audit     # paint audit only
//! cargo run -p peakd --example probe -- --frames    # frame rate (see note)
//! ./target/release/peakd --benchmark                # reliable timing
//! ```

use std::process::ExitCode;
use std::time::{Duration, Instant};

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

/// Asks the page everything we want to know, in one evaluation.
///
/// Kept as a single script so the probe costs one round trip and cannot
/// half-succeed. Every probe is defensive: a capability that throws must be
/// reported as `false`, not abort the whole measurement.
const PROBE_JS: &str = r#"(function () {
  const out = {};

  // --- WebGL ---------------------------------------------------------------
  try {
    const c = document.createElement('canvas');
    const gl = c.getContext('webgl2') || c.getContext('webgl') || c.getContext('experimental-webgl');
    out.webgl = !!gl;
    if (gl) {
      out.webgl_version = gl.getParameter(gl.VERSION);
      const dbg = gl.getExtension('WEBGL_debug_renderer_info');
      out.webgl_vendor = dbg ? gl.getParameter(dbg.UNMASKED_VENDOR_WEBGL) : gl.getParameter(gl.VENDOR);
      out.webgl_renderer = dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
      out.webgl2 = !!c.getContext('webgl2');
      out.max_texture = gl.getParameter(gl.MAX_TEXTURE_SIZE);
      // Draw one frame: proves the pipeline works, not just that the API exists.
      gl.clearColor(0.2, 0.4, 0.8, 1.0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      const px = new Uint8Array(4);
      gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
      out.webgl_draw_ok = px[2] > 100 && px[0] < 100;
    }
  } catch (e) { out.webgl = false; out.webgl_error = String(e); }

  // --- Hardware concurrency / memory hints ---------------------------------
  out.cores = navigator.hardwareConcurrency || null;
  out.device_memory_gb = navigator.deviceMemory || null;

  // --- Media capture (is the API even present?) ----------------------------
  out.media_devices = !!(navigator.mediaDevices && navigator.mediaDevices.getUserMedia);
  out.secure_context = window.isSecureContext;
  out.origin = location.origin;

  // --- OffscreenCanvas / workers -------------------------------------------
  out.offscreen_canvas = typeof OffscreenCanvas !== 'undefined';
  out.worker = typeof Worker !== 'undefined';
  out.webgpu = !!navigator.gpu;

  return JSON.stringify(out);
})()"#;

/// Signs in through the app's own login form.
///
/// The login screen has no orb and no plugin windows, so measuring there says
/// nothing about the app the user actually drags around. Drives the real form
/// rather than writing `localStorage` so the probe exercises the same path a
/// person does.
fn login_js(user: &str, pass: &str) -> String {
    let u = serde_json::to_string(user).unwrap_or_else(|_| "\"\"".into());
    let p = serde_json::to_string(pass).unwrap_or_else(|_| "\"\"".into());
    format!(
        r#"(function () {{
  const overlay = document.getElementById('login-overlay');
  const app = document.getElementById('app');
  const loggedIn = app && !app.classList.contains('hidden') && overlay && overlay.classList.contains('hidden');
  if (loggedIn) {{ window.__probeLogin = 'already'; return 'already'; }}

  const user = document.getElementById('login-username');
  const pw = document.getElementById('login-password');
  const btn = document.getElementById('login-btn');
  if (!user || !pw || !btn) {{ window.__probeLogin = 'no-form'; return 'no-form'; }}

  // The manual step is where a fresh profile lands; make sure it is showing.
  document.getElementById('login-step-password').classList.remove('hidden');
  document.getElementById('login-step-pick').classList.add('hidden');
  document.getElementById('login-step-register').classList.add('hidden');
  user.classList.remove('hidden');
  user.value = {u};
  pw.value = {p};
  btn.click();
  window.__probeLogin = 'clicked';
  return 'clicked';
}})()"#
    )
}

/// Audits what actually costs paint time in the live page.
///
/// Kept in `js/audit.js` rather than inline: it is a sizeable script, and a
/// raw string literal with escapes inside it is exactly the kind of thing that
/// silently breaks. Editing it as JavaScript also means an editor can check it.
const AUDIT_JS: &str = include_str!("../js/audit.js");

/// Measures the app's animation loop: how many frames in a fixed window.
///
/// `requestAnimationFrame` is the honest measure of what the user perceives —
/// it stalls when the compositor or main thread is busy, which is exactly the
/// symptom being investigated.
fn fps_js(ms: u64) -> String {
    format!(
        r#"(function () {{
  window.__probeFps = {{ running: true }};
  return new Promise(function (resolve) {{
    let frames = 0;
    let long = 0;
    let worst = 0;
    let last = performance.now();
    const start = last;
    function tick(now) {{
      const dt = now - last;
      last = now;
      frames++;
      if (dt > 32) long++;         // slower than ~30fps
      if (dt > worst) worst = dt;
      if (now - start < {ms}) requestAnimationFrame(tick);
      else {{
        const result = {{
          ms: Math.round(now - start),
          frames: frames,
          fps: +(frames / ((now - start) / 1000)).toFixed(1),
          long_frames: long,
          worst_frame_ms: +worst.toFixed(1),
          canvases: document.querySelectorAll('canvas').length,
          tiles: document.querySelectorAll('#tile-grid .tile').length,
          running: false,
        }};
        window.__probeFps = result;
        resolve(JSON.stringify(result));
      }}
    }}
    requestAnimationFrame(tick);
  }});
}})()"#
    )
}

/// Measures a forced synchronous layout + paint of the whole document.
///
/// A drag on macOS resizes the window every frame; if relayout is expensive,
/// that is what makes dragging feel slow. Repeated so we see steady state.
const RELAYOUT_JS: &str = r#"(function () {
  const t0 = performance.now();
  for (let i = 0; i < 20; i++) {
    document.body.style.width = (i % 2 ? '99.9%' : '100%');
    void document.body.offsetHeight; // force synchronous layout
  }
  document.body.style.width = '';
  void document.body.offsetHeight;
  return JSON.stringify({
    relayout_ms: +((performance.now() - t0) / 20).toFixed(2),
    nodes: document.querySelectorAll('*').length,
    canvases: document.querySelectorAll('canvas').length,
  });
})()"#;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let want_frames = args.iter().any(|a| a == "--frames");
    let frames_ms: u64 = args
        .iter()
        .position(|a| a == "--frames-ms")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(3000);
    let url = args
        .iter()
        .position(|a| a == "--url")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "http://127.0.0.1:8080".to_string());
    // Frame-rate numbers are only meaningful in a visible window: a hidden or
    // occluded one has `requestAnimationFrame` throttled by the browser, which
    // would report a low fps that says nothing about the app.
    let visible = args.iter().any(|a| a == "--visible") || want_frames;
    let audit_only = args.iter().any(|a| a == "--audit");
    // A/B switch: measure the same page with every backdrop-filter removed.
    let no_backdrop = args.iter().any(|a| a == "--no-backdrop");
    let user = args
        .iter()
        .position(|a| a == "--user")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let password = args
        .iter()
        .position(|a| a == "--password")
        .and_then(|i| args.get(i + 1))
        .cloned();
    // Frame rate scales with the pixel count being repainted, which is the
    // whole question for "dragging is slow": a drag repaints the full surface.
    // Being able to vary the size turns that into a measurement.
    let (w, h) = args
        .iter()
        .position(|a| a == "--size")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.split_once('x'))
        .and_then(|(a, b)| Some((a.parse::<f64>().ok()?, b.parse::<f64>().ok()?)))
        .unwrap_or((1280.0, 820.0));
    let settle_ms: u64 = args
        .iter()
        .position(|a| a == "--settle")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(6000);

    match run(
        &url,
        want_frames,
        frames_ms,
        settle_ms,
        visible,
        (w, h),
        user.zip(password),
        audit_only,
        no_backdrop,
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("probe: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(
    url: &str,
    want_frames: bool,
    frames_ms: u64,
    settle_ms: u64,
    visible: bool,
    size: (f64, f64),
    credentials: Option<(String, String)>,
    audit_only: bool,
    no_backdrop: bool,
) -> Result<(), String> {
    /// Steps driven from the event loop; each one schedules the next.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Step {
        Login,
        Capabilities,
        Audit,
        Relayout,
        Frames,
        PollFrames,
        Report,
    }

    // A background/occluded WKWebView has `requestAnimationFrame` throttled to
    // a near-stop, so an un-activated probe measures the throttle, not the app.
    // Promote and activate exactly as the real shell does.
    #[cfg(target_os = "macos")]
    {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
        if let Some(mtm) = MainThreadMarker::new() {
            let app = NSApplication::sharedApplication(mtm);
            app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
        }
    }

    let event_loop = EventLoopBuilder::<Step>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title("peakd probe")
        .with_inner_size(LogicalSize::new(size.0, size.1))
        // Hidden by default so a capability probe never steals focus. Frame
        // measurement forces it visible, because rAF is throttled otherwise.
        .with_visible(visible)
        .build(&event_loop)
        .map_err(|e| format!("window: {e}"))?;

    let webview = WebViewBuilder::new()
        .with_url(url)
        .with_devtools(false)
        .build(&window)
        .map_err(|e| format!("webview: {e}"))?;

    println!("probe: loaded {url} (visible={visible}, size={}x{})", size.0, size.1);

    // Kick off after the page has settled, logging in first when asked.
    let first_step = if credentials.is_some() { Step::Login } else { Step::Capabilities };
    {
        let proxy = proxy.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(settle_ms));
            let _ = proxy.send_event(first_step);
        });
    }

    // Only present when the caller asked us to sign in.
    let credentials = credentials.unwrap_or_default();

    let collected: std::sync::Arc<parking_lot::Mutex<Vec<String>>> =
        std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));

    event_loop.run(move |event, _, control_flow| {
        // `Wait` — do real work only in response to our own steps. Sleeping in
        // the handler (as an earlier version did) stalls the very loop that is
        // supposed to print the result.
        *control_flow = ControlFlow::Wait;

        let Event::UserEvent(step) = event else {
            if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
                *control_flow = ControlFlow::Exit;
            }
            return;
        };

        if std::env::var("PROBE_DEBUG").is_ok() {
            eprintln!("step: {step:?}");
        }
        match step {
            Step::Login => {
                let js = login_js(&credentials.0, &credentials.1);
                let next = proxy.clone();
                let _ = webview.evaluate_script_with_callback(&js, move |raw| {
                    println!("login: {}", raw.trim().trim_matches('"'));
                    // The app boots its orb, tiles and map asynchronously; a
                    // measurement started immediately would catch the startup
                    // spike rather than steady state.
                    let next = next.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(9000));
                        let _ = next.send_event(Step::Capabilities);
                    });
                });
            }
            Step::Capabilities => {
                // The callback runs on the main thread just before the next
                // loop turn, so schedule the next step from there.
                let proxy = proxy.clone();
                let _ = webview.evaluate_script_with_callback(PROBE_JS, move |raw| {
                    println!("{}", pretty(&raw));
                    let _ = proxy.send_event(Step::Audit);
                });
            }
            Step::Audit => {
                if no_backdrop {
                    // Applied before the audit script so both the counts and the
                    // frame measurement see the same page.
                    let _ = webview.evaluate_script(
                        "document.documentElement.style.setProperty('--glass-blur','0px');\
                         const s=document.createElement('style');\
                         s.textContent='*,*::before,*::after{backdrop-filter:none !important;-webkit-backdrop-filter:none !important;}';\
                         document.head.appendChild(s); 'ok'",
                    );
                }
                let proxy = proxy.clone();
                let _ = webview.evaluate_script_with_callback(AUDIT_JS, move |raw| {
                    println!("{}", pretty_audit(&raw));
                    let next = if audit_only { Step::Report } else { Step::Relayout };
                    let _ = proxy.send_event(next);
                });
            }
            Step::Relayout => {
                let proxy = proxy.clone();
                let _ = webview.evaluate_script_with_callback(RELAYOUT_JS, move |raw| {
                    println!("relayout: {raw}");
                    let _ = proxy.send_event(if want_frames { Step::Frames } else { Step::Report });
                });
            }
            Step::Frames => {
                // Kick off the measurement, then poll the value it parks on
                // `window.__probeFps`. The promise itself is not deliverable
                // through the script callback, and sleeping here would block
                // the loop that has to read it.
                let _ = webview.evaluate_script(&fps_js(frames_ms));
                let ticker = proxy.clone();
                std::thread::spawn(move || {
                    let deadline = Instant::now() + Duration::from_millis(frames_ms + 5000);
                    loop {
                        std::thread::sleep(Duration::from_millis(250));
                        if ticker.send_event(Step::PollFrames).is_err() || Instant::now() > deadline {
                            break;
                        }
                    }
                });
            }
            Step::PollFrames => {
                let proxy = proxy.clone();
                let collected = collected.clone();
                let _ = webview.evaluate_script_with_callback(
                    "JSON.stringify(window.__probeFps || {running:true})",
                    move |raw| {
                        let text = raw.trim().trim_matches('"').replace("\\\"", "\"");
                        if std::env::var("PROBE_DEBUG").is_ok() {
                            eprintln!("poll raw: {raw}");
                        }
                        if text.contains("\"running\":false") || text.contains("\"running\": false") {
                            let mut done = collected.lock();
                            if done.iter().any(|l| l.starts_with("fps: ")) {
                                return; // already reported; the poller may still be running
                            }
                            done.push(format!("fps: {text}"));
                            drop(done);
                            let _ = proxy.send_event(Step::Report);
                        }
                    },
                );
            }
            Step::Report => {
                let lines: Vec<String> = collected.lock().drain(..).collect();
                if std::env::var("PROBE_DEBUG").is_ok() {
                    eprintln!("report: {} line(s)", lines.len());
                }
                for line in lines {
                    println!("{line}");
                }
                // stdin/stdout is a pipe here; make sure the caller sees the
                // result before the process goes away.
                use std::io::Write;
                let _ = std::io::stdout().flush();
                *control_flow = ControlFlow::Exit;
            }
        }
    });
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Stage {
    WaitSettle,
    Capabilities,
    Frames,
}

/// Same as [`pretty`], but for the audit payload (which is always an object).
fn pretty_audit(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('"').replace("\\\"", "\"");
    match serde_json::from_str::<serde_json::Value>(&trimmed) {
        Ok(v) => format!("audit:\n{}", serde_json::to_string_pretty(&v).unwrap_or(trimmed)),
        Err(_) => format!("audit(raw): {raw}"),
    }
}

/// Re-indent the JSON the page returned so the output is readable.
fn pretty(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('"').replace("\\\"", "\"");
    match serde_json::from_str::<serde_json::Value>(&trimmed) {
        Ok(v) => format!("capabilities: {}", serde_json::to_string_pretty(&v).unwrap_or(trimmed)),
        Err(_) => format!("capabilities(raw): {raw}"),
    }
}
