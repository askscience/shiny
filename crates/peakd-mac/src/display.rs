//! Interface scale for the macOS shell.
//!
//! The choice file, `auto` DPI snapping and the runtime report live in the
//! shared `peakd-core` crate (the Linux shell uses the same code). This shell
//! watches the choice file and applies the resolved factor to its webview.

use std::path::PathBuf;
use std::time::Duration;

use tao::event_loop::EventLoopProxy;

use crate::bench::BenchStep;

pub use peakd_core::display::{auto_scale, modified, read_choice, write_runtime, Choice};

/// Not Linux: the platform webview handles its own display density, so there
/// is nothing to derive — an explicit choice still applies. (The Linux kiosk
/// runs `peakd`, where Qt reports the screen DPI.)
#[must_use]
pub fn panel_info() -> Option<(f64, i32, i32)> {
    None
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
