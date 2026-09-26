//! Interface scale for the kiosk shell.
//!
//! Settings (the app) writes the user's choice to `~/.config/peakd/display.json`;
//! this shell reads it and applies it to the webview as **page zoom**. Page
//! zoom — not a device-scale hack — is the right mechanism here because it
//! *re-lays-out* the page: a scaled-up UI genuinely draws fewer logical pixels,
//! which is where the cost was. On this 226-DPI panel at 100% the terminal
//! alone was a 377×114 grid and text shaping dominated the renderer profile;
//! at 200% it is a quarter of the cells.
//!
//! `auto` follows the panel DPI, snapped to the steps GNOME uses, capped at
//! 200% (a Retina laptop panel's natural scale). What was applied is written to
//! `~/.cache/peakd/display.json` so Settings can show the resolved value.
//!
//! The choice file is watched (mtime poll) so a change in Settings takes effect
//! without restarting the kiosk.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use tao::event_loop::EventLoopProxy;

use crate::bench::BenchStep;

/// Factors `auto` may pick.
const AUTO_STEPS: [f64; 5] = [1.0, 1.25, 1.5, 1.75, 2.0];

/// The stored choice: follow the panel, or an explicit factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Choice {
    Auto,
    Factor(f64),
}

impl Choice {
    fn from_json(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Number(n) => n
                .as_f64()
                .filter(|f| f.is_finite())
                .map(Choice::Factor)
                .unwrap_or(Choice::Auto),
            serde_json::Value::String(s) => s
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|f| f.is_finite())
                .map(Choice::Factor)
                .unwrap_or(Choice::Auto),
            _ => Choice::Auto,
        }
    }

    /// The wire form shared with the event loop: `0` means auto, otherwise
    /// scale×100. An integer keeps `BenchStep` `Eq`/`Copy`.
    #[must_use]
    pub fn as_percent(self) -> u16 {
        match self {
            Choice::Auto => 0,
            Choice::Factor(factor) => (factor * 100.0).round().clamp(100.0, 300.0) as u16,
        }
    }

    #[must_use]
    pub fn from_percent(percent: u16) -> Self {
        if percent == 0 {
            Choice::Auto
        } else {
            Choice::Factor(f64::from(percent) / 100.0)
        }
    }

    /// The factor to apply, given the panel-derived auto factor.
    #[must_use]
    pub fn resolve(self, auto: f64) -> f64 {
        match self {
            Choice::Auto => auto,
            Choice::Factor(factor) => factor,
        }
    }
}

/// Read the stored choice. A missing or corrupt file means auto, not an error:
/// the kiosk must always come up.
#[must_use]
pub fn read_choice(path: &PathBuf) -> Choice {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Choice::Auto;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Choice::Auto;
    };
    value.get("scale").map(Choice::from_json).unwrap_or(Choice::Auto)
}

/// The scale `auto` picks for a panel of `dpi` (96 DPI = 100%).
#[must_use]
pub fn auto_scale(dpi: f64) -> f64 {
    if !dpi.is_finite() || dpi <= 0.0 {
        return 1.0;
    }
    let target = dpi / 96.0;
    AUTO_STEPS
        .iter()
        .copied()
        .min_by(|a, b| {
            (a - target)
                .abs()
                .partial_cmp(&(b - target).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(1.0)
}

/// Not Linux: the platform webview handles its own display density, so there
/// is nothing to derive — an explicit choice still applies. (The Linux kiosk
/// runs `peakd`, where Qt reports the screen DPI.)
#[must_use]
pub fn panel_info() -> Option<(f64, i32, i32)> {
    None
}

/// Record what was applied so Settings can show the resolved value.
pub fn write_runtime(path: &PathBuf, scale: f64, info: Option<(f64, i32, i32)>) {
    let mut obj = serde_json::Map::new();
    obj.insert("scale".into(), serde_json::json!(scale));
    if let Some((dpi, width, height)) = info {
        obj.insert("dpi".into(), serde_json::json!(dpi));
        obj.insert("width".into(), serde_json::json!(width));
        obj.insert("height".into(), serde_json::json!(height));
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(body) = serde_json::to_string_pretty(&serde_json::Value::Object(obj)) {
        let _ = std::fs::write(path, body);
    }
}

/// Watch the choice file and push changes onto the event loop. Never blocks the
/// caller; the thread exits when the loop is gone.
pub fn watch(path: PathBuf, proxy: EventLoopProxy<BenchStep>) {
    let _ = std::thread::Builder::new()
        .name("peakd-display-watch".into())
        .spawn(move || {
            let mut last = modified(&path);
            loop {
                std::thread::sleep(Duration::from_millis(500));
                let current = modified(&path);
                if current == last {
                    continue;
                }
                last = current;
                let percent = read_choice(&path).as_percent();
                if proxy.send_event(BenchStep::SetScale(percent)).is_err() {
                    break;
                }
            }
        });
}

fn modified(path: &PathBuf) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::{auto_scale, Choice};

    #[test]
    fn auto_snaps_to_gnome_steps_and_caps_at_two() {
        assert_eq!(auto_scale(96.0), 1.0);
        assert_eq!(auto_scale(120.0), 1.25);
        assert_eq!(auto_scale(144.0), 1.5);
        // A 226-DPI laptop panel lands on 200%, like GNOME.
        assert_eq!(auto_scale(226.0), 2.0);
        // Absurd values never leave the step list.
        assert_eq!(auto_scale(0.0), 1.0);
        assert_eq!(auto_scale(f64::NAN), 1.0);
        assert_eq!(auto_scale(1000.0), 2.0);
    }

    #[test]
    fn percent_round_trip() {
        assert_eq!(Choice::Auto.as_percent(), 0);
        assert_eq!(Choice::from_percent(0), Choice::Auto);
        assert_eq!(Choice::Factor(1.5).as_percent(), 150);
        assert_eq!(Choice::from_percent(150), Choice::Factor(1.5));
    }
}
