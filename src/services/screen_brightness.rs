//! Host screen brightness — the panel backlight.
//!
//! The panel brightness is a sysfs backlight (`/sys/class/backlight/`), not a
//! GTK/X thing: this reads `brightness`/`max_brightness` and writes
//! `brightness`. On a T2 Mac that is `gmux_backlight`; on other machines it may
//! be `intel_backlight` or `acpi_video0`. The Touch Bar's own
//! `appletb_backlight` is deliberately excluded — it is a two-step backlight
//! for the bar, not the screen.
//!
//! Like the keyboard backlight, the attribute is root-owned by default and only
//! becomes writable after `scripts/touchbar/install-touchbar.sh` installs the
//! udev rule that hands it to the `video` group.

use std::path::PathBuf;

use serde_json::Value;

use super::backlight::SysfsBacklight;

#[derive(Clone)]
pub struct ScreenBrightnessService {
    inner: SysfsBacklight,
}

impl ScreenBrightnessService {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: SysfsBacklight::new(find_panel(), "screen brightness"),
        }
    }

    /// `{ available, writable, brightness, max_brightness, percent }`.
    #[must_use]
    pub fn status(&self) -> Value {
        self.inner.status()
    }

    /// Nudge the brightness by a percentage (negative dims).
    pub fn adjust(&self, delta: i64) -> Result<Value, String> {
        self.inner.adjust(delta)
    }

    /// Set the brightness to an absolute percentage (0–100).
    pub fn set_percent(&self, percent: u32) -> Result<Value, String> {
        self.inner.set_percent(percent)
    }
}

impl Default for ScreenBrightnessService {
    fn default() -> Self {
        Self::new()
    }
}

/// The panel backlight, by the names the usual drivers use. `appletb_backlight`
/// (the Touch Bar) is never a candidate.
fn find_panel() -> Option<PathBuf> {
    const PANEL: &[&str] = &[
        "gmux_backlight",
        "apple-panel-bl",
        "intel_backlight",
        "acpi_video0",
    ];
    let mut fallback: Option<PathBuf> = None;
    for entry in std::fs::read_dir("/sys/class/backlight").ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.contains("appletb") {
            continue;
        }
        let path = entry.path();
        if PANEL.iter().any(|panel| name.contains(panel)) {
            return Some(path);
        }
        fallback.get_or_insert(path);
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::ScreenBrightnessService;

    #[test]
    fn status_is_always_shaped() {
        let status = ScreenBrightnessService::new().status();
        assert!(status
            .get("available")
            .and_then(serde_json::Value::as_bool)
            .is_some());
    }
}
