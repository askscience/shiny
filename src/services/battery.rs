//! Host battery state for the top-bar chip — the charge level, whether the
//! machine is on wall power, and how long is left.
//!
//! Like `network.rs`, `audio.rs` and `bluetooth.rs`, this belongs to the
//! machine rather than a traveler: it is core chrome (a sibling of
//! `hudBattery.js`) and follows the same contract — probe, degrade quietly when
//! there is no battery, cache one snapshot, and broadcast changes.
//!
//! The backend is the kernel's **sysfs** power-supply class
//! (`/sys/class/power_supply/*`), which every Linux laptop exposes through ACPI
//! or the platform driver. That is deliberately dependency-free — no UPower,
//! no D-Bus — so it works in the bare matchbox kiosk, exactly like the sysfs
//! backlight panels. A machine without a battery (a desktop) reports
//! `available: false` and the chip hides itself.
//!
//! `/api/battery/status` reads the cache and `/api/battery/events` relays the
//! broadcast as SSE. There are no mutations: the battery is read-only.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use serde::Serialize;
use tokio::sync::broadcast;

/// The kernel's power-supply class directory. Fixed on Linux; the tests set a
/// process-wide override so they can point at a fixture instead.
#[cfg(target_os = "linux")]
const SYSFS_DIR: &str = "/sys/class/power_supply";
/// Batteries change slowly; half a minute keeps the chip honest for almost no
/// work and avoids waking idle SSE clients for nothing.
const REFRESH: Duration = Duration::from_secs(30);
const BROADCAST_CAPACITY: usize = 64;

#[cfg(target_os = "linux")]
const START_REASON: &str = "no battery was found";
#[cfg(not(target_os = "linux"))]
const START_REASON: &str = "the battery panel requires Linux (sysfs)";

/// The whole snapshot the chip renders from.
#[derive(Clone, Debug, Serialize)]
pub struct BatteryStatus {
    pub available: bool,
    pub reason: Option<String>,
    pub updated_at: String,
    /// True when at least one battery is present.
    pub present: bool,
    /// Charge percentage, 0–100, when the kernel reports one.
    pub percent: Option<u8>,
    /// The coarse 0–4 bucket the chip picks its icon from, or `None` when the
    /// percentage is unknown.
    pub level: Option<u8>,
    /// One of `charging`, `discharging`, `full`, `idle`, `unknown` — the
    /// summary the UI colours the chip by.
    pub state: String,
    /// The kernel's own word, e.g. `Discharging`, when it gave one.
    pub status: Option<String>,
    /// Whether the machine is on wall power, from the `Mains` supply.
    pub ac_online: Option<bool>,
    /// Seconds until empty, when discharging and the kernel exposes enough to
    /// estimate it.
    pub time_to_empty_secs: Option<u64>,
    /// Seconds until full, when charging and estimable.
    pub time_to_full_secs: Option<u64>,
    /// Energy currently in the pack, in Wh, when the kernel reports it.
    pub energy_now_wh: Option<f64>,
}

impl BatteryStatus {
    fn unavailable(reason: &str) -> Self {
        Self {
            available: false,
            reason: Some(reason.to_string()),
            updated_at: now(),
            present: false,
            percent: None,
            level: None,
            state: "unknown".to_string(),
            status: None,
            ac_online: None,
            time_to_empty_secs: None,
            time_to_full_secs: None,
            energy_now_wh: None,
        }
    }

    fn empty() -> Self {
        Self::unavailable(START_REASON)
    }
}

/// Map a percentage to the chip's icon bucket. The thresholds are GNOME-ish:
/// critical, low, medium, good, full.
#[must_use]
pub fn battery_level(percent: u8) -> u8 {
    match percent {
        0..=10 => 0,
        11..=30 => 1,
        31..=60 => 2,
        61..=85 => 3,
        _ => 4,
    }
}

/// Normalize a kernel `status` word (plus the AC state when the kernel is
/// vague) into the small vocabulary the UI colours by.
#[must_use]
pub fn battery_state(status: Option<&str>, ac_online: Option<bool>) -> &'static str {
    let status = status.unwrap_or("").to_ascii_lowercase();
    if status.contains("discharg") {
        "discharging"
    } else if status.contains("charg") && !status.contains("not charg") {
        "charging"
    } else if status.contains("full") {
        "full"
    } else if ac_online == Some(true) {
        // Plugged in but the pack is neither charging nor full: the firmware is
        // holding it (e.g. "Not charging"), which reads as idle on the chip.
        "idle"
    } else if ac_online == Some(false) {
        "discharging"
    } else {
        "unknown"
    }
}

#[derive(Clone)]
pub struct BatteryService {
    inner: Arc<Inner>,
}

struct Inner {
    status: ArcSwap<BatteryStatus>,
    events: broadcast::Sender<Arc<BatteryStatus>>,
    started: AtomicBool,
    /// Last published JSON, so the poll does not re-broadcast an unchanged
    /// snapshot to every SSE client.
    last_json: std::sync::Mutex<String>,
}

impl BatteryService {
    #[must_use]
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            inner: Arc::new(Inner {
                status: ArcSwap::from_pointee(BatteryStatus::empty()),
                events,
                started: AtomicBool::new(false),
                last_json: std::sync::Mutex::new(String::new()),
            }),
        }
    }

    /// Latest snapshot; never touches the filesystem.
    #[must_use]
    pub fn status(&self) -> Arc<BatteryStatus> {
        self.inner.status.load_full()
    }

    /// Subscribe to snapshots, for the SSE relay.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<BatteryStatus>> {
        self.inner.events.subscribe()
    }

    #[must_use]
    pub fn is_available(&self) -> bool {
        self.status().available
    }

    fn publish(&self, status: BatteryStatus) {
        let shared = Arc::new(status);
        self.inner.status.store(shared.clone());
        let _ = self.inner.events.send(shared);
    }

    /// Publish only when the snapshot changed; the reader polls on an interval,
    /// so this is what keeps idle SSE clients from being woken for nothing.
    fn publish_if_changed(&self, status: BatteryStatus) {
        let json = serde_json::to_string(&status).unwrap_or_default();
        {
            let mut last = self
                .inner
                .last_json
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if *last == json {
                return;
            }
            *last = json;
        }
        self.publish(status);
    }

    /// Read the kernel once and publish the result. Used at startup and by the
    /// poll loop.
    pub fn refresh(&self) {
        self.publish_if_changed(collect());
    }

    /// Spawn the reader. Returns immediately: a machine without a battery
    /// boots exactly as it did before.
    pub async fn start(&self) {
        if self.inner.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let service = self.clone();
        tokio::spawn(async move {
            loop {
                service.refresh();
                tokio::time::sleep(REFRESH).await;
            }
        });
    }
}

impl Default for BatteryService {
    fn default() -> Self {
        Self::new()
    }
}

/* ── The sysfs reader ──────────────────────────────────────── */

#[cfg(target_os = "linux")]
#[derive(Default)]
struct Battery {
    /// Last reported percentage.
    capacity: Option<u8>,
    status: Option<String>,
    /// Charge/energy now and full, in the kernel's own units, for weighting.
    now: Option<f64>,
    full: Option<f64>,
    /// Instantaneous draw/current, for the time estimate.
    rate: Option<f64>,
    /// `power_now` is in µW; charge-based parts are in µAh and µA. Kept apart
    /// so the two unit systems never mix.
    power_now_uw: Option<f64>,
    present: bool,
}

#[cfg(target_os = "linux")]
fn collect() -> BatteryStatus {
    let dir = sysfs_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) => return BatteryStatus::unavailable(&format!("could not read {dir}: {err}")),
    };

    let mut batteries: Vec<Battery> = Vec::new();
    let mut ac_online: Option<bool> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        let kind = read_trim(&path.join("type")).unwrap_or_default();
        match kind.as_str() {
            "Battery" => {
                if let Some(battery) = read_battery(&path) {
                    batteries.push(battery);
                }
            }
            "Mains" | "USB" | "USB_C" => {
                if let Some(online) = read_bool(&path.join("online")) {
                    ac_online = Some(ac_online.unwrap_or(false) || online);
                }
            }
            _ => {}
        }
    }

    if batteries.is_empty() {
        return BatteryStatus::unavailable(START_REASON);
    }

    let present = batteries.iter().any(|b| b.present);
    let percent = aggregate_percent(&batteries);
    let status = batteries.iter().filter_map(|b| b.status.clone()).next();
    let state = battery_state(status.as_deref(), ac_online).to_string();

    let (time_to_empty_secs, time_to_full_secs) = time_estimates(&batteries, &state);
    let energy_now_wh = total_energy_wh(&batteries);

    BatteryStatus {
        available: true,
        reason: None,
        updated_at: now(),
        present,
        percent,
        level: percent.map(battery_level),
        state,
        status,
        ac_online,
        time_to_empty_secs,
        time_to_full_secs,
        energy_now_wh,
    }
}

#[cfg(target_os = "linux")]
fn read_battery(path: &std::path::Path) -> Option<Battery> {
    let capacity = read_u8(&path.join("capacity"));
    let status = read_trim(&path.join("status"));
    let present = read_bool(&path.join("present")).unwrap_or(true);

    // Accept either the energy (µWh) or charge (µAh) family; prefer energy.
    let (now, full) = read_pair(path, "energy_now", "energy_full")
        .or_else(|| read_pair(path, "charge_now", "charge_full"))
        .unwrap_or((None, None));
    let power_now_uw = read_f64(&path.join("power_now")).filter(|v| *v > 0.0);
    let rate = read_f64(&path.join("current_now"))
        .filter(|v| *v > 0.0)
        .or(power_now_uw);

    // A directory claiming `type=Battery` but with none of these is not useful;
    // still keep it so `present` can be surfaced.
    Some(Battery {
        capacity,
        status,
        now,
        full,
        rate,
        power_now_uw,
        present,
    })
}

/// Weight the average by pack size so two mismatched batteries are not equally
/// weighted. Falls back to a plain mean, then to the first reading.
#[cfg(target_os = "linux")]
fn aggregate_percent(batteries: &[Battery]) -> Option<u8> {
    let listed: Vec<(f64, u8)> = batteries
        .iter()
        .filter(|b| b.present)
        .filter_map(|b| b.capacity.map(|c| (b.full.unwrap_or(1.0), c)))
        .collect();
    if listed.is_empty() {
        return batteries.iter().find_map(|b| b.capacity);
    }
    let total_weight: f64 = listed.iter().map(|(w, _)| w).sum();
    if total_weight <= 0.0 {
        let sum: u32 = listed.iter().map(|(_, c)| u32::from(*c)).sum();
        return Some((sum / listed.len() as u32) as u8);
    }
    let weighted: f64 = listed.iter().map(|(w, c)| w * f64::from(*c)).sum();
    Some((weighted / total_weight).round().clamp(0.0, 100.0) as u8)
}

/// Seconds remaining/full, derived from the first battery that can estimate it.
#[cfg(target_os = "linux")]
fn time_estimates(batteries: &[Battery], state: &str) -> (Option<u64>, Option<u64>) {
    match state {
        "discharging" => {
            for b in batteries {
                if let (Some(now), Some(rate)) = (b.now, b.rate) {
                    if rate > 0.0 {
                        return (Some((now / rate * 3600.0) as u64), None);
                    }
                }
            }
            (None, None)
        }
        "charging" => {
            for b in batteries {
                if let (Some(now), Some(full), Some(rate)) = (b.now, b.full, b.rate) {
                    if rate > 0.0 && full > now {
                        return (None, Some(((full - now) / rate * 3600.0) as u64));
                    }
                }
            }
            (None, None)
        }
        _ => (None, None),
    }
}

/// Total energy in Wh, when read from the energy (µWh) family. Charge-based
/// packs expose µAh/µV instead, which needs a voltage we do not sum here, so
/// they report `None`.
#[cfg(target_os = "linux")]
fn total_energy_wh(batteries: &[Battery]) -> Option<f64> {
    let wh: f64 = batteries.iter().filter_map(|b| b.now).sum();
    let has_energy = batteries.iter().any(|b| b.power_now_uw.is_some());
    if has_energy && wh > 0.0 {
        Some(wh / 1_000_000.0)
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn read_trim(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(target_os = "linux")]
fn read_u8(path: &std::path::Path) -> Option<u8> {
    read_trim(path).and_then(|s| s.parse().ok())
}

#[cfg(target_os = "linux")]
fn read_f64(path: &std::path::Path) -> Option<f64> {
    read_trim(path).and_then(|s| s.parse().ok())
}

#[cfg(target_os = "linux")]
fn read_bool(path: &std::path::Path) -> Option<bool> {
    read_trim(path).and_then(|s| match s.as_str() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    })
}

#[cfg(target_os = "linux")]
fn read_pair(path: &std::path::Path, now: &str, full: &str) -> Option<(Option<f64>, Option<f64>)> {
    if !path.join(now).exists() && !path.join(full).exists() {
        return None;
    }
    Some((read_f64(&path.join(now)), read_f64(&path.join(full))))
}

/// The live sysfs root. `SHINY_POWER_SUPPLY_DIR` lets the tests point the
/// reader at a fixture directory; production always uses the kernel path.
#[cfg(target_os = "linux")]
fn sysfs_dir() -> String {
    std::env::var("SHINY_POWER_SUPPLY_DIR").unwrap_or_else(|_| SYSFS_DIR.to_string())
}

/* ── Not Linux: inert ──────────────────────────────────────── */

#[cfg(not(target_os = "linux"))]
fn collect() -> BatteryStatus {
    BatteryStatus::unavailable(START_REASON)
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::{battery_level, battery_state, collect};
    use std::fs;
    use std::sync::Mutex;

    /// `SHINY_POWER_SUPPLY_DIR` is process-wide, so the fixture tests must not
    /// race each other when cargo runs them in parallel.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn levels_bucket_the_percentage() {
        assert_eq!(battery_level(0), 0);
        assert_eq!(battery_level(10), 0);
        assert_eq!(battery_level(11), 1);
        assert_eq!(battery_level(30), 1);
        assert_eq!(battery_level(31), 2);
        assert_eq!(battery_level(60), 2);
        assert_eq!(battery_level(61), 3);
        assert_eq!(battery_level(85), 3);
        assert_eq!(battery_level(86), 4);
        assert_eq!(battery_level(100), 4);
    }

    #[test]
    fn states_follow_the_kernel_word_and_wall_power() {
        assert_eq!(
            battery_state(Some("Discharging"), Some(false)),
            "discharging"
        );
        assert_eq!(battery_state(Some("Charging"), Some(true)), "charging");
        assert_eq!(battery_state(Some("Full"), Some(true)), "full");
        // Plugged in but held: idle, not "charging".
        assert_eq!(battery_state(Some("Not charging"), Some(true)), "idle");
        assert_eq!(battery_state(None, Some(true)), "idle");
        assert_eq!(battery_state(None, Some(false)), "discharging");
        assert_eq!(battery_state(None, None), "unknown");
        assert_eq!(battery_state(Some("Unknown"), Some(false)), "discharging");
    }

    #[test]
    fn reads_a_fixture_power_supply() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("shiny-batt-{}", std::process::id()));
        let bat = root.join("BAT0");
        let ac = root.join("ADP1");
        fs::create_dir_all(&bat).unwrap();
        fs::create_dir_all(&ac).unwrap();
        fs::write(bat.join("type"), "Battery\n").unwrap();
        fs::write(bat.join("capacity"), "76\n").unwrap();
        fs::write(bat.join("status"), "Discharging\n").unwrap();
        fs::write(bat.join("present"), "1\n").unwrap();
        fs::write(bat.join("charge_now"), "5000000\n").unwrap();
        fs::write(bat.join("charge_full"), "8000000\n").unwrap();
        fs::write(bat.join("current_now"), "1000000\n").unwrap();
        fs::write(ac.join("type"), "Mains\n").unwrap();
        fs::write(ac.join("online"), "0\n").unwrap();

        std::env::set_var("SHINY_POWER_SUPPLY_DIR", &root);
        let status = collect();
        std::env::remove_var("SHINY_POWER_SUPPLY_DIR");

        assert!(status.available);
        assert!(status.present);
        assert_eq!(status.percent, Some(76));
        assert_eq!(status.level, Some(3));
        assert_eq!(status.state, "discharging");
        assert_eq!(status.ac_online, Some(false));
        assert_eq!(status.time_to_empty_secs, Some(5 * 3600));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_supply_is_unavailable() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("shiny-batt-empty-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("SHINY_POWER_SUPPLY_DIR", &root);
        let status = collect();
        std::env::remove_var("SHINY_POWER_SUPPLY_DIR");
        assert!(!status.available);
        assert!(status.reason.is_some());
        fs::remove_dir_all(&root).ok();
    }
}
