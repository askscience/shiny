//! Distro detection + package listing, with a short-lived in-memory cache so
//! the HUD chip does not shell out to the package manager on every poll.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::distro::{self, PackageManager};
use crate::exec;
use crate::model::{DistroInfo, UpdateStatus};
use crate::ollama;

/// How long a collected list stays fresh.
const CACHE_TTL: Duration = Duration::from_secs(300);
/// Upper bound for a listing command (Portage in particular is slow).
const LIST_TIMEOUT: Duration = Duration::from_secs(180);

struct Cache {
    at: Instant,
    status: UpdateStatus,
}

fn cache() -> &'static Mutex<Option<Cache>> {
    static CACHE: OnceLock<Mutex<Option<Cache>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn cached_fresh() -> Option<UpdateStatus> {
    let guard = cache().lock().ok()?;
    let entry = guard.as_ref()?;
    if entry.at.elapsed() < CACHE_TTL {
        Some(entry.status.clone())
    } else {
        None
    }
}

fn put(status: UpdateStatus) {
    if let Ok(mut guard) = cache().lock() {
        *guard = Some(Cache {
            at: Instant::now(),
            status,
        });
    }
}

/// Drop the cache (after a metadata refresh or an upgrade).
pub fn invalidate() {
    if let Ok(mut guard) = cache().lock() {
        *guard = None;
    }
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The detected distro, as reported to clients.
pub fn distro_info(manager: &dyn PackageManager) -> DistroInfo {
    let os = distro::read_os_release();
    DistroInfo {
        id: os.id,
        name: os.name,
        version: os.version,
        id_like: os.id_like,
        manager: manager.id().to_string(),
        manager_label: manager.label().to_string(),
        supported: manager.supported(),
    }
}

fn first_error(out: &exec::Output) -> String {
    if out.timed_out {
        return "Timed out while querying the package manager".to_string();
    }
    let line = out
        .stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("package manager returned an error");
    format!("{line} (exit code {:?})", out.code)
}

/// Guards against overlapping background refreshes.
static REFRESHING: AtomicBool = AtomicBool::new(false);

/// Non-blocking status for periodic callers (the top-bar chip).
///
/// Returns the cached snapshot immediately and schedules a refresh on a
/// dedicated thread when it is stale. The package manager is never run on the
/// shared plugin runtime, so a slow `apt`/`dnf` cannot stall other plugins.
pub fn snapshot() -> UpdateStatus {
    let cached = cache()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|c| (c.status.clone(), c.at.elapsed())));
    if let Some((mut s, age)) = cached {
        if age >= CACHE_TTL {
            s.stale = true;
            spawn_refresh();
        }
        return s;
    }
    // No cache yet: one synchronous collect so the first caller gets real data.
    collect(true)
}

/// Kick off a background refresh (deduplicated).
pub fn request_refresh() {
    spawn_refresh();
}

fn spawn_refresh() {
    if REFRESHING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let _ = collect(true);
        REFRESHING.store(false, Ordering::SeqCst);
    });
}

/// Build a status snapshot. `force` bypasses the cache.
pub fn collect(force: bool) -> UpdateStatus {
    if !force {
        if let Some(mut cached) = cached_fresh() {
            cached.stale = true;
            return cached;
        }
    }

    let manager = distro::detect();
    let info = distro_info(manager.as_ref());

    let mut error = None;
    let mut packages = Vec::new();
    if manager.supported() {
        match exec::run(&manager.list(), None, LIST_TIMEOUT) {
            Ok(out) => {
                if out.success {
                    packages = manager.parse(&out.stdout);
                } else {
                    error = Some(first_error(&out));
                }
            }
            Err(e) => error = Some(e.to_string()),
        }
    } else {
        error = Some(
            "Unsupported distribution: no known package manager was detected, so updates \
             cannot be managed automatically."
                .to_string(),
        );
    }

    let status = UpdateStatus {
        distro: info,
        count: packages.len(),
        packages,
        checked_at: now_secs(),
        stale: false,
        error,
        ollama: ollama::status(),
    };

    if status.error.is_none() {
        put(status.clone());
    }
    status
}
