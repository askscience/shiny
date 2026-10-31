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
//! Swipes are continuous: once three synchronised fingers have moved far
//! enough to lock an axis the reader emits `begin`, then `update`s carrying a
//! signed progress along that axis while the fingers move, and `end` (with the
//! last progress and the release velocity) when they lift — or `cancel` when
//! the stream breaks. The page follows the fingers and decides on `end`
//! whether the transition completes or springs back. While a gesture is live
//! the bridge asks the shell to pump at frame rate (see [`GestureBridge`]).
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
//! A swipe needs three *moving* fingers, and those fingers must move
//! *together*. Contacts the kernel marks `MT_TOOL_PALM` are ignored, every
//! finger's own travel is tracked separately, and a swipe only fires once
//! three contacts have each travelled most of the gesture's distance — a
//! thumb dragged along by the hand lags well behind the fingers.
//!
//! `MT_TOOL_PALM` is not enough on its own, though: many pads never report
//! it. Apple's `bcm5974` trackpads (the T2 MacBook this kiosk runs on) do not
//! even advertise `ABS_MT_TOOL_TYPE`, so a resting thumb arrives as an
//! ordinary third contact. libinput — the engine behind GNOME's and KDE's
//! gestures — identifies such a thumb by *where* it is and by its size, and
//! keeps that verdict for the life of the touch. This reader does the same:
//! a contact in the bottom strip of the pad is a thumb, the single
//! size/pressure outlier in the lower pad is a thumb, and once a contact is a
//! thumb it stays one until it lifts. Re-deciding every frame (as the first
//! version did) let a thumb read as a finger again on the busy frames of a
//! scroll, which is what popped the window overview.
//!
//! Pads that report neither size nor pressure, and whose range could not be
//! read, fall back to contact travel alone.

use std::fs::File;
use std::io::{BufReader, Read};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// The reference swipe distance, as a fraction of the pad's axis range. It
/// used to be the whole discrete swipe; the continuous one locks its axis at
/// [`LOCK_FRACTION`] of it and reaches progress 1.0 at [`FULL_FACTOR`] times it.
const SWIPE_FRACTION: f64 = 0.12;
/// The floor for a pad whose range could not be read (units are arbitrary).
const SWIPE_MIN_UNITS: i64 = 120;
/// The centre travel, as a fraction of the reference distance, at which three
/// synchronised fingers lock an axis and the gesture begins. Small, so the UI
/// starts following the fingers almost at once.
const LOCK_FRACTION: f64 = 0.3;
/// Progress 1.0 is this many reference distances: a comfortable full swipe.
const FULL_FACTOR: f64 = 2.0;
/// Progress is clamped to this magnitude either way along the locked axis.
const PROGRESS_LIMIT: f64 = 1.5;
/// The release velocity is measured over this trailing window.
const VELOCITY_WINDOW: Duration = Duration::from_millis(80);
/// Updates closer than this to the last one sent are not worth a page event.
const PROGRESS_EPSILON: f64 = 0.002;
/// Movement below this is hand jitter while the fingers rest; the origin and
/// the clock keep resetting until the fingers really move.
const MOVE_EPSILON: i64 = 8;
/// A live contact in the bottom strip of the pad — this fraction of the y-axis
/// range — is a resting thumb. evdev's y grows downward, so the bottom edge is
/// the largest value. libinput calls this the "definitive thumb zone": a thumb
/// rests at the pad's edge, whatever its size. Only used when the axis range
/// is known.
const THUMB_ZONE_FRACTION: f64 = 0.10;
/// The wider bottom strip in which the single largest live contact is treated
/// as a thumb when it dwarfs the next one (libinput's pressure/size thumb
/// area). Size alone, anywhere on the pad, is too noisy: during a genuine
/// three-finger swipe every contact's reported area swings enough that one of
/// them would be misread as a thumb and the swipe lost.
const THUMB_ZONE_EXT_FRACTION: f64 = 0.20;
/// For a swipe, every finger must have travelled at least this fraction of the
/// gesture centre's own travel. Fingers that swiped together move together; a
/// thumb dragged along by the hand does not.
const SYNC_FRACTION: f64 = 0.5;
/// A live contact this many times larger than the next-largest live contact is
/// a thumb or palm, not a gesture finger. This is the portable half of palm
/// detection: pads that report `MT_TOOL_PALM` say so directly, but `bcm5974`
/// (Apple trackpads) does not, while still reporting `ABS_MT_TOUCH_MAJOR`.
/// The ratio is deliberately generous so a fingertip pressed a little harder
/// is never mistaken for a thumb; a real thumb or palm is far larger.
const THUMB_SIZE_RATIO: f64 = 1.7;
/// ... and it must be at least this much larger in absolute units too, so a
/// pad whose contact sizes only wobble is never misread.
const THUMB_SIZE_MIN_DELTA: i32 = 8;

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

    fn axis(self) -> &'static str {
        match self {
            Direction::Left | Direction::Right => "x",
            Direction::Up | Direction::Down => "y",
        }
    }
}

/// Where a continuous swipe is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Begin,
    Update,
    End,
    Cancel,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Phase::Begin => "begin",
            Phase::Update => "update",
            Phase::End => "end",
            Phase::Cancel => "cancel",
        }
    }
}

/// One step of a swipe. `direction` is the direction the gesture began in and
/// stays the same for its whole life; `progress` is signed along that
/// direction (1.0 ≈ a full swipe, negative once the fingers reverse past the
/// origin); `velocity` is progress per second, only meaningful on `End`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GestureEvent {
    pub phase: Phase,
    pub direction: Direction,
    pub progress: f64,
    pub velocity: f64,
}

/// Called on the reader thread when a gesture goes live (`true`) or idle
/// (`false`), so the shell can pump at frame rate only while fingers move.
pub type Waker = fn(bool);

/// Queue of swipe events, filled by the reader thread and drained on the
/// event loop (the only thread allowed to touch the webview). Cheap to clone.
///
/// Consecutive updates coalesce — only the latest one between two drains is
/// worth sending, the page only renders frames — while `begin`, `end` and
/// `cancel` are never dropped.
#[derive(Clone, Default)]
pub struct GestureBridge {
    queue: Arc<Mutex<Vec<GestureEvent>>>,
    waker: Option<Waker>,
}

impl GestureBridge {
    fn with_waker(waker: Waker) -> Self {
        Self {
            queue: Arc::default(),
            waker: Some(waker),
        }
    }

    fn push(&self, event: GestureEvent) {
        {
            let mut queue = self.queue.lock();
            match queue.last_mut() {
                Some(last) if last.phase == Phase::Update && event.phase == Phase::Update => {
                    *last = event;
                }
                _ => queue.push(event),
            }
        }
        // Outside the lock: the waker queues work onto the GUI thread, whose
        // pump takes the same lock.
        if let Some(wake) = self.waker {
            match event.phase {
                Phase::Begin => wake(true),
                Phase::End | Phase::Cancel => wake(false),
                Phase::Update => {}
            }
        }
    }

    pub fn drain(&self) -> Vec<GestureEvent> {
        std::mem::take(&mut *self.queue.lock())
    }
}

/// The page-side launch for one swipe event. Kept in one place so the event
/// name and payload cannot drift between the Rust and JS halves. `direction`
/// rides on every phase, so a listener that only knows the old discrete
/// `{ direction }` detail still reads something sensible.
pub fn dispatch_script(event: &GestureEvent) -> String {
    let finite = |v: f64| if v.is_finite() { v } else { 0.0 };
    format!(
        "window.dispatchEvent(new CustomEvent('trackpad:gesture', \
         {{ detail: {{ phase: \"{}\", axis: \"{}\", direction: \"{}\", \
         progress: {:.4}, velocity: {:.4} }} }}));",
        event.phase.as_str(),
        event.direction.axis(),
        event.direction.as_str(),
        finite(event.progress),
        finite(event.velocity),
    )
}

/// Start the reader if a touchpad can be found and opened. Never fails the
/// shell: a missing device or rule leaves an empty bridge behind.
pub fn spawn(waker: Waker) -> GestureBridge {
    let bridge = GestureBridge::with_waker(waker);
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
        if best.as_ref().is_none_or(|b| score > b.0) {
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
const ABS_MT_TOUCH_MAJOR: u16 = 0x30;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TOOL_TYPE: u16 = 0x37;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const ABS_MT_PRESSURE: u16 = 0x3a;

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
    /// `ABS_MT_TOOL_TYPE == MT_TOOL_PALM`: a palm/thumb the kernel itself
    /// labelled. It carries a slot but must never count towards a swipe.
    palm: bool,
    /// `ABS_MT_TOUCH_MAJOR`: how much of the pad this contact covers. 0 when
    /// the pad does not report a size axis.
    touch: i32,
    /// `ABS_MT_PRESSURE`: the same idea for a pad that reports pressure
    /// instead of (or as well as) contact size. 0 when unreported.
    pressure: i32,
    /// Set once this contact reads as a thumb — the kernel's palm label, a
    /// position in the bottom strip of the pad, or a size/pressure outlier in
    /// the lower pad — and kept until the contact lifts. libinput keeps a
    /// palm/thumb a palm/thumb for the life of the touch; clearing the verdict
    /// every frame let a thumb flicker back to being counted as a finger.
    thumb: bool,
}

impl Slot {
    /// A contact that may take part in a swipe: live, and neither a kernel
    /// palm nor a thumb.
    fn is_finger(&self) -> bool {
        self.id >= 0 && !self.palm && !self.thumb
    }
}

/// One in-flight three-finger contact.
struct Swipe {
    x0: i64,
    y0: i64,
    /// The centre and finger count on the previous frame, so a finger joining
    /// or leaving can shift the origin instead of jumping the progress.
    cx: i64,
    cy: i64,
    fingers: usize,
    /// Per-slot origin, captured when each contact joins the gesture. Tracking
    /// every finger separately is what lets a stationary palm/thumb be ignored
    /// instead of dragging the average along with the two scrolling fingers.
    anchors: Vec<Option<(i64, i64)>>,
    /// Set once the fingers have moved off the spot; until then the origin is
    /// re-anchored every frame.
    moving: bool,
    /// The direction the gesture locked onto at `begin`; `None` until then.
    live: Option<Direction>,
    /// Set once the swipe passed the old discrete threshold and three fingers
    /// were still travelling together there.
    verified: bool,
    /// The gesture was cancelled (or never qualified); ignore the rest of the
    /// touch until the fingers lift.
    spent: bool,
    /// The latest progress, and the last one actually sent as an update.
    progress: f64,
    sent: f64,
    /// Recent `(time, progress)` samples for the release velocity: the newest
    /// ones plus a single sample at or before the start of the window.
    samples: VecDeque<(Instant, f64)>,
}

impl Swipe {
    fn start(slots: &[Slot], cx: i64, cy: i64) -> Self {
        Self {
            x0: cx,
            y0: cy,
            cx,
            cy,
            fingers: slots.iter().filter(|s| s.is_finger()).count(),
            anchors: Self::origins(slots),
            moving: false,
            live: None,
            verified: false,
            spent: false,
            progress: 0.0,
            sent: 0.0,
            samples: VecDeque::new(),
        }
    }

    /// The origin of every live finger, `None` for inactive, palm or thumb
    /// slots.
    fn origins(slots: &[Slot]) -> Vec<Option<(i64, i64)>> {
        slots
            .iter()
            .map(|s| s.is_finger().then_some((s.x as i64, s.y as i64)))
            .collect()
    }

    /// Re-anchor resting contacts.
    fn reanchor(&mut self, slots: &[Slot], cx: i64, cy: i64) {
        self.anchors = Self::origins(slots);
        self.x0 = cx;
        self.y0 = cy;
    }

    /// Remember the progress at `now` for the release velocity.
    fn record(&mut self, now: Instant, progress: f64) {
        self.progress = progress;
        self.samples.push_back((now, progress));
        if let Some(cutoff) = now.checked_sub(VELOCITY_WINDOW) {
            while self.samples.len() > 2 && self.samples[1].0 <= cutoff {
                self.samples.pop_front();
            }
        }
    }

    /// Progress per second over the trailing [`VELOCITY_WINDOW`], measured up
    /// to `now` (the release), so fingers that stopped before lifting read as
    /// a slow release rather than as their last flick.
    fn velocity(&self, now: Instant) -> f64 {
        let Some(&(_, last)) = self.samples.back() else {
            return 0.0;
        };
        let cutoff = now.checked_sub(VELOCITY_WINDOW);
        let (t_ref, p_ref) = self
            .samples
            .iter()
            .rev()
            .find(|(t, _)| cutoff.is_some_and(|c| *t <= c))
            .copied()
            .unwrap_or(self.samples[0]);
        let dt = now.saturating_duration_since(t_ref).as_secs_f64().max(0.008);
        ((last - p_ref) / dt).clamp(-50.0, 50.0)
    }

    /// Record contacts that joined after the gesture began and count how many
    /// have moved off their own origin.
    fn track(&mut self, slots: &[Slot]) -> usize {
        if self.anchors.len() < slots.len() {
            self.anchors.resize(slots.len(), None);
        }
        let mut moving = 0;
        for (i, slot) in slots.iter().enumerate() {
            if !slot.is_finger() {
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

    /// Contacts that have travelled in `dir` by at least the jitter epsilon
    /// and at least [`SYNC_FRACTION`] of the gesture's own travel. A thumb
    /// dragged along by the hand lags well behind the fingers, so it never
    /// qualifies; fingers that swiped together do. This is libinput's rule
    /// that a swipe is fingers moving *synchronously*.
    fn synced(&self, slots: &[Slot], dir: Direction, travel: i64) -> usize {
        let need = ((travel as f64) * SYNC_FRACTION) as i64;
        let need = need.max(MOVE_EPSILON);
        slots
            .iter()
            .enumerate()
            .filter(|(i, slot)| {
                if !slot.is_finger() {
                    return false;
                }
                let Some((x0, y0)) = self.anchors.get(*i).copied().flatten() else {
                    return false;
                };
                let dx = slot.x as i64 - x0;
                let dy = slot.y as i64 - y0;
                match dir {
                    Direction::Left => dx <= -need,
                    Direction::Right => dx >= need,
                    Direction::Up => dy <= -need,
                    Direction::Down => dy >= need,
                }
            })
            .count()
    }
}

struct Reader {
    bridge: GestureBridge,
    /// Per-axis reference swipe distance (the old discrete threshold).
    threshold_x: i64,
    threshold_y: i64,
    /// The largest y is the bottom edge of the pad. At or above `thumb_zone_y`
    /// a contact is a resting thumb; at or above `thumb_ext_y` the single
    /// largest contact is a thumb when it dwarfs the next. `None` when the
    /// pad's range could not be read, in which case position is not used.
    thumb_zone_y: Option<i64>,
    thumb_ext_y: Option<i64>,
    slots: Vec<Slot>,
    current_slot: usize,
    /// Last legacy `ABS_X` / `ABS_Y` sample, used when no MT slots are up.
    legacy: Option<(i32, i32)>,
    tool: [bool; 5], // 1..=4 fingers, from BTN_TOOL_*
    swipe: Option<Swipe>,
    /// A fixed clock for the tests; `None` reads the real one.
    clock: Option<Instant>,
}

impl Reader {
    fn new(file: &File, bridge: GestureBridge) -> Self {
        let fallback = (0, (SWIPE_MIN_UNITS * 8) as i32);
        let ranges = abs_ranges(file.as_raw_fd());
        let (xr, yr) = ranges.unwrap_or((fallback, fallback));
        let threshold_x = ((xr.1 - xr.0) as f64 * SWIPE_FRACTION) as i64;
        let threshold_y = ((yr.1 - yr.0) as f64 * SWIPE_FRACTION) as i64;
        eprintln!(
            "peakd: touchpad ranges x={}..{} y={}..{} (swipe thresholds {}x{})",
            xr.0, xr.1, yr.0, yr.1, threshold_x, threshold_y
        );
        // The bottom thumb zones need the real y range; without it, fall back
        // to travel alone rather than guess where the pad's edge is.
        match ranges {
            Some((_, yr)) => Self::with_geometry(bridge, threshold_x, threshold_y, yr),
            None => Self::with_thresholds(bridge, threshold_x, threshold_y),
        }
    }

    /// The recognition state, with the travel thresholds given directly. Split
    /// out from `new` so the recognition logic is testable without a device.
    /// No thumb zones: tests that want them use [`Reader::with_geometry`].
    fn with_thresholds(bridge: GestureBridge, threshold_x: i64, threshold_y: i64) -> Self {
        Self::with_zones(bridge, threshold_x, threshold_y, None, None)
    }

    /// The recognition state with the pad's y range known, so the bottom
    /// thumb zones can be placed.
    fn with_geometry(
        bridge: GestureBridge,
        threshold_x: i64,
        threshold_y: i64,
        y_range: (i32, i32),
    ) -> Self {
        // evdev y grows downward: the bottom edge, where a thumb rests, is the
        // largest value.
        let span = (y_range.1 - y_range.0) as f64;
        let zone = y_range.1 as i64 - (span * THUMB_ZONE_FRACTION) as i64;
        let ext = y_range.1 as i64 - (span * THUMB_ZONE_EXT_FRACTION) as i64;
        Self::with_zones(bridge, threshold_x, threshold_y, Some(zone), Some(ext))
    }

    fn with_zones(
        bridge: GestureBridge,
        threshold_x: i64,
        threshold_y: i64,
        thumb_zone_y: Option<i64>,
        thumb_ext_y: Option<i64>,
    ) -> Self {
        Self {
            bridge,
            threshold_x: threshold_x.max(SWIPE_MIN_UNITS),
            threshold_y: threshold_y.max(SWIPE_MIN_UNITS),
            thumb_zone_y,
            thumb_ext_y,
            slots: Vec::new(),
            current_slot: 0,
            legacy: None,
            tool: [false; 5],
            swipe: None,
            clock: None,
        }
    }

    fn now(&self) -> Instant {
        self.clock.unwrap_or_else(Instant::now)
    }

    /// Signed progress along `dir` for a centre displacement: 1.0 is a full
    /// swipe, negative once the fingers came back past the origin.
    fn progress(&self, dir: Direction, dx: i64, dy: i64) -> f64 {
        let full_x = self.threshold_x as f64 * FULL_FACTOR;
        let full_y = self.threshold_y as f64 * FULL_FACTOR;
        let along = match dir {
            Direction::Left => -dx as f64 / full_x,
            Direction::Right => dx as f64 / full_x,
            // evdev y grows downward: fingers moving up mean y decreasing.
            Direction::Up => -dy as f64 / full_y,
            Direction::Down => dy as f64 / full_y,
        };
        along.clamp(-PROGRESS_LIMIT, PROGRESS_LIMIT)
    }

    /// The fingers lifted: a live gesture ends with its last progress and the
    /// release velocity.
    fn finish(&mut self, swipe: Swipe) {
        let Some(direction) = swipe.live else { return };
        if swipe.spent {
            return;
        }
        let now = self.now();
        self.bridge.push(GestureEvent {
            phase: Phase::End,
            direction,
            progress: swipe.progress,
            velocity: swipe.velocity(now),
        });
    }

    /// The stream broke (dropped events, device gone): a live gesture is
    /// cancelled so the page never stays half-way.
    fn cancel(&mut self) {
        let Some(swipe) = self.swipe.as_mut() else { return };
        let Some(direction) = swipe.live else { return };
        if swipe.spent {
            return;
        }
        swipe.spent = true;
        let progress = swipe.progress;
        self.bridge.push(GestureEvent {
            phase: Phase::Cancel,
            direction,
            progress,
            velocity: 0.0,
        });
    }

    fn slot(&mut self, index: usize) -> &mut Slot {
        if index >= self.slots.len() {
            self.slots.resize(
                index + 1,
                Slot {
                    id: -1,
                    x: 0,
                    y: 0,
                    palm: false,
                    touch: 0,
                    pressure: 0,
                    thumb: false,
                },
            );
        }
        &mut self.slots[index]
    }

    /// Mark the contacts that are thumbs or palms so they never count towards
    /// a swipe, the way libinput's thumb detection does.
    ///
    /// Three signals, in the order libinput uses them:
    ///
    /// * the kernel's own `MT_TOOL_PALM` label (set in the event handling and
    ///   sticky for the touch);
    /// * position — a contact in the bottom strip of the pad is a resting
    ///   thumb (the "definitive thumb zone"), whatever its size;
    /// * size or pressure — the single contact that dwarfs every other live
    ///   one, but only in the lower pad, is a thumb even without the kernel
    ///   label (Apple's `bcm5974` sends no `MT_TOOL_PALM`).
    ///
    /// The verdict is never cleared while the contact is down: a thumb is a
    /// thumb for the life of the touch, so it cannot flicker back to a finger
    /// on the busy frames of a scroll. It is cleared when the slot's tracking
    /// id changes (see [`Reader::handle`]).
    ///
    /// A pad whose y range was not readable, and which reports neither size
    /// nor pressure, classifies nothing and keeps the older behaviour.
    fn classify_thumbs(&mut self) {
        let zone = self.thumb_zone_y;
        let ext = self.thumb_ext_y;

        for slot in &mut self.slots {
            if slot.id < 0 {
                slot.thumb = false;
                continue;
            }
            if zone.is_some_and(|z| slot.y as i64 >= z) {
                slot.thumb = true;
            }
        }

        let Some(ext) = ext else {
            return;
        };

        // Compare like with like: use the size axis if the pad sends it this
        // frame, else pressure, but never a mix of the two scales.
        let use_touch = self
            .slots
            .iter()
            .any(|s| s.id >= 0 && !s.palm && !s.thumb && s.touch > 0);
        let weight = |s: &Slot| if use_touch { s.touch } else { s.pressure };

        // The single largest live contact, but only one sitting in the lower
        // pad: a size outlier higher up is a finger that happens to read large,
        // not a thumb.
        let mut live: Vec<(usize, i32)> = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.id >= 0 && !s.palm && !s.thumb && weight(s) > 0)
            .map(|(i, s)| (i, weight(s)))
            .collect();
        if live.len() < 2 {
            return;
        }
        live.sort_by_key(|&(_, w)| w);
        let (biggest, big_w) = live[live.len() - 1];
        let second_w = live[live.len() - 2].1;
        if self.slots[biggest].y as i64 >= ext
            && big_w - second_w >= THUMB_SIZE_MIN_DELTA
            && big_w as f64 >= second_w as f64 * THUMB_SIZE_RATIO
        {
            self.slots[biggest].thumb = true;
        }
    }

    fn finger_count(&self) -> usize {
        // Palms and thumbs carry a slot but are not fingers; skipping them
        // here is what stops a hand resting on the pad during a two-finger
        // scroll from reaching three.
        let active = self.slots.iter().filter(|s| s.is_finger()).count();
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
        for s in self.slots.iter().filter(|s| s.is_finger()) {
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
        // Re-classify palms/thumbs for this frame before counting fingers: a
        // thumb that drifted onto the pad mid-scroll must not become the third
        // finger that starts the gesture.
        self.classify_thumbs();

        let count = self.finger_count();
        if count < 3 {
            // Gesture over (or never started): a live one ends here, and the
            // next three-finger contact starts a fresh swipe even while the
            // pad still sees a finger.
            if let Some(swipe) = self.swipe.take() {
                self.finish(swipe);
            }
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
        if swipe.spent {
            self.swipe = Some(swipe);
            return;
        }

        // A fourth finger landing (or lifting) moves the centre without the
        // hand moving: shift the origin by the jump so progress stays put.
        let fingers = self.slots.iter().filter(|s| s.is_finger()).count();
        if fingers != swipe.fingers {
            swipe.x0 += cx - swipe.cx;
            swipe.y0 += cy - swipe.cy;
            swipe.fingers = fingers;
        }
        swipe.cx = cx;
        swipe.cy = cy;

        // How many individual contacts have moved off their own origin. A
        // resting palm/thumb/ghost stays put and never contributes, so a
        // two-finger scroll cannot masquerade as a three-finger swipe.
        let moving = swipe.track(&self.slots);

        let dx = cx - swipe.x0;
        let dy = cy - swipe.y0;
        let (adx, ady) = (dx.abs(), dy.abs());

        let Some(dir) = swipe.live else {
            if !swipe.moving {
                if moving == 0 {
                    // Resting fingers: keep the origin under them.
                    swipe.reanchor(&self.slots, cx, cy);
                    self.swipe = Some(swipe);
                    return;
                }
                swipe.moving = true;
            }
            let lock_x = (self.threshold_x as f64 * LOCK_FRACTION) as i64;
            let lock_y = (self.threshold_y as f64 * LOCK_FRACTION) as i64;
            let dir = if adx >= lock_x && adx > ady {
                Some(if dx < 0 { Direction::Left } else { Direction::Right })
            } else if ady >= lock_y && ady > adx {
                Some(if dy < 0 { Direction::Up } else { Direction::Down })
            } else {
                None
            };
            // Require at least three real contacts to have travelled *with*
            // the gesture: two fingers scrolling beside a resting thumb (a
            // palm, a thumb or a ghost slot) is not a swipe, and neither is a
            // thumb dragged along by the hand.
            if let Some(dir) = dir {
                if swipe.synced(&self.slots, dir, adx.max(ady)) >= 3 {
                    let progress = self.progress(dir, dx, dy);
                    swipe.live = Some(dir);
                    swipe.sent = progress;
                    swipe.record(self.now(), progress);
                    self.bridge.push(GestureEvent {
                        phase: Phase::Begin,
                        direction: dir,
                        progress,
                        velocity: 0.0,
                    });
                }
            }
            self.swipe = Some(swipe);
            return;
        };

        // The axis is locked early, so check synchronisation once more where
        // the discrete swipe used to fire: three fingers must still be moving
        // together at the reference distance, or it was a scroll with a
        // straggler all along and the page rolls back.
        let (signed, threshold) = match dir {
            Direction::Left | Direction::Right => (dx, self.threshold_x),
            Direction::Up | Direction::Down => (dy, self.threshold_y),
        };
        let travel = signed.abs();
        if !swipe.verified && travel >= threshold {
            let now_dir = match (dir, signed < 0) {
                (Direction::Left | Direction::Right, true) => Direction::Left,
                (Direction::Left | Direction::Right, false) => Direction::Right,
                (_, true) => Direction::Up,
                (_, false) => Direction::Down,
            };
            if swipe.synced(&self.slots, now_dir, travel) < 3 {
                self.swipe = Some(swipe);
                self.cancel();
                return;
            }
            swipe.verified = true;
        }

        let progress = self.progress(dir, dx, dy);
        swipe.record(self.now(), progress);
        if (progress - swipe.sent).abs() >= PROGRESS_EPSILON {
            swipe.sent = progress;
            self.bridge.push(GestureEvent {
                phase: Phase::Update,
                direction: dir,
                progress,
                velocity: 0.0,
            });
        }
        self.swipe = Some(swipe);
    }

    /// The kernel dropped events: every cached contact and tool bit may be
    /// stale, so forget them and wait for fresh state.
    fn reset(&mut self) {
        self.cancel();
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
                    let slot = self.slot(index);
                    if value != slot.id {
                        // A new contact in this slot: forget the previous
                        // touch's sticky palm/thumb verdict.
                        slot.palm = false;
                        slot.thumb = false;
                    }
                    slot.id = value;
                }
                ABS_MT_POSITION_X => {
                    let index = self.current_slot;
                    self.slot(index).x = value;
                }
                ABS_MT_POSITION_Y => {
                    let index = self.current_slot;
                    self.slot(index).y = value;
                }
                ABS_MT_TOUCH_MAJOR => {
                    let index = self.current_slot;
                    self.slot(index).touch = value;
                }
                ABS_MT_PRESSURE => {
                    let index = self.current_slot;
                    self.slot(index).pressure = value;
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
            // pointer either way; only gestures are lost — and a live one is
            // cancelled so the page does not stay half-way.
            reader.cancel();
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

    /// Like [`frame`], but each contact also reports its contact area, so the
    /// size-based thumb/palm filter can be exercised (the Apple `bcm5974` pad
    /// never reports `MT_TOOL_PALM`, only `ABS_MT_TOUCH_MAJOR`).
    fn frame_sized(reader: &mut Reader, fingers: &[(i32, i32, i32)]) {
        for (i, (x, y, size)) in fingers.iter().enumerate() {
            let slot = i as i32;
            reader.handle(EV_ABS, ABS_MT_SLOT, slot);
            reader.handle(EV_ABS, ABS_MT_TRACKING_ID, slot + 1);
            reader.handle(EV_ABS, ABS_MT_POSITION_X, *x);
            reader.handle(EV_ABS, ABS_MT_POSITION_Y, *y);
            reader.handle(EV_ABS, ABS_MT_TOUCH_MAJOR, *size);
        }
        reader.handle(EV_SYN, SYN_REPORT, 0);
    }

    /// Like [`frame_sized`], but contact area is reported as pressure on a pad
    /// that has no size axis.
    fn frame_pressured(reader: &mut Reader, fingers: &[(i32, i32, i32)]) {
        for (i, (x, y, pressure)) in fingers.iter().enumerate() {
            let slot = i as i32;
            reader.handle(EV_ABS, ABS_MT_SLOT, slot);
            reader.handle(EV_ABS, ABS_MT_TRACKING_ID, slot + 1);
            reader.handle(EV_ABS, ABS_MT_POSITION_X, *x);
            reader.handle(EV_ABS, ABS_MT_POSITION_Y, *y);
            reader.handle(EV_ABS, ABS_MT_PRESSURE, *pressure);
        }
        reader.handle(EV_SYN, SYN_REPORT, 0);
    }

    fn reader() -> Reader {
        Reader::with_thresholds(GestureBridge::default(), 300, 300)
    }

    /// The directions of the gestures that began since the last drain.
    fn begins(r: &Reader) -> Vec<Direction> {
        r.bridge
            .drain()
            .iter()
            .filter(|e| e.phase == Phase::Begin)
            .map(|e| e.direction)
            .collect()
    }

    fn phases(events: &[GestureEvent]) -> Vec<Phase> {
        events.iter().map(|e| e.phase).collect()
    }

    /// Lift every finger: tracking id -1 on each slot.
    fn lift(r: &mut Reader, slots: i32) {
        for slot in 0..slots {
            r.handle(EV_ABS, ABS_MT_SLOT, slot);
            r.handle(EV_ABS, ABS_MT_TRACKING_ID, -1);
        }
        r.handle(EV_SYN, SYN_REPORT, 0);
    }

    /// Advance the reader's clock to `ms` after `base`.
    fn at(r: &mut Reader, base: Instant, ms: u64) {
        r.clock = Some(base + Duration::from_millis(ms));
    }

    /// A reader whose pad spans y = 0..2000. evdev y grows downward, so the
    /// bottom edge is 2000, the definitive thumb zone starts at 1800 and the
    /// size-outlier zone at 1600.
    fn reader_with_thumb_zone() -> Reader {
        Reader::with_geometry(GestureBridge::default(), 300, 300, (0, 2000))
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
        assert_eq!(begins(&r), vec![Direction::Left]);
    }

    #[test]
    fn three_fingers_right_is_right() {
        let mut r = reader();
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut r, &[(1300, 1000), (1500, 1000), (1100, 1000)]);
        assert_eq!(begins(&r), vec![Direction::Right]);
    }

    #[test]
    fn three_fingers_up_and_down() {
        let mut up = reader();
        frame(&mut up, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut up, &[(1000, 700), (1200, 700), (800, 700)]);
        assert_eq!(begins(&up), vec![Direction::Up]);

        let mut down = reader();
        frame(&mut down, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut down, &[(1000, 1300), (1200, 1300), (800, 1300)]);
        assert_eq!(begins(&down), vec![Direction::Down]);
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
    fn a_large_moving_thumb_does_not_make_a_scroll_a_swipe() {
        // The reported bug: a two-finger scroll while a thumb rests at the
        // bottom of the pad and rides along a little with the hand. The
        // T2/bcm5974 pad never reports MT_TOOL_PALM, but the thumb sits in the
        // bottom thumb zone (and is also far larger), so it is dropped. The
        // kernel still counts it (BTN_TOOL_TRIPLETAP), so the finger total
        // really is three — the thumb filter is what has to stop the swipe.
        let mut r = reader_with_thumb_zone();
        r.handle(EV_KEY, BTN_TOOL_TRIPLETAP, 1);
        frame_sized(&mut r, &[(1000, 500, 20), (1200, 500, 22), (800, 1900, 80)]);
        frame_sized(&mut r, &[(400, 500, 20), (600, 500, 22), (200, 1900, 81)]);
        assert!(r.bridge.drain().is_empty());
    }

    #[test]
    fn a_moving_thumb_is_ignored_by_pressure_too() {
        // A pad with no size axis but an ABS_MT_PRESSURE axis gets the same
        // treatment.
        let mut r = reader_with_thumb_zone();
        r.handle(EV_KEY, BTN_TOOL_TRIPLETAP, 1);
        frame_pressured(&mut r, &[(1000, 500, 20), (1200, 500, 22), (800, 1900, 80)]);
        frame_pressured(&mut r, &[(400, 500, 20), (600, 500, 22), (200, 1900, 81)]);
        assert!(r.bridge.drain().is_empty());
    }

    #[test]
    fn a_bottom_thumb_of_normal_size_is_not_a_finger() {
        // Position alone is enough: a resting thumb at the pad's edge is a
        // thumb even when every contact reads the same size. This is the case
        // the old size-only filter could not see.
        let mut r = reader_with_thumb_zone();
        frame_sized(&mut r, &[(1000, 500, 30), (1200, 500, 30), (800, 1900, 30)]);
        frame_sized(&mut r, &[(400, 500, 30), (600, 500, 30), (200, 1900, 30)]);
        assert!(r.bridge.drain().is_empty());
    }

    #[test]
    fn a_large_contact_in_the_lower_pad_is_a_thumb() {
        // A thumb resting just above the definitive zone is still caught by
        // the size outlier test, but a large contact up with the fingers is
        // not — that is what a real three-finger swipe looks like.
        let mut r = reader_with_thumb_zone();
        frame_sized(&mut r, &[(1000, 500, 20), (1200, 500, 22), (800, 1700, 90)]);
        frame_sized(&mut r, &[(400, 500, 20), (600, 500, 22), (200, 1700, 91)]);
        assert!(r.bridge.drain().is_empty());
    }

    #[test]
    fn a_thumb_stays_a_thumb_for_the_whole_touch() {
        // Once a contact is in the thumb zone it stays a thumb even after it
        // leaves the zone, so it cannot flicker back to a finger mid-scroll.
        let mut r = reader_with_thumb_zone();
        frame_sized(&mut r, &[(1000, 500, 20), (1200, 500, 22), (800, 1900, 80)]);
        // The thumb moves up out of the zone, still travelling with the hand.
        frame_sized(&mut r, &[(400, 500, 20), (600, 500, 22), (200, 1500, 80)]);
        assert!(r.bridge.drain().is_empty());
    }

    #[test]
    fn three_fingers_of_similar_size_still_swipe() {
        // The filter must not eat a real three-finger swipe just because the
        // pad reports sizes.
        let mut r = reader();
        frame_sized(&mut r, &[(1000, 1000, 20), (1200, 1000, 22), (800, 1000, 24)]);
        frame_sized(&mut r, &[(700, 1000, 20), (900, 1000, 22), (500, 1000, 24)]);
        assert_eq!(begins(&r), vec![Direction::Left]);
    }

    #[test]
    fn fluctuating_sizes_in_a_three_finger_swipe_still_swipe() {
        // Regression: on real hardware the reported areas swing from frame to
        // frame. A per-frame size test marked one of the fingers a thumb and
        // ate the swipe; position-based, sticky thumb detection must not.
        let mut r = reader_with_thumb_zone();
        frame_sized(&mut r, &[(1000, 500, 560), (1200, 500, 160), (800, 500, 170)]);
        frame_sized(&mut r, &[(700, 500, 50), (900, 500, 500), (500, 500, 60)]);
        frame_sized(&mut r, &[(400, 500, 200), (600, 500, 90), (200, 500, 420)]);
        assert_eq!(begins(&r), vec![Direction::Left]);
    }

    #[test]
    fn three_large_but_equal_contacts_still_swipe() {
        // A pad that reports big areas for every contact: only the outlier is
        // dropped, never the whole hand.
        let mut r = reader();
        frame_sized(&mut r, &[(1000, 1000, 90), (1200, 1000, 92), (800, 1000, 94)]);
        frame_sized(&mut r, &[(700, 1000, 90), (900, 1000, 92), (500, 1000, 94)]);
        assert_eq!(begins(&r), vec![Direction::Left]);
    }

    #[test]
    fn a_slightly_larger_finger_is_not_a_thumb() {
        // A fingertip pressed a little harder than its neighbours must not be
        // mistaken for a thumb and dropped.
        let mut r = reader();
        frame_sized(&mut r, &[(1000, 1000, 30), (1200, 1000, 22), (800, 1000, 24)]);
        frame_sized(&mut r, &[(700, 1000, 30), (900, 1000, 22), (500, 1000, 24)]);
        assert_eq!(begins(&r), vec![Direction::Left]);
    }

    #[test]
    fn a_thumb_dragged_along_is_not_synchronised_with_the_fingers() {
        // Three contacts, all above the thumb zone, but the third only drifts
        // while the fingers swipe. Synchronous movement is what separates a
        // swipe from a scroll with a stray contact.
        let mut r = reader_with_thumb_zone();
        frame_sized(&mut r, &[(1000, 500, 30), (1200, 500, 30), (800, 700, 30)]);
        frame_sized(&mut r, &[(400, 500, 30), (600, 500, 30), (790, 700, 30)]);
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
    fn a_tiny_move_does_not_begin() {
        // Below the lock distance (30% of the reference swipe) nothing starts.
        let mut r = reader();
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut r, &[(950, 1000), (1150, 1000), (750, 1000)]);
        lift(&mut r, 3);
        assert!(r.bridge.drain().is_empty());
    }

    #[test]
    fn a_short_move_begins_and_ends_short() {
        // Past the lock distance the page starts following, but a short swipe
        // ends well below half-way, so the page rolls it back.
        let mut r = reader();
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut r, &[(900, 1000), (1100, 1000), (700, 1000)]);
        lift(&mut r, 3);
        let events = r.bridge.drain();
        assert_eq!(phases(&events), vec![Phase::Begin, Phase::End]);
        assert!(events[1].progress > 0.0 && events[1].progress < 0.5);
    }

    #[test]
    fn one_swipe_begins_once() {
        let mut r = reader();
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut r, &[(700, 1000), (900, 1000), (500, 1000)]);
        frame(&mut r, &[(400, 1000), (600, 1000), (200, 1000)]);
        frame(&mut r, &[(100, 1000), (300, 1000), (0, 1000)]);
        assert_eq!(begins(&r), vec![Direction::Left]);
    }

    #[test]
    fn a_swipe_is_begin_update_end() {
        let base = Instant::now();
        let mut r = reader();
        at(&mut r, base, 0);
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        at(&mut r, base, 16);
        frame(&mut r, &[(880, 1000), (1080, 1000), (680, 1000)]);
        let first = r.bridge.drain();
        assert_eq!(phases(&first), vec![Phase::Begin]);
        assert_eq!(first[0].direction, Direction::Left);
        // 120 units of a 600-unit full swipe.
        assert!((first[0].progress - 0.2).abs() < 1e-9);

        at(&mut r, base, 32);
        frame(&mut r, &[(700, 1000), (900, 1000), (500, 1000)]);
        let second = r.bridge.drain();
        assert_eq!(phases(&second), vec![Phase::Update]);
        assert!((second[0].progress - 0.5).abs() < 1e-9);

        at(&mut r, base, 48);
        frame(&mut r, &[(520, 1000), (720, 1000), (320, 1000)]);
        at(&mut r, base, 56);
        lift(&mut r, 3);
        let last = r.bridge.drain();
        assert_eq!(phases(&last), vec![Phase::Update, Phase::End]);
        assert!((last[1].progress - 0.8).abs() < 1e-9);
        // 0.6 progress in the 56 ms since the begin sample: a fast flick.
        assert!(last[1].velocity > 5.0, "velocity {}", last[1].velocity);
    }

    #[test]
    fn updates_coalesce_between_drains() {
        let mut r = reader();
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut r, &[(880, 1000), (1080, 1000), (680, 1000)]);
        frame(&mut r, &[(800, 1000), (1000, 1000), (600, 1000)]);
        frame(&mut r, &[(700, 1000), (900, 1000), (500, 1000)]);
        frame(&mut r, &[(640, 1000), (840, 1000), (440, 1000)]);
        lift(&mut r, 3);
        let events = r.bridge.drain();
        // Begin and end are kept; only the newest update survives in between.
        assert_eq!(phases(&events), vec![Phase::Begin, Phase::Update, Phase::End]);
        assert!((events[1].progress - 0.6).abs() < 1e-9);
    }

    #[test]
    fn reversing_past_the_origin_goes_negative() {
        let mut r = reader();
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut r, &[(1000, 880), (1200, 880), (800, 880)]);
        frame(&mut r, &[(1000, 1150), (1200, 1150), (800, 1150)]);
        let events = r.bridge.drain();
        assert_eq!(events[0].direction, Direction::Up);
        assert_eq!(events[1].phase, Phase::Update);
        assert_eq!(events[1].direction, Direction::Up);
        assert!(events[1].progress < 0.0);
    }

    #[test]
    fn a_slow_swipe_is_still_a_swipe() {
        // The discrete recogniser dropped anything slower than 900 ms; a
        // deliberate continuous swipe is fine now.
        let base = Instant::now();
        let mut r = reader();
        at(&mut r, base, 0);
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        at(&mut r, base, 1500);
        frame(&mut r, &[(880, 1000), (1080, 1000), (680, 1000)]);
        at(&mut r, base, 3000);
        frame(&mut r, &[(600, 1000), (800, 1000), (400, 1000)]);
        at(&mut r, base, 3010);
        lift(&mut r, 3);
        let events = r.bridge.drain();
        assert_eq!(phases(&events), vec![Phase::Begin, Phase::Update, Phase::End]);
        assert!(events[2].progress > 0.6);
    }

    #[test]
    fn resting_before_lifting_is_a_slow_release() {
        // A flick reads fast, but fingers that stopped and rested before
        // lifting release with (almost) no velocity.
        let base = Instant::now();
        let mut r = reader();
        at(&mut r, base, 0);
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        at(&mut r, base, 16);
        frame(&mut r, &[(880, 1000), (1080, 1000), (680, 1000)]);
        at(&mut r, base, 32);
        frame(&mut r, &[(700, 1000), (900, 1000), (500, 1000)]);
        at(&mut r, base, 600);
        lift(&mut r, 3);
        let events = r.bridge.drain();
        let end = events.last().unwrap();
        assert_eq!(end.phase, Phase::End);
        assert!(end.velocity.abs() < 1.0, "velocity {}", end.velocity);
    }

    #[test]
    fn syn_dropped_cancels_a_live_swipe() {
        let mut r = reader();
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut r, &[(880, 1000), (1080, 1000), (680, 1000)]);
        r.handle(EV_SYN, SYN_DROPPED, 0);
        lift(&mut r, 3);
        let events = r.bridge.drain();
        assert_eq!(phases(&events), vec![Phase::Begin, Phase::Cancel]);
    }

    #[test]
    fn a_straggler_at_the_reference_distance_cancels() {
        // Three fingers lock the axis together, but by the old discrete
        // threshold the third has stopped: a scroll with a stray contact, so
        // the page is told to roll back and nothing else follows.
        let mut r = reader();
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut r, &[(880, 1000), (1080, 1000), (680, 1000)]);
        frame(&mut r, &[(300, 1000), (500, 1000), (670, 1000)]);
        frame(&mut r, &[(200, 1000), (400, 1000), (670, 1000)]);
        lift(&mut r, 3);
        let events = r.bridge.drain();
        assert_eq!(phases(&events), vec![Phase::Begin, Phase::Cancel]);
    }

    #[test]
    fn a_fourth_finger_does_not_jump_the_progress() {
        let mut r = reader();
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut r, &[(880, 1000), (1080, 1000), (680, 1000)]);
        // A fourth finger lands far to the right, then the hand keeps going.
        frame(&mut r, &[(880, 1000), (1080, 1000), (680, 1000), (1900, 1000)]);
        let events = r.bridge.drain();
        assert_eq!(phases(&events), vec![Phase::Begin]);
        frame(&mut r, &[(820, 1000), (1020, 1000), (620, 1000), (1840, 1000)]);
        let events = r.bridge.drain();
        assert_eq!(phases(&events), vec![Phase::Update]);
        assert!((events[0].progress - 0.3).abs() < 1e-9);
    }

    #[test]
    fn the_bridge_wakes_the_pump_on_begin_and_end() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static LIVE: AtomicUsize = AtomicUsize::new(0);
        static IDLE: AtomicUsize = AtomicUsize::new(0);
        fn wake(live: bool) {
            if live {
                LIVE.fetch_add(1, Ordering::SeqCst);
            } else {
                IDLE.fetch_add(1, Ordering::SeqCst);
            }
        }
        let mut r = Reader::with_thresholds(GestureBridge::with_waker(wake), 300, 300);
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut r, &[(880, 1000), (1080, 1000), (680, 1000)]);
        frame(&mut r, &[(700, 1000), (900, 1000), (500, 1000)]);
        lift(&mut r, 3);
        assert_eq!(LIVE.load(Ordering::SeqCst), 1);
        assert_eq!(IDLE.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn dispatch_carries_phase_axis_and_direction() {
        let script = dispatch_script(&GestureEvent {
            phase: Phase::Update,
            direction: Direction::Up,
            progress: 0.25,
            velocity: f64::NAN,
        });
        assert!(script.contains("'trackpad:gesture'"));
        assert!(script.contains("phase: \"update\""));
        assert!(script.contains("axis: \"y\""));
        assert!(script.contains("direction: \"up\""));
        assert!(script.contains("progress: 0.2500"));
        assert!(script.contains("velocity: 0.0000"));
    }

    #[test]
    fn lifting_the_fingers_rearms_the_gesture() {
        let mut r = reader();
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut r, &[(700, 1000), (900, 1000), (500, 1000)]);
        // Fingers up: tracking id -1 on every slot.
        lift(&mut r, 3);
        // A second, opposite swipe is recognised independently.
        frame(&mut r, &[(1000, 1000), (1200, 1000), (800, 1000)]);
        frame(&mut r, &[(1300, 1000), (1500, 1000), (1100, 1000)]);
        assert_eq!(begins(&r), vec![Direction::Left, Direction::Right]);
    }
}
