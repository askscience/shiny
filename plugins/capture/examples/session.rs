//! Drive a real session from the command line (tests under Xvfb).
//!
//! `session '<plan json>' SECONDS [pause_at resume_at]`

use shiny_capture_plugin::session::{self, Plan};
use std::time::Duration;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let plan: Plan = serde_json::from_str(&a[1]).expect("plan json");
    let secs: f64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let pause: Option<(f64, f64)> = a
        .get(3)
        .zip(a.get(4))
        .and_then(|(p, r)| Some((p.parse().ok()?, r.parse().ok()?)));
    let (tx, rx) = std::sync::mpsc::channel();
    let sh = session::start(
        plan,
        Box::new(move |st, _| {
            let _ = tx.send(st.clone());
        }),
    )
    .expect("start");
    let t0 = std::time::Instant::now();
    let mut paused = false;
    while t0.elapsed().as_secs_f64() < secs {
        std::thread::sleep(Duration::from_millis(250));
        let el = t0.elapsed().as_secs_f64();
        if let Some((p, r)) = pause {
            if !paused && el >= p && el < r {
                sh.ctl.lock().unwrap().paused = true;
                paused = true;
            }
            if paused && el >= r {
                sh.ctl.lock().unwrap().paused = false;
                paused = false;
            }
        }
        let st = sh.status.lock().unwrap().clone();
        eprintln!(
            "{:?} t={:.1}s frames={} cpu={:.1}% enc={:.1}% kbps={:.0} err={:?}",
            st.state,
            st.elapsed_us as f64 / 1e6,
            st.frames,
            st.cpu,
            st.encoder_cpu,
            st.health.kbps,
            st.error
        );
        if !st.health.errors.is_empty() {
            eprintln!("  ffmpeg: {:?}", st.health.errors);
        }
        if matches!(st.state, session::State::Failed | session::State::Done) {
            break;
        }
    }
    session::stop();
    let st = rx.recv_timeout(Duration::from_secs(60)).expect("done");
    println!("{}", serde_json::to_string_pretty(&st).unwrap());
}
