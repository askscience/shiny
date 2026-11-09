//! The camera service: one owner for the webcam, shared by the bubble's
//! on-screen preview and the recording.
//!
//! The capture thread keeps the newest frame in a slot (`seq` tells readers
//! whether it changed) and, only while someone watches the bubble, a small
//! JPEG of it. Nobody asking for frames for a few seconds closes the camera,
//! so its light goes off and USB idles.

use crate::nv12::Nv12;
use crate::v4l2::{self, Camera};
use crate::Result;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// The camera closes after this long without a reader.
const IDLE: Duration = Duration::from_secs(6);

pub struct Latest {
    pub seq: AtomicU64,
    pub frame: Mutex<Nv12>,
    pub jpeg: Mutex<(u64, Option<Arc<Vec<u8>>>)>,
    pub signal: Condvar,
    pub mode: Mutex<(u32, u32)>,
    pub error: Mutex<Option<String>>,
}

struct Running {
    device: PathBuf,
    mode: (u32, u32),
    stop: Arc<AtomicBool>,
    latest: Arc<Latest>,
    used: Arc<Mutex<Instant>>,
    jpeg_wanted: Arc<Mutex<Instant>>,
}

static SERVICE: Mutex<Option<Running>> = Mutex::new(None);

fn epoch() -> Instant {
    static E: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *E.get_or_init(Instant::now)
}

/// The default camera: the first device with a raw mode.
pub fn default_device() -> Option<v4l2::Device> {
    v4l2::devices().into_iter().next()
}

/// Frames from `device` (default camera when `None`), at the smallest mode
/// that covers `need`. Starts the camera if needed; reuses it when it is
/// already open at a mode that covers `need`.
pub fn acquire(device: Option<&Path>, need: (u32, u32)) -> Result<Arc<Latest>> {
    let dev = match device {
        Some(p) => v4l2::devices()
            .into_iter()
            .find(|d| d.path == p)
            .ok_or("camera not found")?,
        None => default_device().ok_or("no camera")?,
    };
    let mode = crate::scale::pick_mode(&dev.modes, need).ok_or("camera has no modes")?;
    let mut svc = SERVICE.lock().map_err(|_| "camera lock")?;
    if let Some(r) = svc.as_ref() {
        let alive = !r.stop.load(Ordering::Relaxed);
        let covers = r.mode == mode || (r.mode.0 >= need.0 && r.mode.1 >= need.1);
        if alive && r.device == dev.path && covers {
            *r.used.lock().unwrap() = Instant::now();
            return Ok(r.latest.clone());
        }
        r.stop.store(true, Ordering::Relaxed);
    }
    let latest = Arc::new(Latest {
        seq: AtomicU64::new(0),
        frame: Mutex::new(Nv12::new(2, 2)),
        jpeg: Mutex::new((0, None)),
        signal: Condvar::new(),
        mode: Mutex::new(mode),
        error: Mutex::new(None),
    });
    let stop = Arc::new(AtomicBool::new(false));
    let used = Arc::new(Mutex::new(Instant::now()));
    let jpeg_wanted = Arc::new(Mutex::new(epoch()));
    let run = Running {
        device: dev.path.clone(),
        mode,
        stop: stop.clone(),
        latest: latest.clone(),
        used: used.clone(),
        jpeg_wanted: jpeg_wanted.clone(),
    };
    let path = dev.path.clone();
    let l2 = latest.clone();
    std::thread::Builder::new()
        .name("capture-camera".into())
        .spawn(move || {
            if let Err(e) = camera_loop(&path, mode, &stop, &l2, &used, &jpeg_wanted) {
                *l2.error.lock().unwrap() = Some(e.to_string());
            }
            stop.store(true, Ordering::Relaxed);
            l2.signal.notify_all();
        })?;
    *svc = Some(run);
    Ok(latest)
}

/// The camera that is open now, if any. Consumers read this every time
/// instead of keeping the handle `acquire` returned: when one of them needs a
/// bigger mode the camera reopens, and everyone follows.
pub fn current() -> Option<Arc<Latest>> {
    let svc = SERVICE.lock().ok()?;
    let r = svc.as_ref()?;
    (!r.stop.load(Ordering::Relaxed)).then(|| r.latest.clone())
}

/// Keep the camera open (call at least every few seconds while in use).
pub fn touch() {
    if let Ok(svc) = SERVICE.lock() {
        if let Some(r) = svc.as_ref() {
            *r.used.lock().unwrap() = Instant::now();
        }
    }
}

/// Ask the capture thread to keep a JPEG of the newest frame.
pub fn want_jpeg() {
    if let Ok(svc) = SERVICE.lock() {
        if let Some(r) = svc.as_ref() {
            *r.jpeg_wanted.lock().unwrap() = Instant::now();
            *r.used.lock().unwrap() = Instant::now();
        }
    }
}

/// Close the camera now.
pub fn release() {
    if let Ok(mut svc) = SERVICE.lock() {
        if let Some(r) = svc.take() {
            r.stop.store(true, Ordering::Relaxed);
        }
    }
}

pub fn is_open() -> bool {
    SERVICE
        .lock()
        .ok()
        .and_then(|s| s.as_ref().map(|r| !r.stop.load(Ordering::Relaxed)))
        .unwrap_or(false)
}

fn camera_loop(
    path: &Path,
    mode: (u32, u32),
    stop: &AtomicBool,
    latest: &Latest,
    used: &Mutex<Instant>,
    jpeg_wanted: &Mutex<Instant>,
) -> Result<()> {
    let mut cam = Camera::open(path, mode.0, mode.1, 30)?;
    *latest.mode.lock().unwrap() = (cam.width as u32, cam.height as u32);
    let mut buf = Nv12::new(cam.width, cam.height);
    let mut preview = crate::jpeg::Preview::new(480);
    let mut last_jpeg = Instant::now() - Duration::from_secs(1);
    while !stop.load(Ordering::Relaxed) {
        if used.lock().unwrap().elapsed() > IDLE {
            break;
        }
        if !cam.next_into(&mut buf, 500)? {
            continue;
        }
        {
            let mut f = latest.frame.lock().unwrap();
            std::mem::swap(&mut *f, &mut buf);
        }
        let seq = latest.seq.fetch_add(1, Ordering::Release) + 1;
        latest.signal.notify_all();
        // The on-screen bubble refreshes at up to 15 fps, only while watched.
        let watched = jpeg_wanted.lock().unwrap().elapsed() < Duration::from_secs(3);
        if watched && last_jpeg.elapsed() >= Duration::from_millis(66) {
            last_jpeg = Instant::now();
            let j = {
                let f = latest.frame.lock().unwrap();
                preview.jpeg(&f, 75)
            };
            if let Some(j) = j {
                *latest.jpeg.lock().unwrap() = (seq, Some(Arc::new(j)));
            }
        }
    }
    Ok(())
}
