//! Phase-0 spike: measure the capture engine.
//!
//! ```sh
//! cargo run --release -p shiny-capture-plugin --example spike -- bench
//! cargo run --release -p shiny-capture-plugin --example spike -- encode [--preset veryfast] [--vaapi]
//! cargo run --release -p shiny-capture-plugin --example spike -- record \
//!     [--seconds 10] [--fps 30] [--region x,y,w,h] [--scale 1|2] [--threads n] \
//!     [--encoder auto|x264|vaapi] [--preset veryfast] [--crf 23] [--out file.mkv] \
//!     [--no-damage] [--no-cursor] [--animate WxH]
//! ```
//!
//! `record` needs `$DISPLAY`; run it under Xvfb for automated checks.

use shiny_capture_plugin::clock::Ticker;
use shiny_capture_plugin::cursor::{self, CursorImage, Patch};
use shiny_capture_plugin::encoder::{self, Backend, Encoder, Purpose, Rate, Settings};
use shiny_capture_plugin::mask::{self, Mask};
use shiny_capture_plugin::nv12::{bgrx_into, Convert, Nv12};
use shiny_capture_plugin::scale::Scaler;
use shiny_capture_plugin::stats::{cpu_time, peak_rss_kib, Samples};
use shiny_capture_plugin::x11::{Changes, Screen};
use shiny_capture_plugin::{Rect, Result};
use std::path::PathBuf;
use std::time::{Duration, Instant};

struct Args(Vec<String>);

impl Args {
    fn flag(&self, k: &str) -> bool {
        self.0.iter().any(|a| a == k)
    }
    fn get(&self, k: &str) -> Option<&str> {
        self.0
            .iter()
            .position(|a| a == k)
            .and_then(|i| self.0.get(i + 1))
            .map(String::as_str)
    }
    fn num<T: std::str::FromStr>(&self, k: &str, d: T) -> T {
        self.get(k).and_then(|v| v.parse().ok()).unwrap_or(d)
    }
}

fn main() -> Result<()> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let a = Args(argv.clone());
    match argv.first().map(String::as_str) {
        Some("bench") => bench(),
        Some("encode") => encode(&a),
        Some("record") => record(&a),
        Some("animate") => animate(&a),
        _ => Err("usage: spike bench | encode | record [options]".into()),
    }
}

fn ffmpeg() -> PathBuf {
    std::env::var_os("FFMPEG")
        .map(PathBuf::from)
        .unwrap_or_else(|| "ffmpeg".into())
}

/// Mean time of `f` over `n` runs (after one warm-up), in microseconds.
fn time<F: FnMut()>(n: u32, mut f: F) -> f64 {
    f();
    let t = Instant::now();
    for _ in 0..n {
        f();
    }
    t.elapsed().as_secs_f64() * 1e6 / n as f64
}

fn noise(n: usize) -> Vec<u8> {
    let mut x = 0x9e37_79b9u32;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

fn bench() -> Result<()> {
    let (w, h) = (1920, 1080);
    let src = noise(w * h * 4);
    let src4k = noise(w * h * 16);
    let mut f = Nv12::new(w, h);
    let full = Convert {
        dx: 0,
        dy: 0,
        w,
        h,
        half: false,
        threads: 1,
        simd: false,
    };
    let mut dst = vec![0u8; src.len()];
    let copy = time(100, || {
        dst.copy_from_slice(std::hint::black_box(&src));
        std::hint::black_box(&dst);
    });
    println!("BGRX → NV12, 1920×1080 (one frame); memcpy of the 8 MB input = {copy:.0} µs");
    let scalar = time(100, || bgrx_into(&mut f, &src, w * 4, full));
    println!("  scalar, 1 thread     {scalar:>8.0} µs");
    let avx = time(100, || {
        bgrx_into(&mut f, &src, w * 4, Convert { simd: true, ..full })
    });
    println!(
        "  avx2,   1 thread     {avx:>8.0} µs   ({:.1}× scalar)",
        scalar / avx
    );
    for t in [2, 4] {
        let v = time(100, || {
            bgrx_into(
                &mut f,
                &src,
                w * 4,
                Convert {
                    simd: true,
                    threads: t,
                    ..full
                },
            )
        });
        println!("  avx2,   {t} threads    {v:>8.0} µs");
    }
    let half = time(50, || {
        bgrx_into(
            &mut f,
            &src4k,
            w * 8,
            Convert {
                simd: true,
                half: true,
                ..full
            },
        )
    });
    println!("  3840×2160 → 1080p (2× box + convert) {half:>6.0} µs");
    let small = time(1000, || {
        bgrx_into(
            &mut f,
            &src[..400 * 120 * 4],
            400 * 4,
            Convert {
                dx: 100,
                dy: 100,
                w: 400,
                h: 120,
                half: false,
                threads: 1,
                simd: true,
            },
        )
    });
    println!("  400×120 damaged area {small:>8.1} µs");

    println!("\nWebcam bubble");
    let build = time(50, || {
        std::hint::black_box(Mask::rounded(320, 320, 1.0));
    });
    let m = Mask::rounded(320, 320, 1.0);
    println!(
        "  mask 320×320 circle  {build:>8.0} µs (once per size change), edge pixels {:.1}%",
        m.luma.edge_ratio() * 100.0
    );
    let mut cam = Nv12::new(640, 480);
    cam.data = noise(cam.len());
    let mut bubble = Nv12::new(320, 320);
    let mut sc = Scaler::new(640, 480, 320, 320);
    let s = time(500, || sc.run(&cam, &mut bubble));
    println!("  scale 640×480 → 320×320 bilinear {s:>6.0} µs");
    let mut b2 = Nv12::new(320, 240);
    let mut sc2 = Scaler::new(640, 480, 320, 240);
    let s2 = time(500, || sc2.run(&cam, &mut b2));
    println!("  scale 640×480 → 320×240 (2× box) {s2:>6.0} µs");
    let bl = time(1000, || mask::blend(&mut f, &bubble, &m, 1500, 700));
    println!("  blend circle onto frame          {bl:>6.1} µs");
    let sq = Mask::rounded(320, 320, 0.0);
    let bq = time(1000, || mask::blend(&mut f, &bubble, &sq, 1500, 700));
    println!("  blend square onto frame          {bq:>6.1} µs");

    println!("\nCursor");
    let c = CursorImage::from_argb(1, 32, 32, 2, 2, &vec![0x80ff_ffffu32; 1024]);
    let mut p = Patch::default();
    let cu = time(5000, || {
        p.restore(&mut f);
        cursor::draw(&mut f, &c, 900, 500, 1, &mut p);
    });
    println!("  restore + draw 32×32             {cu:>6.1} µs");
    let frame_ms = 1e3 / 30.0;
    println!("\nBudget at 30 fps: {frame_ms:.1} ms per frame.");
    Ok(())
}

/// A desktop-like synthetic frame: a static layout plus a moving block.
fn synthetic(f: &mut Nv12, t: usize) {
    let w = f.w;
    let (y, uv) = f.planes_mut();
    for (i, v) in y.iter_mut().enumerate() {
        let (x, r) = (i % w, i / w);
        *v = if r % 40 < 2 || x % 300 < 2 { 60 } else { 200 };
    }
    uv.fill(128);
    let bx = (t * 13) % (w - 200);
    for r in 300..420 {
        y[r * w + bx..r * w + bx + 200].fill(((t * 7) % 200 + 16) as u8);
    }
}

fn encode(a: &Args) -> Result<()> {
    let (w, h, fps, n) = (1920, 1080, 30u32, 300usize);
    let backend = if a.flag("--vaapi") {
        let dev = encoder::probe_vaapi(&ffmpeg())
            .into_iter()
            .next()
            .ok_or("no working VA-API node (install a VA driver)")?;
        Backend::Vaapi { device: dev.path }
    } else {
        Backend::X264 {
            preset: a.get("--preset").unwrap_or("veryfast").into(),
        }
    };
    let s = Settings {
        width: w,
        height: h,
        fps,
        backend: backend.clone(),
        rate: rate(a),
        keyint_s: 2,
        threads: a.get("--threads").and_then(|v| v.parse().ok()),
        vfr: true,
        tune: tune(a),
        out: None,
        audio: Vec::new(),
        live: Vec::new(),
        ts_base_us: 0,
    };
    let mut f = Nv12::new(w, h);
    let cpu0 = cpu_time(true);
    let t = Instant::now();
    let mut enc = Encoder::spawn(&ffmpeg(), &s)?;
    for i in 0..n {
        synthetic(&mut f, i);
        enc.write(i as u64 * 1_000_000 / fps as u64, &f.data)?;
    }
    enc.finish()?;
    let wall = t.elapsed().as_secs_f64();
    let cpu = (cpu_time(true) - cpu0).as_secs_f64();
    println!("{}: {n} frames 1080p in {wall:.2}s = {:.0} fps, ffmpeg CPU {:.1} ms/frame = {:.0}% of one core at 30 fps", backend.label(), n as f64 / wall, cpu * 1e3 / n as f64, cpu / n as f64 * 30.0 * 100.0);
    Ok(())
}

fn rate(a: &Args) -> Rate {
    match a.get("--cbr").and_then(|v| v.parse().ok()) {
        Some(kbps) => Rate::Cbr { kbps },
        None => Rate::Quality(a.num("--crf", 20)),
    }
}

fn tune(a: &Args) -> Option<String> {
    match a.get("--tune").unwrap_or("zerolatency") {
        "none" => None,
        t => Some(t.into()),
    }
}

fn parse_region(s: &str) -> Result<Rect> {
    let v: Vec<i64> = s
        .split(',')
        .map(|p| p.trim().parse())
        .collect::<std::result::Result<_, _>>()?;
    match v[..] {
        [x, y, w, h] if w > 0 && h > 0 => Ok(Rect::new(x as i32, y as i32, w as u32, h as u32)),
        _ => Err("--region x,y,w,h".into()),
    }
}

fn proc_cpu(pid: u32) -> Duration {
    // Fields after the ")" that closes the command name: state is index 0,
    // utime index 11, stime index 12 (fields 14 and 15 of proc(5)).
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    let rest: Vec<&str> = s
        .rsplit_once(')')
        .map(|(_, r)| r)
        .unwrap_or("")
        .split_whitespace()
        .collect();
    let field = |i: usize| rest.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    // SAFETY: sysconf has no preconditions.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as u64;
    Duration::from_millis((field(11) + field(12)) * 1000 / hz)
}

fn record(a: &Args) -> Result<()> {
    let seconds: u32 = a.num("--seconds", 10);
    let fps: u32 = a.num("--fps", 30);
    let scale: u32 = a.num("--scale", 1);
    if scale != 1 && scale != 2 {
        return Err("--scale is 1 or 2".into());
    }
    let threads: usize = a.num("--threads", 1);
    let damage = !a.flag("--no-damage");
    let draw_cursor = !a.flag("--no-cursor");

    // Open once to learn the screen size, then size the segment for the region.
    // Kept open until the real connection exists: an X server with no
    // clients resets itself, and a connection made during that reset fails.
    let probe = Screen::open(None, false)?;
    let bounds = probe.bounds();
    let region = match a.get("--region") {
        Some(r) => parse_region(r)?.intersect(&bounds),
        None => bounds,
    };
    let step = 2 * scale;
    let region = Rect::new(
        region.x,
        region.y,
        region.w / step * step,
        region.h / step * step,
    );
    if region.is_empty() {
        return Err("empty capture region".into());
    }
    let mut screen = Screen::open(Some(region), damage)?;
    drop(probe);
    let (fw, fh) = ((region.w / scale) as usize, (region.h / scale) as usize);
    let local = Rect::new(0, 0, region.w, region.h);

    let ff = ffmpeg();
    let purpose = if a.flag("--cfr") || a.get("--cbr").is_some() {
        Purpose::Live
    } else {
        Purpose::Record
    };
    let backend = match a.get("--encoder").unwrap_or("auto") {
        "x264" => Backend::X264 {
            preset: a.get("--preset").unwrap_or("superfast").into(),
        },
        "vaapi" => Backend::Vaapi {
            device: encoder::probe_vaapi(&ff)
                .into_iter()
                .next()
                .ok_or("no working VA-API node")?
                .path,
        },
        _ => match encoder::auto(&ff, purpose) {
            Backend::X264 { .. } => Backend::X264 {
                preset: a.get("--preset").unwrap_or("superfast").into(),
            },
            b => b,
        },
    };
    let out = PathBuf::from(a.get("--out").unwrap_or("capture-spike.mkv"));
    let settings = Settings {
        width: fw,
        height: fh,
        fps,
        backend: backend.clone(),
        rate: rate(a),
        keyint_s: 2,
        threads: a.get("--enc-threads").and_then(|v| v.parse().ok()),
        vfr: !a.flag("--cfr"),
        tune: tune(a),
        out: Some(out.clone()),
        audio: Vec::new(),
        live: Vec::new(),
        ts_base_us: 0,
    };

    let animator = a.get("--animate").map(|wh| {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "animate",
                "--size",
                wh,
                "--fps",
                &fps.to_string(),
                "--seconds",
                &(seconds + 1).to_string(),
            ])
            .spawn()
    });

    println!(
        "capture {}×{} at ({},{}) → {fw}×{fh} @ {fps} fps, {} , damage={} ({}), threads={threads}",
        region.w,
        region.h,
        region.x,
        region.y,
        backend.label(),
        damage,
        if screen.tracks_damage() {
            "on"
        } else {
            "unavailable"
        }
    );

    let mut frame = Nv12::new(fw, fh);
    let conv = |r: Rect| Convert {
        dx: (r.x as u32 / scale) as usize,
        dy: (r.y as u32 / scale) as usize,
        w: (r.w / scale) as usize,
        h: (r.h / scale) as usize,
        half: scale == 2,
        threads,
        simd: true,
    };
    let px = screen.grab(region)?;
    bgrx_into(&mut frame, px, region.w as usize * 4, conv(local));

    let mut enc = Encoder::spawn(&ff, &settings)?;
    let (mut t_grab, mut t_conv, mut t_cur, mut t_write, mut t_frame) = (
        Samples::default(),
        Samples::default(),
        Samples::default(),
        Samples::default(),
        Samples::default(),
    );
    let (mut static_frames, mut grabbed_px) = (0u64, 0u64);
    let mut last_cursor = (i32::MIN, i32::MIN, 0u32);
    let mut cursor_img: Option<CursorImage> = None;
    let mut patch = Patch::default();
    let total = (seconds * fps) as u64;
    let (cpu0, wall0) = (cpu_time(false), Instant::now());
    let mut ticker = Ticker::new(fps);
    let (mut sent, mut ticks, mut late) = (0u64, 0u64, 0u64);
    enc.write(0, &frame.data)?;
    while ticks < total {
        let missed = ticker.wait();
        late += missed;
        ticks += 1 + missed;
        let t0 = Instant::now();
        let pts = (t0 - wall0).as_micros() as u64;
        enc.wait_drained();
        patch.restore(&mut frame);
        let rects: Vec<Rect> = match if damage {
            screen.changes()?
        } else {
            Changes::All
        } {
            Changes::None => Vec::new(),
            Changes::All => vec![local],
            Changes::Rects(v) => v
                .iter()
                .map(|r| {
                    Rect::new(r.x - region.x, r.y - region.y, r.w, r.h)
                        .align_out(2 * scale as i32, &local)
                })
                .filter(|r| !r.is_empty())
                .collect(),
        };
        let mut changed = !rects.is_empty();
        for r in &rects {
            let g = Instant::now();
            let px = screen.grab(Rect::new(r.x + region.x, r.y + region.y, r.w, r.h))?;
            t_grab.push(g.elapsed());
            let c = Instant::now();
            bgrx_into(&mut frame, px, r.w as usize * 4, conv(*r));
            t_conv.push(c.elapsed());
            grabbed_px += r.area();
        }
        if draw_cursor {
            let c = Instant::now();
            let known = cursor_img.as_ref().map_or(0, |c| c.serial);
            let st = screen.cursor(known)?;
            if (st.x, st.y, st.serial) != last_cursor {
                last_cursor = (st.x, st.y, st.serial);
                changed = true;
            }
            if let Some(argb) = &st.argb {
                cursor_img = Some(CursorImage::from_argb(
                    st.serial, st.w, st.h, st.xhot, st.yhot, argb,
                ));
            }
            if let Some(img) = &cursor_img {
                let (x, y) = (
                    (st.x - region.x) / scale as i32,
                    (st.y - region.y) / scale as i32,
                );
                cursor::draw(&mut frame, img, x, y, scale as usize, &mut patch);
            }
            t_cur.push(c.elapsed());
        }
        if changed {
            let w = Instant::now();
            enc.write(pts, &frame.data)?;
            t_write.push(w.elapsed());
            sent += 1;
        } else {
            static_frames += 1;
        }
        t_frame.push(t0.elapsed());
    }
    // Close on the last tick so the file lasts the full time.
    enc.wait_drained();
    enc.write(wall0.elapsed().as_micros() as u64, &frame.data)?;
    sent += 1;
    let wall = wall0.elapsed().as_secs_f64();
    let cpu = (cpu_time(false) - cpu0).as_secs_f64();
    let ff_cpu = proc_cpu(enc.pid()).as_secs_f64();
    let status = enc.finish()?;
    if let Some(Ok(mut c)) = animator {
        let _ = c.kill();
        let _ = c.wait();
    }
    let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    println!("{}", t_grab.line("grab"));
    println!("{}", t_conv.line("convert"));
    println!("{}", t_cur.line("cursor"));
    println!("{}", t_write.line("write"));
    println!("{}", t_frame.line("frame"));
    let full_px = region.area() as f64 * ticks as f64;
    println!(
        "ticks {ticks} in {wall:.2}s, frames sent {sent}, static ticks skipped {static_frames} ({:.0}%), late ticks {late}, pixels grabbed {:.1}% of full-frame capture",
        static_frames as f64 * 100.0 / ticks.max(1) as f64,
        grabbed_px as f64 * 100.0 / full_px.max(1.0)
    );
    println!(
        "CPU: engine {:.1}% of one core, ffmpeg {:.1}%, peak RSS {} MiB",
        cpu / wall * 100.0,
        ff_cpu / wall * 100.0,
        peak_rss_kib() / 1024
    );
    println!(
        "ffmpeg {status}, {} → {:.2} MiB ({:.0} kbit/s)",
        out.display(),
        size as f64 / 1048576.0,
        size as f64 * 8.0 / 1000.0 / wall
    );
    Ok(())
}

/// Draw a moving block on the root window (a stand-in for screen activity).
fn animate(a: &Args) -> Result<()> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ChangeGCAux, ConnectionExt, CreateGCAux, Rectangle};
    let (bw, bh) = a
        .get("--size")
        .and_then(|s| s.split_once('x'))
        .map(|(w, h)| (w.parse().unwrap_or(200u16), h.parse().unwrap_or(120u16)))
        .unwrap_or((200, 120));
    let fps: u32 = a.num("--fps", 30);
    let seconds: u32 = a.num("--seconds", 10);
    let (conn, n) = x11rb::connect(None)?;
    let screen = &conn.setup().roots[n];
    let (root, sw) = (screen.root, screen.width_in_pixels);
    let gc = conn.generate_id()?;
    conn.create_gc(gc, root, &CreateGCAux::new().foreground(0x00203040))?;
    conn.poly_fill_rectangle(
        root,
        gc,
        &[Rectangle {
            x: 0,
            y: 0,
            width: sw,
            height: screen.height_in_pixels,
        }],
    )?;
    let mut t = Ticker::new(fps);
    for i in 0..(fps * seconds) {
        t.wait();
        let x = ((i * 9) % (sw as u32 - bw as u32)) as i16;
        conn.change_gc(gc, &ChangeGCAux::new().foreground(0x00203040))?;
        conn.poly_fill_rectangle(
            root,
            gc,
            &[Rectangle {
                x: x - 9,
                y: 300,
                width: 9,
                height: bh,
            }],
        )?;
        conn.change_gc(
            gc,
            &ChangeGCAux::new().foreground(0x00ff8000 + (i * 3 % 255)),
        )?;
        conn.poly_fill_rectangle(
            root,
            gc,
            &[Rectangle {
                x,
                y: 300,
                width: bw,
                height: bh,
            }],
        )?;
        conn.flush()?;
    }
    Ok(())
}
