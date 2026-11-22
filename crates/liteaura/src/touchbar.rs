//! The Touch Bar as a Shiny surface (MacBook Pro T2 on Linux).
//!
//! The bar is a second display (`appletbdrm`, 60x2008, mounted sideways) with
//! its own touch digitizer. Instead of leaving it to `tiny-dfr`'s fixed row of
//! keys, the shell renders a page of the app on it (`/touchbar.html`): the
//! Qt side keeps an off-screen web view and hands over its frames
//! (`liteaura_qt_bar_create`), this module puts them on the panel and feeds
//! the touches back to the page. So the bar is drawn with the same HTML/CSS
//! as the rest of the desktop, animations included, and it shares the app's
//! session.
//!
//! Three small parts, none of which can fail the shell:
//!
//! * [`Panel`] — the DRM side: one dumb buffer, a legacy mode-set, and a
//!   `DIRTYFB` per frame. The ioctls are spelled out by hand (as the gesture
//!   reader does for evdev) rather than pulling in a DRM crate for eight
//!   requests.
//! * the touch reader — the digitizer's evdev node, decoded into
//!   down/move/up events in bar coordinates.
//! * the Fn watcher — the internal keyboard's `KEY_FN`, so the page can show
//!   F1–F12 while it is held.
//!
//! Only one DRM master can drive the panel. While `tiny-dfr` holds it the
//! mode-set is refused; the shell then keeps retrying quietly, so the bar
//! comes up as soon as the daemon is stopped
//! (`scripts/touchbar/install-touchbar.sh` does that).

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// Called (from a reader thread) when touches start or stop, so the shell can
/// pump at frame rate while a finger is down.
pub type Waker = fn(bool);

/// Names the kernel gives the bar's digitizer (the same list as the t2linux
/// udev rules).
const TOUCH_NAMES: &[&str] = &[
    "Apple Inc. Touch Bar Display Touchpad",
    "MacBookPro17,1 Touch Bar",
    "Mac14,7 Touch Bar",
];

/// DRM drivers that expose a Touch Bar panel.
const PANEL_DRIVERS: &[&str] = &["appletbdrm", "adp"];

const BACKLIGHT: &str = "/sys/class/backlight/appletb_backlight/brightness";

// ── The panel ────────────────────────────────────────────────────────────

/// The bar's display: a mapped dumb buffer that is scanned out.
pub struct Panel {
    file: File,
    fb_id: u32,
    crtc_id: u32,
    connector_id: u32,
    mode: ModeInfo,
    map: *mut u8,
    map_len: usize,
    pitch: usize,
    /// The panel's own size (60x2008 on a T2: it is mounted sideways).
    native: (usize, usize),
    /// Whether the mode has been set; false while another DRM master owns it.
    lit: bool,
    last_try: Instant,
}

// The mapping is only touched from the Qt main thread (the frame callback);
// the struct sits in a `Mutex` in a static, which needs `Send`.
unsafe impl Send for Panel {}

#[repr(C)]
#[derive(Default)]
struct CardRes {
    fb_id_ptr: u64,
    crtc_id_ptr: u64,
    connector_id_ptr: u64,
    encoder_id_ptr: u64,
    count_fbs: u32,
    count_crtcs: u32,
    count_connectors: u32,
    count_encoders: u32,
    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ModeInfo {
    clock: u32,
    hdisplay: u16,
    hsync_start: u16,
    hsync_end: u16,
    htotal: u16,
    hskew: u16,
    vdisplay: u16,
    vsync_start: u16,
    vsync_end: u16,
    vtotal: u16,
    vscan: u16,
    vrefresh: u32,
    flags: u32,
    type_: u32,
    name: [u8; 32],
}

#[repr(C)]
#[derive(Default)]
struct GetConnector {
    encoders_ptr: u64,
    modes_ptr: u64,
    props_ptr: u64,
    prop_values_ptr: u64,
    count_modes: u32,
    count_props: u32,
    count_encoders: u32,
    encoder_id: u32,
    connector_id: u32,
    connector_type: u32,
    connector_type_id: u32,
    connection: u32,
    mm_width: u32,
    mm_height: u32,
    subpixel: u32,
    pad: u32,
}

#[repr(C)]
struct Crtc {
    set_connectors_ptr: u64,
    count_connectors: u32,
    crtc_id: u32,
    fb_id: u32,
    x: u32,
    y: u32,
    gamma_size: u32,
    mode_valid: u32,
    mode: ModeInfo,
}

#[repr(C)]
#[derive(Default)]
struct CreateDumb {
    height: u32,
    width: u32,
    bpp: u32,
    flags: u32,
    handle: u32,
    pitch: u32,
    size: u64,
}

#[repr(C)]
#[derive(Default)]
struct FbCmd {
    fb_id: u32,
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u32,
    depth: u32,
    handle: u32,
}

#[repr(C)]
#[derive(Default)]
struct MapDumb {
    handle: u32,
    pad: u32,
    offset: u64,
}

#[repr(C)]
#[derive(Default)]
struct FbDirty {
    fb_id: u32,
    flags: u32,
    color: u32,
    num_clips: u32,
    clips_ptr: u64,
}

/// `_IOWR('d', nr, T)` / `_IO('d', nr)`.
const fn iowr<T>(nr: u64) -> libc::c_ulong {
    ((3u64 << 30) | ((std::mem::size_of::<T>() as u64) << 16) | ((b'd' as u64) << 8) | nr)
        as libc::c_ulong
}
const fn io(nr: u64) -> libc::c_ulong {
    (((b'd' as u64) << 8) | nr) as libc::c_ulong
}

const SET_MASTER: libc::c_ulong = io(0x1e);
const GETRESOURCES: libc::c_ulong = iowr::<CardRes>(0xA0);
const SETCRTC: libc::c_ulong = iowr::<Crtc>(0xA2);
const GETCONNECTOR: libc::c_ulong = iowr::<GetConnector>(0xA7);
const ADDFB: libc::c_ulong = iowr::<FbCmd>(0xAE);
const DIRTYFB: libc::c_ulong = iowr::<FbDirty>(0xB1);
const CREATE_DUMB: libc::c_ulong = iowr::<CreateDumb>(0xB2);
const MAP_DUMB: libc::c_ulong = iowr::<MapDumb>(0xB3);

fn ioctl<T>(file: &File, request: libc::c_ulong, arg: &mut T) -> std::io::Result<()> {
    loop {
        let rc = unsafe { libc::ioctl(file.as_raw_fd(), request, arg as *mut T) };
        if rc == 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
}

/// Whether this machine has a Touch Bar at all. True even when its display
/// driver has not produced a card yet (it sometimes binds late, see
/// `scripts/touchbar/shiny-touchbar-heal`): the shell keeps looking.
pub fn expected() -> bool {
    std::path::Path::new("/sys/class/backlight/appletb_backlight").exists()
        || find_panel().is_some()
}

/// The DRM card a Touch Bar driver created, if this machine has one.
pub fn find_panel() -> Option<PathBuf> {
    let entries = std::fs::read_dir("/sys/class/drm").ok()?;
    let mut cards: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| {
            name.strip_prefix("card")
                .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
        })
        .collect();
    cards.sort();
    cards.into_iter().find_map(|card| {
        let driver = std::fs::read_link(format!("/sys/class/drm/{card}/device/driver")).ok()?;
        let driver = driver.file_name()?.to_str()?.to_string();
        PANEL_DRIVERS
            .contains(&driver.as_str())
            .then(|| PathBuf::from(format!("/dev/dri/{card}")))
    })
}

impl Panel {
    /// Open the card and prepare a buffer for its (only) mode. The mode itself
    /// is set lazily in [`Panel::present`], because another master may hold
    /// the card for now.
    pub fn open(path: &std::path::Path) -> Result<Self, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|err| format!("cannot open {}: {err}", path.display()))?;

        let mut res = CardRes::default();
        ioctl(&file, GETRESOURCES, &mut res).map_err(|err| format!("GETRESOURCES: {err}"))?;
        let mut crtcs = vec![0u32; res.count_crtcs as usize];
        let mut connectors = vec![0u32; res.count_connectors as usize];
        let mut res = CardRes {
            crtc_id_ptr: crtcs.as_mut_ptr() as u64,
            connector_id_ptr: connectors.as_mut_ptr() as u64,
            count_crtcs: crtcs.len() as u32,
            count_connectors: connectors.len() as u32,
            ..CardRes::default()
        };
        ioctl(&file, GETRESOURCES, &mut res).map_err(|err| format!("GETRESOURCES: {err}"))?;
        let crtc_id = *crtcs.first().ok_or("the panel has no CRTC")?;

        let mut found = None;
        for &connector_id in &connectors {
            let mut probe = GetConnector {
                connector_id,
                ..GetConnector::default()
            };
            if ioctl(&file, GETCONNECTOR, &mut probe).is_err() || probe.count_modes == 0 {
                continue;
            }
            let mut modes: Vec<ModeInfo> =
                vec![unsafe { std::mem::zeroed() }; probe.count_modes as usize];
            let mut get = GetConnector {
                connector_id,
                modes_ptr: modes.as_mut_ptr() as u64,
                count_modes: modes.len() as u32,
                ..GetConnector::default()
            };
            if ioctl(&file, GETCONNECTOR, &mut get).is_ok() && get.count_modes > 0 {
                found = Some((connector_id, modes[0]));
                break;
            }
        }
        let (connector_id, mode) = found.ok_or("the panel reports no mode")?;
        let native = (usize::from(mode.hdisplay), usize::from(mode.vdisplay));

        let mut dumb = CreateDumb {
            width: native.0 as u32,
            height: native.1 as u32,
            bpp: 32,
            ..CreateDumb::default()
        };
        ioctl(&file, CREATE_DUMB, &mut dumb).map_err(|err| format!("CREATE_DUMB: {err}"))?;
        let mut fb = FbCmd {
            width: dumb.width,
            height: dumb.height,
            pitch: dumb.pitch,
            bpp: 32,
            depth: 24,
            handle: dumb.handle,
            ..FbCmd::default()
        };
        ioctl(&file, ADDFB, &mut fb).map_err(|err| format!("ADDFB: {err}"))?;
        let mut map = MapDumb {
            handle: dumb.handle,
            ..MapDumb::default()
        };
        ioctl(&file, MAP_DUMB, &mut map).map_err(|err| format!("MAP_DUMB: {err}"))?;
        let map_len = dumb.size as usize;
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                map_len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                map.offset as libc::off_t,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(format!("mmap: {}", std::io::Error::last_os_error()));
        }
        unsafe { std::ptr::write_bytes(ptr.cast::<u8>(), 0, map_len) };

        Ok(Self {
            file,
            fb_id: fb.fb_id,
            crtc_id,
            connector_id,
            mode,
            map: ptr.cast(),
            map_len,
            pitch: dumb.pitch as usize,
            native,
            lit: false,
            last_try: Instant::now() - Duration::from_secs(60),
        })
    }

    /// The size of the surface as the user sees it: the long side is always
    /// the width, whichever way the panel is mounted.
    pub fn logical_size(&self) -> (usize, usize) {
        let (w, h) = self.native;
        if h > w {
            (h, w)
        } else {
            (w, h)
        }
    }

    /// Take the card (if nobody else has it) and scan the buffer out.
    fn light(&mut self) -> bool {
        if self.lit {
            return true;
        }
        if self.last_try.elapsed() < Duration::from_secs(3) {
            return false;
        }
        self.last_try = Instant::now();
        // Harmless when we already are the master (first opener).
        unsafe { libc::ioctl(self.file.as_raw_fd(), SET_MASTER, 0) };
        let mut connector = self.connector_id;
        let mut crtc = Crtc {
            set_connectors_ptr: std::ptr::addr_of_mut!(connector) as u64,
            count_connectors: 1,
            crtc_id: self.crtc_id,
            fb_id: self.fb_id,
            x: 0,
            y: 0,
            gamma_size: 0,
            mode_valid: 1,
            mode: self.mode,
        };
        match ioctl(&self.file, SETCRTC, &mut crtc) {
            Ok(()) => {
                println!("liteaura: touch bar: driving the panel");
                self.lit = true;
            }
            Err(err) => {
                static TOLD: AtomicBool = AtomicBool::new(false);
                if !TOLD.swap(true, Ordering::Relaxed) {
                    eprintln!(
                        "liteaura: touch bar: the panel is held by another program ({err}); \
                         stop tiny-dfr to let Shiny draw it"
                    );
                }
            }
        }
        self.lit
    }

    /// Put one frame on the panel. `pixels` is `width * height` 32-bit XRGB in
    /// logical orientation, `stride` bytes per row.
    pub fn present(&mut self, pixels: &[u8], width: usize, height: usize, stride: usize) {
        if !self.light() {
            return;
        }
        let (native_w, native_h) = self.native;
        let rotated = native_h > native_w;
        let (logical_w, logical_h) = self.logical_size();
        let width = width.min(logical_w);
        let height = height.min(logical_h);
        if stride < width * 4 || pixels.len() < stride * height.saturating_sub(1) + width * 4 {
            return;
        }
        let out = unsafe { std::slice::from_raw_parts_mut(self.map, self.map_len) };
        if rotated {
            // A quarter turn: logical (x, y) lands on native column
            // `native_w - 1 - y`, row `x` (the same turn tiny-dfr applies).
            for y in 0..height {
                let row = &pixels[y * stride..y * stride + width * 4];
                let column = (native_w - 1 - y) * 4;
                for x in 0..width {
                    let at = x * self.pitch + column;
                    out[at..at + 4].copy_from_slice(&row[x * 4..x * 4 + 4]);
                }
            }
        } else {
            for y in 0..height {
                let at = y * self.pitch;
                out[at..at + width * 4].copy_from_slice(&pixels[y * stride..y * stride + width * 4]);
            }
        }
        let mut dirty = FbDirty {
            fb_id: self.fb_id,
            ..FbDirty::default()
        };
        if let Err(err) = ioctl(&self.file, DIRTYFB, &mut dirty) {
            // EACCES: another master took the card back. Start over.
            if err.raw_os_error() == Some(libc::EACCES) {
                self.lit = false;
            }
        }
    }
}

impl Drop for Panel {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.map.cast(), self.map_len) };
    }
}

// ── Touches ──────────────────────────────────────────────────────────────

/// One contact update, in fractions of the bar (0..1 along each side).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Touch {
    pub phase: Phase,
    pub id: i32,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Down,
    Move,
    Up,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Phase::Down => "down",
            Phase::Move => "move",
            Phase::Up => "up",
        }
    }
}

/// What the reader threads leave for the pump.
#[derive(Clone, Default)]
pub struct Bridge {
    inner: Arc<Mutex<Pending>>,
}

#[derive(Default)]
struct Pending {
    touches: Vec<Touch>,
    fn_held: Option<bool>,
}

impl Bridge {
    fn push(&self, touch: Touch) {
        let mut pending = self.inner.lock();
        // Coalesce a run of moves of one contact: only the last position of a
        // tick matters, and a fast swipe would otherwise queue dozens.
        if touch.phase == Phase::Move {
            if let Some(last) = pending.touches.last_mut() {
                if last.phase == Phase::Move && last.id == touch.id {
                    *last = touch;
                    return;
                }
            }
        }
        pending.touches.push(touch);
    }

    fn set_fn(&self, held: bool) {
        self.inner.lock().fn_held = Some(held);
    }

    /// The script that delivers everything queued since the last call, or
    /// `None` when there is nothing to say.
    pub fn drain_script(&self) -> Option<String> {
        let mut pending = self.inner.lock();
        if pending.touches.is_empty() && pending.fn_held.is_none() {
            return None;
        }
        let mut script = String::new();
        if !pending.touches.is_empty() {
            script.push_str("window.__shinyBarTouch&&window.__shinyBarTouch([");
            for (i, touch) in pending.touches.iter().enumerate() {
                if i > 0 {
                    script.push(',');
                }
                script.push_str(&format!(
                    "[\"{}\",{},{:.5},{:.5}]",
                    touch.phase.as_str(),
                    touch.id,
                    touch.x,
                    touch.y
                ));
            }
            script.push_str("]);");
            pending.touches.clear();
        }
        if let Some(held) = pending.fn_held.take() {
            script.push_str(&format!(
                "window.__shinyBarFn&&window.__shinyBarFn({held});"
            ));
        }
        Some(script)
    }
}

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0;
const BTN_TOUCH: u16 = 0x14a;
const KEY_FN: u16 = 0x1d0;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const EVENT_SIZE: usize = 24;
const SLOTS: usize = 10;

#[derive(Clone, Copy, Default)]
struct Contact {
    /// Live this frame.
    down: bool,
    /// Reported to the page as down.
    told: bool,
    x: i32,
    y: i32,
    moved: bool,
}

/// Decodes the digitizer's events (multi-touch slots, or a plain single-touch
/// device) into [`Touch`]es, one batch per `SYN_REPORT`.
struct TouchDecoder {
    x_range: (i32, i32),
    y_range: (i32, i32),
    multitouch: bool,
    slot: usize,
    contacts: [Contact; SLOTS],
}

impl TouchDecoder {
    fn new(x_range: (i32, i32), y_range: (i32, i32), multitouch: bool) -> Self {
        Self {
            x_range,
            y_range,
            multitouch,
            slot: 0,
            contacts: [Contact::default(); SLOTS],
        }
    }

    /// Feed one event; returns the touches a completed frame produced.
    fn handle(&mut self, type_: u16, code: u16, value: i32) -> Vec<Touch> {
        match (type_, code) {
            (EV_ABS, ABS_MT_SLOT) if self.multitouch => {
                self.slot = (value.max(0) as usize).min(SLOTS - 1);
            }
            (EV_ABS, ABS_MT_TRACKING_ID) if self.multitouch => {
                self.contacts[self.slot].down = value >= 0;
            }
            (EV_ABS, ABS_MT_POSITION_X) if self.multitouch => {
                let contact = &mut self.contacts[self.slot];
                contact.x = value;
                contact.moved = true;
            }
            (EV_ABS, ABS_MT_POSITION_Y) if self.multitouch => {
                let contact = &mut self.contacts[self.slot];
                contact.y = value;
                contact.moved = true;
            }
            (EV_ABS, ABS_X) if !self.multitouch => {
                self.contacts[0].x = value;
                self.contacts[0].moved = true;
            }
            (EV_ABS, ABS_Y) if !self.multitouch => {
                self.contacts[0].y = value;
                self.contacts[0].moved = true;
            }
            (EV_KEY, BTN_TOUCH) if !self.multitouch => {
                self.contacts[0].down = value != 0;
            }
            (EV_SYN, SYN_REPORT) => return self.flush(),
            _ => {}
        }
        Vec::new()
    }

    fn flush(&mut self) -> Vec<Touch> {
        let mut out = Vec::new();
        let (x_range, y_range) = (self.x_range, self.y_range);
        for (id, contact) in self.contacts.iter_mut().enumerate() {
            let touch = |phase| Touch {
                phase,
                id: id as i32,
                x: fraction(contact.x, x_range),
                y: fraction(contact.y, y_range),
            };
            if contact.down && !contact.told {
                contact.told = true;
                out.push(touch(Phase::Down));
            } else if !contact.down && contact.told {
                contact.told = false;
                out.push(touch(Phase::Up));
            } else if contact.down && contact.moved {
                out.push(touch(Phase::Move));
            }
            contact.moved = false;
        }
        out
    }

    fn any_down(&self) -> bool {
        self.contacts.iter().any(|contact| contact.told)
    }
}

fn fraction(value: i32, (min, max): (i32, i32)) -> f64 {
    if max <= min {
        return 0.0;
    }
    (f64::from(value - min) / f64::from(max - min)).clamp(0.0, 1.0)
}

/// `(event node, name)` of every input device whose name passes `wanted`,
/// from the kernel's device list.
fn input_devices(wanted: impl Fn(&str) -> bool) -> Vec<(PathBuf, String)> {
    let Ok(list) = std::fs::read_to_string("/proc/bus/input/devices") else {
        return Vec::new();
    };
    parse_input_devices(&list, wanted)
}

fn parse_input_devices(list: &str, wanted: impl Fn(&str) -> bool) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    for block in list.split("\n\n") {
        let mut name = None;
        let mut event = None;
        for line in block.lines() {
            if let Some(rest) = line.strip_prefix("N: Name=") {
                name = Some(rest.trim_matches('"').to_string());
            } else if let Some(rest) = line.strip_prefix("H: Handlers=") {
                event = rest
                    .split_whitespace()
                    .find(|handler| handler.starts_with("event"))
                    .map(str::to_string);
            }
        }
        if let (Some(name), Some(event)) = (name, event) {
            if wanted(&name) {
                out.push((PathBuf::from(format!("/dev/input/{event}")), name));
            }
        }
    }
    out
}

fn read_events(mut file: File, mut on_event: impl FnMut(u16, u16, i32)) {
    let mut buf = [0u8; EVENT_SIZE * 32];
    loop {
        let read = match file.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        for event in buf[..read].chunks_exact(EVENT_SIZE) {
            let type_ = u16::from_ne_bytes([event[16], event[17]]);
            let code = u16::from_ne_bytes([event[18], event[19]]);
            let value = i32::from_ne_bytes([event[20], event[21], event[22], event[23]]);
            on_event(type_, code, value);
        }
    }
}

/// Start the touch reader and the Fn watcher. Either may be missing (no rule
/// installed yet): the bar then still draws, it just cannot be touched.
pub fn spawn_input(waker: Waker) -> Bridge {
    let bridge = Bridge::default();

    match input_devices(|name| TOUCH_NAMES.contains(&name)).into_iter().next() {
        None => eprintln!("liteaura: touch bar: no touch surface found"),
        Some((path, name)) => match File::open(&path) {
            Err(err) => eprintln!(
                "liteaura: touch bar: cannot read {} ({err}); run \
                 scripts/touchbar/install-touchbar.sh",
                path.display()
            ),
            Ok(file) => {
                let fd = file.as_raw_fd();
                let mt = crate::gestures::abs_range(fd, u32::from(ABS_MT_POSITION_X))
                    .zip(crate::gestures::abs_range(fd, u32::from(ABS_MT_POSITION_Y)));
                let st = crate::gestures::abs_range(fd, u32::from(ABS_X))
                    .zip(crate::gestures::abs_range(fd, u32::from(ABS_Y)));
                let Some(((x_range, y_range), multitouch)) =
                    mt.map(|ranges| (ranges, true)).or(st.map(|ranges| (ranges, false)))
                else {
                    eprintln!("liteaura: touch bar: {name} reports no position axes");
                    return bridge;
                };
                println!(
                    "liteaura: touch bar: touches from {} ({name}, x {}..{}, y {}..{})",
                    path.display(),
                    x_range.0,
                    x_range.1,
                    y_range.0,
                    y_range.1
                );
                let sink = bridge.clone();
                std::thread::Builder::new()
                    .name("liteaura-touchbar".into())
                    .spawn(move || {
                        let mut decoder = TouchDecoder::new(x_range, y_range, multitouch);
                        let mut live = false;
                        read_events(file, |type_, code, value| {
                            let touches = decoder.handle(type_, code, value);
                            if touches.is_empty() {
                                return;
                            }
                            for touch in touches {
                                sink.push(touch);
                            }
                            let now = decoder.any_down();
                            // Wake on every batch while a finger is down, and
                            // once more on the lift so the `up` is not held
                            // back to the slow tick.
                            if now || live {
                                waker(now);
                            }
                            live = now;
                        });
                    })
                    .ok();
            }
        },
    }

    // Fn lives on the internal keyboard. Reading it needs the keyboard's
    // evdev node, which the installer's udev rule grants; without it the page
    // offers its own switch for the function keys.
    let keyboards = input_devices(|name| {
        name.contains("Apple") && name.contains("Keyboard") && !name.contains("Touch Bar")
    });
    for (path, _) in keyboards {
        let Ok(file) = File::open(&path) else { continue };
        let sink = bridge.clone();
        std::thread::Builder::new()
            .name("liteaura-touchbar-fn".into())
            .spawn(move || {
                read_events(file, |type_, code, value| {
                    // 1 = press, 0 = release; 2 (autorepeat) changes nothing.
                    if type_ == EV_KEY && code == KEY_FN && value != 2 {
                        sink.set_fn(value == 1);
                        waker(false);
                    }
                });
            })
            .ok();
    }

    bridge
}

// ── Backlight ────────────────────────────────────────────────────────────

/// Set the bar's own backlight: 0 off, 1 dim, 2 on. Needs the installer's
/// rule (the attribute is root-only by default); quietly does nothing without.
pub fn set_backlight(level: u8) {
    let _ = std::fs::write(BACKLIGHT, level.min(2).to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_numbers_match_the_kernel_headers() {
        // Values from <drm/drm.h> on x86-64 / aarch64.
        assert_eq!(SET_MASTER, 0x641e);
        assert_eq!(GETRESOURCES, 0xC040_64A0);
        assert_eq!(SETCRTC, 0xC068_64A2);
        assert_eq!(GETCONNECTOR, 0xC050_64A7);
        assert_eq!(ADDFB, 0xC01C_64AE);
        assert_eq!(DIRTYFB, 0xC018_64B1);
        assert_eq!(CREATE_DUMB, 0xC020_64B2);
        assert_eq!(MAP_DUMB, 0xC010_64B3);
        assert_eq!(std::mem::size_of::<ModeInfo>(), 68);
    }

    /// On a T2 MacBook with nothing else holding the bar: paints red, green
    /// and blue thirds (left to right) with a white line along the top, and
    /// leaves them up for a few seconds.
    /// `cargo test -p liteaura paints_the_real_panel -- --ignored --nocapture`
    #[test]
    #[ignore = "needs the Touch Bar hardware"]
    fn paints_the_real_panel() {
        let card = find_panel().expect("a Touch Bar panel");
        let mut panel = Panel::open(&card).expect("open the panel");
        let (width, height) = panel.logical_size();
        println!("{} is {width}x{height}", card.display());
        let mut frame = vec![0u8; width * height * 4];
        for y in 0..height {
            for x in 0..width {
                let pixel = &mut frame[(y * width + x) * 4..][..4];
                // XRGB little-endian: B, G, R, X.
                let rgb: [u8; 3] = match (y < 6, x * 3 / width) {
                    (true, _) => [255, 255, 255],
                    (_, 0) => [0, 0, 255],
                    (_, 1) => [0, 255, 0],
                    _ => [255, 0, 0],
                };
                pixel[..3].copy_from_slice(&rgb);
            }
        }
        panel.present(&frame, width, height, width * 4);
        assert!(panel.lit, "the mode-set was refused");
        std::thread::sleep(Duration::from_secs(6));
    }

    #[test]
    fn multitouch_frames_become_down_move_up() {
        let mut decoder = TouchDecoder::new((0, 2000), (0, 60), true);
        assert!(decoder.handle(EV_ABS, ABS_MT_SLOT, 0).is_empty());
        decoder.handle(EV_ABS, ABS_MT_TRACKING_ID, 7);
        decoder.handle(EV_ABS, ABS_MT_POSITION_X, 1000);
        decoder.handle(EV_ABS, ABS_MT_POSITION_Y, 30);
        let down = decoder.handle(EV_SYN, SYN_REPORT, 0);
        assert_eq!(down, vec![Touch { phase: Phase::Down, id: 0, x: 0.5, y: 0.5 }]);
        assert!(decoder.any_down());

        decoder.handle(EV_ABS, ABS_MT_POSITION_X, 1500);
        let moved = decoder.handle(EV_SYN, SYN_REPORT, 0);
        assert_eq!(moved, vec![Touch { phase: Phase::Move, id: 0, x: 0.75, y: 0.5 }]);

        // A frame with nothing new says nothing.
        assert!(decoder.handle(EV_SYN, SYN_REPORT, 0).is_empty());

        decoder.handle(EV_ABS, ABS_MT_TRACKING_ID, -1);
        let up = decoder.handle(EV_SYN, SYN_REPORT, 0);
        assert_eq!(up, vec![Touch { phase: Phase::Up, id: 0, x: 0.75, y: 0.5 }]);
        assert!(!decoder.any_down());
    }

    #[test]
    fn single_touch_devices_work_too() {
        let mut decoder = TouchDecoder::new((0, 100), (0, 10), false);
        decoder.handle(EV_ABS, ABS_X, 25);
        decoder.handle(EV_ABS, ABS_Y, 10);
        decoder.handle(EV_KEY, BTN_TOUCH, 1);
        let down = decoder.handle(EV_SYN, SYN_REPORT, 0);
        assert_eq!(down, vec![Touch { phase: Phase::Down, id: 0, x: 0.25, y: 1.0 }]);
        decoder.handle(EV_KEY, BTN_TOUCH, 0);
        assert_eq!(decoder.handle(EV_SYN, SYN_REPORT, 0)[0].phase, Phase::Up);
    }

    #[test]
    fn moves_of_one_contact_coalesce_and_drain_as_one_script() {
        let bridge = Bridge::default();
        assert!(bridge.drain_script().is_none());
        bridge.push(Touch { phase: Phase::Down, id: 0, x: 0.1, y: 0.5 });
        bridge.push(Touch { phase: Phase::Move, id: 0, x: 0.2, y: 0.5 });
        bridge.push(Touch { phase: Phase::Move, id: 0, x: 0.3, y: 0.5 });
        bridge.set_fn(true);
        let script = bridge.drain_script().expect("queued");
        assert_eq!(
            script,
            "window.__shinyBarTouch&&window.__shinyBarTouch([[\"down\",0,0.10000,0.50000],\
             [\"move\",0,0.30000,0.50000]]);window.__shinyBarFn&&window.__shinyBarFn(true);"
        );
        assert!(bridge.drain_script().is_none());
    }

    #[test]
    fn the_device_list_is_parsed_by_name() {
        let list = "I: Bus=0003\nN: Name=\"Apple Inc. Touch Bar Display Touchpad\"\n\
                    H: Handlers=mouse0 event2 \n\nI: Bus=0003\nN: Name=\"Other\"\nH: Handlers=kbd event5\n";
        let found = parse_input_devices(list, |name| TOUCH_NAMES.contains(&name));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, PathBuf::from("/dev/input/event2"));
    }
}
