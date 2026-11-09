//! The video project: one JSON document shared by the editor window, the
//! REST routes and the agent's tools.
//!
//! Media files are never modified: a clip points at a media item and says
//! which part of it plays, where on the timeline, how fast, how loud and
//! where on the canvas. Every change goes through [`apply`] (validated,
//! clamped) and every saved change can be undone from the version history.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const MAX_CLIPS: usize = 2000;
pub const MAX_HTML: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect01 {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Default for Rect01 {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        }
    }
}

impl Rect01 {
    fn clamped(self) -> Self {
        let w = self.w.clamp(0.01, 4.0);
        let h = self.h.clamp(0.01, 4.0);
        Self {
            x: self.x.clamp(-2.0, 2.0),
            y: self.y.clamp(-2.0, 2.0),
            w,
            h,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Media {
    pub id: String,
    pub path: PathBuf,
    pub label: String,
    #[serde(flatten)]
    pub info: super::media::Info,
    /// Names of the audio streams in order (`mix`, `mic`, `desktop`) when
    /// known: a Capture recording.
    pub tracks: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Track {
    pub id: String,
    /// `video`, `audio`, `overlay` or `captions`. Video tracks later in the
    /// list are drawn on top.
    pub kind: String,
    pub name: String,
    pub muted: bool,
    pub hidden: bool,
    pub locked: bool,
    pub volume: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Clip {
    pub id: String,
    pub track: String,
    pub media: String,
    /// Timeline start, seconds.
    pub at: f64,
    /// Source in and out points, seconds.
    #[serde(rename = "in")]
    pub src_in: f64,
    pub out: f64,
    pub speed: f64,
    pub volume: f64,
    /// Which audio stream of the media plays (a recording has mix/mic/desktop).
    pub audio_track: u32,
    pub fade_in: f64,
    pub fade_out: f64,
    /// Placement on the canvas, as fractions of it.
    pub rect: Rect01,
    pub opacity: f64,
    /// Corner roundness 0..1 (the webcam bubble look).
    pub radius: f64,
    /// Proposed by the AI and not accepted yet.
    pub ghost: bool,
}

impl Default for Clip {
    fn default() -> Self {
        Self {
            id: String::new(),
            track: String::new(),
            media: String::new(),
            at: 0.0,
            src_in: 0.0,
            out: 0.0,
            speed: 1.0,
            volume: 1.0,
            audio_track: 0,
            fade_in: 0.0,
            fade_out: 0.0,
            rect: Rect01::default(),
            opacity: 1.0,
            radius: 0.0,
            ghost: false,
        }
    }
}

impl Clip {
    /// Length on the timeline.
    pub fn len(&self) -> f64 {
        ((self.out - self.src_in) / self.speed).max(0.0)
    }

    pub fn end(&self) -> f64 {
        self.at + self.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() <= 0.0
    }
}

/// An HTML graphic: a whole document written by the AI (or the user), shown
/// over the video from `at` for `dur` seconds, inside `rect`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Overlay {
    pub id: String,
    pub track: String,
    pub name: String,
    pub at: f64,
    pub dur: f64,
    pub rect: Rect01,
    pub opacity: f64,
    pub html: String,
    pub ghost: bool,
}

impl Default for Overlay {
    fn default() -> Self {
        Self {
            id: String::new(),
            track: String::new(),
            name: String::new(),
            at: 0.0,
            dur: 4.0,
            rect: Rect01::default(),
            opacity: 1.0,
            html: String::new(),
            ghost: false,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Caption {
    pub id: String,
    pub at: f64,
    pub dur: f64,
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Marker {
    pub t: f64,
    pub label: String,
}

/// The visual language the AI keeps across graphics (10.4 of the plan).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct StyleDoc {
    /// Short description: palette, type, motion, density.
    pub guide: String,
    /// CSS custom properties every graphic receives.
    pub css: String,
    /// The user's own guidelines, which win over the AI's.
    pub user: String,
    /// Locked: the AI must keep it.
    pub locked: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Proposal {
    pub id: String,
    pub summary: String,
    pub ops: Vec<Value>,
    /// `pending`, `accepted`, `rejected`.
    pub status: String,
    pub created: u64,
    /// The user's note when asking for changes.
    pub feedback: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Project {
    pub id: String,
    pub title: String,
    pub created: u64,
    pub updated: u64,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub background: String,
    pub media: Vec<Media>,
    pub tracks: Vec<Track>,
    pub clips: Vec<Clip>,
    pub overlays: Vec<Overlay>,
    pub captions: Vec<Caption>,
    pub markers: Vec<Marker>,
    pub style: StyleDoc,
    pub proposals: Vec<Proposal>,
    /// Bumped on every saved change; the window sends it back to detect a
    /// concurrent edit by the agent.
    pub revision: u64,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: "Untitled".into(),
            created: 0,
            updated: 0,
            width: 1920,
            height: 1080,
            fps: 30,
            background: "#000000".into(),
            media: Vec::new(),
            tracks: default_tracks(),
            clips: Vec::new(),
            overlays: Vec::new(),
            captions: Vec::new(),
            markers: Vec::new(),
            style: StyleDoc::default(),
            proposals: Vec::new(),
            revision: 0,
        }
    }
}

fn default_tracks() -> Vec<Track> {
    let t = |id: &str, kind: &str, name: &str| Track {
        id: id.into(),
        kind: kind.into(),
        name: name.into(),
        volume: 1.0,
        ..Track::default()
    };
    vec![
        t("v1", "video", "Video"),
        t("v2", "video", "Camera"),
        t("o1", "overlay", "Graphics"),
        t("a1", "audio", "Voice"),
        t("a2", "audio", "Sound"),
        t("c1", "captions", "Captions"),
    ]
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn new_id(prefix: &str) -> String {
    format!(
        "{prefix}{}",
        &uuid::Uuid::new_v4().simple().to_string()[..10]
    )
}

fn finite(v: f64, d: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        d
    }
}

impl Project {
    pub fn duration(&self) -> f64 {
        let c = self.clips.iter().map(Clip::end).fold(0.0, f64::max);
        let o = self
            .overlays
            .iter()
            .map(|o| o.at + o.dur)
            .fold(0.0, f64::max);
        let k = self
            .captions
            .iter()
            .map(|c| c.at + c.dur)
            .fold(0.0, f64::max);
        c.max(o).max(k)
    }

    pub fn media(&self, id: &str) -> Option<&Media> {
        self.media.iter().find(|m| m.id == id)
    }

    pub fn track(&self, id: &str) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    /// Clamp everything into range and drop what points nowhere. Run on
    /// every document that comes in from outside.
    pub fn sanitize(&mut self) {
        self.title = self
            .title
            .chars()
            .filter(|c| !c.is_control())
            .take(120)
            .collect();
        self.width = (self.width.clamp(160, 3840)) & !1;
        self.height = (self.height.clamp(160, 3840)) & !1;
        self.fps = self.fps.clamp(10, 60);
        if !is_hex_color(&self.background) {
            self.background = "#000000".into();
        }
        if self.tracks.is_empty() {
            self.tracks = default_tracks();
        }
        let mut seen = std::collections::HashSet::new();
        self.tracks.retain(|t| {
            matches!(t.kind.as_str(), "video" | "audio" | "overlay" | "captions")
                && seen.insert(t.id.clone())
        });
        for t in &mut self.tracks {
            t.volume = finite(t.volume, 1.0).clamp(0.0, 4.0);
            t.name = t.name.chars().take(60).collect();
        }
        let media_ids: std::collections::HashSet<String> =
            self.media.iter().map(|m| m.id.clone()).collect();
        let track_ids: std::collections::HashSet<String> =
            self.tracks.iter().map(|t| t.id.clone()).collect();
        self.clips.truncate(MAX_CLIPS);
        self.clips
            .retain(|c| media_ids.contains(&c.media) && track_ids.contains(&c.track));
        let durations: std::collections::HashMap<String, (f64, String)> = self
            .media
            .iter()
            .map(|m| (m.id.clone(), (m.info.duration, m.info.kind.clone())))
            .collect();
        for c in &mut self.clips {
            if c.id.is_empty() {
                c.id = new_id("c");
            }
            let (dur, kind) = durations
                .get(&c.media)
                .cloned()
                .unwrap_or((0.0, String::new()));
            c.speed = finite(c.speed, 1.0).clamp(0.1, 16.0);
            c.at = finite(c.at, 0.0).max(0.0);
            let max = if kind == "image" {
                24.0 * 3600.0
            } else {
                dur.max(0.04)
            };
            c.src_in = finite(c.src_in, 0.0).clamp(0.0, max);
            c.out = finite(c.out, max).clamp(c.src_in + 0.04, max.max(c.src_in + 0.04));
            c.volume = finite(c.volume, 1.0).clamp(0.0, 4.0);
            c.fade_in = finite(c.fade_in, 0.0).clamp(0.0, c.len());
            c.fade_out = finite(c.fade_out, 0.0).clamp(0.0, c.len());
            c.opacity = finite(c.opacity, 1.0).clamp(0.0, 1.0);
            c.radius = finite(c.radius, 0.0).clamp(0.0, 1.0);
            c.rect = c.rect.clamped();
        }
        for o in &mut self.overlays {
            if o.id.is_empty() {
                o.id = new_id("h");
            }
            if !track_ids.contains(&o.track) {
                o.track = self
                    .tracks
                    .iter()
                    .find(|t| t.kind == "overlay")
                    .map(|t| t.id.clone())
                    .unwrap_or_default();
            }
            o.at = finite(o.at, 0.0).max(0.0);
            o.dur = finite(o.dur, 4.0).clamp(0.1, 3600.0);
            o.opacity = finite(o.opacity, 1.0).clamp(0.0, 1.0);
            o.rect = o.rect.clamped();
            o.name = o.name.chars().take(80).collect();
            if o.html.len() > MAX_HTML {
                o.html.truncate(MAX_HTML);
            }
        }
        for c in &mut self.captions {
            if c.id.is_empty() {
                c.id = new_id("t");
            }
            c.at = finite(c.at, 0.0).max(0.0);
            c.dur = finite(c.dur, 2.0).clamp(0.1, 60.0);
            c.text = c.text.chars().take(400).collect();
        }
        self.captions.sort_by(|a, b| a.at.total_cmp(&b.at));
        for m in &mut self.markers {
            m.t = finite(m.t, 0.0).max(0.0);
            m.label = m.label.chars().take(120).collect();
        }
        self.style.guide = self.style.guide.chars().take(4000).collect();
        self.style.css = self.style.css.chars().take(8000).collect();
        self.style.user = self.style.user.chars().take(4000).collect();
        self.proposals.retain(|p| p.status == "pending");
    }

    pub fn first_track(&self, kind: &str) -> Option<String> {
        self.tracks
            .iter()
            .find(|t| t.kind == kind)
            .map(|t| t.id.clone())
    }

    /// Add a media file (probed), returning its id; the same file twice is
    /// one item.
    pub fn add_media(
        &mut self,
        path: &Path,
        info: super::media::Info,
        tracks: Vec<String>,
    ) -> String {
        if let Some(m) = self.media.iter().find(|m| m.path == path) {
            return m.id.clone();
        }
        let id = new_id("m");
        self.media.push(Media {
            id: id.clone(),
            path: path.to_path_buf(),
            label: path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
            info,
            tracks,
        });
        id
    }

    /// Put a whole media item on the timeline at `at`: its picture on a
    /// video track, and each of its audio streams (a recording's mic and
    /// desktop sound, else the first stream) on an audio track.
    pub fn place_media(&mut self, media: &str, at: f64) -> Vec<String> {
        let Some(m) = self.media(media).cloned() else {
            return Vec::new();
        };
        let mut ids = Vec::new();
        let dur = if m.info.kind == "image" {
            5.0
        } else {
            m.info.duration
        };
        if m.info.has_video || m.info.kind == "image" {
            if let Some(t) = self.first_track("video") {
                let id = new_id("c");
                self.clips.push(Clip {
                    id: id.clone(),
                    track: t,
                    media: m.id.clone(),
                    at,
                    out: dur,
                    volume: 0.0,
                    ..Clip::default()
                });
                ids.push(id);
            }
        }
        // A recording's separate sources come after its mix: put each on
        // its own track, so levels can be fixed afterwards.
        let streams: Vec<u32> =
            if m.tracks.first().is_some_and(|t| t == "mix") && m.info.audio_tracks > 1 {
                (1..m.info.audio_tracks).collect()
            } else if m.info.audio_tracks > 0 {
                vec![0]
            } else {
                Vec::new()
            };
        let audio_tracks: Vec<String> = self
            .tracks
            .iter()
            .filter(|t| t.kind == "audio")
            .map(|t| t.id.clone())
            .collect();
        for (i, s) in streams.iter().enumerate() {
            let Some(t) = audio_tracks
                .get(i.min(audio_tracks.len().saturating_sub(1)))
                .cloned()
            else {
                break;
            };
            let id = new_id("c");
            self.clips.push(Clip {
                id: id.clone(),
                track: t,
                media: m.id.clone(),
                at,
                out: dur,
                audio_track: *s,
                ..Clip::default()
            });
            ids.push(id);
        }
        ids
    }
}

fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/* ── operations ──────────────────────────────────────────────── */

fn num(op: &Value, k: &str) -> Option<f64> {
    op.get(k).and_then(|v| v.as_f64()).filter(|v| v.is_finite())
}

fn s<'a>(op: &'a Value, k: &str) -> Option<&'a str> {
    op.get(k).and_then(|v| v.as_str())
}

fn rect(op: &Value) -> Option<Rect01> {
    serde_json::from_value(op.get("rect")?.clone()).ok()
}

/// Remove `[a, b)` of timeline time from every clip (and overlay and
/// caption) on the given tracks, closing the gap: what "cut the part where I
/// look for the file" means.
pub fn cut_range(p: &mut Project, a: f64, b: f64, tracks: Option<&[String]>) {
    if b <= a {
        return;
    }
    let gap = b - a;
    let on = |t: &str| tracks.is_none_or(|ts| ts.iter().any(|x| x == t));
    let mut out = Vec::with_capacity(p.clips.len() + 4);
    for c in p.clips.drain(..) {
        if !on(&c.track) {
            out.push(c);
            continue;
        }
        let (s, e) = (c.at, c.end());
        if e <= a {
            out.push(c);
        } else if s >= b {
            let mut c = c;
            c.at -= gap;
            out.push(c);
        } else {
            // Keep what lies before a and after b.
            if s < a {
                let mut l = c.clone();
                l.out = l.src_in + (a - s) * l.speed;
                l.fade_out = l.fade_out.min(l.len());
                out.push(l);
            }
            if e > b {
                let mut r = c.clone();
                r.id = new_id("c");
                r.src_in += (b - s) * r.speed;
                r.at = a;
                r.fade_in = r.fade_in.min(r.len());
                out.push(r);
            }
        }
    }
    p.clips = out;
    let shift = |at: &mut f64, dur: &mut f64| -> bool {
        let e = *at + *dur;
        if e <= a {
            true
        } else if *at >= b {
            *at -= gap;
            true
        } else {
            let keep = (a - *at).max(0.0) + (e - b).max(0.0);
            *at = at.min(a);
            *dur = keep;
            keep > 0.05
        }
    };
    if tracks.is_none() {
        p.overlays.retain_mut(|o| shift(&mut o.at, &mut o.dur));
        p.captions.retain_mut(|c| shift(&mut c.at, &mut c.dur));
        for m in &mut p.markers {
            if m.t >= b {
                m.t -= gap;
            } else if m.t > a {
                m.t = a;
            }
        }
    }
}

/// Apply one operation. Returns the ids it created, or an error that
/// names what was wrong (the agent reads it and corrects itself).
pub fn apply(p: &mut Project, op: &Value) -> Result<Vec<String>, String> {
    let kind = s(op, "op").ok_or("each operation needs \"op\"")?;
    let clip_idx = |p: &Project, op: &Value| -> Result<usize, String> {
        let id = s(op, "clip")
            .or_else(|| s(op, "id"))
            .ok_or("missing \"clip\"")?;
        p.clips
            .iter()
            .position(|c| c.id == id)
            .ok_or_else(|| format!("no clip {id}"))
    };
    let mut created = Vec::new();
    match kind {
        "add_clip" => {
            let media = s(op, "media").ok_or("missing \"media\"")?;
            let m = p
                .media(media)
                .ok_or_else(|| format!("no media {media}"))?
                .clone();
            let track = match s(op, "track") {
                Some(t) => p
                    .track(t)
                    .ok_or_else(|| format!("no track {t}"))?
                    .id
                    .clone(),
                None => p
                    .first_track(if m.info.has_video || m.info.kind == "image" {
                        "video"
                    } else {
                        "audio"
                    })
                    .ok_or("no track for it")?,
            };
            let dur = if m.info.kind == "image" {
                5.0
            } else {
                m.info.duration
            };
            let id = new_id("c");
            let mut c = Clip {
                id: id.clone(),
                track,
                media: m.id.clone(),
                at: num(op, "at").unwrap_or_else(|| p.duration()),
                src_in: num(op, "in").unwrap_or(0.0),
                out: num(op, "out").unwrap_or(dur),
                speed: num(op, "speed").unwrap_or(1.0),
                volume: num(op, "volume").unwrap_or(1.0),
                audio_track: num(op, "audio_track").unwrap_or(0.0) as u32,
                ghost: op.get("ghost").and_then(|v| v.as_bool()).unwrap_or(false),
                ..Clip::default()
            };
            if let Some(r) = rect(op) {
                c.rect = r;
            }
            if let Some(r) = num(op, "radius") {
                c.radius = r;
            }
            p.clips.push(c);
            created.push(id);
        }
        "place_media" => {
            let media = s(op, "media").ok_or("missing \"media\"")?;
            if p.media(media).is_none() {
                return Err(format!("no media {media}"));
            }
            let at = num(op, "at").unwrap_or_else(|| p.duration());
            created = p.place_media(media, at);
        }
        "move_clip" => {
            let i = clip_idx(p, op)?;
            if let Some(at) = num(op, "at") {
                p.clips[i].at = at;
            }
            if let Some(t) = s(op, "track") {
                let t = p
                    .track(t)
                    .ok_or_else(|| format!("no track {t}"))?
                    .id
                    .clone();
                p.clips[i].track = t;
            }
        }
        "trim_clip" => {
            let i = clip_idx(p, op)?;
            if let Some(v) = num(op, "in") {
                p.clips[i].src_in = v;
            }
            if let Some(v) = num(op, "out") {
                p.clips[i].out = v;
            }
            if let Some(v) = num(op, "at") {
                p.clips[i].at = v;
            }
        }
        "set_clip" => {
            let i = clip_idx(p, op)?;
            let c = &mut p.clips[i];
            for (k, f) in [
                ("speed", &mut c.speed as &mut f64),
                ("volume", &mut c.volume),
                ("fade_in", &mut c.fade_in),
                ("fade_out", &mut c.fade_out),
                ("opacity", &mut c.opacity),
                ("radius", &mut c.radius),
            ] {
                if let Some(v) = num(op, k) {
                    *f = v;
                }
            }
            if let Some(r) = rect(op) {
                c.rect = r;
            }
            if let Some(a) = num(op, "audio_track") {
                c.audio_track = a as u32;
            }
        }
        "split_clip" => {
            let i = clip_idx(p, op)?;
            let t = num(op, "t").ok_or("missing \"t\" (timeline seconds)")?;
            let c = p.clips[i].clone();
            if t <= c.at + 0.04 || t >= c.end() - 0.04 {
                return Err("the split point must be inside the clip".into());
            }
            let cut = c.src_in + (t - c.at) * c.speed;
            p.clips[i].out = cut;
            p.clips[i].fade_out = 0.0;
            let id = new_id("c");
            p.clips.push(Clip {
                id: id.clone(),
                src_in: cut,
                at: t,
                fade_in: 0.0,
                ..c
            });
            created.push(id);
        }
        "delete_clip" => {
            let i = clip_idx(p, op)?;
            p.clips.remove(i);
        }
        "cut_ranges" => {
            let ranges = op
                .get("ranges")
                .and_then(|v| v.as_array())
                .ok_or("missing \"ranges\": [[from, to], …]")?;
            let mut rs: Vec<(f64, f64)> = ranges
                .iter()
                .filter_map(|r| Some((r.get(0)?.as_f64()?, r.get(1)?.as_f64()?)))
                .filter(|(a, b)| b > a)
                .collect();
            // Latest first, so earlier ranges keep their times.
            rs.sort_by(|a, b| b.0.total_cmp(&a.0));
            for (a, b) in rs {
                cut_range(p, a, b, None);
            }
        }
        "add_overlay" | "update_overlay" => {
            let existing = s(op, "id").and_then(|id| p.overlays.iter().position(|o| o.id == id));
            if kind == "update_overlay" && existing.is_none() {
                return Err("no such overlay".into());
            }
            let mut o = existing
                .map(|i| p.overlays[i].clone())
                .unwrap_or_else(|| Overlay {
                    id: new_id("h"),
                    track: p.first_track("overlay").unwrap_or_default(),
                    at: 0.0,
                    ..Overlay::default()
                });
            if let Some(h) = s(op, "html") {
                if h.len() > MAX_HTML {
                    return Err(format!("the HTML is larger than {} KB", MAX_HTML / 1024));
                }
                o.html = h.to_string();
            }
            if let Some(v) = num(op, "at") {
                o.at = v;
            }
            if let Some(v) = num(op, "dur") {
                o.dur = v;
            }
            if let Some(v) = num(op, "opacity") {
                o.opacity = v;
            }
            if let Some(r) = rect(op) {
                o.rect = r;
            }
            if let Some(n) = s(op, "name") {
                o.name = n.to_string();
            }
            if let Some(g) = op.get("ghost").and_then(|v| v.as_bool()) {
                o.ghost = g;
            }
            if o.html.trim().is_empty() {
                return Err("an overlay needs \"html\"".into());
            }
            created.push(o.id.clone());
            match existing {
                Some(i) => p.overlays[i] = o,
                None => p.overlays.push(o),
            }
        }
        "delete_overlay" => {
            let id = s(op, "id").ok_or("missing \"id\"")?;
            let n = p.overlays.len();
            p.overlays.retain(|o| o.id != id);
            if p.overlays.len() == n {
                return Err(format!("no overlay {id}"));
            }
        }
        "add_caption" => {
            let id = new_id("t");
            p.captions.push(Caption {
                id: id.clone(),
                at: num(op, "at").ok_or("missing \"at\"")?,
                dur: num(op, "dur").unwrap_or(2.5),
                text: s(op, "text").ok_or("missing \"text\"")?.to_string(),
            });
            created.push(id);
        }
        "set_captions" => {
            let list = op
                .get("captions")
                .and_then(|v| v.as_array())
                .ok_or("missing \"captions\"")?;
            p.captions = list
                .iter()
                .filter_map(|c| {
                    Some(Caption {
                        id: new_id("t"),
                        at: c.get("at")?.as_f64()?,
                        dur: c.get("dur").and_then(|v| v.as_f64()).unwrap_or(2.5),
                        text: c.get("text")?.as_str()?.to_string(),
                    })
                })
                .collect();
        }
        "delete_caption" => {
            let id = s(op, "id").ok_or("missing \"id\"")?;
            p.captions.retain(|c| c.id != id);
        }
        "add_marker" => p.markers.push(Marker {
            t: num(op, "t").ok_or("missing \"t\"")?,
            label: s(op, "label").unwrap_or("").to_string(),
        }),
        "set_style" => {
            if p.style.locked && op.get("force").and_then(|v| v.as_bool()) != Some(true) {
                return Err("the style is locked by the user; ask before changing it".into());
            }
            if let Some(g) = s(op, "guide") {
                p.style.guide = g.to_string();
            }
            if let Some(c) = s(op, "css") {
                p.style.css = c.to_string();
            }
        }
        "set_track" => {
            let id = s(op, "track").ok_or("missing \"track\"")?;
            let t = p
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or_else(|| format!("no track {id}"))?;
            if let Some(v) = num(op, "volume") {
                t.volume = v;
            }
            if let Some(v) = op.get("muted").and_then(|v| v.as_bool()) {
                t.muted = v;
            }
            if let Some(v) = op.get("hidden").and_then(|v| v.as_bool()) {
                t.hidden = v;
            }
        }
        "set_project" => {
            if let Some(t) = s(op, "title") {
                p.title = t.to_string();
            }
            if let (Some(w), Some(h)) = (num(op, "width"), num(op, "height")) {
                p.width = w as u32;
                p.height = h as u32;
            }
            if let Some(b) = s(op, "background") {
                p.background = b.to_string();
            }
        }
        other => return Err(format!("unknown op {other}")),
    }
    p.sanitize();
    Ok(created)
}

/// Apply a list atomically: all or nothing.
pub fn apply_all(p: &mut Project, ops: &[Value]) -> Result<Vec<String>, String> {
    let mut work = p.clone();
    let mut ids = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        ids.extend(apply(&mut work, op).map_err(|e| format!("operation {}: {e}", i + 1))?);
    }
    *p = work;
    Ok(ids)
}

/* ── storage ─────────────────────────────────────────────────── */

pub fn projects_dir(home: &Path) -> PathBuf {
    crate::library::capture_dir(home).join("Projects")
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

pub fn path_of(home: &Path, id: &str) -> Option<PathBuf> {
    valid_id(id).then(|| projects_dir(home).join(format!("{id}.json")))
}

pub fn load(home: &Path, id: &str) -> Option<Project> {
    let b = std::fs::read(path_of(home, id)?).ok()?;
    let mut p: Project = serde_json::from_slice(&b).ok()?;
    p.id = id.to_string();
    Some(p)
}

/// Save, keeping the previous copy in the version history (last 50).
pub fn save(home: &Path, p: &mut Project) -> Result<(), String> {
    write(home, p, true)
}

/// Save a project restored from version `ts` as one step of Undo: the
/// current state is not pushed on the history and `ts` is taken off it, so
/// the next Undo goes one step further back instead of redoing this one.
pub fn save_undone(home: &Path, p: &mut Project, ts: u64) -> Result<(), String> {
    write(home, p, false)?;
    if valid_id(&p.id) {
        let _ = std::fs::remove_file(
            projects_dir(home)
                .join(".versions")
                .join(&p.id)
                .join(format!("{ts}.json")),
        );
    }
    Ok(())
}

fn write(home: &Path, p: &mut Project, snapshot: bool) -> Result<(), String> {
    let path = path_of(home, &p.id).ok_or("bad project id")?;
    let dir = projects_dir(home);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    if snapshot && path.exists() {
        let vdir = dir.join(".versions").join(&p.id);
        let _ = std::fs::create_dir_all(&vdir);
        let _ = std::fs::copy(&path, vdir.join(format!("{}.json", now_ms())));
        prune_versions(&vdir, 50);
    }
    p.updated = now_ms();
    p.revision += 1;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(p).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

fn prune_versions(dir: &Path, keep: usize) {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    v.sort();
    while v.len() > keep {
        let _ = std::fs::remove_file(v.remove(0));
    }
}

/// Version timestamps, newest first.
pub fn versions(home: &Path, id: &str) -> Vec<u64> {
    if !valid_id(id) {
        return Vec::new();
    }
    let mut v: Vec<u64> = std::fs::read_dir(projects_dir(home).join(".versions").join(id))
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter_map(|e| e.path().file_stem()?.to_str()?.parse().ok())
                .collect()
        })
        .unwrap_or_default();
    v.sort_unstable_by(|a, b| b.cmp(a));
    v
}

pub fn load_version(home: &Path, id: &str, ts: u64) -> Option<Project> {
    if !valid_id(id) {
        return None;
    }
    let b = std::fs::read(
        projects_dir(home)
            .join(".versions")
            .join(id)
            .join(format!("{ts}.json")),
    )
    .ok()?;
    serde_json::from_slice(&b).ok()
}

pub fn list(home: &Path) -> Vec<Value> {
    let mut out: Vec<(u64, Value)> = std::fs::read_dir(projects_dir(home))
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "json"))
                .filter_map(|p| {
                    let id = p.file_stem()?.to_str()?.to_string();
                    let pr = load(home, &id)?;
                    Some((
                        pr.updated,
                        serde_json::json!({
                            "id": id,
                            "title": pr.title,
                            "updated": pr.updated,
                            "duration": pr.duration(),
                            "width": pr.width,
                            "height": pr.height,
                            "clips": pr.clips.len(),
                        }),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|a| std::cmp::Reverse(a.0));
    out.into_iter().map(|(_, v)| v).collect()
}

pub fn create(home: &Path, title: Option<&str>) -> Result<Project, String> {
    let mut p = Project {
        id: uuid::Uuid::new_v4().to_string(),
        title: title.unwrap_or("Untitled").to_string(),
        created: now_ms(),
        ..Project::default()
    };
    p.sanitize();
    save(home, &mut p)?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn proj() -> Project {
        let mut p = Project::default();
        p.media.push(Media {
            id: "m1".into(),
            path: "/v/a.mp4".into(),
            info: super::super::media::Info {
                kind: "video".into(),
                duration: 60.0,
                width: 1920,
                height: 1080,
                fps: 30.0,
                audio_tracks: 3,
                has_video: true,
            },
            tracks: vec!["mix".into(), "mic".into(), "desktop".into()],
            ..Media::default()
        });
        p
    }

    #[test]
    fn undo_steps_back_one_change_at_a_time() {
        let home = std::env::temp_dir().join(format!("capture-undo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let mut p = Project {
            id: "undo-test".into(),
            ..Project::default()
        };
        let pause = || std::thread::sleep(std::time::Duration::from_millis(3));
        for title in ["a", "b", "c"] {
            p.title = title.into();
            save(&home, &mut p).unwrap();
            pause();
        }
        let undo = |home: &Path| {
            let cur = load(home, "undo-test").unwrap();
            let ts = versions(home, "undo-test")[0];
            let mut old = load_version(home, "undo-test", ts).unwrap();
            old.revision = cur.revision;
            save_undone(home, &mut old, ts).unwrap();
            pause();
            load(home, "undo-test").unwrap().title
        };
        assert_eq!(undo(&home), "b");
        assert_eq!(
            undo(&home),
            "a",
            "a second undo goes further back, not forward"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn placing_a_recording_splits_its_sources() {
        let mut p = proj();
        let ids = p.place_media("m1", 0.0);
        assert_eq!(ids.len(), 3);
        let audio: Vec<u32> = p
            .clips
            .iter()
            .filter(|c| c.track.starts_with('a'))
            .map(|c| c.audio_track)
            .collect();
        assert_eq!(audio, vec![1, 2]);
        assert_eq!(p.clips[0].volume, 0.0, "the picture clip is silent");
    }

    #[test]
    fn cut_closes_the_gap_on_every_track() {
        let mut p = proj();
        p.place_media("m1", 0.0);
        apply(
            &mut p,
            &json!({"op": "cut_ranges", "ranges": [[10, 20], [30, 35]]}),
        )
        .unwrap();
        let v: Vec<&Clip> = p.clips.iter().filter(|c| c.track == "v1").collect();
        assert_eq!(v.len(), 3);
        assert!((p.duration() - 45.0).abs() < 1e-9);
        assert!((v[1].src_in - 20.0).abs() < 1e-9 && (v[1].at - 10.0).abs() < 1e-9);
    }

    #[test]
    fn split_and_speed() {
        let mut p = proj();
        let ids = apply(
            &mut p,
            &json!({"op": "add_clip", "media": "m1", "at": 0, "out": 10}),
        )
        .unwrap();
        let new = apply(&mut p, &json!({"op": "split_clip", "clip": ids[0], "t": 4})).unwrap();
        apply(
            &mut p,
            &json!({"op": "set_clip", "clip": new[0], "speed": 2}),
        )
        .unwrap();
        let c = p.clips.iter().find(|c| c.id == new[0]).unwrap();
        assert!((c.len() - 3.0).abs() < 1e-9);
    }

    #[test]
    fn bad_ops_are_explained_and_atomic() {
        let mut p = proj();
        let before = p.clone();
        let e = apply_all(
            &mut p,
            &[
                json!({"op": "add_clip", "media": "m1"}),
                json!({"op": "nope"}),
            ],
        )
        .unwrap_err();
        assert!(e.contains("operation 2"));
        assert_eq!(p, before);
    }

    #[test]
    fn sanitize_clamps_hostile_values() {
        let mut p = proj();
        p.width = 99_999;
        p.clips.push(Clip {
            id: "x".into(),
            track: "v1".into(),
            media: "m1".into(),
            speed: f64::NAN,
            out: 1e9,
            ..Clip::default()
        });
        p.clips.push(Clip {
            id: "y".into(),
            track: "v1".into(),
            media: "ghost".into(),
            ..Clip::default()
        });
        p.sanitize();
        assert_eq!(p.width, 3840);
        assert_eq!(p.clips.len(), 1);
        assert_eq!(p.clips[0].speed, 1.0);
        assert_eq!(p.clips[0].out, 60.0);
    }

    #[test]
    fn locked_style_is_kept() {
        let mut p = proj();
        p.style.locked = true;
        assert!(apply(&mut p, &json!({"op": "set_style", "guide": "neon"})).is_err());
    }
}
