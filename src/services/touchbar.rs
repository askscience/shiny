//! Host Touch Bar capability.
//!
//! Whether this machine has a T2 Touch Bar that `tiny-dfr` is driving. The web
//! layer's `auto` mode follows this (`web/js/touchbar.js`); on any other
//! machine it reports unavailable and the Touch Bar handling stays dormant, so
//! a normal PC never reacts to F13–F21.
//!
//! Like the audio and network panels this is a *host* capability, not a
//! per-user preference: it belongs to the machine, and it is the same sysfs
//! check `crates/peakd/src/touchbar.rs` makes for the kiosk.

use serde_json::{json, Value};

#[derive(Clone)]
pub struct TouchBarService {
    available: bool,
}

impl TouchBarService {
    #[must_use]
    pub fn new() -> Self {
        Self {
            available: detect(),
        }
    }

    /// `{ "available": bool }` — read by the web UI at boot.
    #[must_use]
    pub fn status(&self) -> Value {
        json!({ "available": self.available })
    }
}

impl Default for TouchBarService {
    fn default() -> Self {
        Self::new()
    }
}

/// Look for the T2 `appletb` devices: the upstream `hid-appletb-bl` /
/// `hid-appletb-kbd` driver (kernel 6.15+) or the older `apple-ib-tb` driver.
/// `tiny-dfr` drives whichever is present.
fn detect() -> bool {
    const PATHS: &[&str] = &[
        "/sys/class/backlight/appletb_backlight",
        "/sys/class/leds/appletb_backlight",
        "/sys/module/hid_appletb_kbd",
        "/sys/module/apple_ib_tb",
    ];
    PATHS
        .iter()
        .any(|path| std::path::Path::new(path).exists())
}

#[cfg(test)]
mod tests {
    use super::{detect, TouchBarService};
    use serde_json::Value;

    #[test]
    fn detection_is_quiet_and_status_is_boolean() {
        let _ = detect(); // must not panic on any machine
        let status = TouchBarService::new().status();
        assert!(status.get("available").and_then(Value::as_bool).is_some());
    }
}
