//! What the AI knows about a project, in as few tokens as possible.
//!
//! The model never sees pixels unless it has vision: it gets the timeline
//! (clips with ids and times), each media item's transcript in short
//! segments with ids, the recording's event log (which window was in front,
//! markers), scene changes for imported video, and the style document. The
//! transcript is also mapped onto the timeline, so "cut where I say ehm" is
//! one lookup for the model.

use super::project::{Clip, Project};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Words {
    pub lang: String,
    /// `(word, start, end)` in media seconds.
    pub words: Vec<(String, f64, f64)>,
}

/// Group words into segments at pauses (> 0.6 s) or every ~12 words.
pub fn segments(w: &Words) -> Vec<(f64, f64, String)> {
    let mut out = Vec::new();
    let (mut s, mut e, mut text, mut n) = (0.0, 0.0, String::new(), 0);
    for (word, a, b) in &w.words {
        if n > 0 && (a - e > 0.6 || n >= 12) {
            out.push((s, e, text.trim().to_string()));
            text.clear();
            n = 0;
        }
        if n == 0 {
            s = *a;
        }
        text.push_str(word);
        e = *b;
        n += 1;
    }
    if n > 0 {
        out.push((s, e, text.trim().to_string()));
    }
    out
}

const FILLERS: &[&str] = &[
    "ehm", "ehmm", "uhm", "uh", "um", "umm", "eh", "ah", "mmm", "hmm", "cioè", "tipo", "like",
    "allora",
];

/// Filler words and long silences (media time), for a cleanup pass.
/// `(from, to)` spans in seconds.
pub type Spans = Vec<(f64, f64)>;

pub fn fillers_and_pauses(w: &Words) -> (Spans, Spans) {
    let mut fillers = Vec::new();
    let mut pauses = Vec::new();
    let mut last_end: Option<f64> = None;
    for (word, a, b) in &w.words {
        let clean: String = word
            .trim()
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphabetic())
            .collect();
        if FILLERS.contains(&clean.as_str()) && b - a < 1.5 {
            fillers.push((*a, *b));
        }
        if let Some(e) = last_end {
            if a - e > 1.2 {
                pauses.push((e, *a));
            }
        }
        last_end = Some(*b);
    }
    (fillers, pauses)
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Media time `t` of `c` on the timeline, if the clip plays it.
pub fn to_timeline(c: &Clip, t: f64) -> Option<f64> {
    (t >= c.src_in && t <= c.out).then(|| c.at + (t - c.src_in) / c.speed)
}

pub fn words_for(path: &Path) -> Option<Words> {
    super::transcribe::cached(path)
}

pub fn summary(p: &Project, vision: bool) -> Value {
    let mut media = Vec::new();
    let mut timeline_text = Vec::new();
    for m in &p.media {
        let words = words_for(&m.path);
        let segs = words.as_ref().map(segments).unwrap_or_default();
        let meta: Value = std::fs::read(crate::library::sidecar(&m.path))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null);
        let scenes = if m.info.has_video && meta.is_null() {
            super::media::scenes(&m.path).unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut item = json!({
            "id": m.id,
            "label": m.label,
            "kind": m.info.kind,
            "duration": r2(m.info.duration),
            "size": [m.info.width, m.info.height],
            "audio": m.tracks,
            "transcribed": words.is_some(),
        });
        if !segs.is_empty() {
            item["transcript"] = json!(segs
                .iter()
                .enumerate()
                .map(|(i, (a, b, t))| json!([format!("s{i}"), r2(*a), r2(*b), t]))
                .collect::<Vec<_>>());
            if let Some(w) = &words {
                let (f, pz) = fillers_and_pauses(w);
                item["fillers"] =
                    json!(f.iter().map(|(a, b)| [r2(*a), r2(*b)]).collect::<Vec<_>>());
                item["pauses"] =
                    json!(pz.iter().map(|(a, b)| [r2(*a), r2(*b)]).collect::<Vec<_>>());
            }
        }
        if let Some(ev) = meta.get("events").and_then(|e| e.as_array()) {
            item["events"] = json!(ev
                .iter()
                .take(200)
                .map(|e| json!([
                    r2(e["t_us"].as_f64().unwrap_or(0.0) / 1e6),
                    e["kind"],
                    e["detail"]
                ]))
                .collect::<Vec<_>>());
        }
        if let Some(mk) = meta.get("markers").and_then(|e| e.as_array()) {
            item["markers"] = json!(mk
                .iter()
                .map(|e| json!([r2(e["t_us"].as_f64().unwrap_or(0.0) / 1e6), e["label"]]))
                .collect::<Vec<_>>());
        }
        if !scenes.is_empty() {
            item["scenes"] = json!(scenes.iter().take(80).map(|s| r2(*s)).collect::<Vec<_>>());
        }
        media.push(item);
        // The voice on the timeline: segments that audio clips of this media play.
        for c in p
            .clips
            .iter()
            .filter(|c| c.media == m.id && p.track(&c.track).is_some_and(|t| t.kind == "audio"))
        {
            let voice = m
                .tracks
                .get(c.audio_track as usize)
                .map(|t| t != "desktop")
                .unwrap_or(true);
            if !voice {
                continue;
            }
            for (i, (a, b, t)) in segs.iter().enumerate() {
                if let (Some(ta), Some(tb)) = (to_timeline(c, *a), to_timeline(c, *b)) {
                    timeline_text.push(json!([format!("{}:s{i}", m.id), r2(ta), r2(tb), t]));
                }
            }
        }
    }
    json!({
        "id": p.id,
        "title": p.title,
        "canvas": [p.width, p.height, p.fps],
        "duration": r2(p.duration()),
        "vision": vision,
        "tracks": p.tracks.iter().map(|t| json!([t.id, t.kind, t.name, t.muted])).collect::<Vec<_>>(),
        "clips": p.clips.iter().map(|c| json!({
            "id": c.id, "track": c.track, "media": c.media,
            "at": r2(c.at), "end": r2(c.end()), "in": r2(c.src_in), "out": r2(c.out),
            "speed": c.speed, "volume": c.volume, "audio_track": c.audio_track,
            "rect": [r2(c.rect.x), r2(c.rect.y), r2(c.rect.w), r2(c.rect.h)], "ghost": c.ghost,
        })).collect::<Vec<_>>(),
        "overlays": p.overlays.iter().map(|o| json!({
            "id": o.id, "name": o.name, "at": r2(o.at), "dur": r2(o.dur),
            "rect": [r2(o.rect.x), r2(o.rect.y), r2(o.rect.w), r2(o.rect.h)], "html_bytes": o.html.len(), "ghost": o.ghost,
        })).collect::<Vec<_>>(),
        "captions": p.captions.len(),
        "markers": p.markers.iter().map(|m| json!([r2(m.t), m.label])).collect::<Vec<_>>(),
        "style": { "guide": p.style.guide, "css": p.style.css, "user": p.style.user, "locked": p.style.locked },
        "proposals": p.proposals.iter().map(|x| json!({"id": x.id, "summary": x.summary, "ops": x.ops.len(), "feedback": x.feedback})).collect::<Vec<_>>(),
        "media": media,
        "timeline_transcript": timeline_text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(list: &[(&str, f64, f64)]) -> Words {
        Words {
            lang: "it".into(),
            words: list
                .iter()
                .map(|(a, b, c)| (a.to_string(), *b, *c))
                .collect(),
        }
    }

    #[test]
    fn segments_split_at_pauses() {
        let s = segments(&w(&[
            (" Ciao", 0.0, 0.4),
            (" a", 0.5, 0.6),
            (" tutti", 0.6, 1.0),
            (" Oggi", 2.0, 2.3),
        ]));
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].2, "Ciao a tutti");
        assert_eq!(s[1].0, 2.0);
    }

    #[test]
    fn finds_fillers_and_pauses() {
        let (f, p) = fillers_and_pauses(&w(&[
            (" Allora,", 0.0, 0.3),
            (" ehm", 0.4, 0.9),
            (" sì", 3.0, 3.2),
        ]));
        assert_eq!(f.len(), 2);
        assert_eq!(p, vec![(0.9, 3.0)]);
    }

    #[test]
    fn maps_media_time_to_the_timeline() {
        let c = Clip {
            at: 10.0,
            src_in: 5.0,
            out: 15.0,
            speed: 2.0,
            ..Clip::default()
        };
        assert_eq!(to_timeline(&c, 7.0), Some(11.0));
        assert_eq!(to_timeline(&c, 20.0), None);
    }
}
