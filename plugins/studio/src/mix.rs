//! The mixer: turns an [`Arrangement`] into per-track stems, shared returns
//! and a mastered stereo mix.
//!
//! ```text
//!  clips ─► track buffer ─► track inserts ─► [stem] ─► fader/pan (automated) ─┬─► master sum
//!                                                                             ├─► reverb send ─► reverb return ─┤
//!                                                                             └─► delay send  ─► delay return  ─┘
//!  master sum ─► master inserts ─► bus (gain/drive/width) ─► glue ─► look-ahead limiter ─► loudness trim
//! ```
//!
//! * Stems (everything before the fader) are cached by a hash of what made
//!   them, so editing one track re-renders only that track; the browser plays
//!   the stems through its own faders for a live mixer.
//! * The fader and pan laws match the Web Audio nodes the window uses
//!   (`GainNode` + `StereoPannerNode`), so monitoring and export agree.
//! * The limiter's look-ahead is compensated (the output is not delayed) and
//!   loudness normalisation is applied *before* the limiter, then re-measured,
//!   so a −14 LUFS target never hard-clips.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;

use crate::audio::AudioResolver;
use crate::dsp::delay::StereoDelay;
use crate::dsp::fx::{Effect, EffectKind};
use crate::dsp::limiter::{Compressor, Limiter, MasterBus};
use crate::dsp::loudness;
use crate::dsp::reverb::Reverb;
use crate::dsp::util::{db_to_gain, Smoother};
use crate::engine::{
    envelope_at, render_pattern_clip, stereo_pan, Arrangement, ArrangementClip, ArrangementTrack, AutomationLane,
    AutomationPoint, PlanarRender, TrackSends, MAX_CLIPS, MAX_TRACKS,
};

/// Seconds rendered past the arrangement end for releases and return tails.
const END_TAIL_SECS: f64 = 4.0;

/// What a render should produce.
#[derive(Debug, Clone, Copy)]
pub struct MixOptions {
    /// Keep the per-track stems and returns in the result.
    pub stems: bool,
    /// Apply the `fx.loudness` target.
    pub normalize: bool,
}

impl MixOptions {
    pub fn export() -> Self {
        Self { stems: false, normalize: true }
    }

    pub fn monitor() -> Self {
        Self { stems: true, normalize: true }
    }
}

/// One track's pre-fader audio.
pub struct Stem {
    pub l: Vec<f32>,
    pub r: Vec<f32>,
    pub sample_rate: u32,
}

impl Stem {
    pub fn peak(&self) -> f32 {
        self.l.iter().chain(self.r.iter()).fold(0.0f32, |m, x| m.max(x.abs()))
    }
}

/// A stem produced by a render, with its cache key.
pub struct TrackStem {
    pub track_id: String,
    pub key: String,
    pub stem: Arc<Stem>,
    /// Post-fader loudness/peak of the track (for the AI's mix analysis).
    pub lufs: f64,
    pub peak: f32,
}

pub struct MixResult {
    pub sample_rate: u32,
    pub frames: usize,
    pub master: PlanarRender,
    /// Loudness of the mix before the loudness trim, and the trim applied.
    pub pre_lufs: f64,
    pub trim_db: f64,
    pub tracks: Vec<TrackStem>,
    pub reverb: Option<(String, Arc<Stem>)>,
    pub delay: Option<(String, Arc<Stem>)>,
}

/* ── Stem cache ─────────────────────────────────────────────── */

const STEM_CACHE_FRAMES: usize = 40_000_000;

struct StemCache {
    map: HashMap<String, (Arc<Stem>, u64)>,
    tick: u64,
    frames: usize,
}

fn stem_cache() -> &'static Mutex<StemCache> {
    static C: OnceLock<Mutex<StemCache>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(StemCache { map: HashMap::new(), tick: 0, frames: 0 }))
}

/// A cached stem by key (the `GET /api/studio/mix/stem/:key` route).
pub fn cached_stem(key: &str) -> Option<Arc<Stem>> {
    let mut c = stem_cache().lock().ok()?;
    c.tick += 1;
    let t = c.tick;
    c.map.get_mut(key).map(|e| {
        e.1 = t;
        e.0.clone()
    })
}

fn put_stem(key: &str, stem: Arc<Stem>) {
    let Ok(mut c) = stem_cache().lock() else { return };
    c.tick += 1;
    let t = c.tick;
    let size = stem.l.len();
    if let Some(old) = c.map.insert(key.to_string(), (stem, t)) {
        c.frames -= old.0.l.len();
    }
    c.frames += size;
    while c.frames > STEM_CACHE_FRAMES && c.map.len() > 1 {
        let Some(oldest) = c.map.iter().min_by_key(|(_, v)| v.1).map(|(k, _)| k.clone()) else { break };
        if let Some(v) = c.map.remove(&oldest) {
            c.frames -= v.0.l.len();
        }
    }
}

fn hash_key(parts: &[&str]) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in parts {
        p.hash(&mut h);
    }
    // Two independent hashes → 128-bit key; collisions are not a concern.
    let a = h.finish();
    let mut h2 = std::collections::hash_map::DefaultHasher::new();
    a.hash(&mut h2);
    for p in parts.iter().rev() {
        p.hash(&mut h2);
    }
    format!("{:016x}{:016x}", a, h2.finish())
}

/* ── Automation ─────────────────────────────────────────────── */

/// A cursor-based envelope reader for per-sample evaluation in time order.
struct EnvReader<'a> {
    points: &'a [AutomationPoint],
    idx: usize,
}

impl<'a> EnvReader<'a> {
    fn new(points: &'a [AutomationPoint]) -> Self {
        Self { points, idx: 0 }
    }

    #[inline]
    fn at(&mut self, beat: f64, fallback: f64) -> f64 {
        let p = self.points;
        if p.is_empty() {
            return fallback;
        }
        if beat <= p[0].beat {
            return p[0].value;
        }
        let last = &p[p.len() - 1];
        if beat >= last.beat {
            return last.value;
        }
        while self.idx + 1 < p.len() && p[self.idx + 1].beat < beat {
            self.idx += 1;
        }
        let (a, b) = (&p[self.idx], &p[(self.idx + 1).min(p.len() - 1)]);
        let span = b.beat - a.beat;
        if span <= 0.0 {
            return b.value;
        }
        a.value + (b.value - a.value) * ((beat - a.beat) / span)
    }
}

fn lane<'a>(t: &'a ArrangementTrack, names: &[&str]) -> Option<&'a AutomationLane> {
    t.automation.lanes.iter().find(|l| names.contains(&l.param.as_str()) && !l.points.is_empty())
}

/// Effective sends: explicit, else derived from the clips' pattern fx (and
/// the arrangement's legacy global `reverb_mix`/`delay_mix`).
pub fn effective_sends(a: &Arrangement, t: &ArrangementTrack) -> TrackSends {
    if let Some(s) = t.sends {
        return TrackSends { reverb: s.reverb.clamp(0.0, 1.0), delay: s.delay.clamp(0.0, 1.0) };
    }
    let mut s = TrackSends::default();
    for c in a.clips.iter().filter(|c| c.track == t.id && c.audio.is_none()) {
        s.reverb = s.reverb.max(c.pattern.fx.get("reverb_mix").copied().unwrap_or(0.0));
        s.delay = s.delay.max(c.pattern.fx.get("delay_mix").copied().unwrap_or(0.0));
    }
    s.reverb = s.reverb.max(a.fx.get("reverb_mix").copied().unwrap_or(0.0)).clamp(0.0, 1.0);
    s.delay = s.delay.max(a.fx.get("delay_mix").copied().unwrap_or(0.0)).clamp(0.0, 1.0);
    s
}

/// Return parameters: explicit `returns`, else the first pattern (or the
/// arrangement fx) that configured its reverb/delay, else musical defaults.
fn return_params(a: &Arrangement) -> (HashMap<String, f64>, HashMap<String, f64>) {
    let mut rev = a.returns.reverb.clone();
    let mut del = a.returns.delay.clone();
    let legacy = std::iter::once(&a.fx).chain(a.clips.iter().map(|c| &c.pattern.fx));
    for fx in legacy {
        let pairs_r = [("reverb_size", "size"), ("reverb_damp", "damping"), ("reverb_predelay", "predelay"), ("reverb_width", "width")];
        let pairs_d = [("delay_time", "time"), ("feedback", "feedback"), ("delay_damp", "damp"), ("delay_ping_pong", "ping_pong")];
        if a.returns.reverb.is_empty() && fx.get("reverb_mix").copied().unwrap_or(0.0) > 0.0 {
            for (from, to) in pairs_r {
                if let Some(v) = fx.get(from) {
                    rev.entry(to.to_string()).or_insert(*v);
                }
            }
        }
        if a.returns.delay.is_empty() && fx.get("delay_mix").copied().unwrap_or(0.0) > 0.0 {
            for (from, to) in pairs_d {
                if let Some(v) = fx.get(from) {
                    del.entry(to.to_string()).or_insert(*v);
                }
            }
        }
    }
    (rev, del)
}

fn fx_val(fx: &HashMap<String, f64>, key: &str, default: f64) -> f64 {
    fx.get(key).copied().filter(|v| v.is_finite()).unwrap_or(default)
}

/* ── Track rendering ────────────────────────────────────────── */

/// Render every clip of one track into a buffer of `total` frames, then run
/// the track inserts. Pure function of its inputs (cached by the caller).
fn render_track_stem(
    a: &Arrangement,
    t: &ArrangementTrack,
    clips: &[(usize, &ArrangementClip)],
    resolver: &dyn AudioResolver,
    sr: f64,
    total: usize,
) -> Result<Stem, String> {
    let bpm = a.bpm;
    let spb = sr * 60.0 / bpm;
    let mut l = vec![0.0f32; total];
    let mut r = vec![0.0f32; total];
    for (ci, clip) in clips {
        let offset = (clip.start * spb).round() as usize;
        if offset >= total {
            continue;
        }
        let gain = db_to_gain(clip.gain_db);
        if let Some(audio) = &clip.audio {
            let path = resolver.resolve(&audio.src)?;
            let d = crate::audio::load_at(&path, sr as u32)?;
            let src_l = &d.channels[0];
            let src_r = d.channels.get(1).unwrap_or(&d.channels[0]);
            let start = (audio.offset.max(0.0) * sr).round() as usize;
            let avail = src_l.len().saturating_sub(start);
            let len = match clip.length_beats {
                Some(b) => ((b * spb).round() as usize).min(avail),
                None => avail,
            }
            .min(total - offset);
            let fi = ((audio.fade_in.max(0.0) * sr) as usize).min(len);
            // A minimum 2 ms fade-out avoids a click when the clip is trimmed
            // mid-waveform.
            let fo = ((audio.fade_out.max(0.002) * sr) as usize).min(len);
            for i in 0..len {
                let mut g = gain;
                if i < fi {
                    g *= (std::f32::consts::FRAC_PI_2 * i as f32 / fi as f32).sin();
                }
                if i + fo > len {
                    let k = (len - i) as f32 / fo as f32;
                    g *= (std::f32::consts::FRAC_PI_2 * k).sin();
                }
                l[offset + i] += src_l[start + i] * g;
                r[offset + i] += src_r[start + i] * g;
            }
        } else {
            let beats = clip.length_beats.unwrap_or_else(|| crate::engine::pattern_beats(&clip.pattern));
            let seed = 0x51D_1000 + *ci as u64 * 7919;
            let (cl, cr) = render_pattern_clip(&clip.pattern, bpm, clip.start, beats, clip.looped, &t.automation.lanes, seed, sr)?;
            let n = cl.len().min(total - offset);
            for i in 0..n {
                l[offset + i] += cl[i] * gain;
                r[offset + i] += cr[i] * gain;
            }
        }
    }

    // Track inserts, with `fx.<j>.<key>` automation at control rate.
    if !t.fx.is_empty() {
        let mut chain: Vec<(usize, Effect)> = Vec::new();
        for (j, e) in t.fx.iter().enumerate() {
            if e.bypass {
                continue;
            }
            let kind = EffectKind::from_name(&e.kind).ok_or_else(|| format!("unknown effect `{}`", e.kind))?;
            chain.push((j, Effect::new(kind, &e.params, sr)));
        }
        let lanes: Vec<(usize, &str, &AutomationLane)> = t
            .automation
            .lanes
            .iter()
            .filter_map(|ln| {
                let rest = ln.param.strip_prefix("fx.")?;
                let (j, key) = rest.split_once('.')?;
                Some((j.parse().ok()?, key, ln))
            })
            .filter(|(_, _, ln)| !ln.points.is_empty())
            .collect();
        let block = crate::dsp::BLOCK;
        let mut pos = 0;
        while pos < total {
            let n = block.min(total - pos);
            if !lanes.is_empty() {
                let beat = pos as f64 / spb;
                for (j, key, ln) in &lanes {
                    let v = envelope_at(&ln.points, beat, f64::NAN);
                    if v.is_finite() {
                        if let Some((_, e)) = chain.iter_mut().find(|(jj, _)| jj == j) {
                            e.set_param(key, v);
                        }
                    }
                }
            }
            for (_, e) in chain.iter_mut() {
                for k in pos..pos + n {
                    let (a, b) = e.tick(l[k], r[k]);
                    l[k] = a;
                    r[k] = b;
                }
            }
            pos += n;
        }
    }
    Ok(Stem { l, r, sample_rate: sr as u32 })
}

/// The cache key of a track's stem: everything that shapes the pre-fader
/// audio (the track minus its fader/pan/mute/solo/sends, its clips, tempo,
/// rate, length and the audio files' identity).
fn stem_key(a: &Arrangement, t: &ArrangementTrack, clips: &[(usize, &ArrangementClip)], resolver: &dyn AudioResolver, sr: f64, total: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("v3|{}|{}|{}", a.bpm, sr, total));
    parts.push(serde_json::to_string(&t.fx).unwrap_or_default());
    let lanes: Vec<&AutomationLane> = t
        .automation
        .lanes
        .iter()
        .filter(|l| l.param.starts_with("voice.") || l.param.starts_with("fx."))
        .collect();
    parts.push(serde_json::to_string(&lanes).unwrap_or_default());
    for (ci, c) in clips {
        parts.push(format!("{ci}"));
        parts.push(serde_json::to_string(c).unwrap_or_default());
        if let Some(audio) = &c.audio {
            if let Ok(p) = resolver.resolve(&audio.src) {
                if let Ok(m) = std::fs::metadata(&p) {
                    parts.push(format!("{}|{}|{:?}", p.display(), m.len(), m.modified().ok()));
                }
            }
        }
    }
    let refs: Vec<&str> = parts.iter().map(|s| s.as_str()).collect();
    hash_key(&refs)
}

/// Apply the fader (level + pan, automated) to a stem, adding the result to
/// `out` and the post-fader sends to the return inputs.
#[allow(clippy::too_many_arguments)]
fn mix_track(
    t: &ArrangementTrack,
    sends: TrackSends,
    stem: &Stem,
    spb: f64,
    sr: f64,
    out: (&mut [f32], &mut [f32]),
    rev_in: (&mut [f32], &mut [f32]),
    del_in: (&mut [f32], &mut [f32]),
    post: Option<(&mut Vec<f32>, &mut Vec<f32>)>,
) {
    let level_lane = lane(t, &["track.level"]);
    let pan_lane = lane(t, &["track.pan"]);
    let rev_lane = lane(t, &["send.reverb", "master.reverb_mix"]);
    let del_lane = lane(t, &["send.delay", "master.delay_mix"]);
    let mut lr = level_lane.map(|l| EnvReader::new(&l.points));
    let mut pr = pan_lane.map(|l| EnvReader::new(&l.points));
    let mut rr = rev_lane.map(|l| EnvReader::new(&l.points));
    let mut dr = del_lane.map(|l| EnvReader::new(&l.points));
    let base_level = t.level.clamp(0.0, 4.0) as f64;
    let base_pan = t.pan.clamp(-1.0, 1.0) as f64;
    // Light smoothing so automation breakpoints never zipper.
    let l0 = level_lane.map(|l| envelope_at(&l.points, 0.0, base_level)).unwrap_or(base_level);
    let p0 = pan_lane.map(|l| envelope_at(&l.points, 0.0, base_pan)).unwrap_or(base_pan);
    let mut lsm = Smoother::with_sr(l0, 0.005, sr);
    let mut psm = Smoother::with_sr(p0, 0.005, sr);
    let (ol, or) = out;
    let (rl, rrr) = rev_in;
    let (dl, drr) = del_in;
    let mut post = post;
    let n = stem.l.len().min(ol.len());
    for i in 0..n {
        let beat = i as f64 / spb;
        let level = lr.as_mut().map(|e| e.at(beat, base_level)).unwrap_or(base_level).clamp(0.0, 4.0);
        let pan = pr.as_mut().map(|e| e.at(beat, base_pan)).unwrap_or(base_pan).clamp(-1.0, 1.0);
        lsm.aim(level);
        psm.aim(pan);
        let g = lsm.next() as f32;
        let (l, r) = stereo_pan(stem.l[i] * g, stem.r[i] * g, psm.next());
        ol[i] += l;
        or[i] += r;
        if let Some((pl, prr)) = post.as_mut() {
            pl[i] = l;
            prr[i] = r;
        }
        let sv = rr.as_mut().map(|e| e.at(beat, sends.reverb)).unwrap_or(sends.reverb) as f32;
        if sv > 0.0 {
            rl[i] += l * sv;
            rrr[i] += r * sv;
        }
        let dv = dr.as_mut().map(|e| e.at(beat, sends.delay)).unwrap_or(sends.delay) as f32;
        if dv > 0.0 {
            dl[i] += l * dv;
            drr[i] += r * dv;
        }
    }
}

/* ── Master ─────────────────────────────────────────────────── */

/// Everything up to (not including) the limiter.
fn master_pre(a: &Arrangement, l: &mut [f32], r: &mut [f32], sr: f64) -> Result<(), String> {
    for e in &a.master_fx {
        if e.bypass {
            continue;
        }
        let kind = EffectKind::from_name(&e.kind).ok_or_else(|| format!("unknown effect `{}`", e.kind))?;
        let mut fx = Effect::new(kind, &e.params, sr);
        for i in 0..l.len() {
            let (a, b) = fx.tick(l[i], r[i]);
            l[i] = a;
            r[i] = b;
        }
    }
    let fx = &a.fx;
    let oversample = fx_val(fx, "master_oversample", 2.0).round().clamp(1.0, 8.0) as usize;
    let mut bus = MasterBus::new(sr, oversample);
    bus.gain = (a.master as f64 * fx_val(fx, "master_gain", 1.0)).clamp(0.0, 4.0);
    bus.drive = fx_val(fx, "master_drive", 0.0).clamp(0.0, 1.0);
    bus.width = fx_val(fx, "master_width", 1.0).clamp(0.0, 2.0);
    let glue = fx_val(fx, "glue", 0.0).clamp(0.0, 1.0);
    let mut comp = if glue > 0.0 {
        let mut c = Compressor::new(-18.0, 2.0, 30.0, 200.0);
        c.set_sample_rate(sr);
        c.knee_db = 8.0;
        c.makeup_db = 2.0 * glue;
        c.mix = glue;
        Some(c)
    } else {
        None
    };
    // `master.*` automation lanes (from any track) drive the bus gain/width.
    let gain_lane = a.tracks.iter().find_map(|t| lane(t, &["master.master_gain"]));
    let width_lane = a.tracks.iter().find_map(|t| lane(t, &["master.master_width"]));
    let spb = sr * 60.0 / a.bpm;
    let base_gain = bus.gain;
    for i in 0..l.len() {
        if i % crate::dsp::BLOCK == 0 && (gain_lane.is_some() || width_lane.is_some()) {
            let beat = i as f64 / spb;
            if let Some(ln) = gain_lane {
                bus.gain = (a.master as f64 * envelope_at(&ln.points, beat, base_gain)).clamp(0.0, 4.0);
            }
            if let Some(ln) = width_lane {
                bus.width = envelope_at(&ln.points, beat, 1.0).clamp(0.0, 2.0);
            }
        }
        let (x, y) = bus.tick(l[i], r[i]);
        let (x, y) = match comp.as_mut() {
            Some(c) => c.tick(x, y),
            None => (x, y),
        };
        l[i] = x;
        r[i] = y;
    }
    Ok(())
}

/// Run the look-ahead limiter over `pre × gain`, latency-compensated.
fn limit(pre_l: &[f32], pre_r: &[f32], gain: f32, ceiling_db: f64, sr: f64) -> (Vec<f32>, Vec<f32>) {
    let mut lim = Limiter::new(ceiling_db, 80.0, sr);
    let lat = lim.latency_frames();
    let n = pre_l.len();
    let mut l = Vec::with_capacity(n);
    let mut r = Vec::with_capacity(n);
    for i in 0..n + lat {
        let (x, y) = if i < n { (pre_l[i] * gain, pre_r[i] * gain) } else { (0.0, 0.0) };
        let (a, b) = lim.tick(x, y);
        if i >= lat {
            l.push(a);
            r.push(b);
        }
    }
    (l, r)
}

/* ── Entry point ────────────────────────────────────────────── */

/// Render an arrangement: stems (cached), returns, master.
pub fn render_mix(a: &Arrangement, resolver: &dyn AudioResolver, opts: &MixOptions) -> Result<MixResult, String> {
    crate::dsp::util::denormals_off();
    let t0 = std::time::Instant::now();
    let trace = std::env::var_os("STUDIO_TRACE").is_some();
    let tr = |what: &str| if trace { eprintln!("[mix] {what}: {:.3}s", t0.elapsed().as_secs_f64()) };
    if a.tracks.is_empty() {
        return Err("arrangement needs at least one track".into());
    }
    if a.tracks.len() > MAX_TRACKS {
        return Err(format!("too many tracks (max {MAX_TRACKS})"));
    }
    if a.clips.len() > MAX_CLIPS {
        return Err(format!("too many clips (max {MAX_CLIPS})"));
    }
    for c in &a.clips {
        if !a.tracks.iter().any(|t| t.id == c.track) {
            return Err(format!("clip references unknown track `{}`", c.track));
        }
    }
    let sr = crate::engine::arrangement_sr(a);
    let bpm = a.bpm.clamp(40.0, 240.0);
    let spb = sr * 60.0 / bpm;
    // Long enough for the arrangement, every clip's natural end and a tail.
    let mut end_beats = a.length_beats;
    for c in &a.clips {
        let len = c.length_beats.unwrap_or_else(|| if c.audio.is_some() { 0.0 } else { crate::engine::pattern_beats(&c.pattern) });
        end_beats = end_beats.max(c.start + len);
    }
    let total = (end_beats * spb).ceil() as usize + (END_TAIL_SECS * sr) as usize;
    let any_solo = a.tracks.iter().any(|t| t.solo);

    // Stems: from the cache, else rendered in parallel.
    struct Job<'a> {
        track: &'a ArrangementTrack,
        clips: Vec<(usize, &'a ArrangementClip)>,
        key: String,
        stem: Option<Arc<Stem>>,
    }
    let mut jobs: Vec<Job> = a
        .tracks
        .iter()
        .map(|t| {
            let clips: Vec<(usize, &ArrangementClip)> = a.clips.iter().enumerate().filter(|(_, c)| c.track == t.id).collect();
            let key = stem_key(a, t, &clips, resolver, sr, total);
            let stem = cached_stem(&key);
            Job { track: t, clips, key, stem }
        })
        .collect();
    let needed: Vec<usize> = jobs
        .iter()
        .enumerate()
        .filter(|(_, j)| j.stem.is_none() && (opts.stems || !(j.track.mute || (any_solo && !j.track.solo))))
        .map(|(i, _)| i)
        .collect();
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).clamp(1, 4);
    let mut results: Vec<(usize, Result<Stem, String>)> = Vec::new();
    if needed.len() <= 1 || workers == 1 {
        for &i in &needed {
            results.push((i, render_track_stem(a, jobs[i].track, &jobs[i].clips, resolver, sr, total)));
        }
    } else {
        let chunk = needed.len().div_ceil(workers);
        let jobs_ref = &jobs;
        std::thread::scope(|scope| {
            let handles: Vec<_> = needed
                .chunks(chunk)
                .map(|ids| {
                    scope.spawn(move || {
                        crate::dsp::util::denormals_off();
                        ids.iter()
                            .map(|&i| (i, render_track_stem(a, jobs_ref[i].track, &jobs_ref[i].clips, resolver, sr, total)))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            for h in handles {
                match h.join() {
                    Ok(v) => results.extend(v),
                    Err(_) => results.push((usize::MAX, Err("a track render panicked".into()))),
                }
            }
        });
    }
    for (i, res) in results {
        if i == usize::MAX {
            return Err(res.err().unwrap_or_default());
        }
        let stem = Arc::new(res.map_err(|e| format!("track `{}`: {e}", jobs[i].track.name))?);
        put_stem(&jobs[i].key, stem.clone());
        jobs[i].stem = Some(stem);
    }

    tr("stems");
    // Faders, sends, returns.
    let mut sum_l = vec![0.0f32; total];
    let mut sum_r = vec![0.0f32; total];
    let mut rev_l = vec![0.0f32; total];
    let mut rev_r = vec![0.0f32; total];
    let mut del_l = vec![0.0f32; total];
    let mut del_r = vec![0.0f32; total];
    let mut stems_out = Vec::new();
    let mut post_l = Vec::new();
    let mut post_r = Vec::new();
    for j in &jobs {
        let Some(stem) = &j.stem else { continue };
        let audible = !(j.track.mute || (any_solo && !j.track.solo));
        let sends = effective_sends(a, j.track);
        if audible {
            if opts.stems {
                post_l.resize(total, 0.0);
                post_r.resize(total, 0.0);
            }
            mix_track(
                j.track,
                sends,
                stem,
                spb,
                sr,
                (&mut sum_l, &mut sum_r),
                (&mut rev_l, &mut rev_r),
                (&mut del_l, &mut del_r),
                if opts.stems { Some((&mut post_l, &mut post_r)) } else { None },
            );
        }
        if opts.stems {
            let (lufs, peak) = if audible {
                let rep = loudness::measure(&[post_l.clone(), post_r.clone()], sr);
                (rep.integrated_lufs, rep.peak)
            } else {
                (-70.0, 0.0)
            };
            stems_out.push(TrackStem { track_id: j.track.id.clone(), key: j.key.clone(), stem: stem.clone(), lufs, peak });
        }
    }

    let (rev_p, del_p) = return_params(a);
    let has_rev = rev_l.iter().any(|x| *x != 0.0);
    let has_del = del_l.iter().any(|x| *x != 0.0) || del_r.iter().any(|x| *x != 0.0);
    let mut reverb_stem = None;
    let mut delay_stem = None;
    if has_del {
        let time = match rev_or(&del_p, "sync") {
            Some(beats) if beats > 0.0 => beats * 60_000.0 / bpm,
            _ => fx_val(&del_p, "time", 0.75 * 60_000.0 / bpm),
        };
        let mut d = StereoDelay::new(time.clamp(1.0, 4000.0), fx_val(&del_p, "feedback", 0.35).clamp(0.0, 0.95), 1.0);
        d.set_sample_rate(sr);
        d.ping_pong = fx_val(&del_p, "ping_pong", 0.5).clamp(0.0, 1.0);
        d.damp = fx_val(&del_p, "damp", 0.4).clamp(0.0, 1.0);
        d.update_damping();
        let g = a.returns.delay_level.unwrap_or(1.0).clamp(0.0, 4.0) as f32;
        let mut sl = vec![0.0f32; total];
        let mut sr_ = vec![0.0f32; total];
        for i in 0..total {
            let (x, y) = d.tick(del_l[i], del_r[i]);
            sl[i] = x;
            sr_[i] = y;
            sum_l[i] += x * g;
            sum_r[i] += y * g;
        }
        if opts.stems {
            delay_stem = Some(Arc::new(Stem { l: sl, r: sr_, sample_rate: sr as u32 }));
        }
    }
    if has_rev {
        let mut rv = Reverb::new(
            fx_val(&rev_p, "size", 0.55).clamp(0.05, 1.6),
            fx_val(&rev_p, "damping", 0.45).clamp(0.0, 1.0),
            1.0,
        );
        rv.set_sample_rate(sr);
        rv.predelay_ms = fx_val(&rev_p, "predelay", 18.0).clamp(0.0, 250.0);
        rv.width = fx_val(&rev_p, "width", 1.0).clamp(0.0, 2.0);
        rv.mod_amount = fx_val(&rev_p, "mod", 0.5).clamp(0.0, 1.0);
        rv.update_damping();
        let g = a.returns.reverb_level.unwrap_or(1.0).clamp(0.0, 4.0) as f32;
        let mut sl = vec![0.0f32; total];
        let mut sr_ = vec![0.0f32; total];
        for i in 0..total {
            let (x, y) = rv.tick(rev_l[i], rev_r[i]);
            sl[i] = x;
            sr_[i] = y;
            sum_l[i] += x * g;
            sum_r[i] += y * g;
        }
        if opts.stems {
            reverb_stem = Some(Arc::new(Stem { l: sl, r: sr_, sample_rate: sr as u32 }));
        }
    }

    tr("faders+returns");
    // Master.
    master_pre(a, &mut sum_l, &mut sum_r, sr)?;
    tr("master_pre");
    let ceiling = fx_val(&a.fx, "ceiling", -1.0).clamp(-24.0, 0.0);
    let (mut out_l, mut out_r) = limit(&sum_l, &sum_r, 1.0, ceiling, sr);
    tr("limit");
    let pre = loudness::measure(&[out_l.clone(), out_r.clone()], sr).integrated_lufs;
    tr("measure");
    let mut trim_db = 0.0;
    let target = fx_val(&a.fx, "loudness", 0.0);
    if opts.normalize && target < -1.0 && target > -60.0 && pre.is_finite() && pre > -60.0 {
        // Two passes: the limiter eats some of the first gain change.
        let mut gain_db = (target - pre).clamp(-30.0, 24.0);
        for _ in 0..4 {
            let (l, r) = limit(&sum_l, &sum_r, db_to_gain(gain_db), ceiling, sr);
            let got = loudness::measure(&[l.clone(), r.clone()], sr).integrated_lufs;
            out_l = l;
            out_r = r;
            if !got.is_finite() || (target - got).abs() < 0.2 {
                break;
            }
            gain_db = (gain_db + (target - got)).clamp(-30.0, 24.0);
        }
        trim_db = gain_db;
    }

    // Trim trailing digital silence past the arrangement end (keep ≥ 50 ms).
    let min_len = ((a.length_beats * spb).ceil() as usize).min(out_l.len());
    let mut len = out_l.len();
    while len > min_len && out_l[len - 1].abs() < 1e-5 && out_r[len - 1].abs() < 1e-5 {
        len -= 1;
    }
    len = (len + (0.05 * sr) as usize).min(out_l.len());
    out_l.truncate(len);
    out_r.truncate(len);

    let master = PlanarRender {
        sample_rate: sr as u32,
        duration_ms: (len as f64 / sr * 1000.0).round() as u32,
        frames: len,
        channels: vec![out_l, out_r],
    };
    let rkey = |name: &str| hash_key(&[name, &master.frames.to_string(), &jobs.iter().map(|j| j.key.as_str()).collect::<Vec<_>>().join(",")]);
    let reverb = reverb_stem.map(|s| {
        let k = format!("rv{}", &rkey("reverb")[2..]);
        put_stem(&k, s.clone());
        (k, s)
    });
    let delay = delay_stem.map(|s| {
        let k = format!("dl{}", &rkey("delay")[2..]);
        put_stem(&k, s.clone());
        (k, s)
    });
    Ok(MixResult { sample_rate: sr as u32, frames: total, master, pre_lufs: pre, trim_db, tracks: stems_out, reverb, delay })
}

fn rev_or(m: &HashMap<String, f64>, k: &str) -> Option<f64> {
    m.get(k).copied().filter(|v| v.is_finite())
}

/// Per-track and master measurements for the AI (`studio_mix_analyze`).
#[derive(Debug, Serialize)]
pub struct TrackReport {
    pub track_id: String,
    pub name: String,
    pub lufs: f64,
    pub peak: f32,
    pub muted: bool,
}

pub fn track_reports(a: &Arrangement, mix: &MixResult) -> Vec<TrackReport> {
    let any_solo = a.tracks.iter().any(|t| t.solo);
    mix.tracks
        .iter()
        .filter_map(|s| {
            let t = a.tracks.iter().find(|t| t.id == s.track_id)?;
            Some(TrackReport {
                track_id: t.id.clone(),
                name: t.name.clone(),
                lufs: (s.lufs * 10.0).round() / 10.0,
                peak: s.peak,
                muted: t.mute || (any_solo && !t.solo),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{parse_arrangement, parse_config, render_planar_track};
    use serde_json::json;

    fn hf_ratio(ch: &[f32], sr: f64, hz: f64) -> f64 {
        // Energy above `hz` (8th-order Butterworth-ish high-pass) / total.
        use crate::dsp::util::Biquad;
        let mut hp: Vec<Biquad> = [0.51, 0.6, 0.9, 2.56].iter().map(|q| Biquad::highpass64(hz, *q, sr)).collect();
        let (mut hi, mut all) = (0.0, 0.0);
        for &x in ch {
            let x = x as f64;
            let mut y = x;
            for f in hp.iter_mut() {
                y = f.tick64(y);
            }
            hi += y * y;
            all += x * x;
        }
        hi / all.max(1e-30)
    }

    fn kit() -> serde_json::Value {
        json!({"title":"Kit","bpm":120,"steps":16,"voices":[
            {"kind":"kick","rhythm":"e4,0"},{"kind":"snare","rhythm":"e2,4"},{"kind":"hat","rhythm":"e8,1"}]})
    }

    #[test]
    fn every_kind_renders_audible_audio() {
        for kind in crate::voices::KINDS {
            if matches!(*kind, "grid" | "sampler" | "sfkit") {
                continue;
            }
            let cfg = parse_config(&json!({"bpm":120,"steps":16,"voices":[{"kind":kind,"rhythm":"x...x...","octave":3}]})).unwrap();
            let p = render_planar_track(&cfg).unwrap();
            let peak = p.channels[0].iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(peak > 0.01, "{kind} is silent");
            assert!(peak <= 0.95, "{kind} exceeds the ceiling: {peak}");
        }
    }

    #[test]
    fn filtered_pad_has_no_high_frequency_hash() {
        // The old engine ran L and R through one filter state, smearing a
        // Nyquist-rate rattle over every stereo voice.
        let cfg = parse_config(&json!({"bpm":100,"steps":16,"voices":[{"kind":"pad","octave":3,
            "notes":[{"step":0,"length":16,"degree":0,"octave":-1},{"step":0,"length":16,"degree":4,"octave":-1}],
            "synth":{"unison":6,"spread":1.0,"cutoff":900,"fenv":0}}]})).unwrap();
        let p = render_planar_track(&cfg).unwrap();
        for ch in &p.channels {
            let r = hf_ratio(ch, 44_100.0, 12_000.0);
            assert!(r < 1e-6, "hf ratio {r}");
        }
    }

    #[test]
    fn silence_renders_digital_silence() {
        let cfg = parse_config(&json!({"bpm":120,"steps":16,"voices":[{"kind":"lead","rhythm":""}],
            "fx":{"reverb_mix":0.3,"delay_mix":0.3,"glue":0.4}})).unwrap();
        let p = render_planar_track(&cfg).unwrap();
        assert!(p.channels.iter().flatten().all(|x| *x == 0.0));
    }

    #[test]
    fn limiter_holds_the_ceiling_and_loudness_target() {
        let mut v = kit();
        v["fx"] = json!({"loudness": -12.0, "ceiling": -1.0});
        v["voices"][0]["level"] = json!(2.0);
        let cfg = parse_config(&v).unwrap();
        let p = render_planar_track(&cfg).unwrap();
        let ceil = 10f32.powf(-1.0 / 20.0);
        let peak = p.channels.iter().flatten().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak <= ceil + 1e-4, "peak {peak}");
        let lufs = p.lufs();
        assert!((lufs + 12.0).abs() < 1.0, "lufs {lufs}");
    }

    fn arrangement(extra_tracks: serde_json::Value) -> Arrangement {
        let mut a = json!({"title":"A","bpm":120,"length_beats":16,
            "tracks":[{"id":"d","name":"Drums"},{"id":"b","name":"Bass"}],
            "clips":[{"track":"d","start":0,"length_beats":16,"pattern":kit()},
                     {"track":"b","start":0,"pattern":{"steps":16,"voices":[{"kind":"bass","rhythm":"x.x.x.x.","octave":1}]}}]});
        if let Some(t) = extra_tracks.as_array() {
            a["tracks"].as_array_mut().unwrap().extend(t.iter().cloned());
        }
        parse_arrangement(&a).unwrap()
    }

    #[test]
    fn patterns_loop_to_fill_their_clip() {
        let a = arrangement(json!([]));
        let mix = render_mix(&a, &crate::audio::NoAudio, &MixOptions::monitor()).unwrap();
        let drums = &mix.tracks[0].stem;
        let spb = 44_100.0 * 0.5;
        // Energy in the fourth bar (beats 12–16) — only there if it looped.
        let s = (12.0 * spb) as usize;
        let e = (16.0 * spb) as usize;
        let energy: f64 = drums.l[s..e].iter().map(|x| (*x as f64).powi(2)).sum();
        assert!(energy > 1.0, "clip did not loop: {energy}");
    }

    #[test]
    fn stems_are_cached_and_mute_solo_apply() {
        let mut a = arrangement(json!([]));
        let m1 = render_mix(&a, &crate::audio::NoAudio, &MixOptions::monitor()).unwrap();
        a.tracks[1].level = 0.3; // fader moves never re-render a stem
        let m2 = render_mix(&a, &crate::audio::NoAudio, &MixOptions::monitor()).unwrap();
        assert_eq!(m1.tracks[0].key, m2.tracks[0].key);
        assert_eq!(m1.tracks[1].key, m2.tracks[1].key);
        a.tracks[0].solo = true;
        let solo = render_mix(&a, &crate::audio::NoAudio, &MixOptions::export()).unwrap();
        a.tracks[0].solo = false;
        a.tracks[0].mute = true;
        let muted = render_mix(&a, &crate::audio::NoAudio, &MixOptions::export()).unwrap();
        let e = |p: &PlanarRender| p.channels[0].iter().map(|x| (*x as f64).powi(2)).sum::<f64>();
        assert!(e(&solo.master) > 0.0 && e(&muted.master) > 0.0);
        assert!((e(&solo.master) - e(&muted.master)).abs() > 1.0);
    }

    #[test]
    fn audio_clips_play_from_the_home_sandbox() {
        let dir = std::env::temp_dir().join(format!("studio-audio-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Music")).unwrap();
        let tone: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let wav = crate::wav::encode_wav_bits(&[tone.clone(), tone], 48_000, 16);
        std::fs::write(dir.join("Music/song.wav"), wav).unwrap();
        let a = parse_arrangement(&json!({"bpm":120,"length_beats":8,
            "tracks":[{"id":"s","name":"Song","kind":"audio","level":1.0}],
            "clips":[{"track":"s","start":2,"audio":{"src":"Music/song.wav","fade_in":0.01}}]})).unwrap();
        let res = crate::audio::HomeResolver { home: dir.clone() };
        let mix = render_mix(&a, &res, &MixOptions::monitor()).unwrap();
        let stem = &mix.tracks[0].stem;
        // Silent before beat 2 (1 s at 120 BPM), audible after.
        assert!(stem.l[..44_000].iter().all(|x| *x == 0.0));
        assert!(stem.l[44_100 + 2000..44_100 + 4000].iter().any(|x| x.abs() > 0.3));
        // Escapes are refused.
        let bad = parse_arrangement(&json!({"bpm":120,"length_beats":8,"tracks":[{"id":"s"}],
            "clips":[{"track":"s","audio":{"src":"../x.wav"}}]})).unwrap();
        assert!(render_mix(&bad, &res, &MixOptions::export()).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn sends_feed_the_returns() {
        let mut a = arrangement(json!([]));
        a.tracks[0].sends = Some(TrackSends { reverb: 0.5, delay: 0.3 });
        let mix = render_mix(&a, &crate::audio::NoAudio, &MixOptions::monitor()).unwrap();
        assert!(mix.reverb.is_some() && mix.delay.is_some());
        let (_, rv) = mix.reverb.as_ref().unwrap();
        assert!(rv.peak() > 0.01);
    }

    #[test]
    fn track_level_automation_is_applied() {
        let mut a = arrangement(json!([]));
        a.tracks[1].mute = true;
        a.tracks[0].automation.lanes.push(AutomationLane {
            param: "track.level".into(),
            points: vec![AutomationPoint { beat: 0.0, value: 0.0 }, AutomationPoint { beat: 8.0, value: 0.0 }, AutomationPoint { beat: 8.01, value: 1.0 }],
        });
        let mix = render_mix(&a, &crate::audio::NoAudio, &MixOptions::export()).unwrap();
        let spb = 22_050usize;
        let first: f32 = mix.master.channels[0][..spb * 7].iter().fold(0.0, |m, x| m.max(x.abs()));
        let second: f32 = mix.master.channels[0][spb * 9..spb * 15].iter().fold(0.0, |m, x| m.max(x.abs()));
        assert!(first < 1e-3 && second > 0.05, "first {first} second {second}");
    }
}
