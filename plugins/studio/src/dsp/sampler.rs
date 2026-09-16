//! SoundFont (`.sf2`) sampler backed by [`rustysynth`].
//!
//! Real sampled instruments — pianos, strings, brass, drums — live outside the
//! codebase: the user drops `.sf2` banks into the plugin's `soundfonts/`
//! directory and the sampler renders the selected preset offline, in the same
//! block buffers as every other instrument. There is deliberately **no bundled
//! bank**: banks are large and their licences vary.
//!
//! Melodic voices map a pattern's resolved frequency onto a MIDI key on
//! channel 0 and select a preset with bank/program. `sfkit` voices play a
//! percussion kit from channel 9, with pad index → GM drum key (`36 + pad`).
//!
//! Loading a bank is expensive, so parsed [`SoundFont`]s are cached by path and
//! shared (`Arc`) across voices, clips and the parallel clip renderer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use serde_json::{json, Value};

use super::synth::{NoteKind, NoteMsg};

static DIR: OnceLock<PathBuf> = OnceLock::new();
static CACHE: OnceLock<Mutex<HashMap<String, Arc<SoundFont>>>> = OnceLock::new();

/// The directory the plugin looks for `.sf2` banks in. Set once at registration
/// from the host's plugins dir (`<plugins_dir>/studio/soundfonts`).
pub fn set_dir(dir: PathBuf) {
    // Create it up front so the (empty) `soundfonts/` folder is discoverable
    // and users have an obvious place to drop a bank.
    let _ = std::fs::create_dir_all(&dir);
    let _ = DIR.set(dir);
}

/// The soundfonts directory, if the plugin has registered it yet.
pub fn dir() -> Option<&'static Path> {
    DIR.get().map(|p| p.as_path())
}

fn cache() -> &'static Mutex<HashMap<String, Arc<SoundFont>>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Every `.sf2` file in the soundfonts directory, sorted by path.
pub fn files() -> Vec<PathBuf> {
    let Some(d) = dir() else { return Vec::new() };
    let mut out: Vec<PathBuf> = match std::fs::read_dir(d) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .map(|e| e.eq_ignore_ascii_case("sf2"))
                    .unwrap_or(false)
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    out.sort();
    out
}

/// A catalog-friendly description: each bank with its presets and drum kits.
pub fn list() -> Vec<Value> {
    files()
        .into_iter()
        .map(|p| {
            let name = p
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            let presets = match load_file(&p) {
                Ok(sf) => sf
                    .get_presets()
                    .iter()
                    .map(|pr| {
                        json!({
                            "name": pr.get_name(),
                            "bank": pr.get_bank_number(),
                            "program": pr.get_patch_number(),
                        })
                    })
                    .collect::<Vec<_>>(),
                Err(_) => Vec::new(),
            };
            json!({ "file": name, "presets": presets })
        })
        .collect()
}

fn resolve(name: Option<&str>) -> Result<PathBuf, String> {
    let found = files();
    if found.is_empty() {
        let where_ = dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "the plugin's soundfonts/ directory".into());
        return Err(format!(
            "no SoundFonts found — put a .sf2 bank in {where_}"
        ));
    }
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => {
            let want = Path::new(n).file_name();
            found
                .iter()
                .find(|p| {
                    p.file_name() == want
                        || p.to_str() == Some(n)
                        || p.file_name().and_then(|f| f.to_str()) == Some(n)
                })
                .cloned()
                .ok_or_else(|| {
                    let avail = found
                        .iter()
                        .filter_map(|p| p.file_name().and_then(|f| f.to_str()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("SoundFont `{n}` not found (available: {avail})")
                })
        }
        None => Ok(found[0].clone()),
    }
}

fn load_file(path: &Path) -> Result<Arc<SoundFont>, String> {
    let key = path.to_string_lossy().to_string();
    if let Some(sf) = cache().lock().unwrap().get(&key).cloned() {
        return Ok(sf);
    }
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let sf = SoundFont::new(&mut file).map_err(|e| format!("parse {}: {e}", path.display()))?;
    let sf = Arc::new(sf);
    cache().lock().unwrap().insert(key, sf.clone());
    Ok(sf)
}

/// Load (or fetch from cache) the named bank; `None` picks the first found.
pub fn load(name: Option<&str>) -> Result<Arc<SoundFont>, String> {
    let path = resolve(name)?;
    load_file(&path)
}

fn freq_to_key(freq: f64) -> i32 {
    if freq <= 0.0 {
        return 60;
    }
    (69.0 + 12.0 * (freq / 440.0).log2()).round().clamp(0.0, 127.0) as i32
}

/// One SoundFont-backed instrument (melodic sampler or percussion kit).
pub struct Sampler {
    synth: Synthesizer,
    drum: bool,
    /// Held melodic notes (id → MIDI key), so note-off can find the key.
    held: Vec<(u64, i32)>,
    /// Frames of tail left once nothing is held (so a channel keeps rendering).
    tail: i64,
    max_tail: i64,
}

impl Sampler {
    /// Build a sampler for a bank. `drum` selects channel 9 (percussion);
    /// otherwise `bank`/`program` select a melodic preset on channel 0.
    pub fn new(
        sf: Arc<SoundFont>,
        drum: bool,
        program: i32,
        bank: i32,
        sr: f64,
    ) -> Result<Self, String> {
        let mut settings = SynthesizerSettings::new(sr.round().clamp(16_000.0, 192_000.0) as i32);
        // Small internal blocks so a note event lands within a few samples of
        // its step rather than at the next 64-frame engine block.
        settings.block_size = 8;
        settings.maximum_polyphony = 64;
        // Our own insert/master reverb is the only reverb; keep the bank dry.
        settings.enable_reverb_and_chorus = false;
        let mut synth = Synthesizer::new(&sf, &settings).map_err(|e| e.to_string())?;
        synth.set_master_volume(0.8);
        let channel = if drum { 9 } else { 0 };
        if drum {
            synth.process_midi_message(channel, 0xC0, program.clamp(0, 127), 0);
        } else {
            synth.process_midi_message(channel, 0xB0, 0x00, bank.clamp(0, 127));
            synth.process_midi_message(channel, 0xC0, program.clamp(0, 127), 0);
        }
        let max_tail = (sr * if drum { 3.0 } else { 2.0 }) as i64;
        Ok(Self {
            synth,
            drum,
            held: Vec::new(),
            tail: max_tail,
            max_tail,
        })
    }

    fn key_for(&self, pad: usize, freq: f64) -> i32 {
        if self.drum {
            (36 + pad as i32).clamp(0, 127)
        } else {
            freq_to_key(freq)
        }
    }

    pub fn is_active(&self) -> bool {
        !self.held.is_empty() || self.tail > 0
    }

    pub fn reset(&mut self) {
        self.synth.reset();
        self.held.clear();
        self.tail = self.max_tail;
    }

    /// Render one block, applying note messages at their offsets.
    pub fn render(&mut self, l: &mut [f32], r: &mut [f32], msgs: &[NoteMsg], frames: usize) {
        let channel = if self.drum { 9 } else { 0 };
        let mut mi = 0usize;
        let mut i = 0usize;
        while i < frames {
            while mi < msgs.len() && msgs[mi].at <= i {
                let m = msgs[mi];
                match m.kind {
                    NoteKind::On { freq, velocity } => {
                        let key = self.key_for(m.pad, freq);
                        let vel = (velocity.clamp(0.0, 1.0) * 127.0).round().clamp(1.0, 127.0) as i32;
                        self.synth.note_on(channel, key, vel);
                        if self.drum {
                            self.tail = self.max_tail;
                        } else {
                            self.held.retain(|(id, _)| *id != m.id);
                            self.held.push((m.id, key));
                        }
                    }
                    NoteKind::Off => {
                        if !self.drum {
                            if let Some(pos) = self.held.iter().position(|(id, _)| *id == m.id) {
                                let (_, key) = self.held.remove(pos);
                                self.synth.note_off(channel, key);
                            }
                        }
                    }
                }
                mi += 1;
            }
            let next = msgs.get(mi).map(|m| m.at).unwrap_or(frames).min(frames);
            let n = (next.saturating_sub(i)).clamp(1, frames - i);
            self.synth.render(&mut l[i..i + n], &mut r[i..i + n]);
            i += n;
        }
        if self.drum || self.held.is_empty() {
            self.tail -= frames as i64;
        } else {
            self.tail = self.max_tail;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freq_maps_to_midi_keys() {
        assert_eq!(freq_to_key(440.0), 69);
        assert_eq!(freq_to_key(880.0), 81);
        assert_eq!(freq_to_key(220.0), 57);
        assert_eq!(freq_to_key(0.0), 60);
    }

    #[test]
    fn missing_bank_is_an_actionable_error() {
        // Tests never register a soundfonts dir, so there is nothing to load.
        let err = load(None).unwrap_err();
        assert!(
            err.contains("no SoundFonts") || err.contains("not found"),
            "unhelpful error: {err}"
        );
    }

    /// Smoke test against a real bank, opt-in via `STUDIO_TEST_SF2=<path>`.
    #[test]
    fn renders_a_supplied_bank() {
        let Ok(path) = std::env::var("STUDIO_TEST_SF2") else {
            return;
        };
        let sf = load_file(Path::new(&path)).expect("load SoundFont");
        let mut s = Sampler::new(sf, false, 0, 0, 44_100.0).expect("build sampler");
        let msgs = [NoteMsg {
            at: 0,
            id: 1,
            pad: 0,
            kind: NoteKind::On { freq: 440.0, velocity: 1.0 },
        }];
        let mut l = vec![0.0f32; 2048];
        let mut r = vec![0.0f32; 2048];
        s.render(&mut l, &mut r, &msgs, 2048);
        let peak = l
            .iter()
            .chain(r.iter())
            .fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak.is_finite(), "sampler produced non-finite output");
        assert!(peak > 0.001, "sampler rendered silence (peak {peak})");
    }
}
