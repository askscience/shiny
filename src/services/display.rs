//! Host display scale — the interface scale the kiosk shell (`peakd`) applies
//! to the webview as page zoom.
//!
//! This is deliberately a *host* setting, not a per-user preference: the scale
//! belongs to the panel, and the shell has to know it before anyone signs in
//! (the login screen is rendered at the same scale). It is also the only way
//! the shell and the app can agree — `peakd` cannot read the database, and the
//! server cannot call into WebKit.
//!
//! Two small JSON files are the bridge:
//!
//! * `~/.config/peakd/display.json` — the **choice** (`"auto"` or a factor),
//!   written here when Settings changes it and read by `peakd`.
//! * `~/.cache/peakd/display.json` — the **resolved** value `peakd` actually
//!   applied (plus the panel DPI it used), so Settings can show "Auto (~200%)"
//!   instead of guessing.
//!
//! Paths are overridable with `PEAKD_DISPLAY_FILE` / `PEAKD_DISPLAY_RUNTIME_FILE`
//! so a bundled install can keep the shell and the server pointed at the same
//! place; both sides resolve them identically.

use std::path::PathBuf;

use serde_json::{json, Value};

/// Range the API accepts. The UI offers 100–250% today; a little headroom
/// keeps an explicit value from being rejected when a future panel wants it.
pub const MIN_SCALE: f64 = 1.0;
pub const MAX_SCALE: f64 = 3.0;

#[derive(Clone)]
pub struct DisplayService {
    config_path: PathBuf,
    runtime_path: PathBuf,
}

impl DisplayService {
    #[must_use]
    pub fn new() -> Self {
        Self {
            config_path: config_file(),
            runtime_path: runtime_file(),
        }
    }

    /// The stored choice plus what the shell last applied. `resolved`/`dpi`
    /// are `null` until the shell has run once.
    #[must_use]
    pub fn status(&self) -> Value {
        let scale = read_choice(&self.config_path);
        let runtime = read_json(&self.runtime_path);
        json!({
            "scale": scale,
            "resolved": runtime.get("scale").cloned().unwrap_or(Value::Null),
            "dpi": runtime.get("dpi").cloned().unwrap_or(Value::Null),
            "width": runtime.get("width").cloned().unwrap_or(Value::Null),
            "height": runtime.get("height").cloned().unwrap_or(Value::Null),
        })
    }

    /// Validate and persist a choice (`"auto"`, `null`, or a factor).
    pub fn set_scale(&self, value: &Value) -> Result<Value, String> {
        let scale = normalize(value)?;
        write_json(&self.config_path, &json!({ "scale": scale }))?;
        Ok(self.status())
    }
}

impl Default for DisplayService {
    fn default() -> Self {
        Self::new()
    }
}

/// Coerce a request value to the stored form: the string `"auto"` or a finite
/// factor in range. A bad value is refused rather than silently clamped — a
/// typo'd scale that does nothing is worse than an error the UI can show.
fn normalize(value: &Value) -> Result<Value, String> {
    match value {
        Value::Null => Ok(Value::String("auto".into())),
        Value::String(s) if s.trim().is_empty() || s.eq_ignore_ascii_case("auto") => {
            Ok(Value::String("auto".into()))
        }
        Value::String(s) => {
            let n: f64 = s
                .trim()
                .parse()
                .map_err(|_| format!("invalid scale: {s}"))?;
            normalize_number(n)
        }
        Value::Number(n) => {
            let n = n
                .as_f64()
                .ok_or_else(|| "invalid scale".to_string())?;
            normalize_number(n)
        }
        other => Err(format!("invalid scale: {other}")),
    }
}

fn normalize_number(n: f64) -> Result<Value, String> {
    if !n.is_finite() || !(MIN_SCALE..=MAX_SCALE).contains(&n) {
        return Err(format!("scale must be between {MIN_SCALE} and {MAX_SCALE}"));
    }
    // Two decimals is the finest step the UI offers; keeps the file tidy.
    let rounded = (n * 100.0).round() / 100.0;
    Ok(json!(rounded))
}

/// The stored choice, tolerating a missing/corrupt file (→ `"auto"`).
fn read_choice(path: &PathBuf) -> Value {
    read_json(path)
        .get("scale")
        .cloned()
        .filter(|v| v.is_string() || v.is_number())
        .and_then(|v| normalize(&v).ok())
        .unwrap_or_else(|| Value::String("auto".into()))
}

fn read_json(path: &PathBuf) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .unwrap_or(Value::Null)
}

fn write_json(path: &PathBuf, value: &Value) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    // Write-then-rename: the shell watches this file's mtime, and a reader must
    // never see a half-written document.
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into());
    std::fs::write(&tmp, body).map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("could not replace {}: {e}", path.display()))
}

fn config_file() -> PathBuf {
    if let Some(p) = std::env::var_os("PEAKD_DISPLAY_FILE") {
        return PathBuf::from(p);
    }
    base_dir("XDG_CONFIG_HOME", ".config").join("display.json")
}

fn runtime_file() -> PathBuf {
    if let Some(p) = std::env::var_os("PEAKD_DISPLAY_RUNTIME_FILE") {
        return PathBuf::from(p);
    }
    base_dir("XDG_CACHE_HOME", ".cache").join("display.json")
}

/// `<config|cache>/peakd/`.
fn base_dir(xdg: &str, fallback: &str) -> PathBuf {
    let root = std::env::var_os(xdg)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback)))
        .unwrap_or_else(|| PathBuf::from(fallback));
    root.join("peakd")
}

#[cfg(test)]
mod tests {
    use super::{normalize, MAX_SCALE, MIN_SCALE};
    use serde_json::{json, Value};

    #[test]
    fn auto_and_null_mean_auto() {
        assert_eq!(normalize(&json!("auto")).unwrap(), json!("auto"));
        assert_eq!(normalize(&json!("AUTO")).unwrap(), json!("auto"));
        assert_eq!(normalize(&Value::Null).unwrap(), json!("auto"));
        assert_eq!(normalize(&json!("")).unwrap(), json!("auto"));
    }

    #[test]
    fn factors_are_validated_and_rounded() {
        assert_eq!(normalize(&json!(1.5)).unwrap(), json!(1.5));
        assert_eq!(normalize(&json!("1.25")).unwrap(), json!(1.25));
        // Rounded to the UI's step.
        assert_eq!(normalize(&json!(1.333)).unwrap(), json!(1.33));
        assert!(normalize(&json!(MIN_SCALE - 0.1)).is_err());
        assert!(normalize(&json!(MAX_SCALE + 0.1)).is_err());
        assert!(normalize(&json!("nonsense")).is_err());
        assert!(normalize(&json!({ "nested": true })).is_err());
    }
}
