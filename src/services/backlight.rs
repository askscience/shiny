//! Shared sysfs backlight helper.
//!
//! Both the keyboard backlight (`leds/*kbd_backlight`) and the panel brightness
//! (`backlight/gmux_backlight`, `intel_backlight`, …) are sysfs attributes with
//! the same shape: `brightness`, `max_brightness`, and a permission that is
//! root-only until a udev rule grants the desktop user access. This is the
//! common read/adjust/set logic; the two services only differ in which device
//! they find and how they describe it.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

#[derive(Clone)]
pub(crate) struct SysfsBacklight {
    dir: Option<PathBuf>,
    /// Human name used in the "not writable" error, e.g. "keyboard backlight".
    label: &'static str,
}

impl SysfsBacklight {
    pub(crate) fn new(dir: Option<PathBuf>, label: &'static str) -> Self {
        Self { dir, label }
    }

    /// `{ available, writable, brightness, max_brightness, percent }`.
    pub(crate) fn status(&self) -> Value {
        let Some(dir) = &self.dir else {
            return json!({ "available": false });
        };
        let max = read_u32(&dir.join("max_brightness")).unwrap_or(0);
        let brightness = read_u32(&dir.join("brightness")).unwrap_or(0);
        json!({
            "available": true,
            "writable": is_writable(&dir.join("brightness")),
            "brightness": brightness,
            "max_brightness": max,
            "percent": percent_of(brightness, max),
        })
    }

    /// Nudge by a percentage (negative dims). A no-op when the machine has no
    /// such device; an error when it exists but is not writable.
    pub(crate) fn adjust(&self, delta: i64) -> Result<Value, String> {
        let Some(dir) = &self.dir else {
            return Ok(json!({ "available": false }));
        };
        let max = read_u32(&dir.join("max_brightness")).unwrap_or(0);
        if max == 0 {
            return Err(format!("{} reports no max_brightness", self.label));
        }
        if delta == 0 {
            return Ok(self.status());
        }
        let current = read_u32(&dir.join("brightness")).unwrap_or(0);
        let step = ((max as i64 * delta.abs() / 100).max(1)) * delta.signum();
        let next = (current as i64 + step).clamp(0, max as i64) as u32;
        self.write_value(next)?;
        Ok(self.status())
    }

    /// Set an absolute percentage (0–100).
    pub(crate) fn set_percent(&self, percent: u32) -> Result<Value, String> {
        let Some(dir) = &self.dir else {
            return Ok(json!({ "available": false }));
        };
        let max = read_u32(&dir.join("max_brightness")).unwrap_or(0);
        let value = ((percent.min(100) as u64 * max as u64 + 50) / 100) as u32;
        self.write_value(value)?;
        Ok(self.status())
    }

    fn write_value(&self, value: u32) -> Result<(), String> {
        let dir = self
            .dir
            .as_ref()
            .ok_or_else(|| format!("no {}", self.label))?;
        std::fs::write(dir.join("brightness"), format!("{value}\n")).map_err(|err| {
            format!(
                "could not change the {} ({err}); run scripts/touchbar/install-touchbar.sh to grant access",
                self.label
            )
        })
    }
}

pub(crate) fn read_u32(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Opening the attribute for writing is the definitive permission test; it does
/// not change the value.
pub(crate) fn is_writable(path: &Path) -> bool {
    std::fs::OpenOptions::new().write(true).open(path).is_ok()
}

pub(crate) fn percent_of(value: u32, max: u32) -> u32 {
    if max == 0 {
        0
    } else {
        ((value as u64 * 100 + max as u64 / 2) / max as u64) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::percent_of;

    #[test]
    fn percent_rounds_to_nearest() {
        assert_eq!(percent_of(0, 100), 0);
        assert_eq!(percent_of(50, 100), 50);
        assert_eq!(percent_of(100, 100), 100);
        assert_eq!(percent_of(0, 0), 0);
        assert_eq!(percent_of(7000, 14660), 48);
    }
}
