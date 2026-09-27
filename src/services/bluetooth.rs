//! Host Bluetooth state for the top-bar chip and the Bluetooth menu — the
//! adapter, its power and discovery state, and the devices BlueZ knows about.
//!
//! Like `network.rs` and `audio.rs`, this belongs to the machine rather than a
//! traveler: it is core chrome (a sibling of `hudNetwork.js` / `hudAudio.js`)
//! and follows the same contract — probe, degrade quietly when the daemon is
//! absent, cache one snapshot, and broadcast changes.
//!
//! The backend is **BlueZ over the system D-Bus** (via `zbus`). The object
//! manager hands back every adapter and device in one call, so a refresh is a
//! single round trip; the service re-reads on a short interval and publishes
//! only when something actually changed. Pairing registers a small
//! `NoInputNoOutput` agent so "just works" devices can complete without a PIN
//! prompt — the right default for a kiosk.
//!
//! `/api/bluetooth/status` reads the cache, `/api/bluetooth/events` relays the
//! broadcast as SSE, and mutations (power, scan, pair/connect/disconnect,
//! forget) are accepted only from the local machine.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use serde::Serialize;
use tokio::sync::broadcast;

#[cfg(target_os = "linux")]
use std::collections::HashMap;
#[cfg(target_os = "linux")]
use tokio::sync::Mutex;
#[cfg(target_os = "linux")]
use zbus::names::OwnedInterfaceName;
#[cfg(target_os = "linux")]
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

/// BlueZ is local and cheap to read; a short poll keeps the chip honest
/// without wiring up per-object D-Bus signal plumbing.
#[cfg(target_os = "linux")]
const REFRESH: Duration = Duration::from_secs(3);
/// Re-probe cadence when BlueZ is absent (no adapter, or `bluetoothd` down).
const RETRY: Duration = Duration::from_secs(30);
/// How long a discovery scan runs before it is stopped automatically.
#[cfg(target_os = "linux")]
const SCAN_WINDOW: Duration = Duration::from_secs(15);
/// A Bluetooth action (pair, connect) can involve the peer device; cap it.
#[cfg(target_os = "linux")]
const COMMAND_TIMEOUT: Duration = Duration::from_secs(45);
const BROADCAST_CAPACITY: usize = 64;

#[cfg(target_os = "linux")]
const START_REASON: &str = "BlueZ is not reachable";
#[cfg(not(target_os = "linux"))]
const START_REASON: &str = "the Bluetooth panel requires Linux (BlueZ)";

/// One Bluetooth device BlueZ knows about (paired or merely seen).
#[derive(Clone, Debug, Serialize)]
pub struct BluetoothDevice {
    /// The BlueZ D-Bus object path; the opaque handle used by every action.
    pub id: String,
    pub address: String,
    /// BlueZ `Alias`, falling back to `Name`, then the address.
    pub name: String,
    /// BlueZ's icon name (e.g. `audio-headset`), when it has one.
    pub icon: String,
    /// Coarse bucket derived from [`device_kind`], for picking an icon.
    pub kind: String,
    pub paired: bool,
    pub trusted: bool,
    pub connected: bool,
    pub blocked: bool,
    pub rssi: Option<i16>,
    pub battery: Option<u8>,
}

/// The whole snapshot the chip and the menu render from.
#[derive(Clone, Debug, Serialize)]
pub struct BluetoothStatus {
    pub available: bool,
    pub reason: Option<String>,
    pub updated_at: String,
    /// True when at least one Bluetooth adapter exists on the machine.
    pub present: bool,
    pub powered: bool,
    pub discoverable: bool,
    pub pairable: bool,
    pub discovering: bool,
    /// The adapter's name, when there is one.
    pub adapter: Option<String>,
    pub devices: Vec<BluetoothDevice>,
}

impl BluetoothStatus {
    fn unavailable(reason: &str) -> Self {
        Self {
            available: false,
            reason: Some(reason.to_string()),
            updated_at: now(),
            present: false,
            powered: false,
            discoverable: false,
            pairable: false,
            discovering: false,
            adapter: None,
            devices: Vec::new(),
        }
    }
}

/// Map a BlueZ `Icon`/class hint to the small set of buckets the UI draws.
/// Unknown devices fall back to a generic Bluetooth glyph.
pub fn device_kind(icon: &str) -> &'static str {
    let icon = icon.to_ascii_lowercase();
    if icon.contains("headset") {
        "headset"
    } else if icon.contains("headphone") {
        "headphones"
    } else if icon.contains("speaker") {
        "speaker"
    } else if icon.contains("keyboard") {
        "keyboard"
    } else if icon.contains("mouse") {
        "mouse"
    } else if icon.contains("phone") {
        "phone"
    } else if icon.contains("computer") {
        "computer"
    } else if icon.contains("audio") {
        "audio"
    } else {
        "other"
    }
}

#[derive(Clone)]
pub struct BluetoothService {
    inner: Arc<Inner>,
}

struct Inner {
    status: ArcSwap<BluetoothStatus>,
    events: broadcast::Sender<Arc<BluetoothStatus>>,
    started: AtomicBool,
    #[cfg(target_os = "linux")]
    /// The live system-bus connection the reader thread owns, so mutations and
    /// the registered pairing agent share it.
    connection: Mutex<Option<zbus::Connection>>,
    /// Serializes mutations so two actions cannot interleave.
    #[cfg(target_os = "linux")]
    command: Mutex<()>,
    /// Last published JSON, so the poll does not re-broadcast an unchanged
    /// snapshot to every SSE client.
    #[cfg(target_os = "linux")]
    last_json: std::sync::Mutex<String>,
}

impl BluetoothService {
    #[must_use]
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            inner: Arc::new(Inner {
                status: ArcSwap::from_pointee(BluetoothStatus::unavailable(START_REASON)),
                events,
                started: AtomicBool::new(false),
                #[cfg(target_os = "linux")]
                connection: Mutex::new(None),
                #[cfg(target_os = "linux")]
                command: Mutex::new(()),
                #[cfg(target_os = "linux")]
                last_json: std::sync::Mutex::new(String::new()),
            }),
        }
    }

    /// Latest snapshot; never touches D-Bus.
    #[must_use]
    pub fn status(&self) -> Arc<BluetoothStatus> {
        self.inner.status.load_full()
    }

    /// Subscribe to snapshots, for the SSE relay.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<BluetoothStatus>> {
        self.inner.events.subscribe()
    }

    #[must_use]
    pub fn is_available(&self) -> bool {
        self.status().available
    }

    fn publish(&self, status: BluetoothStatus) {
        let shared = Arc::new(status);
        self.inner.status.store(shared.clone());
        let _ = self.inner.events.send(shared);
    }

    /// Publish only when the snapshot changed. The reader polls on an interval,
    /// so this is what keeps idle SSE clients from being woken for nothing.
    #[cfg(target_os = "linux")]
    fn publish_if_changed(&self, status: BluetoothStatus) {
        let json = serde_json::to_string(&status).unwrap_or_default();
        {
            let mut last = self.inner.last_json.lock().unwrap_or_else(|e| e.into_inner());
            if *last == json {
                return;
            }
            *last = json;
        }
        self.publish(status);
    }
}

impl Default for BluetoothService {
    fn default() -> Self {
        Self::new()
    }
}

/* ── Linux: BlueZ over the system D-Bus ────────────────────── */

#[cfg(target_os = "linux")]
impl BluetoothService {
    /// Spawn the reader. Returns immediately: a machine without BlueZ or a
    /// Bluetooth adapter boots exactly as it did before.
    pub async fn start(&self) {
        if self.inner.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let service = self.clone();
        tokio::spawn(async move { service.run().await });
    }

    async fn run(self) {
        loop {
            match zbus::Connection::system().await {
                Ok(connection) => {
                    *self.inner.connection.lock().await = Some(connection.clone());
                    if let Err(err) = register_agent(&connection).await {
                        tracing::warn!("bluetooth: pairing agent unavailable: {err}");
                    }
                    loop {
                        match collect(&connection).await {
                            Ok(status) => self.publish_if_changed(status),
                            Err(err) => {
                                self.publish_if_changed(BluetoothStatus::unavailable(&err));
                                break;
                            }
                        }
                        tokio::time::sleep(REFRESH).await;
                    }
                    *self.inner.connection.lock().await = None;
                }
                Err(err) => {
                    self.publish_if_changed(BluetoothStatus::unavailable(&format!(
                        "BlueZ is not reachable ({err})"
                    )));
                }
            }
            tokio::time::sleep(RETRY).await;
        }
    }

    /// The shared system-bus connection, opened on demand if the reader has
    /// not established one yet.
    async fn connection(&self) -> Result<zbus::Connection, String> {
        if let Some(connection) = self.inner.connection.lock().await.clone() {
            return Ok(connection);
        }
        zbus::Connection::system().await.map_err(|err| err.to_string())
    }

    /// Force a snapshot so a mutation is reflected without waiting for the poll.
    pub async fn refresh(&self) {
        match self.connection().await {
            Ok(connection) => match collect(&connection).await {
                Ok(status) => self.publish_if_changed(status),
                Err(err) => self.publish_if_changed(BluetoothStatus::unavailable(&err)),
            },
            Err(err) => self.publish_if_changed(BluetoothStatus::unavailable(&err)),
        }
    }

    /// Power the adapter on or off.
    pub async fn set_power(&self, on: bool) -> Result<(), String> {
        let _command = self.inner.command.lock().await;
        let connection = self.connection().await?;
        let adapter = adapter_path(&connection).await?;
        let proxy = adapter_proxy(&connection, &adapter).await?;
        proxy
            .set_property("Powered", on)
            .await
            .map_err(|err| friendly(err.to_string()))?;
        drop(_command);
        self.refresh().await;
        Ok(())
    }

    /// Start a discovery scan; it stops by itself after [`SCAN_WINDOW`].
    pub async fn scan(&self) -> Result<(), String> {
        let _command = self.inner.command.lock().await;
        let connection = self.connection().await?;
        let adapter = adapter_path(&connection).await?;
        let proxy = adapter_proxy(&connection, &adapter).await?;
        proxy
            .call_method("StartDiscovery", &())
            .await
            .map_err(|err| friendly(err.to_string()))?;
        drop(_command);

        let connection = connection.clone();
        tokio::spawn(async move {
            tokio::time::sleep(SCAN_WINDOW).await;
            if let Ok(proxy) = adapter_proxy(&connection, &adapter).await {
                let _ = proxy.call_method("StopDiscovery", &()).await;
            }
        });
        self.refresh().await;
        Ok(())
    }

    /// Pair, trust and (best-effort) connect a device.
    pub async fn pair(&self, id: &str) -> Result<(), String> {
        let path = device_path(id)?;
        let _command = self.inner.command.lock().await;
        let connection = self.connection().await?;
        let proxy = device_proxy(&connection, &path).await?;
        call_with_timeout(&proxy, "Pair").await?;
        let _ = proxy.set_property("Trusted", true).await;
        let _ = call_with_timeout(&proxy, "Connect").await;
        drop(_command);
        self.refresh().await;
        Ok(())
    }

    /// Connect an already-paired device.
    pub async fn connect(&self, id: &str) -> Result<(), String> {
        let path = device_path(id)?;
        let _command = self.inner.command.lock().await;
        let connection = self.connection().await?;
        let proxy = device_proxy(&connection, &path).await?;
        call_with_timeout(&proxy, "Connect").await.map_err(friendly)?;
        drop(_command);
        self.refresh().await;
        Ok(())
    }

    /// Disconnect a device without forgetting it.
    pub async fn disconnect(&self, id: &str) -> Result<(), String> {
        let path = device_path(id)?;
        let _command = self.inner.command.lock().await;
        let connection = self.connection().await?;
        let proxy = device_proxy(&connection, &path).await?;
        call_with_timeout(&proxy, "Disconnect").await.map_err(friendly)?;
        drop(_command);
        self.refresh().await;
        Ok(())
    }

    /// Forget a device entirely (removes the pairing).
    pub async fn forget(&self, id: &str) -> Result<(), String> {
        let path = device_path(id)?;
        let _command = self.inner.command.lock().await;
        let connection = self.connection().await?;
        let adapter = adapter_path(&connection).await?;
        let proxy = adapter_proxy(&connection, &adapter).await?;
        let object = zbus::zvariant::ObjectPath::try_from(path.as_str())
            .map_err(|err| err.to_string())?;
        proxy
            .call_method("RemoveDevice", &(object,))
            .await
            .map_err(|err| friendly(err.to_string()))?;
        drop(_command);
        self.refresh().await;
        Ok(())
    }

    /// Mark a device trusted so it reconnects without prompting.
    pub async fn set_trusted(&self, id: &str, trusted: bool) -> Result<(), String> {
        let path = device_path(id)?;
        let _command = self.inner.command.lock().await;
        let connection = self.connection().await?;
        let proxy = device_proxy(&connection, &path).await?;
        proxy
            .set_property("Trusted", trusted)
            .await
            .map_err(|err| friendly(err.to_string()))?;
        drop(_command);
        self.refresh().await;
        Ok(())
    }
}

/* ── Linux: BlueZ helpers ──────────────────────────────────── */

#[cfg(target_os = "linux")]
type Interfaces = HashMap<OwnedInterfaceName, HashMap<String, OwnedValue>>;

#[cfg(target_os = "linux")]
fn iface<'a>(interfaces: &'a Interfaces, name: &str) -> Option<&'a HashMap<String, OwnedValue>> {
    interfaces
        .iter()
        .find(|(key, _)| key.as_str() == name)
        .map(|(_, props)| props)
}

#[cfg(target_os = "linux")]
fn prop_bool(props: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    props.get(key).and_then(|value| bool::try_from(value).ok())
}

#[cfg(target_os = "linux")]
fn prop_string(props: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    props
        .get(key)
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| String::try_from(value).ok())
}

#[cfg(target_os = "linux")]
fn prop_i16(props: &HashMap<String, OwnedValue>, key: &str) -> Option<i16> {
    props.get(key).and_then(|value| i16::try_from(value).ok())
}

#[cfg(target_os = "linux")]
fn prop_u8(props: &HashMap<String, OwnedValue>, key: &str) -> Option<u8> {
    props.get(key).and_then(|value| u8::try_from(value).ok())
}

/// Read every adapter and device in one object-manager call.
#[cfg(target_os = "linux")]
async fn collect(connection: &zbus::Connection) -> Result<BluetoothStatus, String> {
    let manager = zbus::fdo::ObjectManagerProxy::new(connection, "org.bluez", "/")
        .await
        .map_err(|err| err.to_string())?;
    let objects: HashMap<OwnedObjectPath, Interfaces> = manager
        .get_managed_objects()
        .await
        .map_err(|err| err.to_string())?;

    let mut adapter_name: Option<String> = None;
    let mut present = false;
    let mut powered = false;
    let mut discoverable = false;
    let mut pairable = false;
    let mut discovering = false;

    let mut devices: Vec<BluetoothDevice> = Vec::new();
    for (path, interfaces) in &objects {
        if let Some(props) = iface(interfaces, "org.bluez.Adapter1") {
            present = true;
            powered |= prop_bool(props, "Powered").unwrap_or(false);
            discoverable |= prop_bool(props, "Discoverable").unwrap_or(false);
            pairable |= prop_bool(props, "Pairable").unwrap_or(false);
            discovering |= prop_bool(props, "Discovering").unwrap_or(false);
            if adapter_name.is_none() {
                adapter_name = prop_string(props, "Alias")
                    .or_else(|| prop_string(props, "Name"))
                    .or_else(|| prop_string(props, "Address"));
            }
        }

        if let Some(props) = iface(interfaces, "org.bluez.Device1") {
            let address = prop_string(props, "Address").unwrap_or_default();
            let name = prop_string(props, "Alias")
                .or_else(|| prop_string(props, "Name"))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| address.clone());
            let icon = prop_string(props, "Icon").unwrap_or_default();
            let battery = iface(interfaces, "org.bluez.Battery1")
                .and_then(|battery| prop_u8(battery, "Percentage"));
            devices.push(BluetoothDevice {
                id: path.as_str().to_string(),
                address,
                name,
                kind: device_kind(&icon).to_string(),
                icon,
                paired: prop_bool(props, "Paired").unwrap_or(false),
                trusted: prop_bool(props, "Trusted").unwrap_or(false),
                connected: prop_bool(props, "Connected").unwrap_or(false),
                blocked: prop_bool(props, "Blocked").unwrap_or(false),
                rssi: prop_i16(props, "RSSI"),
                battery,
            });
        }
    }

    // Connected first, then paired, then strongest signal, then name.
    devices.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.paired.cmp(&a.paired))
            .then(b.rssi.cmp(&a.rssi))
            .then(a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
    });

    Ok(BluetoothStatus {
        available: true,
        reason: None,
        updated_at: now(),
        present,
        powered,
        discoverable,
        pairable,
        discovering,
        adapter: adapter_name,
        devices,
    })
}

/// The first adapter's object path, or a friendly error when there is none.
#[cfg(target_os = "linux")]
async fn adapter_path(connection: &zbus::Connection) -> Result<String, String> {
    let manager = zbus::fdo::ObjectManagerProxy::new(connection, "org.bluez", "/")
        .await
        .map_err(|err| err.to_string())?;
    let objects = manager
        .get_managed_objects()
        .await
        .map_err(|err| err.to_string())?;
    for (path, interfaces) in &objects {
        if iface(interfaces, "org.bluez.Adapter1").is_some() {
            return Ok(path.as_str().to_string());
        }
    }
    Err("no Bluetooth adapter is present".to_string())
}

#[cfg(target_os = "linux")]
async fn adapter_proxy<'a>(
    connection: &'a zbus::Connection,
    path: &'a str,
) -> Result<zbus::Proxy<'a>, String> {
    zbus::Proxy::new(connection, "org.bluez", path, "org.bluez.Adapter1")
        .await
        .map_err(|err| err.to_string())
}

#[cfg(target_os = "linux")]
async fn device_proxy<'a>(
    connection: &'a zbus::Connection,
    path: &'a str,
) -> Result<zbus::Proxy<'a>, String> {
    zbus::Proxy::new(connection, "org.bluez", path, "org.bluez.Device1")
        .await
        .map_err(|err| err.to_string())
}

/// Validate a device handle came from BlueZ before calling into D-Bus.
#[cfg(target_os = "linux")]
fn device_path(id: &str) -> Result<String, String> {
    if id.starts_with("/org/bluez/") && id.contains("/dev_") {
        Ok(id.to_string())
    } else {
        Err("unknown Bluetooth device".to_string())
    }
}

/// Call a device method with a cap, so a wedged peer cannot pin a request.
#[cfg(target_os = "linux")]
async fn call_with_timeout(proxy: &zbus::Proxy<'_>, method: &str) -> Result<(), String> {
    match tokio::time::timeout(COMMAND_TIMEOUT, proxy.call_method(method, &())).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(err)) => Err(err.to_string()),
        Err(_) => Err(format!("{method} timed out")),
    }
}

/// Turn BlueZ's terse D-Bus errors into something a person can read.
#[cfg(target_os = "linux")]
fn friendly(message: String) -> String {
    if message.contains("InProgress") {
        "the device is busy — try again in a moment".to_string()
    } else if message.contains("AlreadyConnected") {
        "already connected".to_string()
    } else if message.contains("AuthenticationFailed")
        || message.contains("AuthenticationCanceled")
    {
        "authentication failed".to_string()
    } else if message.contains("NotReady") {
        "the Bluetooth adapter is off".to_string()
    } else {
        message
    }
}

/// BlueZ `Agent1`: auto-confirm "just works" pairing, which is all a kiosk
/// needs. PIN and passkey entry are not offered.
#[cfg(target_os = "linux")]
struct PairingAgent;

#[cfg(target_os = "linux")]
#[zbus::interface(name = "org.bluez.Agent1")]
impl PairingAgent {
    async fn release(&self) {}

    async fn request_confirmation(
        &self,
        _device: zbus::zvariant::ObjectPath<'_>,
        _passkey: u32,
    ) -> zbus::fdo::Result<()> {
        Ok(())
    }

    async fn request_authorization(
        &self,
        _device: zbus::zvariant::ObjectPath<'_>,
    ) -> zbus::fdo::Result<()> {
        Ok(())
    }

    async fn authorize_service(
        &self,
        _device: zbus::zvariant::ObjectPath<'_>,
        _uuid: String,
    ) -> zbus::fdo::Result<()> {
        Ok(())
    }

    async fn cancel(&self) {}

    async fn request_pin_code(
        &self,
        _device: zbus::zvariant::ObjectPath<'_>,
    ) -> zbus::fdo::Result<String> {
        Err(zbus::fdo::Error::NotSupported(
            "PIN entry is not available on this device".into(),
        ))
    }

    async fn request_passkey(
        &self,
        _device: zbus::zvariant::ObjectPath<'_>,
    ) -> zbus::fdo::Result<u32> {
        Err(zbus::fdo::Error::NotSupported(
            "passkey entry is not available on this device".into(),
        ))
    }

    async fn display_pin_code(
        &self,
        _device: zbus::zvariant::ObjectPath<'_>,
        _pincode: String,
    ) -> zbus::fdo::Result<()> {
        Ok(())
    }

    async fn display_passkey(
        &self,
        _device: zbus::zvariant::ObjectPath<'_>,
        _passkey: u32,
        _entered: u16,
    ) -> zbus::fdo::Result<()> {
        Ok(())
    }
}

/// Register the pairing agent with BlueZ. Failing is not fatal: without an
/// agent "just works" devices may still pair, and everything else works.
#[cfg(target_os = "linux")]
async fn register_agent(connection: &zbus::Connection) -> Result<(), String> {
    const AGENT_PATH: &str = "/org/shiny/BluetoothAgent";
    connection
        .object_server()
        .at(AGENT_PATH, PairingAgent)
        .await
        .map_err(|err| err.to_string())?;
    let path = zbus::zvariant::ObjectPath::try_from(AGENT_PATH).map_err(|err| err.to_string())?;
    let manager = zbus::Proxy::new(connection, "org.bluez", "/org/bluez", "org.bluez.AgentManager1")
        .await
        .map_err(|err| err.to_string())?;
    manager
        .call_method("RegisterAgent", &(path.clone(), "NoInputNoOutput"))
        .await
        .map_err(|err| err.to_string())?;
    let _ = manager.call_method("RequestDefaultAgent", &(path,)).await;
    Ok(())
}

/* ── Not Linux: inert stub ─────────────────────────────────── */

#[cfg(not(target_os = "linux"))]
impl BluetoothService {
    /// No-op: `BluetoothStatus::unavailable(START_REASON)` already explains why.
    pub async fn start(&self) {}

    pub async fn refresh(&self) {}

    pub async fn set_power(&self, _on: bool) -> Result<(), String> {
        Err(START_REASON.to_string())
    }

    pub async fn scan(&self) -> Result<(), String> {
        Err(START_REASON.to_string())
    }

    pub async fn pair(&self, _id: &str) -> Result<(), String> {
        Err(START_REASON.to_string())
    }

    pub async fn connect(&self, _id: &str) -> Result<(), String> {
        Err(START_REASON.to_string())
    }

    pub async fn disconnect(&self, _id: &str) -> Result<(), String> {
        Err(START_REASON.to_string())
    }

    pub async fn forget(&self, _id: &str) -> Result<(), String> {
        Err(START_REASON.to_string())
    }

    pub async fn set_trusted(&self, _id: &str, _trusted: bool) -> Result<(), String> {
        Err(START_REASON.to_string())
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::{device_kind, BluetoothService};

    #[test]
    fn device_kinds_match_bluez_icons() {
        assert_eq!(device_kind("audio-headset"), "headset");
        assert_eq!(device_kind("audio-headphones"), "headphones");
        assert_eq!(device_kind("audio-card"), "audio");
        assert_eq!(device_kind("input-keyboard"), "keyboard");
        assert_eq!(device_kind("input-mouse"), "mouse");
        assert_eq!(device_kind("phone"), "phone");
        assert_eq!(device_kind("computer"), "computer");
        assert_eq!(device_kind(""), "other");
        assert_eq!(device_kind("vendor-thing"), "other");
    }

    #[test]
    fn starts_unavailable() {
        let service = BluetoothService::new();
        let status = service.status();
        assert!(!status.available);
        assert!(!service.is_available());
        assert!(status.reason.is_some());
        assert!(status.devices.is_empty());
    }
}
