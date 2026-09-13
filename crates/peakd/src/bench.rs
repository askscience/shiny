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

/// Events the benchmark schedules onto the event loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BenchStep {
    /// Inject the instrumentation and open the measurement window.
    Inject,
    /// Read the report back.
    Collect,
    /// Leave.
    Done,
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
    started: Option<Instant>,
    observed: Arc<Mutex<Observations>>,
}

impl BenchState {
    pub fn new(config: Benchmark) -> Self {
        Self {
            config,
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

            Event::UserEvent(BenchStep::Done) => *control_flow = ControlFlow::Exit,

            Event::LoopDestroyed => {
                let _ = target;
            }
            _ => {}
        }
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
