//! Host network state — Wi-Fi radio, the active link, visible networks and
//! saved profiles — read from NetworkManager over the system D-Bus.
//!
//! Why this is core and not a plugin: the state belongs to the machine, not to
//! a traveler, and the top-bar chip that shows it is core chrome (a sibling of
//! `hudLeft.js`). It follows the same contract as the other daemon
//! integrations (`gpsd.rs`, `whisper.rs`): probe, degrade quietly when the
//! daemon is absent, and stream changes instead of polling.
//!
//! `nmrs` supplies the D-Bus calls and the signal feed; this module turns them
//! into one small serializable snapshot (`NetworkStatus`). D-Bus round-trips
//! are far too expensive to run per HTTP request, so the snapshot is cached
//! and broadcast: `/api/network/status` reads the cache and
//! `/api/network/events` relays the broadcast as SSE.
//!
//! The crate is Linux-only, so the service compiles to an inert stub
//! elsewhere — the workspace also builds on macOS for the `peakd` shell.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use serde::Serialize;
use tokio::sync::{broadcast, Mutex};

/// How long to let NetworkManager settle after a burst of signals before
/// paying for a snapshot (scans emit a flurry of access-point changes).
const DEBOUNCE: Duration = Duration::from_millis(500);
/// Re-probe cadence when NetworkManager is absent (or went away).
const RETRY: Duration = Duration::from_secs(30);
/// Safety net: refresh even with no signals, to catch anything missed (a
/// dropped subscription, a silently restarted daemon).
const SAFETY_REFRESH: Duration = Duration::from_secs(60);
const BROADCAST_CAPACITY: usize = 64;

#[cfg(target_os = "linux")]
const START_REASON: &str = "NetworkManager is not reachable";
#[cfg(not(target_os = "linux"))]
const START_REASON: &str = "the network panel requires Linux (NetworkManager)";

/// Everything the UI needs to draw the network chip and the network window.
///
/// Deliberately our own shape rather than `nmrs` types: the API contract stays
/// stable if the underlying library changes, and `nmrs` models are not all
/// `Serialize`.
#[derive(Debug, Clone, Serialize)]
pub struct NetworkStatus {
    /// False when NetworkManager is missing/unreachable; the UI hides then.
    pub available: bool,
    /// Why the panel is unavailable (shown in the window, not the chip).
    pub reason: Option<String>,
    /// When this snapshot was taken (RFC 3339).
    pub updated_at: String,
    pub wifi: WifiRadioStatus,
    pub connectivity: ConnectivityStatus,
    /// The primary link: Wi-Fi when one is active, else wired Ethernet.
    pub connection: Option<ActiveLink>,
    pub wired: Vec<WiredLink>,
    /// Non-loopback interfaces that have an address, as the kernel sees them
    /// — including the ones NetworkManager does not manage (see
    /// [`KernelLink`]).
    pub links: Vec<KernelLink>,
    /// Visible access points, one row per SSID (strongest BSSID wins).
    pub networks: Vec<VisibleNetwork>,
    /// Saved Wi-Fi profiles.
    pub saved: Vec<SavedNetwork>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WifiRadioStatus {
    /// A wireless device exists at all.
    pub present: bool,
    /// Software switch (`nmcli radio wifi`).
    pub enabled: bool,
    /// Hardware switch / rfkill. When false the UI must not offer to enable.
    pub hardware_enabled: bool,
    /// Wireless interface names (e.g. `["wlp5s0"]`).
    pub interfaces: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectivityStatus {
    /// NetworkManager's connectivity verdict, lowercased: `full`, `portal`,
    /// `limited`, `none` or `unknown`.
    pub state: String,
    /// True only for `full` — connected and internet-reachable.
    pub internet: bool,
    pub captive_portal_url: Option<String>,
}

/// The link the machine is actually using.
#[derive(Debug, Clone, Serialize)]
pub struct ActiveLink {
    /// `"wifi"` or `"ethernet"`.
    pub kind: String,
    pub interface: String,
    pub ssid: Option<String>,
    pub bssid: Option<String>,
    pub strength: Option<u8>,
    /// Icon bucket 0–4, see [`signal_level`].
    pub level: u8,
    pub frequency_mhz: Option<u32>,
    pub band: Option<String>,
    /// Human label: `WPA3`, `WPA2/WPA3`, `802.1X`, `WEP`, `OWE`, `Open`, …
    pub security: String,
    pub secured: bool,
    pub known: bool,
    pub ip4: Option<String>,
    pub ip6: Option<String>,
    pub speed_mbps: Option<u32>,
    /// False when NetworkManager does not manage this interface (the link is
    /// configured by ifupdown/systemd-networkd/…).
    pub managed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WiredLink {
    pub interface: String,
    pub state: String,
    pub connected: bool,
    pub ip4: Option<String>,
    pub speed_mbps: Option<u32>,
    pub managed: bool,
}

/// A non-loopback interface as the kernel sees it.
///
/// NetworkManager does not manage every interface — a Debian box with
/// `/etc/network/interfaces` (ifupdown) and `[ifupdown] managed=false` in
/// NetworkManager.conf reports such links as `unmanaged`, with no address.
/// Without this view the panel would claim the machine is offline while it is
/// online through the ifupdown link, so the kernel is queried directly.
#[derive(Debug, Clone, Serialize)]
pub struct KernelLink {
    pub interface: String,
    /// Operstate is `up` (RFC 2863 via `getifaddrs`).
    pub up: bool,
    pub ip4: Option<String>,
    pub ip6: Option<String>,
    /// Carries the default route (`/proc/net/route`).
    pub default_route: bool,
    /// Link speed from `/sys/class/net/<if>/speed`, when the driver reports it.
    pub speed_mbps: Option<u32>,
    /// Has a `/sys/class/net/<if>/wireless` directory.
    pub wireless: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct VisibleNetwork {
    pub ssid: String,
    pub interface: String,
    pub bssid: Option<String>,
    pub strength: u8,
    pub level: u8,
    pub frequency_mhz: Option<u32>,
    pub band: Option<String>,
    pub secured: bool,
    pub security: String,
    pub known: bool,
    pub active: bool,
    /// SSID is hidden (empty probe response).
    pub hidden: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SavedNetwork {
    pub uuid: String,
    pub id: String,
    pub ssid: String,
    pub interface: Option<String>,
    pub autoconnect: bool,
    /// Last activation, Unix seconds (0 = never).
    pub last_used_unix: u64,
}

impl NetworkStatus {
    fn unavailable(reason: &str) -> Self {
        Self {
            available: false,
            reason: Some(reason.to_string()),
            updated_at: now(),
            wifi: WifiRadioStatus {
                present: false,
                enabled: false,
                hardware_enabled: false,
                interfaces: Vec::new(),
            },
            connectivity: ConnectivityStatus {
                state: "unknown".into(),
                internet: false,
                captive_portal_url: None,
            },
            connection: None,
            wired: Vec::new(),
            links: Vec::new(),
            networks: Vec::new(),
            saved: Vec::new(),
        }
    }
}

/// Bucket a NetworkManager signal strength (0–100) into the five icon levels
/// the HUD draws.
///
/// The thresholds match GNOME Shell's Wi-Fi indicator (`>80 / >55 / >30 / >5`)
/// so the bars mean the same thing as everywhere else on a Linux desktop. The
/// mapping lives here, not in the frontend, so both the chip and any future
/// consumer agree by construction.
#[must_use]
pub fn signal_level(strength: u8) -> u8 {
    match strength {
        s if s > 80 => 4,
        s if s > 55 => 3,
        s if s > 30 => 2,
        s if s > 5 => 1,
        _ => 0,
    }
}

#[derive(Clone)]
pub struct NetworkService {
    inner: Arc<Inner>,
}

struct Inner {
    status: ArcSwap<NetworkStatus>,
    events: broadcast::Sender<Arc<NetworkStatus>>,
    /// Serializes snapshots: a burst of signals must not stack N D-Bus
    /// round-trips.
    refreshing: Mutex<()>,
    started: AtomicBool,
    #[cfg(target_os = "linux")]
    nm: Mutex<Option<nmrs::NetworkManager>>,
}

impl NetworkService {
    #[must_use]
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            inner: Arc::new(Inner {
                status: ArcSwap::from_pointee(NetworkStatus::unavailable(START_REASON)),
                events,
                refreshing: Mutex::new(()),
                started: AtomicBool::new(false),
                #[cfg(target_os = "linux")]
                nm: Mutex::new(None),
            }),
        }
    }

    /// Latest snapshot; never does D-Bus work.
    #[must_use]
    pub fn status(&self) -> Arc<NetworkStatus> {
        self.inner.status.load_full()
    }

    /// Subscribe to snapshots, for the SSE relay.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<NetworkStatus>> {
        self.inner.events.subscribe()
    }

    fn publish(&self, status: NetworkStatus) {
        let status = Arc::new(status);
        self.inner.status.store(status.clone());
        let _ = self.inner.events.send(status);
    }

    fn set_unavailable(&self, reason: &str) {
        self.publish(NetworkStatus::unavailable(reason));
    }
}

impl Default for NetworkService {
    fn default() -> Self {
        Self::new()
    }
}

/* ── Linux: the real thing ─────────────────────────────────── */

#[cfg(target_os = "linux")]
impl NetworkService {
    /// Probe NetworkManager and subscribe to its change signals.
    ///
    /// Returns immediately — probe and event loop live in a background task,
    /// so a machine without NetworkManager boots exactly as it did before.
    /// Safe to call once per process.
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
            match nmrs::NetworkManager::new().await {
                Ok(nm) => {
                    *self.inner.nm.lock().await = Some(nm.clone());
                    self.refresh().await;
                    tracing::info!("NetworkManager connected — network panel enabled");

                    match nm.network_events().await {
                        Ok(stream) => self.consume(stream).await,
                        Err(err) => {
                            tracing::warn!("NetworkManager event stream failed: {err}");
                        }
                    }

                    *self.inner.nm.lock().await = None;
                    let reason = "NetworkManager event stream closed".to_string();
                    self.set_unavailable(&reason);
                    last_reason = Some(reason);
                }
                Err(err) => {
                    let reason = format!("NetworkManager is not reachable ({err})");
                    if last_reason.as_deref() != Some(reason.as_str()) {
                        tracing::warn!("{reason} — panel disabled, retrying every 30s");
                        self.set_unavailable(&reason);
                        last_reason = Some(reason);
                    }
                }
            }
            tokio::time::sleep(RETRY).await;
        }
    }

    /// Consume NetworkManager signals until the stream ends, debouncing bursts
    /// into one snapshot and refreshing periodically as a safety net.
    async fn consume(&self, mut stream: nmrs::NetworkEventStream) {
        use futures::StreamExt;

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
                item = stream.next() => match item {
                    Some(Ok(event)) => {
                        tracing::trace!("network event: {event:?}");
                        pending_at = Some(tokio::time::Instant::now());
                    }
                    Some(Err(err)) => tracing::debug!("network event stream error: {err}"),
                    None => {
                        tracing::debug!("network event stream ended");
                        break;
                    }
                },
                _ = tick.tick() => self.refresh().await,
                _ = debounce => {
                    pending_at = None;
                    self.refresh().await;
                }
            }
        }
    }

    /// Take a fresh snapshot and publish it. Cheap to call: a no-op when
    /// NetworkManager is absent or a snapshot is already in flight.
    pub async fn refresh(&self) {
        let nm = self.inner.nm.lock().await.clone();
        let Some(nm) = nm else {
            return;
        };
        let Ok(_guard) = self.inner.refreshing.try_lock() else {
            return;
        };
        match collect(&nm).await {
            Ok(status) => self.publish(status),
            Err(err) => tracing::debug!("network snapshot failed: {err}"),
        }
    }

    /// Ask NetworkManager for a fresh scan. The scan itself is asynchronous,
    /// so this returns once the request is accepted and re-snapshots after the
    /// results have had time to land (the event stream usually beats us to it).
    pub async fn scan(&self) -> Result<(), String> {
        let nm = self.inner.nm.lock().await.clone();
        let Some(nm) = nm else {
            return Err("NetworkManager is not available".to_string());
        };
        nm.scan_networks(None).await.map_err(|err| err.to_string())?;

        let service = self.clone();
        tokio::spawn(async move {
            for delay in [Duration::from_secs(2), Duration::from_secs(4)] {
                tokio::time::sleep(delay).await;
                service.refresh().await;
            }
        });
        Ok(())
    }

    /// Whether a snapshot has ever succeeded, i.e. NetworkManager is reachable.
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.status().available
    }

    /// The live NetworkManager handle, or the reason there is none.
    async fn nm(&self) -> Result<nmrs::NetworkManager, String> {
        self.inner
            .nm
            .lock()
            .await
            .clone()
            .ok_or_else(|| "NetworkManager is not available".to_string())
    }

    /// Connect to a visible Wi-Fi network.
    ///
    /// `password` is only needed for a secured network without a saved
    /// profile; when one exists the profile is activated by UUID so
    /// NetworkManager reuses the stored secret. PSK requests are upgraded to
    /// SAE by `nmrs` when the access point is WPA3-only.
    pub async fn connect(
        &self,
        ssid: &str,
        interface: Option<&str>,
        password: Option<&str>,
    ) -> Result<(), String> {
        use nmrs::WifiSecurity;

        let nm = self.nm().await?;
        let status = self.status();
        let network = status
            .networks
            .iter()
            .find(|net| net.ssid == ssid && interface.is_none_or(|iface| net.interface == iface))
            .ok_or_else(|| format!("{ssid} is not in the current scan — scan again"))?;
        let enterprise = network.security.contains("802.1X")
            || network.security.contains("Enterprise");

        if let Some(password) = password {
            if enterprise {
                return Err(enterprise_hint());
            }
            let credentials = if network.secured {
                WifiSecurity::WpaPsk {
                    psk: password.to_string(),
                }
            } else {
                WifiSecurity::Open
            };
            nm.connect(ssid, interface, credentials)
                .await
                .map_err(|err| connect_error(&err))?;
        } else if let Some(saved) = status.saved.iter().find(|saved| saved.ssid == ssid) {
            nm.connect_by_uuid(&saved.uuid, nmrs::ConnectByUuidConfig::default())
                .await
                .map_err(|err| connect_error(&err))?;
        } else if enterprise {
            return Err(enterprise_hint());
        } else if network.secured {
            return Err("this network requires a password".to_string());
        } else {
            nm.connect(ssid, interface, WifiSecurity::Open)
                .await
                .map_err(|err| connect_error(&err))?;
        }

        self.refresh().await;
        Ok(())
    }

    /// Disconnect the active Wi-Fi link.
    pub async fn disconnect(&self) -> Result<(), String> {
        let nm = self.nm().await?;
        let interface = {
            let status = self.status();
            status
                .connection
                .as_ref()
                .filter(|link| link.kind == "wifi")
                .map(|link| link.interface.clone())
                .or_else(|| status.wifi.interfaces.first().cloned())
        };
        nm.disconnect(interface.as_deref())
            .await
            .map_err(|err| connect_error(&err))?;
        self.refresh().await;
        Ok(())
    }

    /// Delete a saved profile by UUID.
    pub async fn forget(&self, uuid: &str) -> Result<(), String> {
        let nm = self.nm().await?;
        nm.delete_saved_connection(uuid)
            .await
            .map_err(|err| err.to_string())?;
        self.refresh().await;
        Ok(())
    }

    /// Turn the Wi-Fi radio on or off (software switch).
    pub async fn set_wifi_enabled(&self, enabled: bool) -> Result<(), String> {
        let nm = self.nm().await?;
        nm.set_wireless_enabled(enabled)
            .await
            .map_err(|err| err.to_string())?;
        self.refresh().await;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn enterprise_hint() -> String {
    "enterprise networks (802.1X) need a username and certificate — not supported yet".to_string()
}

/// Friendly text for the failures a user can actually hit.
#[cfg(target_os = "linux")]
fn connect_error(err: &nmrs::ConnectionError) -> String {
    use nmrs::ConnectionError as E;
    match err {
        E::AuthFailed => "incorrect password".to_string(),
        E::MissingPassword => "a password is required for this network".to_string(),
        E::NotFound => "network not found — it may be out of range".to_string(),
        E::Timeout => "the connection timed out".to_string(),
        E::DhcpFailed => "the network did not assign an address (DHCP failed)".to_string(),
        E::NoWifiDevice => "no Wi-Fi device is available".to_string(),
        E::WifiNotReady => "the Wi-Fi device is not ready yet".to_string(),
        other => other.to_string(),
    }
}

/// One snapshot from NetworkManager. Several calls rather than
/// `NetworkManager::snapshot()` because we only need this subset and want each
/// field's fallback to be explicit.
#[cfg(target_os = "linux")]
async fn collect(nm: &nmrs::NetworkManager) -> nmrs::Result<NetworkStatus> {
    use nmrs::DeviceState;

    let radio = nm.wifi_state().await?;
    let connectivity = nm.connectivity_report().await?;
    let wireless = nm.list_wireless_devices().await?;
    let wired = nm.list_wired_devices().await?;
    let current = nm.current_network().await?;
    let networks = nm.list_networks(None).await?;
    let saved = nm.list_saved_connections().await?;

    let kernels = kernel_links();

    let wifi_link = current.as_ref().map(|net| ActiveLink {
        kind: "wifi".into(),
        interface: net.device.clone(),
        ssid: Some(net.ssid.clone()),
        bssid: net.bssid.clone(),
        strength: net.strength,
        level: signal_level(net.strength.unwrap_or(0)),
        frequency_mhz: net.frequency,
        band: net.frequency.map(band_label),
        security: security_label(&net.security_features),
        secured: net.secured,
        known: net.known,
        ip4: net.ip4_address.clone(),
        ip6: net.ip6_address.clone(),
        speed_mbps: None,
        managed: true,
    });

    let wired_link = wired
        .iter()
        .find(|device| matches!(device.state, DeviceState::Activated))
        .map(|device| ActiveLink {
            kind: "ethernet".into(),
            interface: device.interface.clone(),
            ssid: None,
            bssid: None,
            strength: None,
            level: 0,
            frequency_mhz: None,
            band: None,
            security: String::new(),
            secured: false,
            known: true,
            ip4: device.ip4_address.clone(),
            ip6: device.ip6_address.clone(),
            speed_mbps: device.speed_mbps,
            managed: device.managed.unwrap_or(true),
        });

    // NetworkManager knows nothing about an ifupdown-managed uplink; fall back
    // to the kernel so the chip does not say "not connected" on a machine that
    // is plainly online.
    let kernel_uplink = kernels
        .iter()
        .filter(|link| !link.wireless)
        .find(|link| link.default_route && link.up)
        .or_else(|| kernels.iter().find(|link| link.up && link.ip4.is_some()))
        .map(|link| ActiveLink {
            kind: "ethernet".into(),
            interface: link.interface.clone(),
            ssid: None,
            bssid: None,
            strength: None,
            level: 0,
            frequency_mhz: None,
            band: None,
            security: String::new(),
            secured: false,
            known: true,
            ip4: link.ip4.clone(),
            ip6: link.ip6.clone(),
            speed_mbps: link.speed_mbps,
            managed: false,
        });

    let mut visible: Vec<VisibleNetwork> = networks
        .iter()
        .map(|net| VisibleNetwork {
            ssid: net.ssid.clone(),
            interface: net.device.clone(),
            bssid: net.bssid.clone(),
            strength: net.strength.unwrap_or(0),
            level: signal_level(net.strength.unwrap_or(0)),
            frequency_mhz: net.frequency,
            band: net.frequency.map(band_label),
            secured: net.secured,
            security: security_label(&net.security_features),
            known: net.known,
            active: net.is_active,
            hidden: net.ssid.is_empty(),
        })
        .collect();
    // Connected first, then known networks, then the strongest.
    visible.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then(b.known.cmp(&a.known))
            .then(b.strength.cmp(&a.strength))
            .then(a.ssid.cmp(&b.ssid))
    });

    let saved: Vec<SavedNetwork> = saved
        .iter()
        .filter_map(|profile| match &profile.summary {
            nmrs::SettingsSummary::Wifi { ssid, .. } => Some(SavedNetwork {
                uuid: profile.uuid.clone(),
                id: profile.id.clone(),
                ssid: ssid.clone(),
                interface: profile.interface_name.clone(),
                autoconnect: profile.autoconnect,
                last_used_unix: profile.timestamp_unix,
            }),
            _ => None,
        })
        .collect();

    Ok(NetworkStatus {
        available: true,
        reason: None,
        updated_at: now(),
        wifi: WifiRadioStatus {
            present: radio.present,
            enabled: radio.enabled,
            hardware_enabled: radio.hardware_enabled,
            interfaces: wireless.iter().map(|d| d.interface.clone()).collect(),
        },
        connectivity: ConnectivityStatus {
            state: format!("{:?}", connectivity.state).to_lowercase(),
            internet: connectivity.state.is_usable_for_internet(),
            captive_portal_url: connectivity.captive_portal_url.clone(),
        },
        connection: wifi_link.or(wired_link).or(kernel_uplink),
        wired: wired
            .iter()
            .map(|device| WiredLink {
                interface: device.interface.clone(),
                state: device_state_label(&device.state).to_string(),
                connected: matches!(device.state, DeviceState::Activated),
                ip4: device
                    .ip4_address
                    .clone()
                    .or_else(|| kernel_address(&kernels, &device.interface, false)),
                speed_mbps: device
                    .speed_mbps
                    .or_else(|| kernel_speed(&kernels, &device.interface)),
                managed: device.managed.unwrap_or(true),
            })
            .collect(),
        links: kernels,
        networks: visible,
        saved,
    })
}

/// Interface list + addresses from the kernel, with the default route marked.
#[cfg(target_os = "linux")]
fn kernel_links() -> Vec<KernelLink> {
    use if_addrs::{get_if_addrs, IfAddr, IfOperStatus};

    let default_route = default_route_interface();
    let mut links: Vec<KernelLink> = Vec::new();

    for iface in get_if_addrs().unwrap_or_default() {
        if iface.is_loopback() {
            continue;
        }
        let index = match links.iter().position(|link| link.interface == iface.name) {
            Some(index) => index,
            None => {
                links.push(KernelLink {
                    up: matches!(iface.oper_status, IfOperStatus::Up),
                    default_route: default_route.as_deref() == Some(iface.name.as_str()),
                    speed_mbps: sys_read_u32(&format!("/sys/class/net/{}/speed", iface.name)),
                    wireless: std::path::Path::new(&format!(
                        "/sys/class/net/{}/wireless",
                        iface.name
                    ))
                    .exists(),
                    interface: iface.name.clone(),
                    ip4: None,
                    ip6: None,
                });
                links.len() - 1
            }
        };
        let link = &mut links[index];
        match &iface.addr {
            IfAddr::V4(v4) if link.ip4.is_none() => {
                link.ip4 = Some(format!("{}/{}", v4.ip, v4.prefixlen));
            }
            IfAddr::V6(v6) if link.ip6.is_none() && !v6.ip.is_unspecified() => {
                link.ip6 = Some(format!("{}/{}", v6.ip, v6.prefixlen));
            }
            _ => {}
        }
    }

    links
}

/// The interface carrying the default route, from `/proc/net/route` (hex,
/// little-endian, one line per route; destination `00000000` = default).
#[cfg(target_os = "linux")]
fn default_route_interface() -> Option<String> {
    let table = std::fs::read_to_string("/proc/net/route").ok()?;
    default_route_from(&table)
}

/// Split out from the file read so the parse is unit-testable.
#[cfg(target_os = "linux")]
fn default_route_from(table: &str) -> Option<String> {
    let mut best: Option<(u32, String)> = None;
    for line in table.lines().skip(1) {
        let mut fields = line.split_whitespace();
        let (Some(interface), Some(destination), Some(_gateway), Some(flags), Some(_refcnt),
             Some(_use), Some(metric)) =
            (fields.next(), fields.next(), fields.next(), fields.next(), fields.next(),
             fields.next(), fields.next())
        else {
            continue;
        };
        let (Ok(flags), Ok(metric)) = (u32::from_str_radix(flags, 16), metric.parse::<u32>())
        else {
            continue;
        };
        // RTF_UP without RTF_GATEWAY is a rejected route.
        if destination != "00000000" || flags & 0x1 == 0 || flags & 0x2 == 0 {
            continue;
        }
        if best.as_ref().is_none_or(|(best_metric, _)| metric < *best_metric) {
            best = Some((metric, interface.to_string()));
        }
    }
    best.map(|(_, interface)| interface)
}

#[cfg(target_os = "linux")]
fn kernel_address(links: &[KernelLink], interface: &str, ipv6: bool) -> Option<String> {
    links
        .iter()
        .find(|link| link.interface == interface)
        .and_then(|link| if ipv6 { link.ip6.clone() } else { link.ip4.clone() })
}

#[cfg(target_os = "linux")]
fn kernel_speed(links: &[KernelLink], interface: &str) -> Option<u32> {
    links
        .iter()
        .find(|link| link.interface == interface)
        .and_then(|link| link.speed_mbps)
}

/// Read a single integer from a sysfs attribute (e.g. link speed). Drivers
/// that do not support it return `-1` or an error — both mean "unknown".
#[cfg(target_os = "linux")]
fn sys_read_u32(path: &str) -> Option<u32> {
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse::<i64>()
        .ok()
        .and_then(|value| u32::try_from(value).ok())
}

#[cfg(target_os = "linux")]
fn band_label(frequency_mhz: u32) -> String {
    match frequency_mhz {
        f if f >= 5925 => "6 GHz".into(),
        f if f >= 4900 => "5 GHz".into(),
        _ => "2.4 GHz".into(),
    }
}

/// Short, honest security label for a row/detail line. NetworkManager's flags
/// do not distinguish WPA1 from WPA2 beyond CCMP presence, so mixed-mode
/// networks read `WPA/WPA2` and PSK+SAE reads `WPA2/WPA3`.
#[cfg(target_os = "linux")]
fn security_label(features: &nmrs::SecurityFeatures) -> String {
    if features.eap_suite_b_192 {
        return "WPA3-Enterprise".into();
    }
    if features.eap {
        return "802.1X".into();
    }
    if features.owe || features.owe_transition_mode {
        return "OWE".into();
    }
    if features.wep40 || features.wep104 {
        return "WEP".into();
    }
    if features.sae {
        return if features.psk { "WPA2/WPA3" } else { "WPA3" }.into();
    }
    if features.psk {
        return if features.ccmp { "WPA2" } else { "WPA/WPA2" }.into();
    }
    if features.privacy {
        return "Secured".into();
    }
    "Open".into()
}

#[cfg(target_os = "linux")]
fn device_state_label(state: &nmrs::DeviceState) -> &'static str {
    use nmrs::DeviceState as S;
    match state {
        S::Unmanaged => "unmanaged",
        S::Unavailable => "unavailable",
        S::Disconnected => "disconnected",
        S::Prepare => "preparing",
        S::Config => "configuring",
        S::NeedAuth => "authenticating",
        S::IpConfig => "configuring address",
        S::IpCheck => "checking address",
        S::Secondaries => "connecting secondaries",
        S::Activated => "connected",
        S::Deactivating => "disconnecting",
        S::Failed => "failed",
        _ => "unknown",
    }
}

/* ── Not Linux: inert stub ─────────────────────────────────── */

#[cfg(not(target_os = "linux"))]
impl NetworkService {
    /// No-op: `NetworkStatus::unavailable(START_REASON)` already explains why.
    pub async fn start(&self) {}

    pub async fn refresh(&self) {}

    pub async fn scan(&self) -> Result<(), String> {
        Err(START_REASON.to_string())
    }

    pub async fn connect(
        &self,
        _ssid: &str,
        _interface: Option<&str>,
        _password: Option<&str>,
    ) -> Result<(), String> {
        Err(START_REASON.to_string())
    }

    pub async fn disconnect(&self) -> Result<(), String> {
        Err(START_REASON.to_string())
    }

    pub async fn forget(&self, _uuid: &str) -> Result<(), String> {
        Err(START_REASON.to_string())
    }

    pub async fn set_wifi_enabled(&self, _enabled: bool) -> Result<(), String> {
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
    use super::signal_level;
    #[cfg(target_os = "linux")]
    use super::default_route_from;

    #[test]
    fn signal_levels_match_gnome_buckets() {
        // NetworkManager reports 0–100; the boundaries are GNOME Shell's, so
        // 5 is "no bars", 6 lights the first, 81 the fourth.
        assert_eq!(signal_level(0), 0);
        assert_eq!(signal_level(5), 0);
        assert_eq!(signal_level(6), 1);
        assert_eq!(signal_level(30), 1);
        assert_eq!(signal_level(31), 2);
        assert_eq!(signal_level(55), 2);
        assert_eq!(signal_level(56), 3);
        assert_eq!(signal_level(80), 3);
        assert_eq!(signal_level(81), 4);
        assert_eq!(signal_level(100), 4);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn default_route_picks_the_real_uplink() {
        // Real `/proc/net/route` layout: hex, little-endian, one route per
        // line; a link-scope route and a rejected (not-RTF_UP) default must
        // not win over the actual default route.
        let table = "\
Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
enx0\t00000000\t0A72002B\t0003\t0\t0\t1002\t00000000\t0\t0\t0
wlp5s0\t00000000\t00000000\t0001\t0\t0\t600\t00000000\t0\t0\t0
enx0\t0A720000\t00000000\t0001\t0\t0\t1002\t00FFFFFF\t0\t0\t0
";
        assert_eq!(default_route_from(table).as_deref(), Some("enx0"));

        // Lower metric wins when two default routes are present.
        let two = "\
Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
wlp5s0\t00000000\t0A72002B\t0003\t0\t0\t600\t00000000\t0\t0\t0
enx0\t00000000\t0A72002B\t0003\t0\t0\t1002\t00000000\t0\t0\t0
";
        assert_eq!(default_route_from(two).as_deref(), Some("wlp5s0"));
    }
}
