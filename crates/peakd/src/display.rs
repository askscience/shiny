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
//! The Qt shell forces its own scale factor to 1 and drives everything through
//! this page zoom, matching the GTK shell's behaviour; the choice file is
//! watched on the shell's 100 ms pump (no watcher thread).

use std::path::PathBuf;
use std::time::SystemTime;

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

/// The choice file's mtime, for the pump's change detection.
#[must_use]
pub fn modified(path: &PathBuf) -> Option<SystemTime> {
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
}
