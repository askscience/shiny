//! Media files in a project: what they are, their waveform and a strip of
//! thumbnails. Everything is computed by ffmpeg/ffprobe and cached next to
//! the user's cache dir, keyed by path, size and modification time, so the
//! editor reopens instantly.

use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Info {
    pub kind: String, // video | audio | image
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub audio_tracks: u32,
    pub has_video: bool,
}

const VIDEO: &[&str] = &["mp4", "m4v", "mov", "mkv", "webm", "avi", "ogv"];
const AUDIO: &[&str] = &["mp3", "wav", "ogg", "opus", "flac", "m4a", "aac"];
const IMAGE: &[&str] = &["png", "jpg", "jpeg", "webp", "gif"];

fn ext(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

pub fn kind_of(p: &Path) -> Option<&'static str> {
    let e = ext(p);
    if VIDEO.contains(&e.as_str()) {
        Some("video")
    } else if AUDIO.contains(&e.as_str()) {
        Some("audio")
    } else if IMAGE.contains(&e.as_str()) {
        Some("image")
    } else {
        None
    }
}

/// `p` resolved and inside `home`, and a media file; `None` otherwise.
pub fn allowed(home: &Path, p: &Path) -> Option<PathBuf> {
    let home = home.canonicalize().ok()?;
    let p = p.canonicalize().ok()?;
    (p.starts_with(&home) && p.is_file() && kind_of(&p).is_some()).then_some(p)
}

fn cache_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp".into());
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".cache"))
        .join("liteaura")
        .join("capture")
}

/// A cache file name for `p` that changes when the file does.
fn cache_key(p: &Path, what: &str) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let md = std::fs::metadata(p).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.hash(&mut h);
    md.len().hash(&mut h);
    md.modified().ok()?.hash(&mut h);
    what.hash(&mut h);
    Some(cache_dir().join(format!("{:016x}.{what}", h.finish())))
}

fn cached<T: Serialize + for<'de> Deserialize<'de>>(
    p: &Path,
    what: &str,
    make: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let key = cache_key(p, what);
    if let Some(k) = &key {
        if let Some(v) = std::fs::read(k)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
        {
            return Ok(v);
        }
    }
    let v = make()?;
    if let Some(k) = key {
        let _ = std::fs::create_dir_all(cache_dir());
        let _ = std::fs::write(k, serde_json::to_vec(&v)?);
    }
    Ok(v)
}

fn ratio(s: &str) -> f64 {
    match s.split_once('/') {
        Some((a, b)) => {
            let (a, b): (f64, f64) = (a.parse().unwrap_or(0.0), b.parse().unwrap_or(1.0));
            if b > 0.0 {
                a / b
            } else {
                0.0
            }
        }
        None => s.parse().unwrap_or(0.0),
    }
}

pub fn probe(p: &Path) -> Result<Info> {
    cached(p, "info", || {
        let out = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
            ])
            .arg(p)
            .stdin(Stdio::null())
            .output()?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr)
                .trim()
                .to_string()
                .into());
        }
        let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
        let streams = v["streams"].as_array().cloned().unwrap_or_default();
        let video = streams.iter().find(|s| {
            s["codec_type"] == "video" && s["disposition"]["attached_pic"].as_i64() != Some(1)
        });
        let audio_tracks = streams
            .iter()
            .filter(|s| s["codec_type"] == "audio")
            .count() as u32;
        let duration = v["format"]["duration"]
            .as_str()
            .and_then(|d| d.parse::<f64>().ok())
            .unwrap_or(0.0);
        let kind = kind_of(p).unwrap_or("video");
        Ok(Info {
            kind: if kind == "image" {
                "image".into()
            } else if video.is_some() {
                "video".into()
            } else {
                "audio".into()
            },
            duration: if kind == "image" { 0.0 } else { duration },
            width: video.and_then(|s| s["width"].as_u64()).unwrap_or(0) as u32,
            height: video.and_then(|s| s["height"].as_u64()).unwrap_or(0) as u32,
            fps: video
                .and_then(|s| s["avg_frame_rate"].as_str())
                .map(ratio)
                .unwrap_or(0.0),
            audio_tracks,
            has_video: video.is_some(),
        })
    })
}

/// A waveform: `rate` buckets a second, each a min/max pair in -127..=127.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Peaks {
    pub rate: u32,
    pub min: Vec<i8>,
    pub max: Vec<i8>,
}

/// Peaks of audio track `track` (default 0): decoded at 4 kHz mono, which
/// is plenty for a picture and fast even for an hour of audio.
pub fn peaks(p: &Path, track: Option<u32>) -> Result<Peaks> {
    use std::io::Read;
    let track = track.unwrap_or(0);
    cached(p, &format!("peaks{track}"), || {
        const RATE: u32 = 100;
        const SR: u32 = 4000;
        let mut child = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(p)
            .args([
                "-map".into(),
                format!("0:a:{track}"),
                "-ac".into(),
                "1".into(),
                "-ar".into(),
                SR.to_string(),
                "-f".into(),
                "s16le".into(),
                "-".into(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut out = child.stdout.take().ok_or("no stdout")?;
        let mut ps = shiny_dsp::peaks::PeakStream::new(1, (SR / RATE) as usize);
        let mut buf = vec![0u8; 64 * 1024];
        let mut carry: Option<u8> = None;
        let mut samples: Vec<i16> = Vec::with_capacity(32 * 1024);
        loop {
            let n = out.read(&mut buf)?;
            if n == 0 {
                break;
            }
            samples.clear();
            let mut bytes = &buf[..n];
            if let Some(c) = carry.take() {
                samples.push(i16::from_le_bytes([c, bytes[0]]));
                bytes = &bytes[1..];
            }
            let (pairs, rest) = bytes.as_chunks::<2>();
            samples.extend(pairs.iter().map(|&p| i16::from_le_bytes(p)));
            carry = rest.first().copied();
            ps.push(&samples);
        }
        let _ = child.wait();
        let env = ps.finish();
        let q = |v: i16| ((v as i32) * 127 / 32767).clamp(-127, 127) as i8;
        Ok(Peaks {
            rate: RATE,
            min: env.iter().map(|&(a, _)| q(a)).collect(),
            max: env.iter().map(|&(_, b)| q(b)).collect(),
        })
    })
}

/// `n` thumbnails across the media, side by side in one JPEG (160×90 each).
/// Decodes keyframes only, so even a long video takes a moment.
pub fn strip(p: &Path, n: u32) -> Result<Vec<u8>> {
    let key = cache_key(p, &format!("strip{n}.jpg"));
    if let Some(k) = &key {
        if let Ok(b) = std::fs::read(k) {
            return Ok(b);
        }
    }
    let info = probe(p)?;
    if !info.has_video && info.kind != "image" {
        return Err("no pictures in an audio file".into());
    }
    let tmp = cache_dir().join(format!(".strip-{}.jpg", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(cache_dir());
    let dur = info.duration.max(0.1);
    let tile = "scale=160:90:force_original_aspect_ratio=decrease,pad=160:90:(ow-iw)/2:(oh-ih)/2:color=black";
    let vf = if info.kind == "image" {
        format!("{tile},tile={n}x1")
    } else {
        format!("fps={}/{:.3},{tile},tile={n}x1", n, dur)
    };
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
    if info.kind != "image" {
        cmd.args(["-skip_frame", "nokey"]);
    } else {
        cmd.args(["-loop", "1", "-t", &format!("{n}"), "-framerate", "1"]);
    }
    let out = cmd
        .arg("-i")
        .arg(p)
        .args(["-an", "-vf", &vf, "-frames:v", "1", "-q:v", "5"])
        .arg(&tmp)
        .stdin(Stdio::null())
        .output()?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr)
            .trim()
            .to_string()
            .into());
    }
    let b = std::fs::read(&tmp)?;
    match key {
        Some(k) => {
            let _ = std::fs::rename(&tmp, k);
        }
        None => {
            let _ = std::fs::remove_file(&tmp);
        }
    }
    Ok(b)
}

/// One frame at `t` seconds, at most `max_w` wide (JPEG): what a vision
/// model looks at.
pub fn frame_at(p: &Path, t: f64, max_w: u32) -> Result<Vec<u8>> {
    let out = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &format!("{:.3}", t.max(0.0)),
            "-i",
        ])
        .arg(p)
        .args([
            "-frames:v",
            "1",
            "-vf",
            &format!("scale='min({max_w},iw)':-2"),
            "-q:v",
            "4",
            "-f",
            "image2",
            "-c:v",
            "mjpeg",
            "-",
        ])
        .stdin(Stdio::null())
        .output()?;
    if !out.status.success() || out.stdout.is_empty() {
        return Err("could not read a frame there".into());
    }
    Ok(out.stdout)
}

/// Scene changes (seconds) found by ffmpeg's `scdet`: where an imported
/// video cuts, for an AI that has no event log for it.
pub fn scenes(p: &Path) -> Result<Vec<f64>> {
    cached(p, "scenes", || {
        let out = Command::new("ffmpeg")
            .args(["-hide_banner", "-nostats", "-skip_frame", "nokey", "-i"])
            .arg(p)
            .args([
                "-an",
                "-vf",
                "scale=160:-2,scdet=threshold=12",
                "-f",
                "null",
                "-",
            ])
            .stdin(Stdio::null())
            .output()?;
        let log = String::from_utf8_lossy(&out.stderr);
        Ok(log
            .lines()
            .filter_map(|l| l.split("lavfi.scd.time: ").nth(1))
            .filter_map(|t| t.trim().parse::<f64>().ok())
            .collect())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_extension() {
        assert_eq!(kind_of(Path::new("a.MP4")), Some("video"));
        assert_eq!(kind_of(Path::new("a.flac")), Some("audio"));
        assert_eq!(kind_of(Path::new("a.png")), Some("image"));
        assert_eq!(kind_of(Path::new("a.txt")), None);
    }

    #[test]
    fn allowed_stays_in_home() {
        let home = std::env::temp_dir().join(format!("capture-home-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let f = home.join("x.mp4");
        std::fs::write(&f, b"x").unwrap();
        assert!(allowed(&home, &f).is_some());
        assert!(allowed(&home, Path::new("/etc/passwd")).is_none());
        assert!(allowed(&home, &home.join("../x.mp4")).is_none());
        let _ = std::fs::remove_dir_all(home);
    }
}
