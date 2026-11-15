//! Host pointer settings — the mouse and touchpad options of Settings.
//!
//! The kiosk session is Xorg with the `libinput` input driver, so every option
//! a desktop offers for a pointer (tap to click, natural scrolling, speed,
//! disable while typing, left-handed buttons…) is an XInput2 **device
//! property** that driver exposes (`libinput Tapping Enabled`, …). The server
//! talks to the X server directly over x11rb — no `xinput` binary, no desktop
//! daemon — the same way the capture plugin finds the display.
//!
//! The user's choice is a small JSON file, `~/.config/liteaura/pointer.json`
//! (`LITEAURA_POINTER_FILE` overrides it), holding only the options the user
//! has touched; anything absent stays at the driver default. A background
//! thread applies the file whenever the X server comes up and again whenever a
//! pointer is plugged in (an XInput2 hierarchy event), because properties live
//! on the device and a replugged mouse starts from its defaults.
//!
//! Settings apply per **class**, not per device: every touchpad gets the
//! touchpad options, every mouse the mouse options. A device is a touchpad
//! when the driver gives it the tapping property (only touch surfaces have
//! one); any other libinput pointer with an acceleration property is a mouse.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Options for every touchpad. `None` leaves the driver default alone.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TouchpadSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tap_to_click: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub natural_scroll: Option<bool>,
    /// libinput acceleration speed, -1.0 (slowest) … 1.0 (fastest).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_while_typing: Option<bool>,
    /// `"areas"` (bottom-right corner is the right button) or `"fingers"`
    /// (two-finger click is the right button).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub click_method: Option<String>,
    /// `"two_finger"` or `"edge"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll_method: Option<String>,
}

/// Options for every mouse.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MouseSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    /// `"adaptive"` (pointer acceleration) or `"flat"` (none).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accel_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub natural_scroll: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left_handed: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PointerSettings {
    pub touchpad: TouchpadSettings,
    pub mouse: MouseSettings,
}

impl PointerSettings {
    /// Merge a partial request (`{ "touchpad": { "speed": 0.3 } }`) into the
    /// stored choice. A `null` value forgets that option (back to the driver
    /// default). Unknown keys and out-of-range values are refused.
    pub fn merge(&mut self, patch: &Value) -> Result<(), String> {
        let obj = patch.as_object().ok_or("expected an object")?;
        for (class, values) in obj {
            let values = values
                .as_object()
                .ok_or_else(|| format!("{class}: expected an object"))?;
            for (key, value) in values {
                match (class.as_str(), key.as_str()) {
                    ("touchpad", "tap_to_click") => self.touchpad.tap_to_click = opt_bool(key, value)?,
                    ("touchpad", "natural_scroll") => self.touchpad.natural_scroll = opt_bool(key, value)?,
                    ("touchpad", "speed") => self.touchpad.speed = opt_speed(key, value)?,
                    ("touchpad", "disable_while_typing") => {
                        self.touchpad.disable_while_typing = opt_bool(key, value)?
                    }
                    ("touchpad", "click_method") => {
                        self.touchpad.click_method = opt_choice(key, value, &["areas", "fingers"])?
                    }
                    ("touchpad", "scroll_method") => {
                        self.touchpad.scroll_method = opt_choice(key, value, &["two_finger", "edge"])?
                    }
                    ("mouse", "speed") => self.mouse.speed = opt_speed(key, value)?,
                    ("mouse", "accel_profile") => {
                        self.mouse.accel_profile = opt_choice(key, value, &["adaptive", "flat"])?
                    }
                    ("mouse", "natural_scroll") => self.mouse.natural_scroll = opt_bool(key, value)?,
                    ("mouse", "left_handed") => self.mouse.left_handed = opt_bool(key, value)?,
                    _ => return Err(format!("unknown option {class}.{key}")),
                }
            }
        }
        Ok(())
    }
}

fn opt_bool(key: &str, value: &Value) -> Result<Option<bool>, String> {
    match value {
        Value::Null => Ok(None),
        Value::Bool(b) => Ok(Some(*b)),
        _ => Err(format!("{key}: expected true or false")),
    }
}

fn opt_speed(key: &str, value: &Value) -> Result<Option<f64>, String> {
    match value {
        Value::Null => Ok(None),
        Value::Number(n) => {
            let v = n.as_f64().filter(|v| v.is_finite()).ok_or_else(|| format!("{key}: not a number"))?;
            if !(-1.0..=1.0).contains(&v) {
                return Err(format!("{key}: must be between -1 and 1"));
            }
            Ok(Some(v))
        }
        _ => Err(format!("{key}: expected a number")),
    }
}

fn opt_choice(key: &str, value: &Value, allowed: &[&str]) -> Result<Option<String>, String> {
    match value {
        Value::Null => Ok(None),
        Value::String(s) if allowed.contains(&s.as_str()) => Ok(Some(s.clone())),
        _ => Err(format!("{key}: expected one of {}", allowed.join(", "))),
    }
}

/// `~/.config/liteaura/pointer.json`, or `$LITEAURA_POINTER_FILE`.
fn settings_file() -> PathBuf {
    if let Some(p) = std::env::var_os("LITEAURA_POINTER_FILE") {
        return PathBuf::from(p);
    }
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    root.join("liteaura").join("pointer.json")
}

fn load_settings(path: &PathBuf) -> PointerSettings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_settings(path: &PathBuf, settings: &PointerSettings) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(settings).unwrap_or_else(|_| "{}".into());
    std::fs::write(&tmp, body).map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("could not replace {}: {e}", path.display()))
}

#[derive(Clone)]
pub struct PointerService {
    path: PathBuf,
    lock: std::sync::Arc<std::sync::Mutex<()>>,
}

impl Default for PointerService {
    fn default() -> Self {
        Self::new()
    }
}

impl PointerService {
    #[must_use]
    pub fn new() -> Self {
        Self { path: settings_file(), lock: Default::default() }
    }

    pub fn settings(&self) -> PointerSettings {
        load_settings(&self.path)
    }

    /// Devices and the values they have now (read back from the X server, so
    /// the page shows what the pointer really does, defaults included).
    pub async fn status(&self) -> Value {
        tokio::task::spawn_blocking(x11::status)
            .await
            .unwrap_or_else(|_| unavailable("pointer probe failed"))
    }

    /// Store a partial change and apply the whole choice to every device.
    pub async fn update(&self, patch: Value) -> Result<Value, String> {
        let svc = self.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = svc.lock.lock().unwrap_or_else(|e| e.into_inner());
            let mut settings = load_settings(&svc.path);
            settings.merge(&patch)?;
            save_settings(&svc.path, &settings)?;
            if let Err(err) = x11::apply(&settings) {
                tracing::warn!("pointer: could not apply settings: {err}");
            }
            Ok(x11::status())
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// Apply the stored choice now and after every hotplug, for as long as the
    /// process runs. A plain thread: the X connection blocks.
    pub fn start(&self) {
        let path = self.path.clone();
        let spawned = std::thread::Builder::new()
            .name("pointer-settings".into())
            .spawn(move || x11::watch(path));
        if let Err(err) = spawned {
            tracing::warn!("pointer: could not start the hotplug watcher: {err}");
        }
    }
}

fn unavailable(reason: &str) -> Value {
    json!({ "available": false, "reason": reason, "touchpad": { "present": false }, "mouse": { "present": false } })
}

#[cfg(target_os = "linux")]
mod x11 {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::Duration;

    use serde_json::{json, Value};
    use x11rb::connection::Connection;
    use x11rb::protocol::xinput::{self, ConnectionExt as _, DeviceType, XIChangePropertyAux, XIGetPropertyItems};
    use x11rb::protocol::xproto::{self, ConnectionExt as _};
    use x11rb::protocol::Event;
    use x11rb::rust_connection::RustConnection;

    use super::{load_settings, unavailable, MouseSettings, PointerSettings, TouchpadSettings};

    const TAPPING: &str = "libinput Tapping Enabled";
    const NATURAL: &str = "libinput Natural Scrolling Enabled";
    const SPEED: &str = "libinput Accel Speed";
    const PROFILE: &str = "libinput Accel Profile Enabled";
    const DWT: &str = "libinput Disable While Typing Enabled";
    const CLICK: &str = "libinput Click Method Enabled";
    const CLICKS_AVAILABLE: &str = "libinput Click Methods Available";
    const SCROLL: &str = "libinput Scroll Method Enabled";
    const SCROLLS_AVAILABLE: &str = "libinput Scroll Methods Available";
    const LEFT: &str = "libinput Left Handed Enabled";

    #[derive(PartialEq, Clone, Copy)]
    enum Class {
        Touchpad,
        Mouse,
    }

    struct Device {
        id: u16,
        name: String,
        class: Class,
    }

    /// The display to use: `$DISPLAY`, else the lowest local X socket (the
    /// server is a systemd user service and rarely inherits `$DISPLAY`).
    fn display_name() -> Option<String> {
        if let Some(d) = std::env::var_os("DISPLAY").filter(|d| !d.is_empty()) {
            return Some(d.to_string_lossy().into_owned());
        }
        std::fs::read_dir("/tmp/.X11-unix")
            .ok()?
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_string_lossy().strip_prefix('X')?.parse::<u32>().ok())
            .min()
            .map(|n| format!(":{n}"))
    }

    fn connect() -> Result<(RustConnection, usize), String> {
        let name = display_name().ok_or("no X display")?;
        let (conn, screen) = RustConnection::connect(Some(&name)).map_err(|e| format!("cannot open {name}: {e}"))?;
        conn.xinput_xi_query_version(2, 2)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|_| "the X server has no XInput2".to_string())?;
        Ok((conn, screen))
    }

    /// Interned atoms by name, created lazily (`only_if_exists` so asking for
    /// a property the driver does not have never creates junk atoms).
    struct Atoms<'a> {
        conn: &'a RustConnection,
        cache: HashMap<&'static str, u32>,
    }

    impl<'a> Atoms<'a> {
        fn new(conn: &'a RustConnection) -> Self {
            Self { conn, cache: HashMap::new() }
        }

        fn get(&mut self, name: &'static str) -> u32 {
            if let Some(atom) = self.cache.get(name) {
                return *atom;
            }
            let atom = self
                .conn
                .intern_atom(true, name.as_bytes())
                .ok()
                .and_then(|c| c.reply().ok())
                .map(|r| r.atom)
                .unwrap_or(0);
            self.cache.insert(name, atom);
            atom
        }
    }

    fn devices(conn: &RustConnection, atoms: &mut Atoms<'_>) -> Vec<Device> {
        let Ok(reply) = conn.xinput_xi_query_device(0u16).map_err(|e| e.to_string()).and_then(|c| c.reply().map_err(|e| e.to_string())) else {
            return Vec::new();
        };
        let tapping = atoms.get(TAPPING);
        let speed = atoms.get(SPEED);
        let mut out = Vec::new();
        for info in reply.infos {
            if info.type_ != DeviceType::SLAVE_POINTER || !info.enabled {
                continue;
            }
            let props = conn
                .xinput_xi_list_properties(info.deviceid)
                .ok()
                .and_then(|c| c.reply().ok())
                .map(|r| r.properties)
                .unwrap_or_default();
            let class = if tapping != 0 && props.contains(&tapping) {
                Class::Touchpad
            } else if speed != 0 && props.contains(&speed) {
                Class::Mouse
            } else {
                continue; // XTEST, touchscreens, tablets: not ours
            };
            out.push(Device { id: info.deviceid, name: String::from_utf8_lossy(&info.name).into_owned(), class });
        }
        out
    }

    /// A property's raw value: 8-bit items as bytes, 32-bit items as words.
    enum Prop {
        Bytes(Vec<u8>),
        Words(Vec<u32>, u32),
    }

    fn get_prop(conn: &RustConnection, dev: u16, atom: u32) -> Option<Prop> {
        if atom == 0 {
            return None;
        }
        let reply = conn
            .xinput_xi_get_property(dev, false, atom, 0u32, 0, 64)
            .ok()?
            .reply()
            .ok()?;
        if reply.type_ == 0 {
            return None; // the device does not have it
        }
        match reply.items {
            XIGetPropertyItems::Data8(v) => Some(Prop::Bytes(v)),
            XIGetPropertyItems::Data32(v) => Some(Prop::Words(v, reply.type_)),
            _ => None,
        }
    }

    fn get_bool(conn: &RustConnection, dev: u16, atom: u32) -> Option<bool> {
        match get_prop(conn, dev, atom)? {
            Prop::Bytes(v) => v.first().map(|b| *b != 0),
            Prop::Words(..) => None,
        }
    }

    fn get_bytes(conn: &RustConnection, dev: u16, atom: u32) -> Option<Vec<u8>> {
        match get_prop(conn, dev, atom)? {
            Prop::Bytes(v) => Some(v),
            Prop::Words(..) => None,
        }
    }

    fn get_float(conn: &RustConnection, dev: u16, atom: u32) -> Option<f64> {
        match get_prop(conn, dev, atom)? {
            Prop::Words(v, _) => v.first().map(|w| f64::from(f32::from_bits(*w))),
            Prop::Bytes(_) => None,
        }
    }

    fn set_bytes(conn: &RustConnection, dev: u16, atom: u32, values: Vec<u8>) {
        if atom == 0 {
            return;
        }
        let n = values.len() as u32;
        let _ = conn.xinput_xi_change_property(
            dev,
            xproto::PropMode::REPLACE,
            atom,
            u32::from(xproto::AtomEnum::INTEGER),
            n,
            &XIChangePropertyAux::Data8(values),
        );
    }

    fn set_bool(conn: &RustConnection, dev: u16, atom: u32, on: bool) {
        // Only touch what the device has: writing a property the driver did
        // not create would just add a dead one.
        if get_bool(conn, dev, atom).is_some() {
            set_bytes(conn, dev, atom, vec![u8::from(on)]);
        }
    }

    fn set_speed(conn: &RustConnection, dev: u16, atoms: &mut Atoms<'_>, value: f64) {
        let atom = atoms.get(SPEED);
        let Some(Prop::Words(_, float_type)) = get_prop(conn, dev, atom) else {
            return;
        };
        let bits = (value.clamp(-1.0, 1.0) as f32).to_bits();
        let _ = conn.xinput_xi_change_property(
            dev,
            xproto::PropMode::REPLACE,
            atom,
            float_type,
            1,
            &XIChangePropertyAux::Data32(vec![bits]),
        );
    }

    /// One-hot write of a libinput "method" property (`[a, b, c]` flags),
    /// sized like the current value, and only when the method is available.
    fn set_one_hot(conn: &RustConnection, dev: u16, enabled: u32, available: u32, index: usize) {
        let Some(current) = get_bytes(conn, dev, enabled) else {
            return;
        };
        if index >= current.len() {
            return;
        }
        if available != 0 {
            if let Some(avail) = get_bytes(conn, dev, available) {
                if avail.get(index).copied().unwrap_or(0) == 0 {
                    return;
                }
            }
        }
        let mut values = vec![0u8; current.len()];
        values[index] = 1;
        set_bytes(conn, dev, enabled, values);
    }

    fn apply_touchpad(conn: &RustConnection, dev: u16, atoms: &mut Atoms<'_>, s: &TouchpadSettings) {
        if let Some(v) = s.tap_to_click {
            set_bool(conn, dev, atoms.get(TAPPING), v);
        }
        if let Some(v) = s.natural_scroll {
            set_bool(conn, dev, atoms.get(NATURAL), v);
        }
        if let Some(v) = s.speed {
            set_speed(conn, dev, atoms, v);
        }
        if let Some(v) = s.disable_while_typing {
            set_bool(conn, dev, atoms.get(DWT), v);
        }
        if let Some(method) = s.click_method.as_deref() {
            let index = if method == "fingers" { 1 } else { 0 };
            set_one_hot(conn, dev, atoms.get(CLICK), atoms.get(CLICKS_AVAILABLE), index);
        }
        if let Some(method) = s.scroll_method.as_deref() {
            let index = if method == "edge" { 1 } else { 0 };
            set_one_hot(conn, dev, atoms.get(SCROLL), atoms.get(SCROLLS_AVAILABLE), index);
        }
    }

    fn apply_mouse(conn: &RustConnection, dev: u16, atoms: &mut Atoms<'_>, s: &MouseSettings) {
        if let Some(v) = s.speed {
            set_speed(conn, dev, atoms, v);
        }
        if let Some(profile) = s.accel_profile.as_deref() {
            let index = if profile == "flat" { 1 } else { 0 };
            set_one_hot(conn, dev, atoms.get(PROFILE), 0, index);
        }
        if let Some(v) = s.natural_scroll {
            set_bool(conn, dev, atoms.get(NATURAL), v);
        }
        if let Some(v) = s.left_handed {
            set_bool(conn, dev, atoms.get(LEFT), v);
        }
    }

    fn apply_on(conn: &RustConnection, settings: &PointerSettings) {
        let mut atoms = Atoms::new(conn);
        for dev in devices(conn, &mut atoms) {
            match dev.class {
                Class::Touchpad => apply_touchpad(conn, dev.id, &mut atoms, &settings.touchpad),
                Class::Mouse => apply_mouse(conn, dev.id, &mut atoms, &settings.mouse),
            }
        }
        let _ = conn.flush();
    }

    pub fn apply(settings: &PointerSettings) -> Result<(), String> {
        let (conn, _) = connect()?;
        apply_on(&conn, settings);
        // A round trip so every change has reached the server before the
        // status read that usually follows.
        conn.get_input_focus().map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
        Ok(())
    }

    fn one_hot_index(v: Option<Vec<u8>>) -> Option<usize> {
        v?.iter().position(|b| *b != 0)
    }

    pub fn status() -> Value {
        let (conn, _) = match connect() {
            Ok(c) => c,
            Err(err) => return unavailable(&err),
        };
        let mut atoms = Atoms::new(&conn);
        let devs = devices(&conn, &mut atoms);
        let touchpads: Vec<&Device> = devs.iter().filter(|d| d.class == Class::Touchpad).collect();
        let mice: Vec<&Device> = devs.iter().filter(|d| d.class == Class::Mouse).collect();

        let touchpad = match touchpads.first() {
            None => json!({ "present": false, "devices": [] }),
            Some(d) => {
                let click = one_hot_index(get_bytes(&conn, d.id, atoms.get(CLICK)));
                let clicks = get_bytes(&conn, d.id, atoms.get(CLICKS_AVAILABLE)).unwrap_or_default();
                let scroll = one_hot_index(get_bytes(&conn, d.id, atoms.get(SCROLL)));
                let scrolls = get_bytes(&conn, d.id, atoms.get(SCROLLS_AVAILABLE)).unwrap_or_default();
                json!({
                    "present": true,
                    "devices": touchpads.iter().map(|d| d.name.clone()).collect::<Vec<_>>(),
                    "tap_to_click": get_bool(&conn, d.id, atoms.get(TAPPING)),
                    "natural_scroll": get_bool(&conn, d.id, atoms.get(NATURAL)),
                    "speed": get_float(&conn, d.id, atoms.get(SPEED)),
                    "disable_while_typing": get_bool(&conn, d.id, atoms.get(DWT)),
                    "click_method": click.map(|i| if i == 1 { "fingers" } else { "areas" }),
                    "click_methods": method_names(&clicks, &["areas", "fingers"]),
                    "scroll_method": scroll.and_then(|i| match i { 0 => Some("two_finger"), 1 => Some("edge"), _ => None }),
                    "scroll_methods": method_names(&scrolls, &["two_finger", "edge"]),
                })
            }
        };
        let mouse = match mice.first() {
            None => json!({ "present": false, "devices": [] }),
            Some(d) => {
                let profile = one_hot_index(get_bytes(&conn, d.id, atoms.get(PROFILE)));
                json!({
                    "present": true,
                    "devices": mice.iter().map(|d| d.name.clone()).collect::<Vec<_>>(),
                    "speed": get_float(&conn, d.id, atoms.get(SPEED)),
                    "accel_profile": profile.and_then(|i| match i { 0 => Some("adaptive"), 1 => Some("flat"), _ => None }),
                    "natural_scroll": get_bool(&conn, d.id, atoms.get(NATURAL)),
                    "left_handed": get_bool(&conn, d.id, atoms.get(LEFT)),
                })
            }
        };
        json!({ "available": true, "touchpad": touchpad, "mouse": mouse })
    }

    fn method_names(flags: &[u8], names: &[&'static str]) -> Vec<&'static str> {
        names
            .iter()
            .enumerate()
            .filter(|(i, _)| flags.get(*i).copied().unwrap_or(0) != 0)
            .map(|(_, n)| *n)
            .collect()
    }

    /// Apply the stored choice on connect and after every pointer hotplug;
    /// reconnect (the X server restarts at every sign-in) forever.
    pub fn watch(path: PathBuf) {
        let mut logged = false;
        loop {
            match connect() {
                Ok((conn, screen)) => {
                    logged = false;
                    tracing::info!("pointer: applying settings on the X display");
                    apply_on(&conn, &load_settings(&path));
                    let root = conn.setup().roots[screen].root;
                    let mask = xinput::EventMask { deviceid: 0, mask: vec![xinput::XIEventMask::HIERARCHY] };
                    if conn.xinput_xi_select_events(root, &[mask]).is_err() || conn.flush().is_err() {
                        std::thread::sleep(Duration::from_secs(5));
                        continue;
                    }
                    loop {
                        match conn.wait_for_event() {
                            Ok(Event::XinputHierarchy(ev)) => {
                                let flags = u32::from(ev.flags);
                                let added = u32::from(xinput::HierarchyMask::SLAVE_ADDED)
                                    | u32::from(xinput::HierarchyMask::DEVICE_ENABLED);
                                if flags & added != 0 {
                                    // Let the driver finish initialising the
                                    // new device before writing to it.
                                    std::thread::sleep(Duration::from_millis(300));
                                    apply_on(&conn, &load_settings(&path));
                                }
                            }
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                }
                Err(err) => {
                    if !logged {
                        tracing::info!("pointer: waiting for an X display ({err})");
                        logged = true;
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(5));
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod x11 {
    use std::path::PathBuf;

    use serde_json::Value;

    use super::{unavailable, PointerSettings};

    pub fn status() -> Value {
        unavailable("pointer settings are handled by the operating system")
    }

    pub fn apply(_settings: &PointerSettings) -> Result<(), String> {
        Ok(())
    }

    pub fn watch(_path: PathBuf) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_sets_and_forgets_options() {
        let mut s = PointerSettings::default();
        s.merge(&json!({ "touchpad": { "tap_to_click": true, "speed": 0.5 }, "mouse": { "accel_profile": "flat" } }))
            .unwrap();
        assert_eq!(s.touchpad.tap_to_click, Some(true));
        assert_eq!(s.touchpad.speed, Some(0.5));
        assert_eq!(s.mouse.accel_profile.as_deref(), Some("flat"));
        s.merge(&json!({ "touchpad": { "speed": null } })).unwrap();
        assert_eq!(s.touchpad.speed, None);
        assert_eq!(s.touchpad.tap_to_click, Some(true));
    }

    #[test]
    fn merge_refuses_bad_values() {
        let mut s = PointerSettings::default();
        assert!(s.merge(&json!({ "touchpad": { "speed": 2 } })).is_err());
        assert!(s.merge(&json!({ "mouse": { "accel_profile": "fast" } })).is_err());
        assert!(s.merge(&json!({ "mouse": { "volume": 1 } })).is_err());
        assert!(s.merge(&json!({ "touchpad": { "tap_to_click": "yes" } })).is_err());
        assert_eq!(s, PointerSettings::default());
    }

    #[test]
    fn untouched_options_are_not_stored() {
        let mut s = PointerSettings::default();
        s.merge(&json!({ "mouse": { "left_handed": true } })).unwrap();
        let text = serde_json::to_string(&s).unwrap();
        assert_eq!(text, r#"{"touchpad":{},"mouse":{"left_handed":true}}"#);
    }
}

#[cfg(test)]
mod live {
    /// Read-only probe of the real display: `cargo test -- --ignored pointer_live`.
    #[test]
    #[ignore]
    fn pointer_live_status() {
        println!("{}", serde_json::to_string_pretty(&super::x11::status()).unwrap());
    }
}
