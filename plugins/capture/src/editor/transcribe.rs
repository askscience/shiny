//! Word-timed transcripts of media, from the core's Whisper sidecar.
//!
//! ffmpeg decodes the voice track to 16 kHz mono (what Whisper eats) and
//! the audio goes in 30 s blocks with a little overlap, so a word cut by a
//! block edge is heard whole in the next block; times are shifted back to
//! media time and the overlap is de-duplicated. The result is cached per
//! file like the waveform.

use super::summary::Words;
use crate::Result;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

const SR: usize = 16_000;
const BLOCK_S: usize = 30;
const OVERLAP_S: usize = 1;

fn whisper_url() -> String {
    std::env::var("WHISPER_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:7789".into())
}

fn cache_file(p: &Path) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let md = std::fs::metadata(p).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.hash(&mut h);
    md.len().hash(&mut h);
    md.modified().ok()?.hash(&mut h);
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(
        home.join(".cache/liteaura/capture")
            .join(format!("{:016x}.words", h.finish())),
    )
}

pub fn cached(p: &Path) -> Option<Words> {
    serde_json::from_slice(&std::fs::read(cache_file(p)?).ok()?).ok()
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct State {
    pub state: String, // running | done | failed
    pub progress: f32,
    pub error: Option<String>,
}

static JOBS: Mutex<Option<HashMap<PathBuf, Arc<Mutex<State>>>>> = Mutex::new(None);

pub fn state(p: &Path) -> Option<State> {
    if cached(p).is_some() {
        return Some(State {
            state: "done".into(),
            progress: 1.0,
            error: None,
        });
    }
    JOBS.lock()
        .ok()?
        .as_ref()?
        .get(p)
        .map(|s| s.lock().unwrap().clone())
}

/// Start transcribing `track` of `p` in the background (once).
pub fn start(p: PathBuf, track: u32, lang: Option<String>) {
    if cached(&p).is_some() {
        return;
    }
    let mut jobs = JOBS.lock().unwrap();
    let map = jobs.get_or_insert_with(HashMap::new);
    if map
        .get(&p)
        .is_some_and(|s| s.lock().unwrap().state == "running")
    {
        return;
    }
    let st = Arc::new(Mutex::new(State {
        state: "running".into(),
        ..State::default()
    }));
    map.insert(p.clone(), st.clone());
    drop(jobs);
    std::thread::Builder::new()
        .name("capture-transcribe".into())
        .spawn(move || {
            let r = run(&p, track, lang.as_deref(), &st);
            let mut s = st.lock().unwrap();
            match r {
                Ok(()) => {
                    s.state = "done".into();
                    s.progress = 1.0;
                }
                Err(e) => {
                    s.state = "failed".into();
                    s.error = Some(e.to_string());
                }
            }
        })
        .ok();
}

fn post(block: &[u8], lang: Option<&str>) -> Result<Vec<(String, f64, f64)>> {
    let mut url = format!("{}/stt/transcribe", whisper_url());
    if let Some(l) = lang {
        url.push_str(&format!("?lang={l}"));
    }
    // A blocking request on this worker thread: no runtime needed.
    let resp = ureq_post(&url, block)?;
    let v: serde_json::Value = serde_json::from_str(&resp)?;
    Ok(v["words"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|w| {
                    Some((
                        w.get(0)?.as_str()?.to_string(),
                        w.get(1)?.as_f64()?,
                        w.get(2)?.as_f64()?,
                    ))
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Minimal HTTP/1.1 POST over a loopback TCP socket (the sidecar is local),
/// so this module needs no HTTP client crate.
fn ureq_post(url: &str, body: &[u8]) -> Result<String> {
    use std::io::Write;
    let rest = url
        .strip_prefix("http://")
        .ok_or("the Whisper URL must be http://")?;
    let (hostport, path) = rest
        .split_once('/')
        .map(|(h, p)| (h, format!("/{p}")))
        .unwrap_or((rest, "/".into()));
    let mut s = std::net::TcpStream::connect(hostport)?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(600)))?;
    write!(
        s,
        "POST {path} HTTP/1.1\r\nHost: {hostport}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    s.write_all(body)?;
    let mut resp = Vec::new();
    s.read_to_end(&mut resp)?;
    let text = String::from_utf8_lossy(&resp).to_string();
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or("bad reply from Whisper")?;
    if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
        return Err(format!("Whisper answered: {}", head.lines().next().unwrap_or("")).into());
    }
    // The sidecar may answer chunked.
    if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        let mut out = String::new();
        let mut rest = body;
        while let Some((len, tail)) = rest.split_once("\r\n") {
            let n = usize::from_str_radix(len.trim(), 16).unwrap_or(0);
            if n == 0 || tail.len() < n {
                break;
            }
            out.push_str(&tail[..n]);
            rest = tail[n..].trim_start_matches("\r\n");
        }
        return Ok(out);
    }
    Ok(body.to_string())
}

fn run(p: &Path, track: u32, lang: Option<&str>, st: &Mutex<State>) -> Result<()> {
    let info = super::media::probe(p)?;
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
    let mut out = child.stdout.take().ok_or("no audio")?;
    let mut pcm = Vec::new();
    out.read_to_end(&mut pcm)?;
    let _ = child.wait();
    let total = pcm.len() / 2;
    let step = BLOCK_S * SR;
    let mut words: Vec<(String, f64, f64)> = Vec::new();
    let mut start = 0usize;
    while start < total {
        let end = (start + step + OVERLAP_S * SR).min(total);
        let off = start as f64 / SR as f64;
        for (w, a, b) in post(&pcm[start * 2..end * 2], lang)? {
            let (a, b) = (a + off, b + off);
            // The overlap: keep a word only once.
            if words.last().is_some_and(|l| a < l.2 - 0.05) {
                continue;
            }
            words.push((w, a, b));
        }
        start += step;
        st.lock().unwrap().progress = (start.min(total) as f64 / total.max(1) as f64) as f32;
        let _ = info.duration;
    }
    let w = Words {
        lang: lang.unwrap_or("").to_string(),
        words,
    };
    if let Some(f) = cache_file(p) {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        std::fs::write(f, serde_json::to_vec(&w)?)?;
    }
    Ok(())
}

/// Store a transcript given by the caller (tests, or one made elsewhere).
pub fn store(p: &Path, w: &Words) -> Result<()> {
    let f = cache_file(p).ok_or("no such file")?;
    std::fs::create_dir_all(f.parent().unwrap())?;
    std::fs::write(f, serde_json::to_vec(w)?)?;
    Ok(())
}
