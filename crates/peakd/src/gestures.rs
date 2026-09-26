//! Three-finger trackpad gestures for the Linux kiosk.
//!
//! Neither WebKitGTK nor Chromium delivers three-finger swipes to the page:
//! X11 has no protocol for multi-touch pad gestures, and the usual userspace
//! bridge (`touchegg`, `libinput-gestures`) is not part of the image. On this
//! kiosk a three-finger swipe therefore reaches neither the DOM nor the
//! toolkit, and there is no web-layer signal to key off.
//!
//! So the shell reads the internal trackpad's multi-touch stream straight from
//! its evdev node in a background thread and recognises a three-finger swipe
//! itself. The device is opened read-only and never grabbed, so X keeps the
//! pointer and scrolling; the reader only watches, exactly like `evtest`.
//! A recognised swipe is queued and the event loop evaluates a
//! `trackpad:gesture` event in the page (`web/js/gestures.js` maps it onto the
//! desktop), the same shape as the Touch Bar bridge.
//!
//! Reading the evdev node needs permission. The kiosk runs as an unprivileged
//! user, so the device must carry a logind `uaccess` tag (an ACL for the active
//! seat session); `scripts/install-touchpad-gestures.sh` installs the rule.
//! Without it `spawn` finds no readable device and everything is a no-op — the
//! kiosk is never worse off for the feature missing.
//!
//! The gesture set is deliberately the one the web layer already understands:
//! left/right switch workspaces, up opens the window overview, down opens the
//! plugin launcher.
//!
//! A swipe needs three *moving* fingers. Contacts the kernel marks
//! `MT_TOOL_PALM` are ignored, and every finger's own travel is tracked
//! separately, so a palm, thumb or ghost slot resting on the pad during a
//! two-finger scroll neither counts towards the finger total nor drags the
//! average with the scrolling fingers.

use std::fs::File;
use std::io::{BufReader, Read};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// How far the fingers must travel, as a fraction of the pad's axis range.
const SWIPE_FRACTION: f64 = 0.12;
/// The floor for a pad whose range could not be read (units are arbitrary).
const SWIPE_MIN_UNITS: i64 = 120;
/// A swipe slower than this is a drag, not a gesture. The clock starts when the
/// fingers actually move, not when they land, so resting a hand on the pad
/// first does not spend the gesture.
const SWIPE_MAX_MS: u128 = 900;
/// Movement below this is hand jitter while the fingers rest; the origin and
/// the clock keep resetting until the fingers really move.
const MOVE_EPSILON: i64 = 8;

/// A recognised swipe direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Direction::Left => "left",
            Direction::Right => "right",
            Direction::Up => "up",
            Direction::Down => "down",
        }
    }
}

/// Queue of recognised swipes, filled by the reader thread and drained on the
/// event loop (the only thread allowed to touch the webview). Cheap to clone.
#[derive(Clone, Default)]
pub struct GestureBridge {
    queue: Arc<Mutex<Vec<Direction>>>,
}

impl GestureBridge {
    fn push(&self, dir: Direction) {
        self.queue.lock().push(dir);
    }

    pub fn drain(&self) -> Vec<Direction> {
        std::mem::take(&mut *self.queue.lock())
    }
}

/// The page-side launch for one swipe. Kept in one place so the event name and
/// payload cannot drift between the Rust and JS halves.
pub fn dispatch_script(dir: Direction) -> String {
    format!(
        "window.dispatchEvent(new CustomEvent('trackpad:gesture', \
         {{ detail: {{ direction: \"{}\" }} }}));",
        dir.as_str()
    )
}

/// Start the reader if a touchpad can be found and opened. Never fails the
/// shell: a missing device or rule leaves an empty bridge behind.
pub fn spawn() -> GestureBridge {
    let bridge = GestureBridge::default();
    let Some((path, name)) = find_touchpad() else {
        eprintln!("peakd: no touchpad found; three-finger gestures disabled");
        return bridge;
    };
    match File::open(&path) {
        Ok(file) => {
            println!(
                "peakd: reading three-finger gestures from {} ({name})",
                path.display()
            );
            let bridge_thread = bridge.clone();
            std::thread::Builder::new()
                .name("peakd-gestures".into())
                .spawn(move || run(file, &bridge_thread))
                .ok();
        }
        Err(err) => {
            eprintln!(
                "peakd: cannot read {} for gestures ({err}); install the udev rule with \
                 scripts/install-touchpad-gestures.sh",
                path.display()
            );
        }
    }
    bridge
}

/// Locate the internal trackpad's evdev node from the kernel's device list.
///
/// `/proc/bus/input/devices` groups one block per input device. Naming alone is
/// not enough on a T2 MacBook: the Touch Bar's touch surface is also called a
/// "Touchpad" (`Apple Inc. Touch Bar Display Touchpad`) and would be picked
/// first, but it is a *direct* input device, not a trackpad — and it is not
/// tagged for this user, so opening it fails. The real trackpad is
/// `Apple Inc. Apple Internal Keyboard / Trackpad` (the `…/1.2` interface).
///
/// So: match Touchpad/Trackpad by name, require absolute axes (the keyboard
/// half of a combined device has none), and reject `INPUT_PROP_DIRECT` (which
/// marks touchscreens and the Touch Bar surface). `PEAKD_TOUCHPAD` overrides
/// the search for unusual hardware.
fn find_touchpad() -> Option<(PathBuf, String)> {
    if let Ok(path) = std::env::var("PEAKD_TOUCHPAD") {
        if !path.is_empty() {
            return Some((PathBuf::from(path), "PEAKD_TOUCHPAD".into()));
        }
    }
    let text = std::fs::read_to_string("/proc/bus/input/devices").ok()?;
    parse_touchpad(&text)
}

/// The device-list half of [`find_touchpad`], split out so it can be tested
/// against a captured `/proc/bus/input/devices`.
fn parse_touchpad(text: &str) -> Option<(PathBuf, String)> {
    // `INPUT_PROP_DIRECT` — X11/input.h. Direct devices are touchscreens, not
    // trackpads. The Touch Bar surface carries it; the internal trackpad does
    // not.
    const INPUT_PROP_DIRECT: u32 = 0x2;

    let mut best: Option<(u8, PathBuf, String)> = None;
    for block in text.split("\n\n") {
        let name = block
            .lines()
            .find_map(|l| l.strip_prefix("N: Name="))
            .map(|s| s.trim().trim_matches('"').to_string());
        let lower = name.as_deref().unwrap_or("").to_lowercase();
        // Prefer an explicit "trackpad"; accept "touchpad" but never the Touch
        // Bar surface.
        let score = if lower.contains("trackpad") {
            2
        } else if lower.contains("touchpad") && !lower.contains("touch bar") {
            1
        } else {
            continue;
        };
        let Some(handlers) = block.lines().find_map(|l| l.strip_prefix("H: Handlers=")) else {
            continue;
        };
        // A trackpad is the interface with absolute axes; ignore the keyboard
        // half of a combined device.
        let has_abs = block
            .lines()
            .any(|l| matches!(l.strip_prefix("B: ABS="), Some(v) if v.trim() != "0"));
        if !has_abs {
            continue;
        }
        let prop = block
            .lines()
            .find_map(|l| l.strip_prefix("B: PROP="))
            .and_then(|v| u32::from_str_radix(v.trim(), 16).ok())
            .unwrap_or(0);
        if prop & INPUT_PROP_DIRECT != 0 {
            continue;
        }
        let Some(event) = handlers.split_whitespace().find(|h| h.starts_with("event")) else {
            continue;
        };
        if best.as_ref().map_or(true, |b| score > b.0) {
            best = Some((
                score,
                PathBuf::from(format!("/dev/input/{event}")),
                name.unwrap_or_else(|| "touchpad".into()),
            ));
        }
    }
    best.map(|(_, path, name)| (path, name))
}

/* ── evdev decoding ─────────────────────────────────────────── */

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0x00;
/// The kernel dropped events (buffer overrun); every cached contact may be
/// stale and the client must re-synchronise.
const SYN_DROPPED: u16 = 0x03;

const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TOOL_TYPE: u16 = 0x37;
const ABS_MT_TRACKING_ID: u16 = 0x39;

/// `MT_TOOL_PALM` (linux/input.h): a palm/thumb contact the kernel reports as a
/// slot but which is not a finger for gesture purposes.
const MT_TOOL_PALM: i32 = 0x02;

const BTN_TOOL_FINGER: u16 = 0x145;
const BTN_TOOL_DOUBLETAP: u16 = 0x14d;
const BTN_TOOL_TRIPLETAP: u16 = 0x14e;
const BTN_TOOL_QUADTAP: u16 = 0x14f;

/// `struct input_event` is 24 bytes on 64-bit Linux: two `long`s of timeval,
/// then type/code/value. Read as raw bytes so no alignment assumptions leak in.
const EVENT_SIZE: usize = 24;

#[derive(Clone, Copy)]
struct Slot {
    id: i32,
    x: i32,
    y: i32,
    /// `ABS_MT_TOOL_TYPE == MT_TOOL_PALM`: a palm/thumb resting on the pad. It
    /// carries a slot but must never count towards a three-finger swipe.
    palm: bool,
}

/// One in-flight three-finger contact.
struct Swipe {
    started: Instant,
    x0: i64,
    y0: i64,
    /// Per-slot origin, captured when each contact joins the gesture. Tracking
    /// every finger separately is what lets a stationary palm/thumb be ignored
    /// instead of dragging the average along with the two scrolling fingers.
    anchors: Vec<Option<(i64, i64)>>,
    /// Set once the fingers have moved off the spot; until then the origin and
    /// clock are re-anchored every frame.
    moving: bool,
    fired: bool,
}

impl Swipe {
    fn start(slots: &[Slot], cx: i64, cy: i64) -> Self {
        Self {
            started: Instant::now(),
            x0: cx,
            y0: cy,
            anchors: Self::origins(slots),
            moving: false,
            fired: false,
        }
    }

    /// The origin of every live finger, `None` for inactive or palm slots.
    fn origins(slots: &[Slot]) -> Vec<Option<(i64, i64)>> {
        slots
            .iter()
            .map(|s| (s.id >= 0 && !s.palm).then_some((s.x as i64, s.y as i64)))
            .collect()
    }

    /// Re-anchor resting contacts and restart the clock.
    fn reanchor(&mut self, slots: &[Slot], cx: i64, cy: i64) {
        self.anchors = Self::origins(slots);
        self.x0 = cx;
        self.y0 = cy;
        self.started = Instant::now();
    }

    /// Record contacts that joined after the gesture began and count how many
    /// have moved off their own origin.
    fn track(&mut self, slots: &[Slot]) -> usize {
        if self.anchors.len() < slots.len() {
            self.anchors.resize(slots.len(), None);
        }
        let mut moving = 0;
        for (i, slot) in slots.iter().enumerate() {
            if slot.id < 0 || slot.palm {
                self.anchors[i] = None;
                continue;
            }
            let anchor = match self.anchors[i] {
                Some(anchor) => anchor,
                None => {
                    self.anchors[i] = Some((slot.x as i64, slot.y as i64));
                    continue;
                }
            };
            let dx = (slot.x as i64 - anchor.0).abs();
            let dy = (slot.y as i64 - anchor.1).abs();
            if dx.max(dy) >= MOVE_EPSILON {
                moving += 1;
            }
        }
        moving
    }

    /// Contacts that have travelled in `dir` by at least the jitter epsilon.
    /// A palm/thumb, or a finger that only wobbled, does not qualify.
    fn aligned(&self, slots: &[Slot], dir: Direction) -> usize {
        slots
            .iter()
            .enumerate()
            .filter(|(i, slot)| {
                if slot.id < 0 || slot.palm {
                    return false;
                }
                let Some((x0, y0)) = self.anchors.get(*i).copied().flatten() else {
                    return false;
                };
                let dx = slot.x as i64 - x0;
                let dy = slot.y as i64 - y0;
                match dir {
                    Direction::Left => dx <= -MOVE_EPSILON,
                    Direction::Right => dx >= MOVE_EPSILON,
                    Direction::Up => dy <= -MOVE_EPSILON,
                    Direction::Down => dy >= MOVE_EPSILON,
                }
            })
            .count()
    }
}

struct Reader {
    bridge: GestureBridge,
    /// Per-axis travel that counts as a swipe.
    threshold_x: i64,
    threshold_y: i64,
    slots: Vec<Slot>,
    current_slot: usize,
    /// Last legacy `ABS_X` / `ABS_Y` sample, used when no MT slots are up.
    legacy: Option<(i32, i32)>,
    tool: [bool; 5], // 1..=4 fingers, from BTN_TOOL_*
    swipe: Option<Swipe>,
}

impl Reader {
    fn new(file: &File, bridge: GestureBridge) -> Self {
        let fallback = (0, (SWIPE_MIN_UNITS * 8) as i32);
        let (xr, yr) = abs_ranges(file.as_raw_fd()).unwrap_or((fallback, fallback));
        let threshold_x = ((xr.1 - xr.0) as f64 * SWIPE_FRACTION) as i64;
        let threshold_y = ((yr.1 - yr.0) as f64 * SWIPE_FRACTION) as i64;
        eprintln!(
            "peakd: touchpad ranges x={}..{} y={}..{} (swipe thresholds {}x{})",
            xr.0, xr.1, yr.0, yr.1, threshold_x, threshold_y
        );
        Self::with_thresholds(bridge, threshold_x, threshold_y)
    }

    /// The recognition state, with the travel thresholds given directly. Split
    /// out from `new` so the recognition logic is testable without a device.
    fn with_thresholds(bridge: GestureBridge, threshold_x: i64, threshold_y: i64) -> Self {
        Self {
            bridge,
            threshold_x: threshold_x.max(SWIPE_MIN_UNITS),
            threshold_y: threshold_y.max(SWIPE_MIN_UNITS),
            slots: Vec::new(),
            current_slot: 0,
            legacy: None,
            tool: [false; 5],
            swipe: None,
        }
    }

    fn slot(&mut self, index: usize) -> &mut Slot {
        if index >= self.slots.len() {
            self.slots
                .resize(index + 1, Slot { id: -1, x: 0, y: 0, palm: false });
        }
        &mut self.slots[index]
    }

    fn finger_count(&self) -> usize {
        // Palms carry a slot but are not fingers; skipping them here is what
        // stops a hand resting on the pad during a two-finger scroll from
        // reaching three.
        let active = self.slots.iter().filter(|s| s.id >= 0 && !s.palm).count();
        let tools = if self.tool[4] {
            4
        } else if self.tool[3] {
            3
        } else if self.tool[2] {
            2
        } else if self.tool[1] {
            1
        } else {
            0
        };
        active.max(tools)
    }

    /// Centre of the three-finger contact, preferring the MT slots and falling
    /// back to the legacy single-touch axes some pads report meanwhile.
    fn centre(&self) -> Option<(i64, i64)> {
        let mut n = 0i64;
        let mut sx = 0i64;
        let mut sy = 0i64;
        for s in self.slots.iter().filter(|s| s.id >= 0 && !s.palm) {
            n += 1;
            sx += s.x as i64;
            sy += s.y as i64;
        }
        if n > 0 {
            return Some((sx / n, sy / n));
        }
        self.legacy.map(|(x, y)| (x as i64, y as i64))
    }

    fn on_frame(&mut self) {
        let count = self.finger_count();
        if count < 3 {
            // Gesture over (or never started): the next three-finger contact
            // starts a fresh swipe even while the pad still sees a finger.
            self.swipe = None;
            return;
        }

        let Some((cx, cy)) = self.centre() else {
            return;
        };

        // Take the swipe out so the per-slot tracking can read `self.slots`
        // while it updates the anchors.
        let Some(mut swipe) = self.swipe.take() else {
            self.swipe = Some(Swipe::start(&self.slots, cx, cy));
            return;
        };
        if swipe.fired {
            self.swipe = Some(swipe);
            return;
        }

        // How many individual contacts have moved off their own origin. A
        // resting palm/thumb/ghost stays put and never contributes, so a
        // two-finger scroll cannot masquerade as a three-finger swipe.
        let moving = swipe.track(&self.slots);

        let dx = cx - swipe.x0;
        let dy = cy - swipe.y0;
        let (adx, ady) = (dx.abs(), dy.abs());

        if !swipe.moving {
            if moving == 0 {
                // Resting fingers: keep the origin under them and restart the
                // clock, so only the actual swipe counts against the timeout.
                swipe.reanchor(&self.slots, cx, cy);
                self.swipe = Some(swipe);
                return;
            }
            swipe.moving = true;
            swipe.started = Instant::now();
        }

        if swipe.started.elapsed() > Duration::from_millis(SWIPE_MAX_MS as u64) {
            swipe.fired = true; // too slow: spend the gesture without acting
            self.swipe = Some(swipe);
            return;
        }

        let dir = if adx >= self.threshold_x && adx > ady {
            Some(if dx < 0 { Direction::Left } else { Direction::Right })
        } else if ady >= self.threshold_y && ady > adx {
            // evdev y grows downward: fingers moving up mean y decreasing.
            Some(if dy < 0 { Direction::Up } else { Direction::Down })
        } else {
            None
        };

        if let Some(dir) = dir {
            // Require at least three real contacts to have travelled in the
            // gesture's own direction: two fingers scrolling beside a resting
            // third one (a palm, a thumb or a ghost slot) is not a swipe.
            if swipe.aligned(&self.slots, dir) >= 3 {
                swipe.fired = true;
                self.bridge.push(dir);
            }
        }
        self.swipe = Some(swipe);
    }

    /// The kernel dropped events: every cached contact and tool bit may be
    /// stale, so forget them and wait for fresh state.
    fn reset(&mut self) {
        self.slots.clear();
        self.current_slot = 0;
        self.legacy = None;
        self.tool = [false; 5];
        self.swipe = None;
    }

    fn handle(&mut self, type_: u16, code: u16, value: i32) {
        match type_ {
            EV_ABS => match code {
                ABS_MT_SLOT => {
                    let index = value.max(0) as usize;
                    self.current_slot = index;
                    let _ = self.slot(index);
                }
                ABS_MT_TRACKING_ID => {
                    let index = self.current_slot;
                    self.slot(index).id = value;
                }
                ABS_MT_POSITION_X => {
                    let index = self.current_slot;
                    self.slot(index).x = value;
                }
                ABS_MT_POSITION_Y => {
                    let index = self.current_slot;
                    self.slot(index).y = value;
                }
                ABS_MT_TOOL_TYPE => {
                    let index = self.current_slot;
                    self.slot(index).palm = value == MT_TOOL_PALM;
                }
                ABS_X => self.legacy = Some((value, self.legacy.map(|l| l.1).unwrap_or(0))),
                ABS_Y => self.legacy = Some((self.legacy.map(|l| l.0).unwrap_or(0), value)),
                _ => {}
            },
            EV_KEY => match code {
                BTN_TOOL_FINGER => self.tool[1] = value != 0,
                BTN_TOOL_DOUBLETAP => self.tool[2] = value != 0,
                BTN_TOOL_TRIPLETAP => self.tool[3] = value != 0,
                BTN_TOOL_QUADTAP => self.tool[4] = value != 0,
                _ => {}
            },
            EV_SYN if code == SYN_DROPPED => self.reset(),
            EV_SYN if code == SYN_REPORT => self.on_frame(),
            _ => {}
        }
    }
}

fn run(file: File, bridge: &GestureBridge) {
    let mut reader = Reader::new(&file, bridge.clone());
    let mut input = BufReader::new(file);
    let mut buf = [0u8; EVENT_SIZE];
    let mut live = false;
    loop {
        if input.read_exact(&mut buf).is_err() {
            // Device gone (unplug, suspend): stop quietly. The kiosk keeps the
            // pointer either way; only gestures are lost.
            return;
        }
        if !live {
            // Proof the stream is actually arriving (not grabbed by X): the
            // first read only returns once the pad reports something.
            eprintln!("peakd: touchpad stream live");
            live = true;
        }
        let type_ = u16::from_ne_bytes([buf[16], buf[17]]);
        let code = u16::from_ne_bytes([buf[18], buf[19]]);
        let value = i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]);
        reader.handle(type_, code, value);
    }
}

/// The `EVIOCGABS(abs)` request and a one-axis min/max read, without pulling in
/// a bindgen crate for two ioctls.
fn abs_ranges(fd: std::os::unix::io::RawFd) -> Option<((i32, i32), (i32, i32))> {
    let x = abs_range(fd, ABS_MT_POSITION_X as u32)?;
    let y = abs_range(fd, ABS_MT_POSITION_Y as u32)?;
    Some((x, y))
}

fn abs_range(fd: std::os::unix::io::RawFd, axis: u32) -> Option<(i32, i32)> {
    // _IOR('E', 0x40 + axis, struct input_absinfo), spelled out so the crate
    // needs no bindgen for the one request it makes.
    const SIZE: libc::c_ulong = std::mem::size_of::<libc::input_absinfo>() as libc::c_ulong;
    let request: libc::c_ulong =
        (2 << 30) | (SIZE << 16) | ((b'E' as libc::c_ulong) << 8) | (0x40 + axis as libc::c_ulong);
    let mut info: libc::input_absinfo = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::ioctl(fd, request, &mut info) };
    if rc == 0 {
        Some((info.minimum, info.maximum))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed one multi-touch frame: every finger's slot, id and position, then
    /// the SYN_REPORT that commits it.
    fn frame(reader: &mut Reader, fingers: &[(i32, i32)]) {
        for (i, (x, y)) in fingers.iter().enumerate() {
            let slot = i as i32;
            reader.handle(EV_ABS, ABS_MT_SLOT, slot);
            reader.handle(EV_ABS, ABS_MT_TRACKING_ID, slot + 1);
            reader.handle(EV_ABS, ABS_MT_POSITION_X, *x);
            reader.handle(EV_ABS, ABS_MT_POSITION_Y, *y);
        }
        reader.handle(EV_SYN, SYN_REPORT, 0);
    }

    fn reader() -> Reader {
        Reader::with_thresholds(GestureBridge::default(), 300, 300)
    }

    /// The device list captured from the T2 MacBook this feature targets. The
    /// Touch Bar surface comes first and must not be chosen.
    const T2_DEVICES: &str = r#"I: Bus=0003 Vendor=05ac Product=0340 Version=0101
N: Name="Apple Inc. Apple Internal Keyboard / Trackpad"
H: Handlers=sysrq kbd leds event6 
B: PROP=0
B: KEY=10000 0 0 0 101007b02001007

I: Bus=0003 Vendor=05ac Product=8302 Version=0101
N: Name="Apple Inc. Touch Bar Display Touchpad"
H: Handlers=mouse0 event8 
B: PROP=3
B: KEY=e520 0 0 0 0 0
B: ABS=a60800000000003

I: Bus=0003 Vendor=05ac Product=0340 Version=0101
N: Name="Apple Inc. Apple Internal Keyboard / Trackpad"
H: Handlers=mouse1 event9 
B: PROP=5
B: KEY=e520 10000 0 0 0 0
B: ABS=67f800001000003

"#;

    #[test]
    fn picks_the_trackpad_not_the_touch_bar() {
        let (path, name) = parse_touchpad(T2_DEVICES).expect("a trackpad");
        assert_eq!(path, PathBuf::from("/dev/input/event9"));
        assert!(name.contains("Trackpad"));
    }

    #[test]
    fn skips_a_direct_input_touchpad() {
        // A touchscreen named "Touchpad" (INPUT_PROP_DIRECT) is not a trackpad.
        let devices = r#"N: Name="Some Device Touchpad"
H: Handlers=event4 
B: PROP=3
B: ABS=67f800001000003

"#;
        assert!(parse_touchpad(devices).is_none());
    }

    #[test]
    fn three_fingers_left_is_left() {
        let mut r = reader();
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut r, &[(700, 1000), (900, 1000), (500, 1000)]);
        assert_eq!(r.bridge.drain(), vec![Direction::Left]);
    }

    #[test]
    fn three_fingers_right_is_right() {
        let mut r = reader();
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut r, &[(1300, 1000), (1500, 1000), (1100, 1000)]);
        assert_eq!(r.bridge.drain(), vec![Direction::Right]);
    }

    #[test]
    fn three_fingers_up_and_down() {
        let mut up = reader();
        frame(&mut up, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut up, &[(1000, 700), (1200, 700), (800, 700)]);
        assert_eq!(up.bridge.drain(), vec![Direction::Up]);

        let mut down = reader();
        frame(&mut down, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut down, &[(1000, 1300), (1200, 1300), (800, 1300)]);
        assert_eq!(down.bridge.drain(), vec![Direction::Down]);
    }

    #[test]
    fn two_fingers_are_not_a_gesture() {
        let mut r = reader();
        frame(&mut r, &[(1000, 1000), (1200, 1000)]);
        frame(&mut r, &[(600, 1000), (800, 1000)]);
        assert!(r.bridge.drain().is_empty());
    }

    #[test]
    fn a_resting_third_contact_is_not_a_swipe() {
        // Two fingers scroll; a thumb/ghost contact rests in the third slot and
        // never moves. The centre still travels far enough for a swipe, but
        // only two contacts did the moving, so it must not fire.
        let mut r = reader();
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut r, &[(400, 1000), (600, 1000), (800, 1000)]);
        assert!(r.bridge.drain().is_empty());
    }

    #[test]
    fn a_palm_contact_does_not_count_as_a_finger() {
        // Two fingers plus a palm the kernel reports as `MT_TOOL_PALM`.
        let mut r = reader();
        frame(&mut r, &[(1000, 1000), (1200, 1000)]);
        r.handle(EV_ABS, ABS_MT_SLOT, 2);
        r.handle(EV_ABS, ABS_MT_TRACKING_ID, 3);
        r.handle(EV_ABS, ABS_MT_POSITION_X, 800);
        r.handle(EV_ABS, ABS_MT_POSITION_Y, 1000);
        r.handle(EV_ABS, ABS_MT_TOOL_TYPE, MT_TOOL_PALM);
        r.handle(EV_SYN, SYN_REPORT, 0);
        frame(&mut r, &[(400, 1000), (600, 1000)]);
        assert!(r.bridge.drain().is_empty());
    }

    #[test]
    fn syn_dropped_forgets_stale_contacts() {
        // A three-finger contact, then the kernel drops events. The stale slots
        // must not linger and let a later two-finger scroll read as three.
        let mut r = reader();
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        r.handle(EV_SYN, SYN_DROPPED, 0);
        frame(&mut r, &[(1000, 1000), (1200, 1000)]);
        frame(&mut r, &[(400, 1000), (600, 1000)]);
        assert!(r.bridge.drain().is_empty());
    }

    #[test]
    fn a_short_move_is_not_a_swipe() {
        let mut r = reader();
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut r, &[(900, 1000), (1100, 1000), (700, 1000)]);
        assert!(r.bridge.drain().is_empty());
    }

    #[test]
    fn one_swipe_fires_once() {
        let mut r = reader();
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut r, &[(700, 1000), (900, 1000), (500, 1000)]);
        frame(&mut r, &[(400, 1000), (600, 1000), (200, 1000)]);
        frame(&mut r, &[(100, 1000), (300, 1000), (0, 1000)]);
        assert_eq!(r.bridge.drain(), vec![Direction::Left]);
    }

    #[test]
    fn lifting_the_fingers_rearms_the_gesture() {
        let mut r = reader();
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut r, &[(700, 1000), (900, 1000), (500, 1000)]);
        // Fingers up: tracking id -1 on every slot.
        for slot in 0..3 {
            r.handle(EV_ABS, ABS_MT_SLOT, slot);
            r.handle(EV_ABS, ABS_MT_TRACKING_ID, -1);
        }
        r.handle(EV_SYN, SYN_REPORT, 0);
        // A second, opposite swipe is recognised independently.
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut r, &[(1300, 1000), (1500, 1000), (1100, 1000)]);
        assert_eq!(r.bridge.drain(), vec![Direction::Left, Direction::Right]);
    }
}
