//! Spoken lines for a project from the local text-to-speech sidecars: one
//! narrator or a dialogue (a podcast with two or three characters). Every
//! line is synthesised on its own, decoded to 48 kHz mono and laid end to
//! end with its pause, so the caller gets one WAV and the exact time of
//! each line (for captions and graphics in sync with the voice).

use super::http;
use crate::Result;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub const SR: u32 = 48_000;
/// Supertonic's built-in voice styles.
pub const SUPERTONIC_VOICES: &[&str] =
    &["M1", "F1", "M2", "F2", "M3", "F3", "M4", "F4", "M5", "F5"];
/// Qwen3-TTS CustomVoice speakers.
pub const QWEN_VOICES: &[&str] = &[
    "vivian", "ryan", "serena", "aiden", "eric", "sohee", "dylan", "ono_anna", "uncle_fu",
];
pub const MAX_LINES: usize = 120;
const MAX_CHARS: usize = 600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Supertonic,
    Qwen,
}

impl Engine {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "supertonic" => Some(Self::Supertonic),
            "qwen" | "qwen3" | "qwen-tts" | "qwen3-tts" => Some(Self::Qwen),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Supertonic => "supertonic",
            Self::Qwen => "qwen",
        }
    }
    pub fn voices(self) -> &'static [&'static str] {
        match self {
            Self::Supertonic => SUPERTONIC_VOICES,
            Self::Qwen => QWEN_VOICES,
        }
    }
}

pub fn supertonic_url(configured: &str) -> String {
    let u = configured.trim().trim_end_matches('/');
    if u.is_empty() {
        "http://127.0.0.1:7788".into()
    } else {
        u.to_string()
    }
}

pub fn qwen_url() -> String {
    std::env::var("QWEN_TTS_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .unwrap_or_else(|| "http://127.0.0.1:7787".into())
}

#[derive(Clone, Debug)]
pub struct Line {
    pub speaker: String,
    pub text: String,
    /// Silence after the line, seconds.
    pub pause: f64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Timed {
    pub speaker: String,
    pub voice: String,
    pub text: String,
    /// Start and length inside the generated file, seconds.
    pub at: f64,
    pub dur: f64,
}

/// One voice per speaker: the caller's choice when it names a known voice,
/// otherwise the next unused voice of the engine (alternating male/female
/// for Supertonic), in order of first appearance.
pub fn cast(
    engine: Engine,
    lines: &[Line],
    chosen: &HashMap<String, String>,
) -> HashMap<String, String> {
    let known = |v: &str| {
        engine
            .voices()
            .iter()
            .find(|k| k.eq_ignore_ascii_case(v))
            .map(|k| k.to_string())
    };
    let mut out: HashMap<String, String> = HashMap::new();
    let mut used: Vec<String> = Vec::new();
    for l in lines {
        if out.contains_key(&l.speaker) {
            continue;
        }
        if let Some(v) = chosen.get(&l.speaker).and_then(|v| known(v)) {
            used.push(v.clone());
            out.insert(l.speaker.clone(), v);
        }
    }
    for l in lines {
        if out.contains_key(&l.speaker) {
            continue;
        }
        let v = engine
            .voices()
            .iter()
            .find(|v| !used.iter().any(|u| u == *v))
            .unwrap_or(&engine.voices()[0])
            .to_string();
        used.push(v.clone());
        out.insert(l.speaker.clone(), v);
    }
    out
}

/// Synthesise one line; returns 48 kHz mono s16 samples.
fn speak(
    engine: Engine,
    base: &str,
    text: &str,
    voice: &str,
    lang: &str,
    speed: f32,
) -> Result<Vec<i16>> {
    let t = Duration::from_secs(180);
    match engine {
        Engine::Supertonic => {
            let body = serde_json::json!({ "text": text, "voice": voice, "lang": lang, "speed": speed.clamp(0.7, 2.0) });
            let wav = http::post(
                &format!("{base}/v1/tts"),
                "application/json",
                body.to_string().as_bytes(),
                t,
            )?;
            decode(&wav, &[])
        }
        Engine::Qwen => {
            let body = serde_json::json!({
                "input": text,
                "voice": voice,
                "language": qwen_language(lang),
                "response_format": "pcm",
                // The sidecar renders up to this many 1/12.5 s frames; text
                // longer than its budget is cut, so give it room.
                "max_new_tokens": (text.chars().count() * 5).max(50),
                "repetition_penalty": 1.1,
            });
            let pcm = http::post(
                &format!("{base}/v1/audio/speech"),
                "application/json",
                body.to_string().as_bytes(),
                t,
            )?;
            decode(&pcm, &["-f", "s16le", "-ar", "24000", "-ac", "1"])
        }
    }
}

fn qwen_language(lang: &str) -> String {
    match lang.trim().to_lowercase().as_str() {
        "en" => "English",
        "zh" | "cmn" => "Chinese",
        "ja" => "Japanese",
        "ko" => "Korean",
        "de" => "German",
        "fr" => "French",
        "ru" => "Russian",
        "pt" => "Portuguese",
        "es" => "Spanish",
        "it" => "Italian",
        other => return other.to_string(),
    }
    .to_string()
}

/// Any audio bytes to 48 kHz mono s16 through ffmpeg (`input` describes
/// raw input; empty for a file format ffmpeg can probe).
fn decode(bytes: &[u8], input: &[&str]) -> Result<Vec<i16>> {
    let mut child = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error"])
        .args(input)
        .args([
            "-i",
            "pipe:0",
            "-ac",
            "1",
            "-ar",
            &SR.to_string(),
            "-f",
            "s16le",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let data = bytes.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&data);
    });
    let mut raw = Vec::new();
    child
        .stdout
        .take()
        .ok_or("no stdout")?
        .read_to_end(&mut raw)?;
    let _ = writer.join();
    let _ = child.wait();
    if raw.len() < 2 {
        return Err("the voice came back empty".into());
    }
    Ok(raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b))
        .collect())
}

/// Lay the lines end to end: (samples, timings). `speak` is injected so the
/// layout is testable without a sidecar.
pub fn assemble(
    lines: &[Line],
    voices: &HashMap<String, String>,
    mut speak: impl FnMut(&Line, &str) -> Result<Vec<i16>>,
) -> Result<(Vec<i16>, Vec<Timed>)> {
    let mut out: Vec<i16> = Vec::new();
    let mut timed = Vec::with_capacity(lines.len());
    for l in lines {
        let voice = voices.get(&l.speaker).cloned().unwrap_or_default();
        let mut s = speak(l, &voice)?;
        trim_silence(&mut s);
        let at = out.len() as f64 / SR as f64;
        let dur = s.len() as f64 / SR as f64;
        out.extend_from_slice(&s);
        out.extend(std::iter::repeat_n(
            0i16,
            (l.pause.clamp(0.0, 10.0) * SR as f64) as usize,
        ));
        timed.push(Timed {
            speaker: l.speaker.clone(),
            voice,
            text: l.text.clone(),
            at,
            dur,
        });
    }
    Ok((out, timed))
}

/// Drop leading and trailing near-silence (the engines pad both ends), so
/// pauses are the ones asked for.
pub fn trim_silence(s: &mut Vec<i16>) {
    const FLOOR: i16 = 300; // about -40 dBFS
    let pad = (SR / 50) as usize; // keep 20 ms around the speech
    let first = s.iter().position(|v| v.unsigned_abs() > FLOOR as u16);
    let last = s.iter().rposition(|v| v.unsigned_abs() > FLOOR as u16);
    if let (Some(a), Some(b)) = (first, last) {
        let a = a.saturating_sub(pad);
        let b = (b + pad + 1).min(s.len());
        s.truncate(b);
        s.drain(..a);
    }
}

pub fn write_wav(path: &Path, samples: &[i16]) -> Result<()> {
    let data = (samples.len() * 2) as u32;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?; // PCM
    f.write_all(&1u16.to_le_bytes())?; // mono
    f.write_all(&SR.to_le_bytes())?;
    f.write_all(&(SR * 2).to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data.to_le_bytes())?;
    for v in samples {
        f.write_all(&v.to_le_bytes())?;
    }
    f.flush()?;
    Ok(())
}

pub struct Request {
    pub engine: Engine,
    pub base_url: String,
    pub lang: String,
    pub speed: f32,
    pub lines: Vec<Line>,
    pub voices: HashMap<String, String>,
}

/// Synthesise everything into `<dir>/<stem>.wav`.
pub fn render(req: &Request, dir: &Path, stem: &str) -> Result<(PathBuf, Vec<Timed>)> {
    if req.lines.is_empty() {
        return Err("no lines to say".into());
    }
    if req.lines.len() > MAX_LINES {
        return Err(format!("at most {MAX_LINES} lines at a time").into());
    }
    if let Some(l) = req
        .lines
        .iter()
        .find(|l| l.text.chars().count() > MAX_CHARS)
    {
        return Err(format!(
            "a line is longer than {MAX_CHARS} characters; split it: {:.40}…",
            l.text
        )
        .into());
    }
    let voices = cast(req.engine, &req.lines, &req.voices);
    let (samples, timed) = assemble(&req.lines, &voices, |l, v| {
        speak(req.engine, &req.base_url, &l.text, v, &req.lang, req.speed)
    })?;
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{stem}.wav"));
    write_wav(&path, &samples)?;
    Ok((path, timed))
}

/// Which engines answer, and their voices.
pub fn engines(supertonic: &str) -> serde_json::Value {
    let up = |url: String| http::get(&url, Duration::from_secs(2)).is_ok();
    serde_json::json!({
        "supertonic": { "available": up(format!("{}/v1/health", supertonic_url(supertonic))), "voices": SUPERTONIC_VOICES,
            "note": "fast; M = male, F = female styles" },
        "qwen": { "available": up(format!("{}/v1/audio/voices", qwen_url())), "voices": QWEN_VOICES,
            "note": "slower, more expressive; needs the Qwen3-TTS voice installed in Settings" },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(s: &str, t: &str, pause: f64) -> Line {
        Line {
            speaker: s.into(),
            text: t.into(),
            pause,
        }
    }

    #[test]
    fn every_speaker_gets_a_distinct_voice() {
        let lines = [
            line("Anna", "a", 0.0),
            line("Luca", "b", 0.0),
            line("Anna", "c", 0.0),
            line("Sara", "d", 0.0),
        ];
        let mut chosen = HashMap::new();
        chosen.insert("Luca".to_string(), "f2".to_string());
        let v = cast(Engine::Supertonic, &lines, &chosen);
        assert_eq!(
            v["Luca"], "F2",
            "a chosen voice is kept, case-insensitively"
        );
        assert_eq!(v["Anna"], "M1");
        assert_eq!(v["Sara"], "F1");
    }

    #[test]
    fn lines_are_laid_end_to_end_with_their_pauses() {
        let lines = [line("A", "uno", 0.5), line("B", "due", 0.0)];
        let voices = cast(Engine::Supertonic, &lines, &HashMap::new());
        // Each fake line: 0.1 s of silence, 1 s of tone, 0.1 s of silence.
        let fake = |_: &Line, _: &str| -> Result<Vec<i16>> {
            let mut s = vec![0i16; 4800];
            s.extend(std::iter::repeat_n(8000i16, 48_000));
            s.extend(vec![0i16; 4800]);
            Ok(s)
        };
        let (samples, timed) = assemble(&lines, &voices, fake).unwrap();
        assert_eq!(timed[0].at, 0.0);
        assert!(
            (timed[0].dur - 1.04).abs() < 0.01,
            "padding trimmed to 20 ms each side"
        );
        assert!((timed[1].at - (timed[0].dur + 0.5)).abs() < 0.001);
        assert!((samples.len() as f64 / SR as f64 - (timed[1].at + timed[1].dur)).abs() < 0.001);
        assert_eq!(timed[1].voice, "F1");
    }

    #[test]
    fn engine_names() {
        assert_eq!(Engine::parse("Qwen3-TTS"), Some(Engine::Qwen));
        assert_eq!(Engine::parse(""), Some(Engine::Supertonic));
        assert_eq!(Engine::parse("bark"), None);
    }
}
