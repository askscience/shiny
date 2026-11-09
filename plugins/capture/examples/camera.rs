//! Open the camera, take frames for a few seconds, write one bubble JPEG.
use shiny_capture_plugin::{
    bubble::{Bubble, Style},
    camera, jpeg,
    nv12::Nv12,
};
use std::time::{Duration, Instant};

fn main() {
    for d in shiny_capture_plugin::v4l2::devices() {
        println!(
            "{} {:?} nv12={} modes={:?}",
            d.path.display(),
            d.name,
            d.nv12,
            d.modes
        );
    }
    let l = camera::acquire(None, (640, 480)).expect("camera");
    let t0 = Instant::now();
    let mut seen = 0;
    let mut frames = 0;
    let mut b = Bubble::new(Style {
        w: 320,
        h: 320,
        border: 6,
        border_color: "#ff8800".into(),
        ..Style::default()
    });
    let mut upd = Duration::ZERO;
    while t0.elapsed() < Duration::from_secs(4) {
        camera::touch();
        let s = l.seq.load(std::sync::atomic::Ordering::Acquire);
        if s != seen {
            seen = s;
            frames += 1;
            let f = l.frame.lock().unwrap();
            let t = Instant::now();
            b.update(&f);
            upd += t.elapsed();
        }
        if let Some(e) = l.error.lock().unwrap().clone() {
            panic!("{e}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let mode = *l.mode.lock().unwrap();
    println!(
        "mode {:?}: {} frames in 4 s, bubble update {:.0} µs/frame",
        mode,
        frames,
        upd.as_micros() as f64 / frames.max(1) as f64
    );
    let mut canvas = Nv12::new(480, 480);
    canvas.data.fill(128);
    let mut patch = shiny_capture_plugin::cursor::Patch::default();
    b.draw(&mut canvas, 80, 80, &mut patch);
    std::fs::write(
        std::env::args().nth(1).unwrap_or("/tmp/bubble.jpg".into()),
        jpeg::encode(&canvas, 85).unwrap(),
    )
    .unwrap();
    camera::release();
}
