//! Render a project to a file with one ffmpeg run.
//!
//! - Each clip is its own input, opened at its in-point (`-ss`/`-t` before
//!   `-i`): ffmpeg seeks to the nearest keyframe and decodes only what plays,
//!   instead of decoding the whole file and trimming it in a filter.
//! - Pictures are laid over a solid canvas in track order with `overlay`,
//!   each shifted to its timeline position with `setpts`, so the graph has no
//!   per-frame expressions.
//! - Rounded corners use the same anti-aliased mask as the live bubble,
//!   written once as a grey image and merged with `alphamerge`.
//! - HTML graphics come pre-rendered with alpha ([`super::render`]);
//!   captions are an ASS file drawn by libass.
//! - Audio clips are trimmed, re-timed (`atempo`), faded, delayed and mixed
//!   without normalisation, so levels stay what the user set.

use super::project::{Clip, Project};
use crate::encoder::{self, Backend, Purpose};
use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Preset {
    /// `mp4`, `gif` or `webm`.
    pub format: String,
    /// Output size; 0 keeps the project's.
    pub width: u32,
    pub height: u32,
    /// `contain` (bars) or `cover` (crop): how the canvas fits another shape.
    pub fit: String,
    /// Lower is better (CRF/QP).
    pub quality: u32,
    /// Only `[from, to)` of the timeline.
    pub from: Option<f64>,
    pub to: Option<f64>,
}

impl Default for Preset {
    fn default() -> Self {
        Self {
            format: "mp4".into(),
            width: 0,
            height: 0,
            fit: "cover".into(),
            quality: 20,
            from: None,
            to: None,
        }
    }
}

impl Preset {
    pub fn named(name: &str) -> Self {
        let p = |w, h| Preset {
            width: w,
            height: h,
            ..Preset::default()
        };
        match name {
            "1080p" => p(1920, 1080),
            "720p" => p(1280, 720),
            "4k" => p(3840, 2160),
            "vertical" => p(1080, 1920),
            "square" => p(1080, 1080),
            "gif" => Preset {
                format: "gif".into(),
                width: 480,
                height: 0,
                fit: "contain".into(),
                ..Preset::default()
            },
            _ => Preset::default(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Job {
    pub id: String,
    pub project: String,
    /// `rendering`, `encoding`, `done`, `failed`.
    pub state: String,
    pub progress: f32,
    pub output: Option<PathBuf>,
    pub error: Option<String>,
    /// What was left out (graphics without the renderer).
    pub warnings: Vec<String>,
    pub started: u64,
}

static JOBS: Mutex<Option<HashMap<String, Arc<Mutex<Job>>>>> = Mutex::new(None);

pub fn job(id: &str) -> Option<Job> {
    JOBS.lock()
        .ok()?
        .as_ref()?
        .get(id)
        .map(|j| j.lock().unwrap().clone())
}

fn esc_filter_path(p: &Path) -> String {
    // For filter arguments: escape `\`, `'`, `:` the filtergraph way.
    p.display()
        .to_string()
        .replace('\\', "\\\\\\\\")
        .replace('\'', "\\\\\\'")
        .replace(':', "\\\\:")
}

fn even(v: f64) -> u32 {
    ((v.round() as i64).max(2) as u32) & !1
}

/// `atempo` accepts 0.5..=100 per instance: chain them for slower speeds.
fn atempo(speed: f64) -> String {
    let mut s = speed;
    let mut parts = Vec::new();
    while s < 0.5 {
        parts.push("atempo=0.5".to_string());
        s /= 0.5;
    }
    while s > 100.0 {
        parts.push("atempo=100".to_string());
        s /= 100.0;
    }
    if (s - 1.0).abs() > 1e-6 {
        parts.push(format!("atempo={s:.6}"));
    }
    parts.join(",")
}

/// ASS timestamp `H:MM:SS.cc`.
fn ass_time(t: f64) -> String {
    let cs = (t.max(0.0) * 100.0).round() as u64;
    format!(
        "{}:{:02}:{:02}.{:02}",
        cs / 360_000,
        (cs / 6000) % 60,
        (cs / 100) % 60,
        cs % 100
    )
}

pub fn captions_ass(p: &Project, offset: f64) -> String {
    let size = (p.height as f64 * 0.045).round().max(12.0);
    let margin = (p.height as f64 * 0.06).round();
    let mut s = format!(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: {}\nPlayResY: {}\nWrapStyle: 0\n\n\
         [V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
         Style: Default,Inter,{size},&H00FFFFFF,&H00FFFFFF,&H99000000,&H99000000,1,0,0,0,100,100,0,0,3,{},0,2,{m},{m},{margin},1\n\n\
         [Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
        p.width,
        p.height,
        (size * 0.35).round(),
        m = (p.width as f64 * 0.08).round(),
    );
    for c in &p.captions {
        let text = c
            .text
            .replace('\n', "\\N")
            .replace('{', "(")
            .replace('}', ")");
        let _ = writeln!(
            s,
            "Dialogue: 0,{},{},Default,,0,0,0,,{}",
            ass_time(c.at - offset),
            ass_time(c.at + c.dur - offset),
            text
        );
    }
    s
}

/// Write an 8-bit PGM of the rounded mask for a `w×h` clip.
fn mask_file(dir: &Path, w: u32, h: u32, radius: f64) -> Result<PathBuf> {
    let m = crate::mask::Mask::rounded(w as usize, h as usize, radius as f32);
    let path = dir.join(format!("mask-{w}x{h}-{:.3}.pgm", radius));
    if !path.exists() {
        let mut b = format!("P5\n{w} {h}\n255\n").into_bytes();
        b.extend_from_slice(&m.luma.alpha);
        std::fs::write(&path, b)?;
    }
    Ok(path)
}

pub struct Graph {
    pub args: Vec<String>,
    pub duration: f64,
}

/// Build the ffmpeg command for `p`. `overlays` maps an overlay id to its
/// pre-rendered alpha video.
pub fn build(
    p: &Project,
    preset: &Preset,
    overlays: &HashMap<String, PathBuf>,
    work: &Path,
    out: &Path,
    backend: &Backend,
) -> Result<Graph> {
    let from = preset.from.unwrap_or(0.0).max(0.0);
    let to = preset.to.unwrap_or_else(|| p.duration()).min(p.duration());
    let total = (to - from).max(0.1);
    let (cw, ch) = (p.width, p.height);
    let fps = p.fps;
    let mut a: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostats", "-y"]
        .map(String::from)
        .to_vec();
    if let Backend::Vaapi { device } = backend {
        if preset.format == "mp4" {
            a.extend(["-vaapi_device".into(), device.display().to_string()]);
        }
    }
    let mut f = String::new(); // filtergraph
    let mut n_in = 0usize;
    let mut add_input = |a: &mut Vec<String>, pre: &[String], path: &Path| -> usize {
        a.extend(pre.iter().cloned());
        a.extend(["-i".into(), path.display().to_string()]);
        n_in += 1;
        n_in - 1
    };
    let _ = write!(
        f,
        "color=c={}:s={cw}x{ch}:r={fps}:d={total:.3},format=yuv420p[base0];",
        p.background.replace('#', "0x")
    );
    let mut base = "base0".to_string();
    let mut nb = 0;
    let mut audio: Vec<String> = Vec::new();

    // Clips that play inside the range, shifted to it.
    let visible = |c: &Clip| c.end() > from && c.at < to && !c.ghost;
    let track_order: HashMap<&str, usize> = p
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id.as_str(), i))
        .collect();
    let mut clips: Vec<&Clip> = p.clips.iter().filter(|c| visible(c)).collect();
    clips.sort_by_key(|c| track_order.get(c.track.as_str()).copied().unwrap_or(0));

    // Overlays (HTML) go on their tracks too, in the same order.
    enum Layer<'a> {
        Clip(&'a Clip),
        Html(&'a super::project::Overlay, &'a PathBuf),
    }
    let mut layers: Vec<(usize, Layer)> = clips
        .iter()
        .map(|c| {
            (
                track_order.get(c.track.as_str()).copied().unwrap_or(0),
                Layer::Clip(c),
            )
        })
        .collect();
    for o in &p.overlays {
        if o.ghost || o.at + o.dur <= from || o.at >= to {
            continue;
        }
        if let Some(file) = overlays.get(&o.id) {
            layers.push((
                track_order
                    .get(o.track.as_str())
                    .copied()
                    .unwrap_or(usize::MAX),
                Layer::Html(o, file),
            ));
        }
    }
    layers.sort_by_key(|(k, _)| *k);

    for (_, layer) in &layers {
        match layer {
            Layer::Clip(c) => {
                let Some(m) = p.media(&c.media) else { continue };
                let tr = p.track(&c.track);
                let muted = tr.is_some_and(|t| t.muted);
                let hidden = tr.is_some_and(|t| t.hidden);
                // Part of the clip inside the range.
                let skip = (from - c.at).max(0.0);
                let at = (c.at - from).max(0.0);
                let len = (c.len() - skip).min(to - c.at.max(from)).max(0.0);
                if len <= 0.0 {
                    continue;
                }
                let src_in = c.src_in + skip * c.speed;
                let src_len = len * c.speed;
                let is_image = m.info.kind == "image";
                let wants_video = (m.info.has_video || is_image)
                    && !hidden
                    && tr.is_none_or(|t| t.kind == "video");
                let wants_audio =
                    m.info.audio_tracks > c.audio_track && !muted && c.volume > 0.0 && !is_image;
                if !wants_video && !wants_audio {
                    continue;
                }
                let pre: Vec<String> = if is_image {
                    vec![
                        "-loop".into(),
                        "1".into(),
                        "-framerate".into(),
                        fps.to_string(),
                        "-t".into(),
                        format!("{len:.3}"),
                    ]
                } else {
                    vec![
                        "-ss".into(),
                        format!("{src_in:.3}"),
                        "-t".into(),
                        format!("{src_len:.3}"),
                    ]
                };
                let k = add_input(&mut a, &pre, &m.path);
                if wants_video {
                    let (w, h) = (even(c.rect.w * cw as f64), even(c.rect.h * ch as f64));
                    let (x, y) = (
                        (c.rect.x * cw as f64).round() as i64,
                        (c.rect.y * ch as f64).round() as i64,
                    );
                    let mut chain = format!("[{k}:v]setpts=(PTS-STARTPTS)/{:.6},fps={fps},scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},format=yuva420p", c.speed);
                    if c.opacity < 0.999 {
                        let _ = write!(chain, ",colorchannelmixer=aa={:.3}", c.opacity);
                    }
                    if c.fade_in > 0.0 && skip == 0.0 {
                        let _ = write!(chain, ",fade=t=in:st=0:d={:.3}:alpha=1", c.fade_in);
                    }
                    if c.fade_out > 0.0 {
                        let _ = write!(
                            chain,
                            ",fade=t=out:st={:.3}:d={:.3}:alpha=1",
                            (len - c.fade_out).max(0.0),
                            c.fade_out
                        );
                    }
                    if c.radius > 0.0 {
                        let mpath = mask_file(work, w, h, c.radius)?;
                        let mk = add_input(
                            &mut a,
                            &[
                                "-loop".into(),
                                "1".into(),
                                "-framerate".into(),
                                fps.to_string(),
                                "-t".into(),
                                format!("{len:.3}"),
                            ],
                            &mpath,
                        );
                        let _ = write!(f, "{chain}[cv{k}];[{mk}:v]format=gray,setpts=PTS-STARTPTS[mk{k}];[cv{k}][mk{k}]alphamerge,setpts=PTS+{at:.3}/TB[l{k}];");
                    } else {
                        let _ = write!(f, "{chain},setpts=PTS+{at:.3}/TB[l{k}];");
                    }
                    nb += 1;
                    let _ = write!(f, "[{base}][l{k}]overlay=x={x}:y={y}:eof_action=pass:format=yuv420[base{nb}];");
                    base = format!("base{nb}");
                }
                if wants_audio {
                    let tv = tr.map(|t| t.volume).unwrap_or(1.0);
                    let mut chain = format!(
                        "[{k}:a:{}]asetpts=PTS-STARTPTS,aresample=48000",
                        c.audio_track
                    );
                    let t = atempo(c.speed);
                    if !t.is_empty() {
                        let _ = write!(chain, ",{t}");
                    }
                    let _ = write!(chain, ",volume={:.4}", c.volume * tv);
                    if c.fade_in > 0.0 && skip == 0.0 {
                        let _ = write!(chain, ",afade=t=in:st=0:d={:.3}", c.fade_in);
                    }
                    if c.fade_out > 0.0 {
                        let _ = write!(
                            chain,
                            ",afade=t=out:st={:.3}:d={:.3}",
                            (len - c.fade_out).max(0.0),
                            c.fade_out
                        );
                    }
                    let _ = write!(
                        chain,
                        ",atrim=0:{len:.3},adelay={}:all=1[a{k}];",
                        (at * 1000.0).round() as i64
                    );
                    f.push_str(&chain);
                    audio.push(format!("[a{k}]"));
                }
            }
            Layer::Html(o, file) => {
                let skip = (from - o.at).max(0.0);
                let at = (o.at - from).max(0.0);
                let len = (o.dur - skip).min(to - o.at.max(from));
                let k = add_input(
                    &mut a,
                    &[
                        "-ss".into(),
                        format!("{skip:.3}"),
                        "-t".into(),
                        format!("{len:.3}"),
                    ],
                    file,
                );
                let (w, h) = (even(o.rect.w * cw as f64), even(o.rect.h * ch as f64));
                let (x, y) = (
                    (o.rect.x * cw as f64).round() as i64,
                    (o.rect.y * ch as f64).round() as i64,
                );
                let mut chain = format!("[{k}:v]setpts=PTS-STARTPTS,scale={w}:{h},format=yuva420p");
                if o.opacity < 0.999 {
                    let _ = write!(chain, ",colorchannelmixer=aa={:.3}", o.opacity);
                }
                let _ = write!(f, "{chain},setpts=PTS+{at:.3}/TB[l{k}];");
                nb += 1;
                let _ = write!(
                    f,
                    "[{base}][l{k}]overlay=x={x}:y={y}:eof_action=pass:format=yuv420[base{nb}];"
                );
                base = format!("base{nb}");
            }
        }
    }

    // Captions.
    if p.captions.iter().any(|c| c.at + c.dur > from && c.at < to) {
        let ass = work.join("captions.ass");
        std::fs::write(&ass, captions_ass(p, from))?;
        nb += 1;
        let _ = write!(f, "[{base}]ass='{}'[base{nb}];", esc_filter_path(&ass));
        base = format!("base{nb}");
    }

    // Output size.
    let (ow, oh) = match (preset.width, preset.height) {
        (0, 0) => (cw, ch),
        (w, 0) => (w & !1, even(w as f64 * ch as f64 / cw as f64)),
        (0, h) => (even(h as f64 * cw as f64 / ch as f64), h & !1),
        (w, h) => (w & !1, h & !1),
    };
    let fitf = if (ow, oh) == (cw, ch) {
        String::new()
    } else if preset.fit == "contain" {
        format!(",scale={ow}:{oh}:force_original_aspect_ratio=decrease,pad={ow}:{oh}:(ow-iw)/2:(oh-ih)/2:color={}", p.background.replace('#', "0x"))
    } else {
        format!(",scale={ow}:{oh}:force_original_aspect_ratio=increase,crop={ow}:{oh}")
    };
    let gif = preset.format == "gif";
    if gif {
        let _ = write!(f, "[{base}]fps=12{fitf},split[g1][g2];[g1]palettegen=stats_mode=diff[pal];[g2][pal]paletteuse=dither=bayer:bayer_scale=4[vout]");
    } else {
        let hw = if matches!(backend, Backend::Vaapi { .. }) && preset.format == "mp4" {
            ",format=nv12,hwupload"
        } else {
            ""
        };
        let _ = write!(f, "[{base}]null{fitf}{hw}[vout]");
        if audio.is_empty() {
            let _ = write!(f, ";anullsrc=r=48000:cl=stereo,atrim=0:{total:.3}[aout]");
        } else {
            let _ = write!(
                f,
                ";{}amix=inputs={}:normalize=0:duration=longest,atrim=0:{total:.3}[aout]",
                audio.join(""),
                audio.len()
            );
        }
    }
    let script = work.join("graph.txt");
    std::fs::write(&script, &f)?;
    a.extend([
        "-filter_complex_script".into(),
        script.display().to_string(),
        "-map".into(),
        "[vout]".into(),
    ]);
    if gif {
        a.extend(["-loop".into(), "0".into()]);
    } else {
        a.extend(["-map".into(), "[aout]".into()]);
        match backend {
            Backend::Vaapi { .. } if preset.format == "mp4" => {
                a.extend(["-c:v", "h264_vaapi", "-rc_mode", "CQP"].map(String::from));
                a.extend(["-qp".into(), preset.quality.clamp(10, 40).to_string()]);
            }
            _ if preset.format == "webm" => {
                a.extend(
                    [
                        "-c:v",
                        "libvpx-vp9",
                        "-row-mt",
                        "1",
                        "-deadline",
                        "good",
                        "-cpu-used",
                        "4",
                        "-b:v",
                        "0",
                    ]
                    .map(String::from),
                );
                a.extend([
                    "-crf".into(),
                    (preset.quality + 12).clamp(15, 50).to_string(),
                ]);
            }
            _ => {
                a.extend(
                    [
                        "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p",
                    ]
                    .map(String::from),
                );
                a.extend(["-crf".into(), preset.quality.clamp(10, 40).to_string()]);
            }
        }
        if preset.format == "webm" {
            a.extend(["-c:a", "libopus", "-b:a", "160k"].map(String::from));
        } else {
            a.extend(["-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart"].map(String::from));
        }
        a.extend(["-r".into(), fps.to_string()]);
    }
    a.extend([
        "-t".into(),
        format!("{total:.3}"),
        out.display().to_string(),
    ]);
    Ok(Graph {
        args: a,
        duration: total,
    })
}

/// Start an export in the background; returns the job id at once.
pub fn start(home: PathBuf, p: Project, preset: Preset) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let job = Arc::new(Mutex::new(Job {
        id: id.clone(),
        project: p.id.clone(),
        state: "rendering".into(),
        started: super::project::now_ms(),
        ..Job::default()
    }));
    JOBS.lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(id.clone(), job.clone());
    std::thread::Builder::new()
        .name("capture-export".into())
        .spawn(move || {
            let r = run(&home, &p, &preset, &job);
            let mut j = job.lock().unwrap();
            match r {
                Ok(out) => {
                    j.state = "done".into();
                    j.progress = 1.0;
                    j.output = Some(out);
                }
                Err(e) => {
                    j.state = "failed".into();
                    j.error = Some(e.to_string());
                }
            }
        })
        .ok();
    id
}

fn run(home: &Path, p: &Project, preset: &Preset, job: &Arc<Mutex<Job>>) -> Result<PathBuf> {
    let work =
        std::env::temp_dir().join(format!("liteaura-capture-export-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&work)?;
    let r = (|| {
        // HTML graphics first.
        let mut overlays = HashMap::new();
        let mut shown: Vec<_> = p.overlays.iter().filter(|o| !o.ghost).collect();
        if !shown.is_empty() && super::render::renderer().is_none() {
            job.lock().unwrap().warnings.push(format!(
                "{} graphics left out: the renderer (liteaura-capture-render) is not installed",
                shown.len()
            ));
            shown.clear();
        }
        for (i, o) in shown.iter().enumerate() {
            let file = super::render::render_overlay(p, o, &work)?;
            overlays.insert(o.id.clone(), file);
            job.lock().unwrap().progress = 0.3 * (i + 1) as f32 / shown.len() as f32;
        }
        let base = if overlays.is_empty() { 0.0 } else { 0.3 };
        job.lock().unwrap().state = "encoding".into();
        let dir = crate::library::capture_dir(home).join("Exports");
        std::fs::create_dir_all(&dir)?;
        let ext = match preset.format.as_str() {
            "gif" => "gif",
            "webm" => "webm",
            _ => "mp4",
        };
        let name: String = p
            .title
            .chars()
            .map(|c| if c == '/' || c.is_control() { '-' } else { c })
            .collect();
        let out = unique(&dir.join(format!("{} {}.{ext}", name.trim(), stamp())));
        let backend = encoder::choose(crate::session::va_nodes(), Purpose::Record);
        let g = build(p, preset, &overlays, &work, &out, &backend)?;
        let mut child = std::process::Command::new("ffmpeg")
            .args(["-progress", "pipe:1"])
            .args(&g.args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let err = child.stderr.take();
        let err_text = Arc::new(Mutex::new(String::new()));
        let et = err_text.clone();
        let errt = std::thread::spawn(move || {
            use std::io::Read;
            if let Some(mut e) = err {
                let mut s = String::new();
                let _ = e.read_to_string(&mut s);
                *et.lock().unwrap() = s;
            }
        });
        if let Some(out) = child.stdout.take() {
            use std::io::BufRead;
            for line in std::io::BufReader::new(out).lines().map_while(|l| l.ok()) {
                if let Some(v) = line.strip_prefix("out_time_us=") {
                    if let Ok(us) = v.trim().parse::<f64>() {
                        let frac = (us / 1e6 / g.duration).clamp(0.0, 1.0) as f32;
                        job.lock().unwrap().progress = base + (1.0 - base) * frac * 0.99;
                    }
                }
            }
        }
        let st = child.wait()?;
        let _ = errt.join();
        if !st.success() {
            let _ = std::fs::remove_file(&out);
            let e = err_text.lock().unwrap().trim().to_string();
            return Err(if e.is_empty() {
                "the export failed".into()
            } else {
                e.into()
            });
        }
        Ok(out)
    })();
    let _ = std::fs::remove_dir_all(&work);
    r
}

fn stamp() -> String {
    let t = (super::project::now_ms() / 1000) as libc::time_t;
    // SAFETY: localtime_r fills `tm`.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    format!(
        "{:04}-{:02}-{:02} {:02}.{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
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
    (2..)
        .map(|i| p.with_file_name(format!("{stem} ({i}).{ext}")))
        .find(|c| !c.exists())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::project::{apply, Media};
    use serde_json::json;

    fn proj() -> Project {
        let mut p = Project::default();
        p.media.push(Media {
            id: "m1".into(),
            path: "/v/a.mp4".into(),
            info: crate::editor::media::Info {
                kind: "video".into(),
                duration: 20.0,
                width: 1920,
                height: 1080,
                fps: 30.0,
                audio_tracks: 1,
                has_video: true,
            },
            ..Media::default()
        });
        p
    }

    #[test]
    fn clips_seek_at_the_input_and_mix_audio() {
        let mut p = proj();
        apply(&mut p, &json!({"op": "add_clip", "media": "m1", "track": "v1", "at": 2, "in": 5, "out": 9, "speed": 2})).unwrap();
        apply(
            &mut p,
            &json!({"op": "add_clip", "media": "m1", "track": "a1", "at": 0, "in": 0, "out": 3}),
        )
        .unwrap();
        let dir = std::env::temp_dir();
        let g = build(
            &p,
            &Preset::default(),
            &HashMap::new(),
            &dir,
            Path::new("/o.mp4"),
            &Backend::X264 { preset: "x".into() },
        )
        .unwrap();
        let a = g.args.join(" ");
        assert!(a.contains("-ss 5.000 -t 4.000 -i /v/a.mp4"));
        assert!((g.duration - 4.0).abs() < 1e-9);
        let graph = std::fs::read_to_string(dir.join("graph.txt")).unwrap();
        assert!(graph.contains("setpts=(PTS-STARTPTS)/2.000000"));
        assert!(graph.contains("setpts=PTS+2.000/TB"));
        assert!(graph.contains("amix=inputs=2:normalize=0"));
        assert!(graph.contains("atempo=2.000000"));
    }

    #[test]
    fn atempo_chains_for_slow_motion() {
        assert_eq!(atempo(0.25), "atempo=0.5,atempo=0.500000");
        assert_eq!(atempo(1.0), "");
    }

    #[test]
    fn ass_times() {
        assert_eq!(ass_time(3661.25), "1:01:01.25");
    }
}
