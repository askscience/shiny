//! In-app frame benchmark for the Qt shell.
//!
//! The page-side instrumentation is the same as the GTK shell's (`js/bench.js`,
//! temporarily copied from `crates/peakd/js/`) and the report format is
//! identical, so kiosk numbers are comparable across the two shells.
//!
//! Scheduling lives on the shell's 100 ms pump instead of a tao event loop:
//! `tick` advances a small state machine (settle → inject → measure →
//! collect), and window moves arrive through the shim's `window`/`move` view
//! event, which is the drag signal the report labels "moving".

use std::io::Write;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::config::Benchmark;

/// The page-side instrumentation, compiled in so there is no path to get wrong
/// at runtime.
pub const BENCH_JS: &str = include_str!("../js/bench.js");

/// Synthetic drag of a floating plugin window, used by `--benchmark-drag-tile`.
pub const DRAG_TILE_JS: &str = include_str!("../js/bench-drag-tile.js");

/// What the pump should do this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Inject the instrumentation (and the tile drag when configured).
    Inject,
    /// Read the report back.
    Collect,
}

#[derive(Clone, Copy)]
enum Phase {
    Waiting { until: Instant },
    Measuring { started: Instant, collect_at: Instant },
    Collecting,
    Done,
}

pub struct BenchState {
    config: Benchmark,
    drag_tile: bool,
    phase: Phase,
    moves: u64,
    first_move_ms: Option<u64>,
}

impl BenchState {
    pub fn new(config: Benchmark, drag_tile: bool) -> Self {
        let until = Instant::now() + Duration::from_secs(config.settle_seconds);
        Self {
            config,
            drag_tile,
            phase: Phase::Waiting { until },
            moves: 0,
            first_move_ms: None,
        }
    }

    #[must_use]
    pub fn drag_tile(&self) -> bool {
        self.drag_tile
    }

    /// Advance the schedule; `None` means "nothing to do this tick".
    pub fn tick(&mut self) -> Option<Step> {
        match self.phase {
            Phase::Waiting { until } if Instant::now() >= until => {
                let started = Instant::now();
                self.phase = Phase::Measuring {
                    started,
                    // Margin so the page has closed its measurement window
                    // before the report is read; reading early truncates it.
                    collect_at: started + Duration::from_millis(self.config.seconds * 1000 + 1200),
                };
                Some(Step::Inject)
            }
            Phase::Measuring { collect_at, .. } if Instant::now() >= collect_at => {
                self.phase = Phase::Collecting;
                Some(Step::Collect)
            }
            _ => None,
        }
    }

    /// Record a window move. The first one, relative to the measurement start,
    /// is what makes the report's "moving" case honest.
    pub fn observe_window_move(&mut self) {
        if let Phase::Measuring { started, .. } = self.phase {
            self.moves += 1;
            if self.first_move_ms.is_none() {
                self.first_move_ms = Some(started.elapsed().as_millis() as u64);
            }
        }
    }

    /// Parse, annotate, print and optionally write the report.
    pub fn finish(&mut self, raw: &str) {
        self.phase = Phase::Done;
        finish(raw, self.moves, self.first_move_ms, self.config.output.as_deref());
    }
}

pub fn announce(config: &Benchmark) {
    eprintln!(
        "benchmark: settling {}s, then measuring {}s",
        config.settle_seconds, config.seconds
    );
    eprintln!("benchmark: drag the window while it measures to get the moving case");
}

/// Turns the raw counters into the one thing a reader wants: which mechanism
/// stalled, if any.
///
/// The comparison is between two independent clocks. `requestAnimationFrame`
/// and `setInterval` are throttled by *different* rules, so:
///
/// * rAF stalls, timers keep ticking  -> the page is being *frame* throttled.
/// * rAF and timers both stall        -> the main thread is starved.
/// * neither stalls, yet it feels slow -> the cost is outside the page
///   entirely: the compositor not taking frames.
fn diagnose(obj: &serde_json::Map<String, Value>) -> Value {
    let fps = obj.get("fps").and_then(Value::as_f64).unwrap_or(0.0);
    let seconds = obj
        .get("durationMs")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        / 1000.0;
    let timer_ticks = obj.get("timerTicks").and_then(Value::as_u64).unwrap_or(0);
    let timer_worst = obj
        .get("timerWorstMs")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let worst = obj.get("worstFrameMs").and_then(Value::as_f64).unwrap_or(0.0);
    let stalls = obj
        .get("stalls")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0);
    let hidden = obj
        .get("hiddenAtStart")
        .and_then(Value::as_bool)
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
            "the page reported document.hidden at the start: the engine throttles \
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
             still felt slow, the cost is outside the page — the compositor \
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
    let mut value: Value = serde_json::from_str(&json).unwrap_or_else(|_| {
        serde_json::json!({ "error": "could not parse the report", "raw": json })
    });

    if let Some(obj) = value.as_object_mut() {
        obj.insert("windowMoves".into(), serde_json::json!(moves));
        if let Some(at) = first_move_ms {
            obj.insert("firstMoveAtMs".into(), serde_json::json!(at));
        }
        // State the verdict in the report so it cannot be quietly misread: a
        // run with no window movement is the idle case and nothing else.
        let fps = obj.get("fps").and_then(Value::as_f64).unwrap_or(0.0);
        let worst = obj.get("worstFrameMs").and_then(Value::as_f64).unwrap_or(0.0);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnose_json(raw: &str) -> Value {
        let value: Value = serde_json::from_str(raw).unwrap();
        diagnose(value.as_object().unwrap())
    }

    #[test]
    fn healthy_page_is_not_stalled() {
        let verdict = diagnose_json(
            r#"{"fps":30,"durationMs":10000,"timerTicks":100,"timerWorstMs":120,"worstFrameMs":20,"stalls":[]}"#,
        );
        assert_eq!(verdict["verdict"], "no-page-side-stall");
    }

    #[test]
    fn hidden_page_is_called_out() {
        let verdict = diagnose_json(r#"{"hiddenAtStart":true,"fps":0}"#);
        assert_eq!(verdict["verdict"], "page-hidden");
    }

    #[test]
    fn stalled_timer_means_starved_main_thread() {
        let verdict = diagnose_json(
            r#"{"fps":5,"durationMs":10000,"timerTicks":10,"timerWorstMs":900,"worstFrameMs":80,"stalls":[]}"#,
        );
        assert_eq!(verdict["verdict"], "main-thread-starved");
    }

    #[test]
    fn window_moves_mark_the_moving_case() {
        let mut state = BenchState::new(
            Benchmark {
                seconds: 1,
                settle_seconds: 0,
                output: None,
            },
            false,
        );
        // Settle is zero, so the first tick starts measuring.
        assert_eq!(state.tick(), Some(Step::Inject));
        state.observe_window_move();
        state.observe_window_move();
        assert_eq!(state.moves, 2);
        assert!(state.first_move_ms.is_some());
        // Still inside the measurement window: nothing to collect yet.
        assert_eq!(state.tick(), None);
    }
}
