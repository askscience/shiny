//! Audio clip sources: imported/AI-generated WAV files placed on audio tracks.
//!
//! * Files are referenced by a path **relative to the user's Files home**
//!   (e.g. `Music/Generated/night-drive-20261101.wav`), resolved and sandboxed
//!   by an [`AudioResolver`] the caller provides — the engine never sees an
//!   absolute path from the request.
//! * Decoded files are cached process-wide, keyed by path + size + mtime, and
//!   resampled once per project rate with a windowed-sinc resampler, so a
//!   three-minute song is decoded a single time however often the arrangement
//!   is played or edited.
//! * [`peaks`] feeds the arranger's waveform drawing; [`estimate_bpm`] gives the
//!   UI and the AI a tempo to match the project to.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use crate::wav::{decode_wav, Decoded};

/// Largest file the importer reads (≈ 20 min of 48 kHz/24-bit stereo).
pub const MAX_FILE_BYTES: u64 = 400 * 1024 * 1024;

/// Total cached sample frames across all entries (≈ 1 GB of f32 stereo).
const CACHE_FRAMES: usize = 120_000_000;

/// Resolves a clip's `src` to a real file. Implementations must reject
/// anything that escapes the user's sandbox.
pub trait AudioResolver: Send + Sync {
    fn resolve(&self, src: &str) -> Result<PathBuf, String>;
}

/// A resolver rooted at a home directory: `src` must be relative, without
/// `..` components, and end in `.wav`.
pub struct HomeResolver {
    pub home: PathBuf,
}

impl AudioResolver for HomeResolver {
    fn resolve(&self, src: &str) -> Result<PathBuf, String> {
        safe_join(&self.home, src)
    }
}

/// Resolver used where no user context exists: every audio clip fails with a
/// clear message (the pattern parts of the project still render).
pub struct NoAudio;

impl AudioResolver for NoAudio {
    fn resolve(&self, src: &str) -> Result<PathBuf, String> {
        Err(format!("audio clip `{src}` cannot be loaded here"))
    }
}

/// Join a user-supplied relative path onto `home`, refusing escapes.
pub fn safe_join(home: &Path, src: &str) -> Result<PathBuf, String> {
    let s = src.trim().trim_start_matches("~/").trim_start_matches('/');
    if s.is_empty() {
        return Err("empty audio path".into());
    }
    let rel = Path::new(s);
    for c in rel.components() {
        match c {
            std::path::Component::Normal(_) => {}
            std::path::Component::CurDir => {}
            _ => return Err(format!("invalid audio path `{src}`")),
        }
    }
    if !s.to_ascii_lowercase().ends_with(".wav") {
        return Err(format!("`{src}` is not a .wav file (WAV is the supported import format)"));
    }
    Ok(home.join(rel))
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    path: PathBuf,
    len: u64,
    mtime: Option<SystemTime>,
    /// 0 = the file's own rate.
    rate: u32,
}

struct Cache {
    map: HashMap<Key, (Arc<Decoded>, u64)>,
    tick: u64,
    frames: usize,
}

fn cache() -> &'static Mutex<Cache> {
    static C: OnceLock<Mutex<Cache>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Cache { map: HashMap::new(), tick: 0, frames: 0 }))
}

fn cache_get(k: &Key) -> Option<Arc<Decoded>> {
    let mut c = cache().lock().ok()?;
    c.tick += 1;
    let t = c.tick;
    c.map.get_mut(k).map(|e| {
        e.1 = t;
        e.0.clone()
    })
}

fn cache_put(k: Key, d: Arc<Decoded>) {
    let Ok(mut c) = cache().lock() else { return };
    let size = d.frames() * d.channels.len();
    c.tick += 1;
    let t = c.tick;
    c.frames += size;
    if let Some(old) = c.map.insert(k, (d, t)) {
        c.frames -= old.0.frames() * old.0.channels.len();
    }
    while c.frames > CACHE_FRAMES && c.map.len() > 1 {
        let Some(oldest) = c.map.iter().min_by_key(|(_, v)| v.1).map(|(k, _)| k.clone()) else { break };
        if let Some(v) = c.map.remove(&oldest) {
            c.frames -= v.0.frames() * v.0.channels.len();
        }
    }
}

fn key_for(path: &Path, rate: u32) -> Result<Key, String> {
    let meta = std::fs::metadata(path).map_err(|_| format!("audio file not found: {}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("not a file: {}", path.display()));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("{} is too large to import", path.display()));
    }
    Ok(Key { path: path.to_path_buf(), len: meta.len(), mtime: meta.modified().ok(), rate })
}

/// Decode a WAV file at its own rate (cached), keeping at most two channels.
pub fn load(path: &Path) -> Result<Arc<Decoded>, String> {
    let k = key_for(path, 0)?;
    if let Some(d) = cache_get(&k) {
        return Ok(d);
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut d = decode_wav(&bytes)?;
    d.channels.truncate(2);
    let d = Arc::new(d);
    cache_put(k, d.clone());
    Ok(d)
}

/// The file resampled to `rate` (cached). Mono files stay mono.
pub fn load_at(path: &Path, rate: u32) -> Result<Arc<Decoded>, String> {
    let src = load(path)?;
    if src.sample_rate == rate {
        return Ok(src);
    }
    let k = key_for(path, rate)?;
    if let Some(d) = cache_get(&k) {
        return Ok(d);
    }
    let channels = src.channels.iter().map(|c| resample(c, src.sample_rate as f64, rate as f64)).collect();
    let d = Arc::new(Decoded { sample_rate: rate, channels });
    cache_put(k, d.clone());
    Ok(d)
}

/* ── Resampling ─────────────────────────────────────────────── */

const TAPS: usize = 32;
const PHASES: usize = 512;

fn sinc_table(cutoff: f64) -> Vec<f32> {
    // Blackman-windowed sinc, PHASES+1 fractional offsets × TAPS taps.
    let half = (TAPS / 2) as f64;
    let mut t = Vec::with_capacity((PHASES + 1) * TAPS);
    for p in 0..=PHASES {
        let frac = p as f64 / PHASES as f64;
        let mut row = [0.0f64; TAPS];
        let mut sum = 0.0;
        for (k, v) in row.iter_mut().enumerate() {
            let x = k as f64 - half + 1.0 - frac;
            let s = if x.abs() < 1e-12 { 1.0 } else { (std::f64::consts::PI * x * cutoff).sin() / (std::f64::consts::PI * x * cutoff) };
            let n = (k as f64 + 1.0 - frac) / TAPS as f64;
            let w = 0.42 - 0.5 * (2.0 * std::f64::consts::PI * n).cos() + 0.08 * (4.0 * std::f64::consts::PI * n).cos();
            *v = s * w.max(0.0);
            sum += *v;
        }
        for v in row {
            t.push((v / sum.max(1e-12)) as f32);
        }
    }
    t
}

/// Band-limited sample-rate conversion (32-tap windowed sinc).
pub fn resample(input: &[f32], from: f64, to: f64) -> Vec<f32> {
    if input.is_empty() || (from - to).abs() < 1e-9 {
        return input.to_vec();
    }
    let ratio = to / from;
    // When downsampling, lower the cutoff to the new Nyquist (with margin).
    let cutoff = if ratio < 1.0 { ratio * 0.95 } else { 0.97 };
    let table = sinc_table(cutoff);
    let out_len = ((input.len() as f64) * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    let step = 1.0 / ratio;
    let half = TAPS as isize / 2;
    let n = input.len() as isize;
    for i in 0..out_len {
        let pos = i as f64 * step;
        let base = pos.floor() as isize;
        let frac = pos - base as f64;
        let ph = (frac * PHASES as f64).round() as usize;
        let row = &table[ph * TAPS..(ph + 1) * TAPS];
        let mut acc = 0.0f32;
        for (k, c) in row.iter().enumerate() {
            let idx = base - half + 1 + k as isize;
            if idx >= 0 && idx < n {
                acc += input[idx as usize] * c;
            }
        }
        out.push(acc);
    }
    out
}

/* ── Analysis helpers ───────────────────────────────────────── */

/// Min/max peaks over `buckets` equal slices (all channels folded together).
pub fn peaks(d: &Decoded, buckets: usize) -> Vec<(f32, f32)> {
    shiny_dsp::peaks::peaks(&d.channels, buckets)
}

/// Estimate the tempo of a piece of music (BPM), or `None` when there is no
/// clear pulse. Onset-strength envelope (rectified energy flux at 100 Hz)
/// followed by a weighted autocorrelation over 60–190 BPM, with a mild
/// preference for the 85–150 BPM range to resolve half/double ambiguity.
pub fn estimate_bpm(d: &Decoded) -> Option<f64> {
    let sr = d.sample_rate as f64;
    let hop = (sr / 100.0).round().max(1.0) as usize;
    let frames = d.frames();
    if frames < hop * 400 {
        return None; // shorter than ~4 s
    }
    // Analyse at most 90 s from the middle of the file (intros are sparse).
    let span = (90.0 * sr) as usize;
    let start = if frames > span { (frames - span) / 2 } else { 0 };
    let end = (start + span).min(frames);
    let mut env: Vec<f64> = Vec::with_capacity((end - start) / hop + 1);
    let mut hp_prev_x = 0.0f64;
    let mut hp_y = 0.0f64;
    let r = (-2.0 * std::f64::consts::PI * 120.0 / sr).exp();
    let mut prev_e = 0.0f64;
    let mut i = start;
    while i + hop <= end {
        let mut e = 0.0f64;
        for k in i..i + hop {
            let mut x = 0.0f64;
            for ch in &d.channels {
                x += ch[k] as f64;
            }
            // High-pass at ~120 Hz so sustained bass doesn't mask the onsets.
            hp_y = x - hp_prev_x + r * hp_y;
            hp_prev_x = x;
            e += hp_y * hp_y;
        }
        let e = (e / hop as f64 + 1e-12).ln();
        env.push((e - prev_e).max(0.0));
        prev_e = e;
        i += hop;
    }
    let mean = env.iter().sum::<f64>() / env.len() as f64;
    for v in env.iter_mut() {
        *v -= mean;
    }
    let rate = 100.0f64;
    let mut best = (0.0f64, 0.0f64);
    let mut bpm = 60.0f64;
    while bpm <= 190.0 {
        let lag = rate * 60.0 / bpm;
        let l0 = lag.floor() as usize;
        let frac = lag - l0 as f64;
        let mut acc = 0.0;
        for t in 0..env.len().saturating_sub(l0 + 1) {
            let a = env[t + l0] * (1.0 - frac) + env[t + l0 + 1] * frac;
            acc += env[t] * a;
        }
        // Log-Gaussian preference centred on 120 BPM.
        let w = (-(bpm / 120.0).log2().powi(2) / (2.0 * 0.9f64.powi(2))).exp();
        let score = acc * w;
        if score > best.1 {
            best = (bpm, score);
        }
        bpm += 0.25;
    }
    let energy: f64 = env.iter().map(|v| v * v).sum();
    if best.1 <= 0.0 || energy <= 0.0 || best.1 / energy < 0.05 {
        return None;
    }
    Some((best.0 * 4.0).round() / 4.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_join_refuses_escapes() {
        let home = Path::new("/home/u");
        assert!(safe_join(home, "Music/Generated/a.wav").is_ok());
        assert!(safe_join(home, "../etc/passwd.wav").is_err());
        assert!(safe_join(home, "Music/../../x.wav").is_err());
        assert!(safe_join(home, "Music/a.mp3").is_err());
        assert_eq!(safe_join(home, "/Music/a.wav").unwrap(), PathBuf::from("/home/u/Music/a.wav"));
    }

    #[test]
    fn resample_keeps_a_tone() {
        let from = 48_000.0;
        let to = 44_100.0;
        let x: Vec<f32> = (0..48_000).map(|i| (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / from).sin() as f32 * 0.5).collect();
        let y = resample(&x, from, to);
        assert!((y.len() as i64 - 44_100).abs() <= 1);
        let peak = y[2000..42_000].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - 0.5).abs() < 0.01, "peak {peak}");
    }

    #[test]
    fn bpm_of_a_click_track() {
        let sr = 44_100u32;
        let bpm = 128.0;
        let secs = 20.0;
        let n = (sr as f64 * secs) as usize;
        let mut ch = vec![0.0f32; n];
        let period = 60.0 / bpm * sr as f64;
        let mut t = 0.0f64;
        while (t as usize) < n {
            let s = t as usize;
            for k in 0..400.min(n - s) {
                let env = (-(k as f64) / 60.0).exp();
                ch[s + k] += (env * ((k as f64) * 0.9).sin()) as f32;
            }
            t += period;
        }
        let d = Decoded { sample_rate: sr, channels: vec![ch] };
        let est = estimate_bpm(&d).expect("tempo");
        assert!((est - bpm).abs() < 1.0, "estimated {est}");
    }
}
