//! A recording or live session: one capture thread that owns the X
//! connection, the frame and the encoder.
//!
//! Per tick (absolute deadlines, [`crate::clock::Ticker`]):
//!
//! 1. wait until ffmpeg has read the previous frame (it was handed over by
//!    reference with `vmsplice`), then take the cursor and bubble off it;
//! 2. ask the X server what changed (XDamage) and convert only that;
//! 3. draw the bubble and the cursor over saved patches;
//! 4. send the frame if anything changed, with its wall-clock timestamp.
//!
//! An unchanged screen sends nothing, so a recording costs almost nothing
//! while the user reads or talks. Pause closes the current file segment and
//! resume opens the next; stop joins them and writes the final MP4 with a
//! mixed audio track first and every source on its own track after it.

use crate::bubble::{Bubble, Style};
use crate::camera;
use crate::clock::Ticker;
use crate::cursor::{self, CursorImage, Patch};
use crate::encoder::{self, Backend, Encoder, Health, Purpose, Rate, Settings};
use crate::nv12::{bgrx_into, Convert, Nv12};
use crate::scale::Scaler;
use crate::x11::{Changes, Screen};
use crate::{Rect, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    #[default]
    Screen,
    Region,
    Window,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CameraPlan {
    pub device: Option<PathBuf>,
    pub style: Style,
    /// Where the bubble sits, in screen pixels.
    pub rect: Rect,
}

/// Everything a session needs to start.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Plan {
    pub source: SourceKind,
    /// Screen pixels; the region or the window's tile.
    pub rect: Option<Rect>,
    /// What the window is (shown in the recording's event log).
    pub window_title: Option<String>,
    pub fps: u32,
    /// Halve the resolution; `None` decides from the source size.
    pub half: Option<bool>,
    pub cursor: bool,
    pub mic: Option<String>,
    pub desktop: Option<String>,
    pub camera: Option<CameraPlan>,
    /// Record into this folder.
    pub record_dir: Option<PathBuf>,
    /// Live ingest URLs (with keys) and the matching platform labels.
    pub live: Vec<String>,
    pub live_labels: Vec<String>,
    pub live_kbps: u32,
    /// Recording quality (QP for VA-API, CRF for x264); lower is better.
    pub quality: u32,
    /// `auto`, `x264` or a VA-API render node.
    pub encoder: String,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            source: SourceKind::Screen,
            rect: None,
            window_title: None,
            fps: 30,
            half: None,
            cursor: true,
            mic: None,
            desktop: None,
            camera: None,
            record_dir: None,
            live: Vec::new(),
            live_labels: Vec::new(),
            live_kbps: 6000,
            quality: 20,
            encoder: "auto".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Marker {
    pub t_us: u64,
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub t_us: u64,
    pub kind: String,
    pub detail: String,
}

/// What the window and the agent can change while the session runs.
#[derive(Default)]
pub struct Control {
    /// New source rectangle (the recorded window moved or resized).
    pub rect: Option<Rect>,
    /// New bubble placement and look; `Some(None)` hides it.
    pub camera: Option<Option<CameraPlan>>,
    pub paused: bool,
    pub stop: bool,
    pub markers: Vec<Marker>,
    pub events: Vec<Event>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    #[default]
    Starting,
    Recording,
    Paused,
    Live,
    Finishing,
    Done,
    Failed,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Status {
    pub state: State,
    pub recording: bool,
    pub live: bool,
    pub live_labels: Vec<String>,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub encoder: String,
    /// Recorded time (pauses excluded), microseconds.
    pub elapsed_us: u64,
    pub frames: u64,
    pub ticks: u64,
    pub late: u64,
    /// This thread's CPU over the last second, percent of one core.
    pub cpu: f32,
    pub encoder_cpu: f32,
    pub health: Health,
    pub error: Option<String>,
    /// The finished recording.
    pub output: Option<PathBuf>,
    pub started_at: u64,
}

pub struct Shared {
    pub ctl: Mutex<Control>,
    pub status: Mutex<Status>,
    pub preview: Mutex<Option<Arc<Vec<u8>>>>,
    preview_wanted: Mutex<Option<Instant>>,
}

impl Shared {
    pub fn want_preview(&self) {
        *self.preview_wanted.lock().unwrap() = Some(Instant::now());
    }
}

struct Handle {
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

static CURRENT: Mutex<Option<Handle>> = Mutex::new(None);

/// The session in progress (or the last finished one, until the next start).
pub fn current() -> Option<Arc<Shared>> {
    CURRENT.lock().ok()?.as_ref().map(|h| h.shared.clone())
}

pub fn is_running() -> bool {
    current()
        .is_some_and(|s| !matches!(s.status.lock().unwrap().state, State::Done | State::Failed))
}

/// Start a session; fails if one is already running.
/// Called on the session thread when it ends, with the final status.
pub type OnDone = Box<dyn FnOnce(&Status, &Plan) + Send>;

/// A window of a new size: its frame, the scaler into the output, and
/// where it lands `(x, y, w, h)`.
type Stage = (Nv12, Scaler, (usize, usize, usize, usize));

pub fn start(plan: Plan, on_done: OnDone) -> Result<Arc<Shared>> {
    if plan.record_dir.is_none() && plan.live.is_empty() {
        return Err("nothing to do: neither recording nor live".into());
    }
    let mut cur = CURRENT.lock().map_err(|_| "session lock")?;
    if let Some(h) = cur.as_ref() {
        let st = h.shared.status.lock().unwrap().state;
        if !matches!(st, State::Done | State::Failed) {
            return Err("a recording or live stream is already running".into());
        }
    }
    if let Some(mut h) = cur.take() {
        if let Some(t) = h.thread.take() {
            let _ = t.join();
        }
    }
    let shared = Arc::new(Shared {
        ctl: Mutex::new(Control::default()),
        status: Mutex::new(Status {
            recording: plan.record_dir.is_some(),
            live: !plan.live.is_empty(),
            live_labels: plan.live_labels.clone(),
            started_at: now_us() / 1000,
            ..Status::default()
        }),
        preview: Mutex::new(None),
        preview_wanted: Mutex::new(None),
    });
    let s2 = shared.clone();
    let thread = std::thread::Builder::new()
        .name("capture-session".into())
        .spawn(move || {
            let r = run(&plan, &s2);
            let mut st = s2.status.lock().unwrap();
            match r {
                Ok(out) => {
                    st.output = out;
                    st.state = State::Done;
                }
                Err(e) => {
                    st.error = Some(e.to_string());
                    st.state = State::Failed;
                }
            }
            let snapshot = st.clone();
            drop(st);
            on_done(&snapshot, &plan);
        })?;
    *cur = Some(Handle {
        shared: shared.clone(),
        thread: Some(thread),
    });
    Ok(shared)
}

/// Ask the running session to stop (it finishes the file on its own thread).
pub fn stop() -> bool {
    match current() {
        Some(s) => {
            s.ctl.lock().unwrap().stop = true;
            true
        }
        None => false,
    }
}

pub fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

fn thread_cpu() -> Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime fills `ts`.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

fn proc_cpu(pid: u32) -> Duration {
    let Ok(s) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return Duration::ZERO;
    };
    let Some(rest) = s.rsplit_once(')').map(|(_, r)| r) else {
        return Duration::ZERO;
    };
    let f: Vec<&str> = rest.split_whitespace().collect();
    let tick = |i: usize| f.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    // SAFETY: sysconf is always safe.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as u64;
    Duration::from_millis((tick(11) + tick(12)) * 1000 / hz)
}

/// Working VA-API nodes, probed once per process (each probe runs ffmpeg).
pub fn va_nodes() -> &'static [encoder::VaNode] {
    static N: std::sync::OnceLock<Vec<encoder::VaNode>> = std::sync::OnceLock::new();
    N.get_or_init(|| encoder::probe_vaapi(Path::new("ffmpeg")))
}

fn backend_for(plan: &Plan, purpose: Purpose) -> Backend {
    match plan.encoder.as_str() {
        "x264" => Backend::X264 {
            preset: if purpose == Purpose::Live {
                "veryfast".into()
            } else {
                "superfast".into()
            },
        },
        "auto" | "" => encoder::choose(va_nodes(), purpose),
        dev => Backend::Vaapi {
            device: PathBuf::from(dev),
        },
    }
}

/// The source rectangle aligned for the scale step, inside the screen.
fn aligned(r: Rect, bounds: &Rect, step: u32) -> Rect {
    let r = r.intersect(bounds);
    let x = r.x & !1;
    let y = r.y & !1;
    let w = (r.right() - x) as u32 / step * step;
    let h = (r.bottom() - y) as u32 / step * step;
    Rect::new(x, y, w, h)
}

/// Fit `w×h` inside `fw×fh` keeping the aspect: `(x, y, w, h)`, all even.
fn fit(w: usize, h: usize, fw: usize, fh: usize) -> (usize, usize, usize, usize) {
    let s = (fw as f64 / w as f64).min(fh as f64 / h as f64);
    let dw = (((w as f64 * s) as usize) & !1).clamp(2, fw);
    let dh = (((h as f64 * s) as usize) & !1).clamp(2, fh);
    (((fw - dw) / 2) & !1, ((fh - dh) / 2) & !1, dw, dh)
}

fn blit(dst: &mut Nv12, src: &Nv12, x: usize, y: usize) {
    let dw = dst.w;
    let (dy, duv) = dst.planes_mut();
    let (sy, suv) = src.planes();
    for r in 0..src.h {
        dy[(y + r) * dw + x..][..src.w].copy_from_slice(&sy[r * src.w..][..src.w]);
    }
    for r in 0..src.h / 2 {
        duv[(y / 2 + r) * dw + x..][..src.w].copy_from_slice(&suv[r * src.w..][..src.w]);
    }
}

fn clear(f: &mut Nv12) {
    let (y, uv) = f.planes_mut();
    y.fill(16);
    uv.fill(128);
}

struct Segments {
    dir: PathBuf,
    stem: String,
    parts: Vec<PathBuf>,
}

impl Segments {
    fn next(&mut self) -> PathBuf {
        let p = self
            .dir
            .join(format!(".{}.part{}.mkv", self.stem, self.parts.len()));
        self.parts.push(p.clone());
        p
    }
}

/// The bubble as the frame sees it: position in frame pixels and a
/// compositor sized for it.
struct Cam {
    plan: CameraPlan,
    bubble: Bubble,
    latest: Option<Arc<camera::Latest>>,
    seen: u64,
    patch: Patch,
    at: (i32, i32),
}

fn place_camera(
    plan: &CameraPlan,
    src: Rect,
    scale: u32,
    fw: usize,
    fh: usize,
    window: bool,
) -> (Style, (i32, i32)) {
    let s = scale as i32;
    let w = ((plan.rect.w / scale) & !1).max(32);
    let h = ((plan.rect.h / scale) & !1).max(32);
    let mut x = (plan.rect.x - src.x) / s;
    let mut y = (plan.rect.y - src.y) / s;
    if window {
        // Outside the recorded window: keep it inside, at the nearest edge.
        x = x.clamp(0, (fw as i32 - w as i32).max(0));
        y = y.clamp(0, (fh as i32 - h as i32).max(0));
    }
    let mut style = plan.style.clone();
    style.w = w;
    style.h = h;
    style.border = plan.style.border / scale;
    (style, (x, y))
}

fn run(plan: &Plan, sh: &Shared) -> Result<Option<PathBuf>> {
    let fps = plan.fps.clamp(5, 60);
    let live = !plan.live.is_empty();
    let purpose = if live { Purpose::Live } else { Purpose::Record };

    // Keep the probe connection until the real one exists: an X server
    // without clients resets, and a connection during the reset fails.
    let probe = Screen::open(None, false)?;
    let bounds = probe.bounds();
    let want = match plan.source {
        SourceKind::Screen => bounds,
        _ => plan.rect.ok_or("a region or window needs a rectangle")?,
    };
    let half = plan
        .half
        .unwrap_or(want.w > 2560 || want.h > 1600 || (live && want.h > 1440));
    let scale: u32 = if half { 2 } else { 1 };
    let step = 2 * scale;
    let mut src = aligned(want, &bounds, step);
    if src.is_empty() {
        return Err("the area to record is empty".into());
    }
    // Room for a window that grows: the segment covers the whole screen.
    let mut screen = Screen::open(Some(bounds), true)?;
    drop(probe);
    let (fw, fh) = ((src.w / scale) as usize, (src.h / scale) as usize);
    let backend = backend_for(plan, purpose);
    {
        let mut st = sh.status.lock().unwrap();
        st.width = fw as u32;
        st.height = fh as u32;
        st.fps = fps;
        st.encoder = backend.friendly();
    }

    let mut frame = Nv12::new(fw, fh);
    let mut stage: Option<Stage> = None;
    let conv = |r: Rect| Convert {
        dx: (r.x as u32 / scale) as usize,
        dy: (r.y as u32 / scale) as usize,
        w: (r.w / scale) as usize,
        h: (r.h / scale) as usize,
        half: scale == 2,
        threads: 1,
        simd: true,
    };

    let audio: Vec<String> = [plan.mic.clone(), plan.desktop.clone()]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    let stem = chrono_stem();
    let mut segs = plan.record_dir.as_ref().map(|d| Segments {
        dir: d.clone(),
        stem: stem.clone(),
        parts: Vec::new(),
    });
    if let Some(d) = &plan.record_dir {
        std::fs::create_dir_all(d)?;
        if let (Some(s), Ok(abs)) = (segs.as_mut(), d.canonicalize()) {
            s.dir = abs;
        }
    }
    let settings = |out: Option<PathBuf>, base: u64| Settings {
        width: fw,
        height: fh,
        fps,
        backend: backend.clone(),
        rate: if live {
            Rate::Cbr {
                kbps: plan.live_kbps.clamp(500, 51_000),
            }
        } else {
            Rate::Quality(plan.quality.clamp(10, 40))
        },
        keyint_s: 2,
        threads: None,
        vfr: !live,
        tune: Some("zerolatency".into()),
        out,
        audio: audio.clone(),
        live: plan.live.clone(),
        ts_base_us: base,
    };

    let mut base = now_us();
    let mut enc = Some(Encoder::spawn(
        Path::new("ffmpeg"),
        &settings(segs.as_mut().map(|s| s.next()), base),
    )?);

    // Webcam.
    let mut cam: Option<Cam> = None;
    let make_cam = |p: &CameraPlan, src: Rect| -> Cam {
        let (style, at) = place_camera(p, src, scale, fw, fh, plan.source == SourceKind::Window);
        let need = (style.w * 2, style.h * 2);
        let latest = camera::acquire(p.device.as_deref(), need).ok();
        let mut bubble = Bubble::new(style);
        // The camera takes a moment to wake up: show the shape at once (dark,
        // with its border) instead of having it pop in mid-video.
        bubble.ready = latest.is_some();
        Cam {
            plan: p.clone(),
            bubble,
            latest,
            seen: 0,
            patch: Patch::default(),
            at,
        }
    };
    if let Some(p) = &plan.camera {
        cam = Some(make_cam(p, src));
    }

    // First full frame.
    let local = |src: &Rect| Rect::new(0, 0, src.w, src.h);
    {
        let px = screen.grab(src)?;
        bgrx_into(&mut frame, px, src.w as usize * 4, conv(local(&src)));
    }

    let mut cursor_img: Option<CursorImage> = None;
    let mut cpatch = Patch::default();
    let mut last_cursor = (i32::MIN, i32::MIN, 0u32);
    let mut ticker = Ticker::new(fps);
    let mut preview = crate::jpeg::Preview::new(640);
    let mut last_preview = Instant::now() - Duration::from_secs(5);
    let (mut frames, mut ticks, mut late) = (0u64, 0u64, 0u64);
    let mut elapsed_before = 0u64; // recorded time of closed segments
    let mut seg_start = Instant::now();
    let mut paused = false;
    let mut force = true;
    let mut cpu_mark = (Instant::now(), thread_cpu(), Duration::ZERO);
    let mut markers: Vec<Marker> = Vec::new();
    let mut events: Vec<Event> = Vec::new();
    if let Some(t) = &plan.window_title {
        events.push(Event {
            t_us: 0,
            kind: "window".into(),
            detail: t.clone(),
        });
    }
    sh.status.lock().unwrap().state = if live { State::Live } else { State::Recording };

    loop {
        let missed = ticker.wait();
        late += missed;
        ticks += 1 + missed;
        let rec_now = elapsed_before
            + if paused {
                0
            } else {
                seg_start.elapsed().as_micros() as u64
            };

        // Commands.
        let (stop, want_pause, new_rect, new_cam) = {
            let mut c = sh.ctl.lock().unwrap();
            for mut m in c.markers.drain(..) {
                m.t_us = rec_now;
                markers.push(m);
            }
            for mut e in c.events.drain(..) {
                e.t_us = rec_now;
                events.push(e);
            }
            (c.stop, c.paused && !live, c.rect.take(), c.camera.take())
        };
        if stop {
            break;
        }
        if want_pause != paused {
            if want_pause {
                elapsed_before += seg_start.elapsed().as_micros() as u64;
                if let Some(mut e) = enc.take() {
                    close_segment(&mut e, &frame, base)?;
                    e.finish()?;
                }
                paused = true;
                sh.status.lock().unwrap().state = State::Paused;
            } else {
                base = now_us();
                enc = Some(Encoder::spawn(
                    Path::new("ffmpeg"),
                    &settings(segs.as_mut().map(|s| s.next()), base),
                )?);
                seg_start = Instant::now();
                paused = false;
                force = true;
                sh.status.lock().unwrap().state = State::Recording;
            }
        }
        if let Some(r) = new_rect {
            let r = aligned(r, &bounds, step);
            if !r.is_empty() && r != src {
                src = r;
                force = true;
                if let Some(c) = &mut cam {
                    let (style, at) = place_camera(
                        &c.plan,
                        src,
                        scale,
                        fw,
                        fh,
                        plan.source == SourceKind::Window,
                    );
                    c.at = at;
                    if style != c.bubble.style {
                        c.bubble = Bubble::new(style);
                        c.bubble.ready = c.latest.is_some();
                        c.seen = 0;
                    }
                }
            }
        }
        if let Some(nc) = new_cam {
            match nc {
                None => {
                    if let Some(mut c) = cam.take() {
                        c.patch.restore(&mut frame);
                    }
                }
                Some(p) => {
                    let same_dev = cam.as_ref().is_some_and(|c| c.plan.device == p.device);
                    let (style, at) =
                        place_camera(&p, src, scale, fw, fh, plan.source == SourceKind::Window);
                    match &mut cam {
                        Some(c) if same_dev => {
                            c.at = at;
                            c.plan = p;
                            if style != c.bubble.style {
                                c.bubble = Bubble::new(style);
                                c.bubble.ready = c.latest.is_some();
                                c.seen = 0;
                            }
                        }
                        _ => cam = Some(make_cam(&p, src)),
                    }
                }
            }
            force = true;
        }
        if paused {
            if let Some(c) = &cam {
                if c.latest.is_some() {
                    camera::touch();
                }
            }
            continue;
        }
        let Some(e) = enc.as_mut() else { continue };
        e.wait_drained();
        cpatch.restore(&mut frame);
        if let Some(c) = &mut cam {
            c.patch.restore(&mut frame);
        }

        // Screen.
        let same_size = (src.w / scale) as usize == fw && (src.h / scale) as usize == fh;
        let changes = screen.changes()?;
        let rects: Vec<Rect> = if force {
            vec![local(&src)]
        } else {
            match changes {
                Changes::None => Vec::new(),
                Changes::All => vec![local(&src)],
                Changes::Rects(v) => v
                    .iter()
                    .map(|r| r.intersect(&src))
                    .filter(|r| !r.is_empty())
                    .map(|r| {
                        Rect::new(r.x - src.x, r.y - src.y, r.w, r.h)
                            .align_out(step as i32, &local(&src))
                    })
                    .filter(|r| !r.is_empty())
                    .collect(),
            }
        };
        let mut changed = !rects.is_empty();
        if same_size {
            stage = None;
            for r in &rects {
                let px = screen.grab(Rect::new(r.x + src.x, r.y + src.y, r.w, r.h))?;
                bgrx_into(&mut frame, px, r.w as usize * 4, conv(*r));
            }
        } else if changed {
            // The window changed size: convert at its own size, then fit
            // it into the fixed output with black bars.
            let (sw, shh) = ((src.w / scale) as usize, (src.h / scale) as usize);
            let fresh = !matches!(&stage, Some((n, _, _)) if (n.w, n.h) == (sw, shh));
            if fresh {
                let place = fit(sw, shh, fw, fh);
                stage = Some((
                    Nv12::new(sw, shh),
                    Scaler::new(sw, shh, place.2, place.3),
                    place,
                ));
                clear(&mut frame);
            }
            let (st, scaler, place) = stage.as_mut().unwrap();
            let px = screen.grab(src)?;
            bgrx_into(st, px, src.w as usize * 4, conv(local(&src)));
            let mut out = Nv12::new(place.2, place.3);
            scaler.run(st, &mut out);
            blit(&mut frame, &out, place.0, place.1);
        }
        force = false;

        // Webcam.
        if let Some(c) = &mut cam {
            // Follow the camera if it was reopened at another mode.
            if c.latest.is_some() {
                if let Some(now) = camera::current() {
                    if !c.latest.as_ref().is_some_and(|l| Arc::ptr_eq(l, &now)) {
                        c.latest = Some(now);
                        c.seen = 0;
                    }
                }
            }
            if let Some(l) = &c.latest {
                camera::touch();
                let seq = l.seq.load(Ordering::Acquire);
                if seq != c.seen {
                    c.seen = seq;
                    let f = l.frame.lock().unwrap();
                    c.bubble.update(&f);
                    changed = true;
                }
            }
            c.bubble.draw(&mut frame, c.at.0, c.at.1, &mut c.patch);
        }

        // Cursor.
        if plan.cursor {
            let known = cursor_img.as_ref().map_or(0, |c| c.serial);
            let st = screen.cursor(known)?;
            if (st.x, st.y, st.serial) != last_cursor {
                last_cursor = (st.x, st.y, st.serial);
                changed = true;
            }
            if let Some(argb) = &st.argb {
                cursor_img = Some(CursorImage::from_argb(
                    st.serial, st.w, st.h, st.xhot, st.yhot, argb,
                ));
            }
            if let (Some(img), true) = (&cursor_img, src.contains(st.x, st.y)) {
                let (x, y) = ((st.x - src.x) / scale as i32, (st.y - src.y) / scale as i32);
                cursor::draw(&mut frame, img, x, y, scale as usize, &mut cpatch);
            }
        }

        if changed {
            let pts = now_us().saturating_sub(base);
            e.write(pts, &frame.data)?;
            frames += 1;
        }
        if !e.alive() {
            let err = e.health.lock().unwrap().errors.join("\n");
            return Err(format!("the encoder stopped: {err}").into());
        }

        // Preview for the Capture window, twice a second while watched.
        let watched = sh
            .preview_wanted
            .lock()
            .unwrap()
            .is_some_and(|t| t.elapsed() < Duration::from_secs(3));
        if watched && last_preview.elapsed() >= Duration::from_millis(500) {
            last_preview = Instant::now();
            if let Some(j) = preview.jpeg(&frame, 70) {
                *sh.preview.lock().unwrap() = Some(Arc::new(j));
            }
        }

        // Status, once a second.
        if cpu_mark.0.elapsed() >= Duration::from_secs(1) {
            let wall = cpu_mark.0.elapsed().as_secs_f32();
            let cpu = thread_cpu();
            let ecpu = proc_cpu(e.pid());
            let mut st = sh.status.lock().unwrap();
            st.cpu = (cpu - cpu_mark.1).as_secs_f32() / wall * 100.0;
            st.encoder_cpu = ecpu.saturating_sub(cpu_mark.2).as_secs_f32() / wall * 100.0;
            st.health = e.health.lock().unwrap().clone();
            cpu_mark = (Instant::now(), cpu, ecpu);
        }
        let mut st = sh.status.lock().unwrap();
        st.elapsed_us = rec_now;
        st.frames = frames;
        st.ticks = ticks;
        st.late = late;
    }

    // Finish.
    sh.status.lock().unwrap().state = State::Finishing;
    if let Some(mut e) = enc.take() {
        elapsed_before += seg_start.elapsed().as_micros() as u64;
        let _ = close_segment(&mut e, &frame, base);
        let _ = e.finish();
    } else if !paused {
        elapsed_before += seg_start.elapsed().as_micros() as u64;
    }
    sh.status.lock().unwrap().elapsed_us = elapsed_before;
    let Some(segs) = segs else {
        return Ok(None);
    };
    let out = finish_recording(&segs, audio.len())?;
    let meta = serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "file": out.file_name().map(|n| n.to_string_lossy().to_string()),
        "created": now_us() / 1000,
        "duration_us": elapsed_before,
        "width": fw,
        "height": fh,
        "fps": fps,
        "source": plan.source,
        "window": plan.window_title,
        "audio": audio_tracks(plan),
        "camera": plan.camera.is_some(),
        "markers": markers,
        "events": events,
    });
    std::fs::write(
        crate::library::sidecar(&out),
        serde_json::to_vec_pretty(&meta)?,
    )?;
    Ok(Some(out))
}

/// Repeat the last frame at the end of a segment, so the video lasts as long
/// as its audio even when the screen stopped changing a while ago.
/// The frame goes twice: the encoder holds its newest frame until it
/// is torn down, so the second copy is the one it may drop.
fn close_segment(e: &mut Encoder, frame: &Nv12, base: u64) -> Result<()> {
    let t = now_us().saturating_sub(base);
    for pts in [t, t + 1000] {
        e.wait_drained();
        e.write(pts, &frame.data)?;
    }
    e.wait_drained();
    Ok(())
}

/// Track names in the order they appear after the mix.
pub fn audio_tracks(plan: &Plan) -> Vec<&'static str> {
    let mut v = Vec::new();
    if plan.mic.as_deref().is_some_and(|s| !s.is_empty()) {
        v.push("mic");
    }
    if plan.desktop.as_deref().is_some_and(|s| !s.is_empty()) {
        v.push("desktop");
    }
    v
}

fn chrono_stem() -> String {
    // Local time without a date crate: ask libc.
    let t = (now_us() / 1_000_000) as libc::time_t;
    // SAFETY: localtime_r fills `tm`.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    format!(
        "Capture {:04}-{:02}-{:02} {:02}.{:02}.{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

fn ffmpeg_ok(args: &[String]) -> Result<()> {
    let out = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr)
            .trim()
            .to_string()
            .into())
    }
}

/// Join the segments and write `<stem>.mp4`: video copied, a mixed audio
/// track first (what players play), then each source untouched.
fn finish_recording(segs: &Segments, n_audio: usize) -> Result<PathBuf> {
    let parts: Vec<&PathBuf> = segs.parts.iter().filter(|p| p.exists()).collect();
    if parts.is_empty() {
        return Err("nothing was recorded".into());
    }
    let joined = if parts.len() == 1 {
        parts[0].clone()
    } else {
        let list = segs.dir.join(format!(".{}.parts.txt", segs.stem));
        let body: String = parts
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| format!("file '{}'\n", n.to_string_lossy().replace('\'', "'\\''")))
            .collect();
        std::fs::write(&list, body)?;
        let j = segs.dir.join(format!(".{}.joined.mkv", segs.stem));
        ffmpeg_ok(&[
            "-f".into(),
            "concat".into(),
            "-safe".into(),
            "0".into(),
            "-i".into(),
            list.display().to_string(),
            "-map".into(),
            "0".into(),
            "-c".into(),
            "copy".into(),
            j.display().to_string(),
        ])?;
        let _ = std::fs::remove_file(&list);
        j
    };
    let out = unique(&segs.dir.join(format!("{}.mp4", segs.stem)));
    let mut a: Vec<String> = vec![
        "-i".into(),
        joined.display().to_string(),
        "-map".into(),
        "0:v".into(),
    ];
    match n_audio {
        0 => {}
        1 => a.extend(["-map", "0:a:0"].map(String::from)),
        n => {
            let ins: String = (0..n).map(|i| format!("[0:a:{i}]")).collect();
            a.extend([
                "-filter_complex".into(),
                format!("{ins}amix=inputs={n}:duration=longest:normalize=0[mix]"),
                "-map".into(),
                "[mix]".into(),
            ]);
            for i in 0..n {
                a.extend(["-map".into(), format!("0:a:{i}")]);
            }
        }
    }
    a.extend(["-c:v", "copy"].map(String::from));
    if n_audio > 1 {
        a.extend(["-c:a:0", "aac", "-b:a:0", "192k"].map(String::from));
        for i in 1..=n_audio {
            a.extend([format!("-c:a:{i}"), "copy".into()]);
        }
        a.extend(["-disposition:a:0", "default"].map(String::from));
        for i in 1..=n_audio {
            a.extend([format!("-disposition:a:{i}"), "0".into()]);
        }
    } else if n_audio == 1 {
        a.extend(["-c:a", "copy"].map(String::from));
    }
    a.extend(["-movflags", "+faststart"].map(String::from));
    a.push(out.display().to_string());
    ffmpeg_ok(&a)?;
    for p in &segs.parts {
        let _ = std::fs::remove_file(p);
    }
    if parts.len() > 1 {
        let _ = std::fs::remove_file(&joined);
    }
    Ok(out)
}

fn unique(p: &Path) -> PathBuf {
    if !p.exists() {
        return p.to_path_buf();
    }
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = p
        .extension()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    for i in 2.. {
        let c = p.with_file_name(format!("{stem} ({i}).{ext}"));
        if !c.exists() {
            return c;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_letterboxes() {
        assert_eq!(fit(1000, 500, 800, 800), (0, 200, 800, 400));
        assert_eq!(fit(400, 800, 800, 400), (300, 0, 200, 400));
    }

    #[test]
    fn aligned_stays_inside_and_even() {
        let b = Rect::new(0, 0, 1920, 1080);
        let r = aligned(Rect::new(-11, 1001, 501, 300), &b, 4);
        assert_eq!(r, Rect::new(0, 1000, 488, 80));
        assert_eq!(r.x % 2, 0);
    }

    #[test]
    fn bubble_stays_inside_a_window() {
        let p = CameraPlan {
            device: None,
            style: Style::default(),
            rect: Rect::new(1500, 900, 200, 200),
        };
        let (st, at) = place_camera(&p, Rect::new(100, 100, 800, 600), 1, 800, 600, true);
        assert_eq!((st.w, st.h), (200, 200));
        assert_eq!(at, (600, 400));
        let (_, free) = place_camera(&p, Rect::new(100, 100, 800, 600), 1, 800, 600, false);
        assert_eq!(free, (1400, 800));
    }
}
