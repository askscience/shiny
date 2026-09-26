//! Host audio state — volume, mute and the default output/input devices —
//! read from PipeWire through its PulseAudio compatibility socket.
//!
//! Why this is core and not a plugin: the audio graph belongs to the machine,
//! not to a traveler, and the top-bar chip that shows it is core chrome (a
//! sibling of `hudNetwork.js`). It follows the same contract as the other
//! daemon integrations (`network.rs`, `gpsd.rs`): probe, degrade quietly when
//! the daemon is absent, and stream changes instead of polling.
//!
//! The backend is `pactl` (pulseaudio-utils) talking to `pipewire-pulse`:
//! `--format=json` snapshots plus `subscribe` for the change feed. PipeWire's
//! own tools are a poor fit here — `wpctl` prints human text and `pw-dump`
//! dumps the whole graph — while the Pulse interface is the documented,
//! machine-readable one and is what WebKit's audio uses on this machine
//! anyway. Every `pactl` call gets a runtime-dir fallback, so a system service
//! (which has no `XDG_RUNTIME_DIR` of its own) still finds the user's socket.
//!
//! The service caches one snapshot and broadcasts it: `/api/audio/status`
//! reads the cache, `/api/audio/events` relays the broadcast as SSE, and
//! mutations (volume / mute / default device) update the cache immediately.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, Mutex};

/// How long to let a volume drag settle before paying for a snapshot — the
/// Pulse server emits one `change` event per step of the slider.
const DEBOUNCE: Duration = Duration::from_millis(250);
/// Re-probe cadence when the audio server is absent (or went away).
const RETRY: Duration = Duration::from_secs(30);
/// Safety net: refresh even with no events, to catch anything missed (a
/// dropped subscription, a silently restarted daemon).
const SAFETY_REFRESH: Duration = Duration::from_secs(60);
const BROADCAST_CAPACITY: usize = 64;
/// A `pactl` call must never wedge a status request or an API handler.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(target_os = "linux")]
const START_REASON: &str = "PipeWire is not reachable";
#[cfg(not(target_os = "linux"))]
const START_REASON: &str = "the sound panel requires Linux (PipeWire)";

/// Which half of the graph an action targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioTarget {
    Sink,
    Source,
}

impl AudioTarget {
    /// `pactl`'s placeholder for "whatever is default", so an action can omit
    /// the node id.
    fn default_token(self) -> &'static str {
        match self {
            Self::Sink => "@DEFAULT_SINK@",
            Self::Source => "@DEFAULT_SOURCE@",
        }
    }

    fn set_volume_flag(self) -> &'static str {
        match self {
            Self::Sink => "set-sink-volume",
            Self::Source => "set-source-volume",
        }
    }

    fn set_mute_flag(self) -> &'static str {
        match self {
            Self::Sink => "set-sink-mute",
            Self::Source => "set-source-mute",
        }
    }

    fn set_default_flag(self) -> &'static str {
        match self {
            Self::Sink => "set-default-sink",
            Self::Source => "set-default-source",
        }
    }
}

/// Everything the UI needs to draw the sound chip and the Sound menu.
#[derive(Debug, Clone, Serialize)]
pub struct AudioStatus {
    /// False when PipeWire/pactl is missing or unreachable; the UI hides then.
    pub available: bool,
    /// Why the panel is unavailable (shown in the menu, not the chip).
    pub reason: Option<String>,
    /// When this snapshot was taken (RFC 3339).
    pub updated_at: String,
    /// The Pulse server string, e.g. `PulseAudio (on PipeWire 1.4.2)`.
    pub server: Option<String>,
    /// Name of the default sink, when one is configured.
    pub default_sink: Option<String>,
    pub default_source: Option<String>,
    pub sinks: Vec<AudioNode>,
    pub sources: Vec<AudioNode>,
}

/// One output (sink) or input (source) node.
#[derive(Debug, Clone, Serialize)]
pub struct AudioNode {
    /// Pulse node index. Volatile across daemon restarts, so the UI always
    /// acts on ids from the latest snapshot.
    pub id: u32,
    pub name: String,
    /// Long label, e.g. `Apple Audio Device Internal Speakers`.
    pub description: String,
    /// Short label from the node itself (`Speaker`, `Digital Mic`), when the
    /// driver provides one — what the chip shows.
    pub nick: Option<String>,
    /// The card/product this node belongs to (`device.description`).
    pub card: Option<String>,
    pub is_default: bool,
    pub volume_percent: u8,
    pub muted: bool,
    /// Icon bucket 0–3, see [`volume_level`].
    pub level: u8,
    /// Pulse state: `RUNNING`, `IDLE`, `SUSPENDED`, …
    pub state: String,
    pub channels: u8,
    /// `speaker`, `headphones`, `headset`, `microphone`, … when known.
    pub form_factor: Option<String>,
    /// Driver icon name (`audio-speakers`, `audio-headphones`, …).
    pub icon: Option<String>,
    /// Active port label, e.g. `[Out] Speaker`.
    pub active_port: Option<String>,
}

impl AudioStatus {
    fn unavailable(reason: &str) -> Self {
        Self {
            available: false,
            reason: Some(reason.to_string()),
            updated_at: now(),
            server: None,
            default_sink: None,
            default_source: None,
            sinks: Vec::new(),
            sources: Vec::new(),
        }
    }
}

/// Bucket a volume percentage into the four icon levels the HUD draws.
///
/// The thresholds follow GNOME Shell's sound indicator so the bars/waves mean
/// the same thing as everywhere else on a Linux desktop. The mapping lives
/// here, not in the frontend, so the chip and any future consumer agree by
/// construction.
#[must_use]
pub fn volume_level(percent: u8, muted: bool) -> u8 {
    if muted || percent == 0 {
        0
    } else if percent < 34 {
        1
    } else if percent < 67 {
        2
    } else {
        3
    }
}

#[derive(Clone)]
pub struct AudioService {
    inner: Arc<Inner>,
}

struct Inner {
    status: ArcSwap<AudioStatus>,
    events: broadcast::Sender<Arc<AudioStatus>>,
    /// Serializes snapshots: a burst of events must not stack N `pactl`
    /// process spawns.
    refreshing: Mutex<()>,
    /// Serializes mutations, so two volume updates cannot interleave and land
    /// out of order (each is its own `pactl` process).
    command: Mutex<()>,
    started: AtomicBool,
}

impl AudioService {
    #[must_use]
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            inner: Arc::new(Inner {
                status: ArcSwap::from_pointee(AudioStatus::unavailable(START_REASON)),
                events,
                refreshing: Mutex::new(()),
                command: Mutex::new(()),
                started: AtomicBool::new(false),
            }),
        }
    }

    /// Latest snapshot; never spawns a process.
    #[must_use]
    pub fn status(&self) -> Arc<AudioStatus> {
        self.inner.status.load_full()
    }

    /// Subscribe to snapshots, for the SSE relay.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<AudioStatus>> {
        self.inner.events.subscribe()
    }

    fn publish(&self, status: AudioStatus) {
        let status = Arc::new(status);
        self.inner.status.store(status.clone());
        let _ = self.inner.events.send(status);
    }

    fn set_unavailable(&self, reason: &str) {
        self.publish(AudioStatus::unavailable(reason));
    }
}

impl Default for AudioService {
    fn default() -> Self {
        Self::new()
    }
}

/* ── Linux: the real thing ─────────────────────────────────── */

#[cfg(target_os = "linux")]
impl AudioService {
    /// Probe the audio server and subscribe to its change feed.
    ///
    /// Returns immediately — probe and event loop live in a background task,
    /// so a machine without PipeWire boots exactly as it did before. Safe to
    /// call once per process.
    pub async fn start(&self) {
        if self.inner.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let service = self.clone();
        tokio::spawn(async move { service.run().await });
    }

    async fn run(&self) {
        let mut last_reason: Option<String> = None;
        loop {
            match self.snapshot().await {
                Ok(status) => {
                    self.publish(status);
                    tracing::info!("PipeWire audio connected — sound panel enabled");

                    if let Err(err) = self.consume().await {
                        tracing::debug!("audio event stream ended: {err}");
                    }
                    // The subscription ending means the connection dropped
                    // (daemon restart, socket gone): hide the panel until the
                    // next successful snapshot instead of showing stale levels.
                    let reason = "the PipeWire event stream closed".to_string();
                    self.set_unavailable(&reason);
                    last_reason = Some(reason);
                }
                Err(err) => {
                    if last_reason.as_deref() != Some(err.as_str()) {
                        tracing::warn!("{err} — sound panel disabled, retrying every 30s");
                        self.set_unavailable(&err);
                        last_reason = Some(err);
                    }
                }
            }
            tokio::time::sleep(RETRY).await;
        }
    }

    /// One snapshot: server info plus the two node lists.
    async fn snapshot(&self) -> Result<AudioStatus, String> {
        let info = run_pactl(&["--format=json", "info"]).await?;
        let sinks = run_pactl(&["--format=json", "list", "sinks"]).await?;
        let sources = run_pactl(&["--format=json", "list", "sources"]).await?;

        let info: serde_json::Value = serde_json::from_slice(&info)
            .map_err(|err| format!("pactl info returned invalid JSON: {err}"))?;
        let sinks: serde_json::Value = serde_json::from_slice(&sinks)
            .map_err(|err| format!("pactl list sinks returned invalid JSON: {err}"))?;
        let sources: serde_json::Value = serde_json::from_slice(&sources)
            .map_err(|err| format!("pactl list sources returned invalid JSON: {err}"))?;

        Ok(parse_status(&info, &sinks, &sources))
    }

    /// Take a fresh snapshot and publish it. Cheap to call: a no-op while a
    /// snapshot is already in flight.
    pub async fn refresh(&self) {
        let Ok(_guard) = self.inner.refreshing.try_lock() else {
            return;
        };
        match self.snapshot().await {
            Ok(status) => self.publish(status),
            Err(err) => tracing::debug!("audio snapshot failed: {err}"),
        }
    }

    /// Consume `pactl subscribe` until the connection ends, debouncing bursts
    /// (a slider drag) into one snapshot and refreshing periodically as a
    /// safety net.
    async fn consume(&self) -> Result<(), String> {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let mut child = tokio::process::Command::new("pactl")
            .arg("subscribe")
            .envs(client_env())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|err| pactl_error(&err))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "pactl subscribe produced no stdout".to_string())?;
        let mut lines = BufReader::new(stdout).lines();

        let mut tick = tokio::time::interval(SAFETY_REFRESH);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await; // the first tick fires immediately

        let mut pending_at: Option<tokio::time::Instant> = None;
        loop {
            // Armed only while an event is pending; re-created every iteration,
            // so each new event restarts the quiet period.
            let debounce = async {
                match pending_at {
                    Some(at) => tokio::time::sleep_until(at + DEBOUNCE).await,
                    None => std::future::pending::<()>().await,
                }
            };

            tokio::select! {
                line = lines.next_line() => match line {
                    Ok(Some(line)) => {
                        if is_relevant_event(&line) {
                            pending_at = Some(tokio::time::Instant::now());
                        }
                    }
                    Ok(None) => return Ok(()),
                    Err(err) => return Err(err.to_string()),
                },
                _ = tick.tick() => self.refresh().await,
                _ = debounce => {
                    pending_at = None;
                    self.refresh().await;
                }
            }
        }
    }

    /// Whether a snapshot has ever succeeded, i.e. PipeWire is reachable.
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.status().available
    }

    /// Set an absolute volume (0–100 %). `id: None` means the default device.
    ///
    /// 100 % is the ceiling on purpose: this machine drives the T2's speaker
    /// array, and pushing past unity into digital gain risks the hardware
    /// (and sounds worse than the hardware amp anyway).
    pub async fn set_volume(
        &self,
        target: AudioTarget,
        id: Option<u32>,
        percent: u8,
    ) -> Result<(), String> {
        if percent > 100 {
            return Err("volume must be between 0 and 100%".to_string());
        }
        let _guard = self.inner.command.lock().await;
        let spec = node_spec(target, id);
        run_pactl(&[
            target.set_volume_flag(),
            spec.as_str(),
            &format!("{percent}%"),
        ])
        .await?;
        drop(_guard);
        self.refresh().await;
        Ok(())
    }

    /// Set (or toggle, when `muted` is `None`) the mute flag.
    pub async fn set_mute(
        &self,
        target: AudioTarget,
        id: Option<u32>,
        muted: Option<bool>,
    ) -> Result<(), String> {
        let _guard = self.inner.command.lock().await;
        let spec = node_spec(target, id);
        let value = match muted {
            Some(true) => "1",
            Some(false) => "0",
            None => "toggle",
        };
        run_pactl(&[target.set_mute_flag(), spec.as_str(), value]).await?;
        drop(_guard);
        self.refresh().await;
        Ok(())
    }

    /// Make a node the default target for new streams.
    pub async fn set_default(&self, target: AudioTarget, id: u32) -> Result<(), String> {
        let _guard = self.inner.command.lock().await;
        run_pactl(&[target.set_default_flag(), &id.to_string()]).await?;
        drop(_guard);
        self.refresh().await;
        Ok(())
    }
}

/// `pactl` accepts either a node index or the `@DEFAULT_*@` placeholder.
#[cfg(target_os = "linux")]
fn node_spec(target: AudioTarget, id: Option<u32>) -> String {
    match id {
        Some(id) => id.to_string(),
        None => target.default_token().to_string(),
    }
}

/// Environment a `pactl` child needs to find the user's audio socket.
///
/// `shiny.service` is a system service: it inherits neither `XDG_RUNTIME_DIR`
/// nor a session bus, so libpulse would look in the wrong place (or nowhere).
/// When the variables are unset, point the child at this user's runtime dir —
/// the same fallback systemd would set up for a login session.
#[cfg(target_os = "linux")]
fn client_env() -> Vec<(&'static str, std::ffi::OsString)> {
    use std::path::PathBuf;

    let mut env = Vec::new();
    let runtime_dir: Option<PathBuf> = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(user_runtime_dir);

    if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
        if let Some(dir) = &runtime_dir {
            env.push(("XDG_RUNTIME_DIR", dir.clone().into_os_string()));
        }
    }
    if std::env::var_os("PULSE_SERVER").is_none() {
        if let Some(socket) = runtime_dir
            .map(|dir| dir.join("pulse/native"))
            .filter(|socket| socket.exists())
        {
            env.push(("PULSE_SERVER", format!("unix:{}", socket.display()).into()));
        }
    }
    env
}

/// `/run/user/<uid>`, as systemd would provide for a login session.
#[cfg(target_os = "linux")]
fn user_runtime_dir() -> Option<std::path::PathBuf> {
    use std::os::unix::fs::MetadataExt;

    let uid = std::fs::metadata("/proc/self").ok()?.uid();
    let dir = std::path::PathBuf::from(format!("/run/user/{uid}"));
    dir.is_dir().then_some(dir)
}

/// Run one `pactl` invocation and return stdout, with the client environment
/// applied and a hard timeout so a stuck daemon cannot wedge the caller.
#[cfg(target_os = "linux")]
async fn run_pactl(args: &[&str]) -> Result<Vec<u8>, String> {
    let mut command = tokio::process::Command::new("pactl");
    command
        .args(args)
        .envs(client_env())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let child = command.spawn().map_err(|err| pactl_error(&err))?;
    let output = tokio::time::timeout(COMMAND_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| format!("pactl {} timed out", args.join(" ")))?
        .map_err(|err| format!("pactl {} failed: {err}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        return Err(if detail.is_empty() {
            format!("pactl {} failed", args.join(" "))
        } else {
            format!("PipeWire is not reachable ({detail})")
        });
    }
    Ok(output.stdout)
}

#[cfg(target_os = "linux")]
fn pactl_error(err: &std::io::Error) -> String {
    if err.kind() == std::io::ErrorKind::NotFound {
        "pactl is not installed (install pulseaudio-utils)".to_string()
    } else {
        format!("could not run pactl: {err}")
    }
}

/// `pactl` calls in this module each connect as a short-lived Pulse client,
/// so every snapshot we take emits `Event 'new'/'remove' on client #N`.
/// Reacting to those would loop forever (snapshot → client event → snapshot),
/// so only events about the graph itself arm a refresh.
#[cfg(target_os = "linux")]
fn is_relevant_event(line: &str) -> bool {
    // `Event 'change' on sink #63` → 4th token is the facility.
    match line.split_whitespace().nth(3) {
        Some("client") => false,
        Some(_) => true,
        None => false,
    }
}

/// Turn the three `pactl --format=json` documents into one snapshot.
#[cfg(target_os = "linux")]
fn parse_status(
    info: &serde_json::Value,
    sinks: &serde_json::Value,
    sources: &serde_json::Value,
) -> AudioStatus {
    let default_sink = default_name(info.get("default_sink_name"));
    let default_source = default_name(info.get("default_source_name"));

    AudioStatus {
        available: true,
        reason: None,
        updated_at: now(),
        server: info
            .get("server_name")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        sinks: parse_nodes(sinks, default_sink.as_deref()),
        sources: parse_nodes(sources, default_source.as_deref()),
        default_sink,
        default_source,
    }
}

/// `pactl info` reports the placeholder `@DEFAULT_SINK@` when no default is
/// configured; treat that as "none", not as a device with that odd name.
#[cfg(target_os = "linux")]
fn default_name(value: Option<&serde_json::Value>) -> Option<String> {
    let name = value?.as_str()?;
    (!name.starts_with('@')).then(|| name.to_string())
}

#[cfg(target_os = "linux")]
fn parse_nodes(list: &serde_json::Value, default_name: Option<&str>) -> Vec<AudioNode> {
    let Some(entries) = list.as_array() else {
        return Vec::new();
    };
    let mut nodes: Vec<AudioNode> = entries.iter().filter_map(parse_node).collect();
    for node in &mut nodes {
        node.is_default = default_name == Some(node.name.as_str());
    }
    // Default first, then anything actively playing, then by label.
    nodes.sort_by(|a, b| {
        b.is_default
            .cmp(&a.is_default)
            .then_with(|| {
                b.state
                    .eq_ignore_ascii_case("RUNNING")
                    .cmp(&a.state.eq_ignore_ascii_case("RUNNING"))
            })
            .then_with(|| a.description.cmp(&b.description))
    });
    nodes
}

#[cfg(target_os = "linux")]
fn parse_node(entry: &serde_json::Value) -> Option<AudioNode> {
    let id = entry.get("index").and_then(serde_json::Value::as_u64)?;
    let name = entry
        .get("name")
        .and_then(serde_json::Value::as_str)?
        .to_string();
    let properties = entry.get("properties");
    let property = |key: &str| {
        properties
            .and_then(|props| props.get(key))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };

    let muted = entry
        .get("mute")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let volume_percent = entry.get("volume").map(volume_percent).unwrap_or(0);

    // Monitor sources are loopbacks of a sink, not capture hardware; the Sound
    // menu lists real inputs only (GNOME's panel hides them too).
    let is_monitor = name.ends_with(".monitor")
        || properties
            .and_then(|props| props.get("device.class"))
            .and_then(serde_json::Value::as_str)
            == Some("monitor");
    if is_monitor {
        return None;
    }

    Some(AudioNode {
        id: u32::try_from(id).ok()?,
        name,
        description: entry
            .get("description")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .unwrap_or_default(),
        nick: property("node.nick"),
        card: property("device.description"),
        is_default: false,
        volume_percent,
        muted,
        level: volume_level(volume_percent, muted),
        state: entry
            .get("state")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        channels: entry
            .get("channel_map")
            .and_then(serde_json::Value::as_str)
            .map(channel_count)
            .unwrap_or(0),
        form_factor: property("device.form_factor"),
        icon: property("device.icon_name"),
        active_port: entry
            .get("active_port")
            .and_then(serde_json::Value::as_str)
            .filter(|port| !port.is_empty())
            .map(str::to_string),
    })
}

/// One number for the UI: the mean of the per-channel percentages (PipeWire
/// keeps every channel in lockstep, so this is the value the user set).
#[cfg(target_os = "linux")]
fn volume_percent(volume: &serde_json::Value) -> u8 {
    let Some(channels) = volume.as_object() else {
        return 0;
    };
    let mut total = 0f64;
    let mut count = 0usize;
    for channel in channels.values() {
        let Some(percent) = channel.get("value_percent") else {
            continue;
        };
        let value = match percent {
            serde_json::Value::String(text) => {
                text.trim_end_matches('%').trim().parse::<f64>().ok()
            }
            other => other.as_f64(),
        };
        if let Some(value) = value {
            total += value;
            count += 1;
        }
    }
    if count == 0 {
        0
    } else {
        (total / count as f64).round().clamp(0.0, 150.0) as u8
    }
}

/// `"front-left,front-right"` → 2.
#[cfg(target_os = "linux")]
fn channel_count(channel_map: &str) -> u8 {
    let trimmed = channel_map.trim();
    if trimmed.is_empty() {
        return 0;
    }
    u8::try_from(trimmed.split(',').count()).unwrap_or(u8::MAX)
}

/* ── Not Linux: inert stub ─────────────────────────────────── */

#[cfg(not(target_os = "linux"))]
impl AudioService {
    /// No-op: `AudioStatus::unavailable(START_REASON)` already explains why.
    pub async fn start(&self) {}

    pub async fn refresh(&self) {}

    pub async fn set_volume(
        &self,
        _target: AudioTarget,
        _id: Option<u32>,
        _percent: u8,
    ) -> Result<(), String> {
        Err(START_REASON.to_string())
    }

    pub async fn set_mute(
        &self,
        _target: AudioTarget,
        _id: Option<u32>,
        _muted: Option<bool>,
    ) -> Result<(), String> {
        Err(START_REASON.to_string())
    }

    pub async fn set_default(&self, _target: AudioTarget, _id: u32) -> Result<(), String> {
        Err(START_REASON.to_string())
    }

    #[must_use]
    pub fn is_available(&self) -> bool {
        false
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::volume_level;

    #[test]
    fn volume_levels_match_gnome_buckets() {
        // Muted and 0 % both draw the silent icon; 33 % is still the low
        // bucket, 34 % the middle one, 67 % the loud one.
        assert_eq!(volume_level(0, false), 0);
        assert_eq!(volume_level(50, true), 0);
        assert_eq!(volume_level(1, false), 1);
        assert_eq!(volume_level(33, false), 1);
        assert_eq!(volume_level(34, false), 2);
        assert_eq!(volume_level(66, false), 2);
        assert_eq!(volume_level(67, false), 3);
        assert_eq!(volume_level(100, false), 3);
    }

    #[cfg(target_os = "linux")]
    mod linux {
        use super::super::{
            default_name, is_relevant_event, parse_nodes, parse_status, volume_percent,
        };
        use serde_json::json;

        #[test]
        fn ignores_client_events() {
            // Our own snapshots connect as clients; only graph events matter.
            assert!(is_relevant_event("Event 'change' on sink #63"));
            assert!(is_relevant_event("Event 'change' on card #50"));
            assert!(is_relevant_event("Event 'new' on source #61"));
            assert!(!is_relevant_event("Event 'new' on client #153"));
            assert!(!is_relevant_event("Event 'remove' on client #153"));
            assert!(!is_relevant_event(""));
        }

        #[test]
        fn averages_channel_volume() {
            let volume = json!({
                "front-left": { "value": 19660, "value_percent": "30%" },
                "front-right": { "value": 39321, "value_percent": "60%" },
            });
            assert_eq!(volume_percent(&volume), 45);

            // Numeric percentages (newer pactl) parse too.
            let numeric = json!({ "mono": { "value_percent": 80.0 } });
            assert_eq!(volume_percent(&numeric), 80);

            assert_eq!(volume_percent(&json!({})), 0);
        }

        #[test]
        fn placeholder_default_means_none() {
            assert_eq!(
                default_name(Some(&json!("alsa_output.speakers"))).as_deref(),
                Some("alsa_output.speakers")
            );
            assert_eq!(default_name(Some(&json!("@DEFAULT_SINK@"))), None);
            assert_eq!(default_name(None), None);
        }

        #[test]
        fn parses_sink_snapshot_with_default_and_levels() {
            let info = json!({
                "server_name": "PulseAudio (on PipeWire 1.4.2)",
                "default_sink_name": "alsa_output.speakers",
            });
            let sinks = json!([
                {
                    "index": 63,
                    "state": "RUNNING",
                    "name": "alsa_output.speakers",
                    "description": "Apple Audio Device Internal Speakers",
                    "channel_map": "front-left,front-right",
                    "mute": false,
                    "volume": {
                        "front-left": { "value_percent": "30%" },
                        "front-right": { "value_percent": "30%" },
                    },
                    "active_port": "[Out] Speaker",
                    "properties": {
                        "node.nick": "Speaker",
                        "device.description": "Apple Audio Device",
                        "device.icon_name": "audio-speakers",
                    },
                },
                {
                    "index": 70,
                    "state": "SUSPENDED",
                    "name": "alsa_output.hdmi",
                    "description": "HDMI Audio",
                    "channel_map": "front-left,front-right",
                    "mute": true,
                    "volume": { "front-left": { "value_percent": "100%" }, "front-right": { "value_percent": "100%" } },
                    "properties": {},
                },
            ]);

            let status = parse_status(&info, &sinks, &json!([]));
            assert!(status.available);
            assert_eq!(
                status.server.as_deref(),
                Some("PulseAudio (on PipeWire 1.4.2)")
            );
            assert_eq!(status.sinks.len(), 2);

            // Default first, and it carries the parsed labels + bucket.
            let sink = &status.sinks[0];
            assert_eq!(sink.name, "alsa_output.speakers");
            assert!(sink.is_default);
            assert_eq!(sink.nick.as_deref(), Some("Speaker"));
            assert_eq!(sink.card.as_deref(), Some("Apple Audio Device"));
            assert_eq!(sink.volume_percent, 30);
            assert_eq!(sink.level, 1);
            assert_eq!(sink.channels, 2);
            assert_eq!(sink.active_port.as_deref(), Some("[Out] Speaker"));

            let hdmi = &status.sinks[1];
            assert!(!hdmi.is_default);
            assert!(hdmi.muted);
            assert_eq!(hdmi.level, 0);
        }

        #[test]
        fn missing_default_leaves_no_node_default() {
            let nodes = parse_nodes(
                &json!([{ "index": 1, "name": "s", "description": "S" }]),
                None,
            );
            assert_eq!(nodes.len(), 1);
            assert!(!nodes[0].is_default);
        }

        #[test]
        fn monitor_sources_are_hidden() {
            let sources = json!([
                {
                    "index": 61,
                    "name": "alsa_input.mic",
                    "description": "Internal Microphone",
                    "properties": { "device.class": "sound" },
                },
                {
                    "index": 62,
                    "name": "alsa_output.speakers.monitor",
                    "description": "Monitor of Speakers",
                    "properties": { "device.class": "monitor" },
                },
            ]);
            let nodes = parse_nodes(&sources, None);
            assert_eq!(nodes.len(), 1);
            assert_eq!(nodes[0].name, "alsa_input.mic");
        }
    }
}
