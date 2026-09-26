//! In-app frame benchmark, driven by the browser's own event loop.
//!
//! Enabled only by `--benchmark`; a normal launch creates no state for it.
//!
//! # Why it lives in the app rather than in a separate measurement window
//!
//! Every out-of-process attempt (synthetic CGEvents, Accessibility moves, an
//! AppleScript bridge, a second `wry` window) failed for the same reason: a
//! WKWebView that macOS judges not to be visible has `requestAnimationFrame`
//! throttled to a near stop. A probe window that is created but never brought
//! to the front reports `document.visibilityState === "hidden"` and produced
//! **0 frames in 5 seconds** while `setInterval` still fired. Measuring inside
//! the window the user is actually looking at removes the problem entirely.
//!
//! # Why `Moved` events are the reliable drag signal
//!
//! The app cannot detect a window drag from JavaScript, and a synthetic mouse
//! drag needs the pointer over the title bar — fragile, and impossible to make
//! repeatable. But tao already reports `WindowEvent::Moved` for every frame of
//! a real drag, so the benchmark can simply *be told* when the window moved.
//! That turns "dragging is slow" into a labelled measurement from one run, with
//! no input simulation at all.

use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use wry::WebView;

use crate::config::Benchmark;

/// The page-side instrumentation, compiled in so there is no path to get wrong
/// at runtime.
const BENCH_JS: &str = include_str!("../js/bench.js");

/// Synthetic drag of a floating plugin window, used by `--benchmark-drag-tile`.
const DRAG_TILE_JS: &str = include_str!("../js/bench-drag-tile.js");

/// Events the benchmark schedules onto the event loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BenchStep {
    /// Inject the instrumentation and open the measurement window.
    Inject,
    /// Read the report back.
    Collect,
    /// Leave.
    Done,
    /// Leave the kiosk, asked from the page (Cmd/Alt+Q → IPC). It lives on
    /// this enum because a tao loop has exactly one user-event type and the
    /// benchmark already defines it; a benchmark run treats it like `Done` so
    /// the shortcut stays honest in every mode.
    Exit,
    /// A new interface scale was written by Settings. Per-cent, 0 = auto; the
    /// shell applies it to the webview. A benchmark run ignores it (it measures
    /// the app at whatever scale it launched with).
    SetScale(u16),
    /// A native-browser-view command or event is waiting to be drained. The
    /// payload lives in the shared queue owned by `browse`; this variant only
    /// wakes the loop, which keeps the enum `Copy`. A benchmark run ignores it.
    View,
}

/// What was observed while measuring, shared with the webview callback.
#[derive(Default)]
struct Observations {
    /// Number of `Moved` events seen. Non-zero means the moving case was
    /// genuinely exercised, which the report states rather than implies.
    moves: u64,
    /// When the first move happened, relative to the start of measurement.
    first_move_ms: Option<u64>,
}

pub struct BenchState {
    config: Benchmark,
    /// Drag a plugin window for the duration, instead of relying on the user to
    /// move the OS window.
    drag_tile: bool,
    started: Option<Instant>,
    observed: Arc<Mutex<Observations>>,
}

impl BenchState {
    pub fn new(config: Benchmark, drag_tile: bool) -> Self {
        Self {
            config,
            drag_tile,
            started: None,
            observed: Arc::new(Mutex::new(Observations::default())),
        }
    }

    /// Record window movement so the report can label the case.
    pub fn observe_window_event(&mut self, event: &WindowEvent) {
        if matches!(event, WindowEvent::Moved(_)) {
            let at = self.started.map(|s| s.elapsed().as_millis() as u64);
            let mut obs = self.observed.lock();
            obs.moves += 1;
            if obs.first_move_ms.is_none() {
                obs.first_move_ms = at;
            }
        }
    }
}

/// Builds the event loop. Always typed with [`BenchStep`] so one loop can serve
/// a normal session and a benchmark; in a normal session the variant is simply
/// never sent.
pub fn event_loop() -> tao::event_loop::EventLoop<BenchStep> {
    EventLoopBuilder::<BenchStep>::with_user_event().build()
}

pub fn announce(config: &Benchmark) {
    eprintln!(
        "benchmark: settling {}s, then measuring {}s",
        config.settle_seconds, config.seconds
    );
    eprintln!("benchmark: drag the window while it measures to get the moving case");
}

/// Drives the benchmark to completion and exits the process.
///
/// Never returns: the benchmark owns the process once started, because falling
/// through into a normal session would leave a window behind and make the run
/// unrepeatable.
pub fn run(
    webview: WebView,
    mut state: BenchState,
    proxy: EventLoopProxy<BenchStep>,
    event_loop: tao::event_loop::EventLoop<BenchStep>,
) -> ! {
    {
        let proxy = proxy.clone();
        let settle = state.config.settle_seconds;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(settle));
            let _ = proxy.send_event(BenchStep::Inject);
        });
    }

    let output = state.config.output.clone();

    event_loop.run(move |event, target: &EventLoopWindowTarget<BenchStep>, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent { ref event, .. } => {
                if let WindowEvent::CloseRequested = event {
                    *control_flow = ControlFlow::Exit;
                    return;
                }
                state.observe_window_event(event);
            }

            Event::UserEvent(BenchStep::Inject) => {
                if let Err(err) = webview.evaluate_script(BENCH_JS) {
                    eprintln!("benchmark: could not instrument the page: {err}");
                    *control_flow = ControlFlow::Exit;
                    return;
                }
                state.started = Some(Instant::now());
                if state.drag_tile {
                    // Started after the frame counter so both run together.
                    match webview.evaluate_script(DRAG_TILE_JS) {
                        Ok(()) => eprintln!("benchmark: dragging a plugin window"),
                        Err(err) => eprintln!("benchmark: tile drag failed: {err}"),
                    }
                }
                eprintln!("benchmark: measuring...");

                let proxy = proxy.clone();
                let seconds = state.config.seconds;
                std::thread::spawn(move || {
                    // Margin so the page has closed its measurement window
                    // before the report is read; reading early truncates it.
                    std::thread::sleep(Duration::from_millis(seconds * 1000 + 1200));
                    let _ = proxy.send_event(BenchStep::Collect);
                });
            }

            Event::UserEvent(BenchStep::Collect) => {
                if state.drag_tile {
                    let _ = webview.evaluate_script(
                        "window.__peakdTileDragStop && window.__peakdTileDragStop()",
                    );
                }
                let proxy = proxy.clone();
                let observed = state.observed.clone();
                let output = output.clone();
                if let Err(err) = webview.evaluate_script_with_callback(
                    "JSON.stringify(window.__peakdBenchReport ? window.__peakdBenchReport() : null)",
                    move |raw| {
                        let obs = observed.lock();
                        finish(&raw, obs.moves, obs.first_move_ms, output.as_deref());
                        drop(obs);
                        let _ = proxy.send_event(BenchStep::Done);
                    },
                ) {
                    eprintln!("benchmark: could not read the report: {err}");
                    *control_flow = ControlFlow::Exit;
                }
            }

            Event::UserEvent(BenchStep::Done | BenchStep::Exit) => {
                *control_flow = ControlFlow::Exit
            }

            Event::LoopDestroyed => {
                let _ = target;
            }
            _ => {}
        }
    })
}

/// Turns the raw counters into the one thing a reader wants: which mechanism
/// stalled, if any.
///
/// The comparison is between two independent clocks. `requestAnimationFrame`
/// and `setInterval` are throttled by *different* rules, so:
///
/// * rAF stalls, timers keep ticking  -> the page is being *frame* throttled.
///   WebKit's visibility/occlusion heuristics, or a compositor that is not
///   taking frames. The main thread is fine.
/// * rAF and timers both stall        -> the main thread is starved.
/// * neither stalls, yet it feels slow -> the cost is outside the page entirely:
///   the window server recompositing a moving window. Nothing in JavaScript can
///   observe that, which is precisely why it must be inferred from the absence
///   of the other two.
fn diagnose(obj: &serde_json::Map<String, serde_json::Value>) -> serde_json::Value {
    let fps = obj.get("fps").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let seconds = obj
        .get("durationMs")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
        / 1000.0;
    let timer_ticks = obj.get("timerTicks").and_then(|v| v.as_u64()).unwrap_or(0);
    let timer_worst = obj
        .get("timerWorstMs")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let worst = obj.get("worstFrameMs").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let stalls = obj
        .get("stalls")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let hidden = obj
        .get("hiddenAtStart")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // The canary fires every 100 ms, so this is the share of expected ticks.
    let expected_ticks = (seconds * 10.0).max(1.0);
    let timer_share = (timer_ticks as f64 / expected_ticks).min(1.0);

    let starved_detail = format!(
        "the timer canary stalled too (worst gap {timer_worst:.0} ms, {:.0}% of expected \
         ticks), so the main thread itself was blocked rather than frames being \
         withheld. Look at longTasks and stalls for the cause",
        timer_share * 100.0
    );

    let (verdict, detail) = if hidden {
        (
            "page-hidden",
            "the page reported document.hidden at the start: WebKit throttles \
             requestAnimationFrame in a hidden or occluded page, so nothing here \
             describes a visible window",
        )
    } else if timer_share < 0.85 || timer_worst > 250.0 {
        ("main-thread-starved", starved_detail.as_str())
    } else if fps > 0.0 && worst <= 40.0 && stalls == 0 {
        (
            "no-page-side-stall",
            "frames were produced on schedule and no stall exceeded 50 ms, so the \
             page and its main thread were healthy for this window. If dragging \
             still felt slow, the cost is outside the page — the window server \
             recompositing a moving window — which JavaScript cannot observe",
        )
    } else if stalls > 0 {
        (
            "frames-withheld",
            "requestAnimationFrame was withheld while the timer canary kept running: \
             frame throttling, not main-thread starvation",
        )
    } else {
        (
            "inconclusive",
            "no single mechanism dominates; read worstFrameMs and stalls directly",
        )
    };

    // Bind first: a leading unary `+` is a JS habit, not Rust, and is not valid
    // where a JSON macro expects an expression.
    let timer_share_pct = (timer_share * 100.0).round() / 100.0;
    serde_json::json!({
        "verdict": verdict,
        "detail": detail,
        "timerShare": timer_share_pct,
    })
}

/// Normalises, annotates, prints and optionally writes the report.
fn finish(raw: &str, moves: u64, first_move_ms: Option<u64>, output: Option<&str>) {
    let json = raw.trim().trim_matches('"').replace("\\\"", "\"");
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap_or_else(|_| {
        serde_json::json!({ "error": "could not parse the report", "raw": json })
    });

    if let Some(obj) = value.as_object_mut() {
        obj.insert("windowMoves".into(), serde_json::json!(moves));
        if let Some(at) = first_move_ms {
            obj.insert("firstMoveAtMs".into(), serde_json::json!(at));
        }
        // State the verdict in the report so it cannot be quietly misread: a
        // run with no window movement is the idle case and nothing else.
        let fps = obj.get("fps").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let worst = obj.get("worstFrameMs").and_then(|v| v.as_f64()).unwrap_or(0.0);
        obj.insert(
            "verdict".into(),
            serde_json::json!({
                "fps": fps,
                "worstFrameMs": worst,
                "case": if moves > 0 { "moving" } else { "idle" },
                "note": if moves > 0 {
                    "the window moved during the run, so this includes the moving case"
                } else {
                    "the window never moved: this is the idle case only"
                },
            }),
        );
    }

    if let Some(obj) = value.as_object_mut() {
        obj.insert("diagnosis".into(), diagnose(obj));
    }

    let pretty = serde_json::to_string_pretty(&value).unwrap_or(json);
    println!("{pretty}");
    // Flush before exiting: with stdout piped, an unflushed buffer is lost.
    let _ = std::io::stdout().flush();

    if let Some(path) = output {
        match std::fs::write(path, format!("{pretty}\n")) {
            Ok(()) => eprintln!("benchmark: report written to {path}"),
            Err(err) => eprintln!("benchmark: could not write {path}: {err}"),
        }
    }
}
