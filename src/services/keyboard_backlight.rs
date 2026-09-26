//! Host keyboard backlight — the Mac's `kbd_backlight` LED.
//!
//! The keyboard backlight is a sysfs LED (`/sys/class/leds/*kbd_backlight/`),
//! not a GTK/X thing: this reads `brightness`/`max_brightness` and writes
//! `brightness`. No GTK, no desktop daemon — which is exactly why it works in
//! the matchbox kiosk, where nothing would handle the `IllumUp`/`IllumDown`
//! keys.
//!
//! The attribute is root-owned by default, so the server (which runs as the
//! desktop user, never root) can only write it after a udev rule makes it
//! group-writable by `video`. `scripts/touchbar/install-touchbar.sh` installs
//! that rule; `status()` reports `writable` so the UI can say so instead of
//! failing silently.

use std::path::PathBuf;

use serde_json::Value;

use super::backlight::SysfsBacklight;

#[derive(Clone)]
pub struct KeyboardBacklightService {
    inner: SysfsBacklight,
}

impl KeyboardBacklightService {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: SysfsBacklight::new(find_led(), "keyboard backlight"),
        }
    }

    /// `{ available, writable, brightness, max_brightness, percent }`.
    #[must_use]
    pub fn status(&self) -> Value {
        self.inner.status()
    }

    /// Nudge the backlight by a percentage (negative dims).
    pub fn adjust(&self, delta: i64) -> Result<Value, String> {
        self.inner.adjust(delta)
    }

    /// Set the backlight to an absolute percentage (0–100).
    pub fn set_percent(&self, percent: u32) -> Result<Value, String> {
        self.inner.set_percent(percent)
    }
}

impl Default for KeyboardBacklightService {
    fn default() -> Self {
        Self::new()
    }
}

/// The `*kbd_backlight` LED directory, preferring the white backlight.
fn find_led() -> Option<PathBuf> {
    let mut fallback: Option<PathBuf> = None;
    for entry in std::fs::read_dir("/sys/class/leds").ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.contains("kbd_backlight") {
            continue;
        }
        let path = entry.path();
        if name.contains("white") {
            return Some(path);
        }
        fallback.get_or_insert(path);
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::KeyboardBacklightService;

    #[test]
    fn status_is_always_shaped() {
        let status = KeyboardBacklightService::new().status();
        assert!(status
            .get("available")
            .and_then(serde_json::Value::as_bool)
            .is_some());
    }
}
