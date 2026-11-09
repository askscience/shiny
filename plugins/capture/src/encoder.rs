//! The encoder: ffmpeg as a child process reading NV12 frames on stdin.
//!
//! - Frames arrive in the encoder's native layout, so ffmpeg runs no scaler
//!   and no colour conversion: `libx264` takes NV12 directly, VA-API only
//!   uploads it.
//! - Frames arrive in a tiny Matroska stream ([`crate::mkv`]) with their own
//!   timestamps, so an unchanged screen sends nothing and costs nothing. A
//!   recording keeps those timestamps (variable frame rate); a live stream
//!   asks ffmpeg for constant frame rate, which the platforms expect.
//! - Keyframes are placed by time, not by frame count, so a recording stays
//!   seekable every `keyint_s` seconds even when frames are sparse.

use crate::Result;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Backend {
    /// Software H.264. `preset` is an x264 preset (`ultrafast` … `medium`).
    X264 { preset: String },
    /// Hardware H.264 through a VA-API render node.
    Vaapi { device: PathBuf },
}

impl Backend {
    /// What the window shows: "Intel GPU", "AMD GPU", "CPU (x264)".
    pub fn friendly(&self) -> String {
        match self {
            Backend::X264 { .. } => "CPU (x264)".into(),
            Backend::Vaapi { device } => {
                let vendor = device
                    .file_name()
                    .map(|n| Path::new("/sys/class/drm").join(n).join("device/vendor"))
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .unwrap_or_default();
                match vendor.trim() {
                    "0x8086" => "Intel GPU".into(),
                    "0x1002" => "AMD GPU".into(),
                    "0x10de" => "NVIDIA GPU".into(),
                    _ => "GPU".into(),
                }
            }
        }
    }

    pub fn label(&self) -> String {
        match self {
            Backend::X264 { preset } => format!("x264/{preset}"),
            Backend::Vaapi { device } => format!("vaapi/{}", device.display()),
        }
    }
}

/// How the encoder spends bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rate {
    /// Constant quality (x264 CRF, VA-API QP); lower is better. Recordings.
    Quality(u32),
    /// Constant bitrate in kbit/s, what streaming platforms ask for.
    Cbr { kbps: u32 },
}

/// What the encoder is for; decides which backend [`auto`] prefers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Record,
    Live,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub width: usize,
    pub height: usize,
    pub fps: u32,
    pub backend: Backend,
    pub rate: Rate,
    /// Keyframe interval in seconds (2 s is what streaming platforms want).
    pub keyint_s: u32,
    /// Encoder threads; `None` lets the encoder decide.
    pub threads: Option<u32>,
    /// Keep the input's timestamps (recordings). `false` resamples to `fps`.
    pub vfr: bool,
    /// x264 tune (`zerolatency` drops lookahead and B-frames: far less CPU).
    pub tune: Option<String>,
    /// Output file (Matroska), or `None` to discard (benchmarks).
    pub out: Option<PathBuf>,
    /// PulseAudio sources recorded alongside (microphone, desktop monitor).
    pub audio: Vec<String>,
    /// Live ingest URLs (with the stream key). Non-empty means live: one
    /// encode feeds every platform and the file through `tee`.
    pub live: Vec<String>,
    /// Wall-clock origin of the video timestamps (µs since the Unix epoch).
    /// PulseAudio stamps audio with the wall clock, so the audio inputs are
    /// shifted by the same origin and every track shares one clock.
    pub ts_base_us: u64,
}

impl Settings {
    pub fn is_live(&self) -> bool {
        !self.live.is_empty()
    }
}

/// `[-]S.uuuuuu` for ffmpeg's duration options.
fn secs(us: u64) -> String {
    format!("-{}.{:06}", us / 1_000_000, us % 1_000_000)
}

/// Escape a value for a `tee` slave URL (`|`, `[`, `]` and `\` are special).
fn tee_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '|' | '[' | ']' | '\\' | '\'') {
            o.push('\\');
        }
        o.push(c);
    }
    o
}

/// The ffmpeg command line for `s`, without the program name.
pub fn args(s: &Settings) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostats", "-y"]
        .map(String::from)
        .to_vec();
    if let Backend::Vaapi { device } = &s.backend {
        a.extend(["-vaapi_device".into(), device.display().to_string()]);
    }
    a.extend(["-thread_queue_size", "64", "-f", "matroska", "-i", "pipe:0"].map(String::from));
    if !s.audio.is_empty() {
        a.push("-copyts".into());
    }
    for src in &s.audio {
        a.extend(
            [
                "-thread_queue_size",
                "1024",
                "-f",
                "pulse",
                "-sample_rate",
                "48000",
                "-fragment_size",
                "3840",
                "-wallclock",
                "1",
                "-itsoffset",
            ]
            .map(String::from),
        );
        a.push(secs(s.ts_base_us));
        a.extend(["-i".into(), src.clone()]);
    }
    let silent = s.is_live() && s.audio.is_empty();
    if silent {
        // Platforms reject a stream without audio.
        a.extend(["-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo"].map(String::from));
    }
    a.extend(["-map", "0:v"].map(String::from));
    if s.is_live() {
        match s.audio.len() {
            0 => a.extend(["-map", "1:a", "-shortest"].map(String::from)),
            1 => a.extend(["-map", "1:a"].map(String::from)),
            n => {
                let ins: String = (1..=n).map(|i| format!("[{i}:a]")).collect();
                a.extend([
                    "-filter_complex".into(),
                    format!("{ins}amix=inputs={n}:duration=longest:normalize=0[mix]"),
                    "-map".into(),
                    "[mix]".into(),
                ]);
            }
        }
        a.extend(["-c:a", "aac", "-b:a", "160k", "-ar", "48000", "-ac", "2"].map(String::from));
    } else {
        // Recordings keep every source on its own track for the editor.
        for i in 1..=s.audio.len() {
            a.extend(["-map".into(), format!("{i}:a")]);
        }
        if !s.audio.is_empty() {
            a.extend(["-c:a", "aac", "-b:a", "192k", "-ar", "48000"].map(String::from));
        }
    }
    if s.vfr {
        a.extend(["-fps_mode", "passthrough"].map(String::from));
    } else {
        a.extend([
            "-fps_mode".into(),
            "cfr".into(),
            "-r".into(),
            s.fps.to_string(),
        ]);
    }
    let k = s.keyint_s.max(1);
    // A generous frame cap; the time rule below places the real keyframes.
    let g = (s.fps * k).to_string();
    let cbr = |k: u32| {
        [
            "-b:v".to_string(),
            format!("{k}k"),
            "-maxrate".into(),
            format!("{k}k"),
            "-bufsize".into(),
            format!("{}k", 2 * k),
        ]
    };
    match &s.backend {
        Backend::X264 { preset } => {
            a.extend(["-c:v", "libx264", "-preset", preset].map(String::from));
            if let Some(t) = &s.tune {
                a.extend(["-tune".into(), t.clone()]);
            }
            match s.rate {
                Rate::Quality(q) => a.extend(["-crf".into(), q.to_string()]),
                Rate::Cbr { kbps } => {
                    a.extend(cbr(kbps));
                    a.extend(["-x264-params", "nal-hrd=cbr"].map(String::from));
                }
            }
            a.extend(["-g", &g, "-pix_fmt", "nv12"].map(String::from));
        }
        Backend::Vaapi { .. } => {
            a.extend(["-vf", "format=nv12,hwupload", "-c:v", "h264_vaapi"].map(String::from));
            match s.rate {
                Rate::Quality(q) => {
                    a.extend(["-rc_mode".into(), "CQP".into(), "-qp".into(), q.to_string()])
                }
                Rate::Cbr { kbps } => {
                    a.extend(["-rc_mode".into(), "CBR".into()]);
                    a.extend(cbr(kbps));
                }
            }
            a.extend(["-g".into(), g]);
        }
    }
    a.extend([
        "-force_key_frames".into(),
        format!("expr:gte(t,n_forced*{k})"),
    ]);
    if let Some(t) = s.threads {
        a.extend(["-threads".into(), t.to_string()]);
    }
    a.extend(
        [
            "-color_range",
            "tv",
            "-colorspace",
            "bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
        ]
        .map(String::from),
    );
    if s.is_live() {
        // One encode, every destination: each slave sits behind its own fifo
        // with recovery, so a platform that stalls or drops reconnects on its
        // own and never blocks the others or the recording.
        // Two escaping levels: tee unescapes once while splitting slaves on
        // `|`, and once more while splitting a slave's options on `:`.
        let fifo = [
            "attempt_recovery=1",
            "recover_any_error=1",
            "recovery_wait_time=2",
            "max_recovery_attempts=0",
            "drop_pkts_on_overflow=1",
            "queue_size=120",
        ]
        .join("\\\\:");
        let fifo = format!("fifo_options={fifo}");
        let mut slaves: Vec<String> = s
            .live
            .iter()
            .map(|u| {
                format!(
                    "[f=flv:flvflags=no_duration_filesize:onfail=ignore:use_fifo=1:{fifo}]{}",
                    tee_escape(u)
                )
            })
            .collect();
        if let Some(p) = &s.out {
            slaves.push(format!(
                "[f=matroska]{}",
                tee_escape(&p.display().to_string())
            ));
        }
        a.extend([
            "-flags".into(),
            "+global_header".into(),
            "-f".into(),
            "tee".into(),
            slaves.join("|"),
        ]);
        return a;
    }
    match &s.out {
        Some(p) => a.extend(["-f".into(), "matroska".into(), p.display().to_string()]),
        None => a.extend(["-f", "null", "-"].map(String::from)),
    }
    a
}

/// What ffmpeg reports about itself while it runs (`-progress`, once a
/// second) and the last lines it printed to stderr.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Health {
    /// Video frames encoded so far.
    pub frame: u64,
    pub fps: f32,
    pub kbps: f32,
    pub bytes: u64,
    pub dropped: u64,
    pub duplicated: u64,
    pub speed: f32,
    pub errors: Vec<String>,
}

pub type SharedHealth = std::sync::Arc<std::sync::Mutex<Health>>;

pub struct Encoder {
    child: Child,
    stdin: Option<ChildStdin>,
    pub health: SharedHealth,
    sent: u64,
    vfr: bool,
}

fn spawn_readers(child: &mut Child, health: &SharedHealth) {
    use std::io::BufRead;
    if let Some(out) = child.stdout.take() {
        let h = health.clone();
        std::thread::Builder::new()
            .name("capture-ffprogress".into())
            .spawn(move || {
                for line in std::io::BufReader::new(out).lines().map_while(|l| l.ok()) {
                    let Some((k, v)) = line.split_once('=') else {
                        continue;
                    };
                    let v = v.trim();
                    let Ok(mut h) = h.lock() else { return };
                    match k {
                        "frame" => h.frame = v.parse().unwrap_or(h.frame),
                        "fps" => h.fps = v.parse().unwrap_or(h.fps),
                        "bitrate" => {
                            h.kbps = v.trim_end_matches("kbits/s").parse().unwrap_or(h.kbps)
                        }
                        "total_size" => h.bytes = v.parse().unwrap_or(h.bytes),
                        "drop_frames" => h.dropped = v.parse().unwrap_or(h.dropped),
                        "dup_frames" => h.duplicated = v.parse().unwrap_or(h.duplicated),
                        "speed" => h.speed = v.trim_end_matches('x').parse().unwrap_or(h.speed),
                        _ => {}
                    }
                }
            })
            .ok();
    }
    if let Some(err) = child.stderr.take() {
        let h = health.clone();
        std::thread::Builder::new()
            .name("capture-fflog".into())
            .spawn(move || {
                for line in std::io::BufReader::new(err).lines().map_while(|l| l.ok()) {
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        continue;
                    }
                    let Ok(mut h) = h.lock() else { return };
                    if h.errors.len() >= 20 {
                        h.errors.remove(0);
                    }
                    h.errors.push(line);
                }
            })
            .ok();
    }
}

impl Encoder {
    pub fn spawn(ffmpeg: &Path, s: &Settings) -> Result<Self> {
        if std::env::var_os("CAPTURE_DEBUG").is_some() {
            eprintln!("capture: ffmpeg {}", args(s).join(" "));
        }
        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-progress", "pipe:1", "-stats_period", "0.25"])
            .args(args(s))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        let health = SharedHealth::default();
        spawn_readers(&mut child, &health);
        let mut stdin = child.stdin.take().ok_or("no stdin")?;
        grow_pipe(&stdin);
        stdin.write_all(&crate::mkv::header(s.width, s.height))?;
        Ok(Self {
            child,
            stdin: Some(stdin),
            health,
            sent: 0,
            vfr: s.vfr,
        })
    }

    /// Whether ffmpeg is still running.
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Send one frame shown from `pts_us` (microseconds since the start).
    ///
    /// The pixels are not copied: `vmsplice` hands their pages to the pipe
    /// and ffmpeg reads them from there. So the caller must not modify
    /// `frame` until [`Encoder::wait_drained`] returns.
    pub fn write(&mut self, pts_us: u64, frame: &[u8]) -> Result<()> {
        let stdin = self.stdin.as_mut().ok_or("encoder closed")?;
        stdin.write_all(&crate::mkv::frame_head(pts_us, frame.len()))?;
        splice_all(stdin, frame)?;
        self.sent += 1;
        Ok(())
    }

    /// Block until ffmpeg has read everything sent so far, so the last frame
    /// may be modified again. Usually returns at once: ffmpeg reads a frame
    /// within a millisecond or two, and the next tick is 33 ms away.
    pub fn wait_drained(&self) {
        use std::os::fd::AsRawFd;
        let Some(stdin) = self.stdin.as_ref() else {
            return;
        };
        let fd = stdin.as_raw_fd();
        loop {
            let mut queued: libc::c_int = 0;
            // SAFETY: FIONREAD writes one int.
            if unsafe { libc::ioctl(fd, libc::FIONREAD, &mut queued) } != 0 || queued == 0 {
                return;
            }
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
    }

    /// Close the input and wait for ffmpeg to flush and exit.
    ///
    /// Audio inputs never end on their own, so once every frame sent has
    /// been encoded (ffmpeg's own count, from `-progress`) ffmpeg gets
    /// SIGINT, its signal for "finish the files and quit". Waiting for the
    /// count matters: SIGINT drops frames still queued in the encoder.
    pub fn finish(mut self) -> Result<ExitStatus> {
        drop(self.stdin.take());
        if self.vfr {
            let (mut last, mut since) = (u64::MAX, std::time::Instant::now());
            loop {
                let done = self.health.lock().map(|h| h.frame).unwrap_or(u64::MAX);
                // The encoder keeps its newest frame until the very end; the
                // caller's last frame is a spare copy (see `close_segment`).
                if done + 1 >= self.sent || matches!(self.child.try_wait(), Ok(Some(_))) {
                    break;
                }
                if done != last {
                    (last, since) = (done, std::time::Instant::now());
                } else if since.elapsed().as_secs() >= 3 {
                    break; // stalled: stop anyway
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        let wait = |child: &mut Child, ms: u64| -> Option<ExitStatus> {
            let t = std::time::Instant::now();
            while t.elapsed().as_millis() < ms as u128 {
                if let Ok(Some(s)) = child.try_wait() {
                    return Some(s);
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            None
        };
        if let Some(s) = wait(&mut self.child, 150) {
            return Ok(s);
        }
        // SAFETY: signalling our own child.
        unsafe { libc::kill(self.child.id() as i32, libc::SIGINT) };
        if let Some(s) = wait(&mut self.child, 15_000) {
            return Ok(s);
        }
        let _ = self.child.kill();
        Ok(self.child.wait()?)
    }
}

/// Move `data` into the pipe by reference (see [`Encoder::write`]).
fn splice_all(stdin: &ChildStdin, mut data: &[u8]) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let fd = stdin.as_raw_fd();
    while !data.is_empty() {
        let iov = libc::iovec {
            iov_base: data.as_ptr() as *mut libc::c_void,
            iov_len: data.len(),
        };
        // SAFETY: the iovec points at `data`, which outlives the call; the
        // caller keeps the bytes unchanged until the pipe is drained.
        let n = unsafe { libc::vmsplice(fd, &iov, 1, 0) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        data = &data[n as usize..];
    }
    Ok(())
}

/// Raise the pipe buffer to the system maximum (1 MiB by default) so short
/// encoder stalls are absorbed instead of blocking the capture loop.
fn grow_pipe(stdin: &ChildStdin) {
    use std::os::fd::AsRawFd;
    let max = std::fs::read_to_string("/proc/sys/fs/pipe-max-size")
        .ok()
        .and_then(|s| s.trim().parse::<libc::c_int>().ok())
        .unwrap_or(1 << 20);
    // SAFETY: fcntl on an fd we own; failure only leaves the default size.
    unsafe { libc::fcntl(stdin.as_raw_fd(), libc::F_SETPIPE_SZ, max) };
}

/// A VA-API render node that encodes H.264.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaNode {
    pub path: PathBuf,
    /// Integrated Intel GPU: always awake, the cheapest to use.
    pub intel: bool,
    /// The driver offers constant bitrate (needed for live streams). Debian's
    /// free Intel driver offers only constant QP on older GPUs.
    pub cbr: bool,
}

fn va_try(ffmpeg: &Path, dev: &Path, rc: &[&str]) -> bool {
    Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "quiet", "-vaapi_device"])
        .arg(dev)
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=256x144:d=0.2",
            "-vf",
            "format=nv12,hwupload",
            "-c:v",
            "h264_vaapi",
        ])
        .args(rc)
        .args(["-f", "null", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Every render node that can encode H.264, integrated GPU first. Run it
/// once at load and keep the answer: each probe starts ffmpeg.
pub fn probe_vaapi(ffmpeg: &Path) -> Vec<VaNode> {
    let mut nodes: Vec<PathBuf> = std::fs::read_dir("/dev/dri")
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("renderD"))
                })
                .collect()
        })
        .unwrap_or_default();
    nodes.sort_by_key(|p| (!is_intel(p), p.clone()));
    nodes
        .into_iter()
        .filter(|dev| va_try(ffmpeg, dev, &["-rc_mode", "CQP"]))
        .map(|path| VaNode {
            intel: is_intel(&path),
            cbr: va_try(ffmpeg, &path, &["-rc_mode", "CBR", "-b:v", "2M"]),
            path,
        })
        .collect()
}

fn is_intel(node: &Path) -> bool {
    let Some(name) = node.file_name() else {
        return false;
    };
    let vendor = Path::new("/sys/class/drm").join(name).join("device/vendor");
    std::fs::read_to_string(vendor).is_ok_and(|v| v.trim() == "0x8086")
}

/// Pick a backend from probed nodes. Recording takes the first node
/// (integrated first); a live stream needs constant bitrate, so it takes the
/// first node that offers it, and falls back to x264 when none does.
pub fn choose(nodes: &[VaNode], purpose: Purpose) -> Backend {
    let node = match purpose {
        Purpose::Record => nodes.first(),
        Purpose::Live => nodes.iter().find(|n| n.cbr),
    };
    match node {
        Some(n) => Backend::Vaapi {
            device: n.path.clone(),
        },
        None => Backend::X264 {
            preset: "superfast".into(),
        },
    }
}

/// Probe and choose in one step.
pub fn auto(ffmpeg: &Path, purpose: Purpose) -> Backend {
    choose(&probe_vaapi(ffmpeg), purpose)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(backend: Backend) -> Settings {
        Settings {
            width: 1920,
            height: 1080,
            fps: 30,
            backend,
            rate: Rate::Quality(23),
            keyint_s: 2,
            threads: None,
            vfr: true,
            tune: Some("zerolatency".into()),
            out: Some("/x/out.mkv".into()),
            audio: Vec::new(),
            live: Vec::new(),
            ts_base_us: 0,
        }
    }

    #[test]
    fn x264_reads_nv12_without_conversion() {
        let a = args(&base(Backend::X264 {
            preset: "veryfast".into(),
        }))
        .join(" ");
        assert!(a.contains("-f matroska -i pipe:0 -map 0:v -fps_mode passthrough"));
        assert!(a.contains(
            "-c:v libx264 -preset veryfast -tune zerolatency -crf 23 -g 60 -pix_fmt nv12"
        ));
        let mut live = base(Backend::X264 {
            preset: "veryfast".into(),
        });
        live.rate = Rate::Cbr { kbps: 6000 };
        let l = args(&live).join(" ");
        assert!(l.contains("-b:v 6000k -maxrate 6000k -bufsize 12000k -x264-params nal-hrd=cbr"));
        assert!(a.contains("-force_key_frames expr:gte(t,n_forced*2)"));
        assert!(a.ends_with("-f matroska /x/out.mkv"));
        assert!(!a.contains("scale"));
    }

    #[test]
    fn vaapi_only_uploads() {
        let a = args(&base(Backend::Vaapi {
            device: "/dev/dri/renderD128".into(),
        }))
        .join(" ");
        assert!(a.starts_with("-hide_banner -loglevel error -nostats -y -vaapi_device /dev/dri/renderD128 -thread_queue_size 64 -f matroska"));
        assert!(a.contains("-vf format=nv12,hwupload -c:v h264_vaapi -rc_mode CQP -qp 23 -g 60"));
    }

    #[test]
    fn audio_shares_the_video_clock() {
        let mut s = base(Backend::X264 {
            preset: "veryfast".into(),
        });
        s.audio = vec!["mic".into(), "desk.monitor".into()];
        s.ts_base_us = 1_700_000_000_123_456;
        let a = args(&s).join(" ");
        assert!(a.contains("-copyts"));
        assert!(a.contains("-itsoffset -1700000000.123456 -i mic"));
        assert!(a.contains("-map 0:v -map 1:a -map 2:a -c:a aac"));
    }

    #[test]
    fn live_mixes_and_tees() {
        let mut s = base(Backend::X264 {
            preset: "veryfast".into(),
        });
        s.audio = vec!["mic".into(), "desk".into()];
        s.live = vec!["rtmp://a/live/k|ey".into(), "rtmps://b/x".into()];
        let a = args(&s);
        let j = a.join(" ");
        assert!(j.contains("[1:a][2:a]amix=inputs=2:duration=longest:normalize=0[mix] -map [mix]"));
        let tee = a.last().unwrap();
        assert!(tee.starts_with("[f=flv:"));
        assert!(tee.contains("]rtmp://a/live/k\\|ey|[f=flv"));
        assert!(tee.ends_with("[f=matroska]/x/out.mkv"));
        s.audio.clear();
        assert!(args(&s).join(" ").contains("anullsrc"));
    }

    #[test]
    fn live_needs_a_cbr_node() {
        let igpu = VaNode {
            path: "/dev/dri/renderD128".into(),
            intel: true,
            cbr: false,
        };
        let dgpu = VaNode {
            path: "/dev/dri/renderD129".into(),
            intel: false,
            cbr: true,
        };
        let both = [igpu.clone(), dgpu.clone()];
        assert_eq!(
            choose(&both, Purpose::Record),
            Backend::Vaapi {
                device: igpu.path.clone()
            }
        );
        assert_eq!(
            choose(&both, Purpose::Live),
            Backend::Vaapi {
                device: dgpu.path.clone()
            }
        );
        assert!(matches!(
            choose(&[igpu], Purpose::Live),
            Backend::X264 { .. }
        ));
    }
}
