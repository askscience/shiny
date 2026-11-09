//! Audio level meters for the Capture window.
//!
//! One tiny `parec` per watched source at 8 kHz mono (a meter needs no
//! more), read in 40 ms blocks into peak and RMS. A meter nobody polled for
//! a few seconds stops, so an open window costs nothing once it is hidden.

use std::collections::HashMap;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RATE: u32 = 8000;
const BLOCK: usize = (RATE as usize / 25) * 2; // 40 ms of s16 mono
const IDLE: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct Level {
    /// Peak and RMS of the last block, dBFS (-90 for silence).
    pub peak_db: f32,
    pub rms_db: f32,
}

struct Meter {
    level: Arc<Mutex<Level>>,
    polled: Arc<Mutex<Instant>>,
    child: Arc<Mutex<Option<Child>>>,
}

static METERS: Mutex<Option<HashMap<String, Meter>>> = Mutex::new(None);

fn db(x: f32) -> f32 {
    if x <= 1e-5 {
        -90.0
    } else {
        20.0 * x.log10()
    }
}

/// Peak and RMS of s16 little-endian samples, linear 0..1.
pub fn measure(bytes: &[u8]) -> (f32, f32) {
    let (mut peak, mut sum, mut n) = (0i32, 0f64, 0usize);
    for &s in bytes.as_chunks::<2>().0 {
        let v = i16::from_le_bytes(s) as i32;
        peak = peak.max(v.abs());
        sum += (v as f64) * (v as f64);
        n += 1;
    }
    let rms = if n > 0 { (sum / n as f64).sqrt() } else { 0.0 };
    (peak as f32 / 32768.0, rms as f32 / 32768.0)
}

fn start(source: &str) -> Option<Meter> {
    let mut child = Command::new("parec")
        .args([
            "--raw",
            "--format=s16le",
            "--channels=1",
            &format!("--rate={RATE}"),
            "--latency-msec=40",
            "--client-name=LiteAura Capture meter",
            "-d",
            source,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let level = Arc::new(Mutex::new(Level {
        peak_db: -90.0,
        rms_db: -90.0,
    }));
    let polled = Arc::new(Mutex::new(Instant::now()));
    let child = Arc::new(Mutex::new(Some(child)));
    let (l2, p2, c2) = (level.clone(), polled.clone(), child.clone());
    let name = source.to_string();
    std::thread::Builder::new()
        .name("capture-meter".into())
        .spawn(move || {
            let mut buf = vec![0u8; BLOCK];
            while out.read_exact(&mut buf).is_ok() {
                let (pk, rms) = measure(&buf);
                *l2.lock().unwrap() = Level {
                    peak_db: db(pk),
                    rms_db: db(rms),
                };
                if p2.lock().unwrap().elapsed() > IDLE {
                    break;
                }
            }
            if let Some(mut c) = c2.lock().unwrap().take() {
                let _ = c.kill();
                let _ = c.wait();
            }
            if let Ok(mut m) = METERS.lock() {
                if let Some(map) = m.as_mut() {
                    if map.get(&name).is_some_and(|x| Arc::ptr_eq(&x.level, &l2)) {
                        map.remove(&name);
                    }
                }
            }
        })
        .ok()?;
    Some(Meter {
        level,
        polled,
        child,
    })
}

/// The current level of `source`, starting its meter on first use.
pub fn level(source: &str) -> Level {
    let mut m = match METERS.lock() {
        Ok(m) => m,
        Err(_) => return Level::default(),
    };
    let map = m.get_or_insert_with(HashMap::new);
    if !map.contains_key(source) {
        match start(source) {
            Some(meter) => {
                map.insert(source.to_string(), meter);
            }
            None => return Level::default(),
        }
    }
    let meter = &map[source];
    *meter.polled.lock().unwrap() = Instant::now();
    let level = *meter.level.lock().unwrap();
    level
}

/// Stop every meter (plugin unload).
pub fn stop_all() {
    if let Ok(mut m) = METERS.lock() {
        if let Some(map) = m.take() {
            for (_, meter) in map {
                if let Some(mut c) = meter.child.lock().unwrap().take() {
                    let _ = c.kill();
                    let _ = c.wait();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_scale_square_is_zero_db() {
        let b: Vec<u8> = [32767i16, -32767]
            .repeat(10)
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let (pk, rms) = measure(&b);
        assert!((db(pk)).abs() < 0.01);
        assert!((db(rms)).abs() < 0.01);
        assert_eq!(db(0.0), -90.0);
    }
}
