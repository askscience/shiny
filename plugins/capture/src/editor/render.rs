//! HTML graphics to video with alpha.
//!
//! The renderer is a separate program (`liteaura-capture-render`, Qt
//! WebEngine offscreen) so a heavy or hostile page never runs inside the
//! server. For every frame it moves the page's clock to `t` (CSS and Web
//! Animations seeked, `requestAnimationFrame` and `performance.now` virtual,
//! see [`RUNTIME`]), waits for the paint and writes the RGBA pixels to stdout;
//! ffmpeg turns them into an FFV1 file with alpha. The same HTML therefore
//! gives the same frames on every export.

use super::project::{Overlay, Project};
use crate::Result;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Injected first in every graphic: a virtual clock the page can read as
/// `capture.time` (seconds), and seeking for CSS and Web Animations.
pub const RUNTIME: &str = r#"<script>(()=>{
const real=performance.now.bind(performance),raf=requestAnimationFrame.bind(window),d0=Date.now();
let T=0,playing=false,origin=0,base=0;
const now=()=>playing?base+(real()-origin):T*1000;
const seekAnims=t=>{for(const a of document.getAnimations()){try{a.pause();a.currentTime=Math.max(0,t*1000);}catch(e){}}};
window.capture={get time(){return now()/1000},duration:__DUR__,width:__W__,height:__H__,
 seek(t){T=t;playing=false;seekAnims(t);},
 play(t){T=t;base=t*1000;origin=real();playing=true;for(const a of document.getAnimations()){try{a.currentTime=Math.max(0,t*1000);a.play();}catch(e){}}},
 pause(){T=now()/1000;playing=false;seekAnims(T);}};
performance.now=now;Date.now=()=>d0+now();
window.requestAnimationFrame=cb=>raf(()=>cb(now()));
addEventListener('message',e=>{const m=e.data||{};if(m.capture==='seek')capture.seek(+m.t||0);else if(m.capture==='play')capture.play(+m.t||0);else if(m.capture==='pause')capture.pause();});
document.addEventListener('DOMContentLoaded',()=>capture.seek(T));
})();</script>"#;

/// The style every graphic shares: the project's tokens plus a transparent
/// page.
fn style_block(p: &Project) -> String {
    let css = p.style.css.replace("</", "<\\/");
    // The style is custom properties (`--accent: #…;`), but a model may send
    // whole rules (`:root{--accent:…}`): wrapping those in `:root{}` again
    // nests them, the browser drops them and every `var()` without a
    // fallback leaves its element invisible.
    let tokens = if css.contains('{') {
        css
    } else {
        format!(":root{{{css}}}")
    };
    format!(
        "<style>html,body{{background:transparent!important;margin:0;width:100%;height:100%;overflow:hidden}}{tokens}</style>"
    )
}

/// Libraries a graphic can ask for by name: the tag is replaced by the code,
/// because graphics render with no network.
const THREE_JS: &str = include_str!("../../vendor/three.min.js");

fn library(src: &str) -> Option<&'static str> {
    let s = src.trim().trim_start_matches("./").to_ascii_lowercase();
    match s.as_str() {
        "lib/three.js" | "lib/three.min.js" | "three.js" | "three.min.js" | "lib:three" => {
            Some(THREE_JS)
        }
        _ => None,
    }
}

/// Replace `<script src="lib/…"></script>` with the library inline.
pub fn inline_libraries(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut from = 0;
    while let Some(i) = lower[from..].find("<script") {
        let start = from + i;
        let Some(gt) = lower[start..].find('>').map(|j| start + j) else {
            break;
        };
        let Some(close) = lower[gt..].find("</script>").map(|j| gt + j) else {
            break;
        };
        let tag = &html[start..=gt];
        let src = super::render::attr(tag, "src");
        match src.as_deref().and_then(library) {
            Some(code) if html[gt + 1..close].trim().is_empty() => {
                out.push_str(&html[from..start]);
                out.push_str("<script>");
                out.push_str(&code.replace("</script", "<\\/script"));
                out.push_str("</script>");
            }
            _ => out.push_str(&html[from..close + "</script>".len()]),
        }
        from = close + "</script>".len();
    }
    out.push_str(&html[from..]);
    out
}

/// The value of `name="…"` (or `'…'`, or bare) in one tag.
pub fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let at = from + i;
        from = at + name.len();
        let before = lower[..at].chars().next_back();
        if !before.is_some_and(|c| c.is_whitespace()) {
            continue;
        }
        let rest = tag[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let q = rest.chars().next()?;
        if q == '"' || q == '\'' {
            let body = &rest[1..];
            return Some(body[..body.find(q).unwrap_or(body.len())].to_string());
        }
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(rest.len());
        return Some(rest[..end].to_string());
    }
    None
}

/// The overlay as a complete document: runtime, shared style, then the
/// AI's HTML.
pub fn document(p: &Project, o: &Overlay, w: u32, h: u32) -> String {
    let rt = RUNTIME
        .replace("__DUR__", &format!("{:.3}", o.dur))
        .replace("__W__", &w.to_string())
        .replace("__H__", &h.to_string());
    let head = format!("<meta charset=\"utf-8\">{rt}{}", style_block(p));
    let html = inline_libraries(o.html.trim());
    let html = html.as_str();
    // Put our head first, whatever the document looks like.
    let lower = html.to_ascii_lowercase();
    if let Some(i) = lower.find("<head>") {
        let at = i + "<head>".len();
        format!("{}{}{}", &html[..at], head, &html[at..])
    } else if let Some(i) = lower.find("<html") {
        let at = html[i..].find('>').map(|j| i + j + 1).unwrap_or(i);
        format!("{}<head>{}</head>{}", &html[..at], head, &html[at..])
    } else {
        format!("<!doctype html><html><head>{head}</head><body>{html}</body></html>")
    }
}

/// The renderer program, next to the server or on `PATH`.
pub fn renderer() -> Option<PathBuf> {
    let name = "liteaura-capture-render";
    if let Some(p) = std::env::var_os("LITEAURA_CAPTURE_RENDER").map(PathBuf::from) {
        if p.is_file() {
            return Some(p);
        }
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf));
    for d in exe_dir.into_iter().chain(
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>()),
    ) {
        let c = d.join(name);
        if c.is_file() {
            return Some(c);
        }
    }
    None
}

/// Render `o` to `<work>/<id>.mkv` (FFV1, yuva420p) at its on-canvas size.
/// `progress` hears the share of frames done (0..1) as they arrive.
pub fn render_overlay(
    p: &Project,
    o: &Overlay,
    work: &Path,
    progress: &mut dyn FnMut(f32),
) -> Result<PathBuf> {
    use std::io::{Read, Write};
    let r = renderer().ok_or("the graphics renderer (liteaura-capture-render) is not installed")?;
    let w = (((o.rect.w * p.width as f64).round() as u32).max(16)) & !1;
    let h = (((o.rect.h * p.height as f64).round() as u32).max(16)) & !1;
    let html = work.join(format!("{}.html", o.id));
    std::fs::write(&html, document(p, o, w, h))?;
    let out = work.join(format!("{}.mkv", o.id));
    let fps = p.fps;
    let total = ((o.dur * fps as f64).round() as usize).max(1);
    let mut rend = Command::new(r)
        .args(["--html".into(), html.display().to_string()])
        .args([
            "--width".into(),
            w.to_string(),
            "--height".into(),
            h.to_string(),
        ])
        .args([
            "--fps".into(),
            fps.to_string(),
            "--duration".into(),
            format!("{:.3}", o.dur),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut from = rend.stdout.take().ok_or("no renderer output")?;
    // Drain stderr as it comes: a chatty page must not block the renderer.
    let mut rerr = rend.stderr.take().ok_or("no renderer errors")?;
    let rerr = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = rerr.read_to_end(&mut v);
        v
    });
    let mut enc = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
        ])
        .args([
            "-s".into(),
            format!("{w}x{h}"),
            "-r".into(),
            fps.to_string(),
        ])
        .args(["-i", "pipe:0", "-c:v", "ffv1", "-pix_fmt", "yuva420p"])
        .arg(&out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut to = enc.stdin.take().ok_or("no encoder input")?;
    let frame = w as usize * h as usize * 4;
    let mut buf = vec![0u8; frame];
    let mut done = 0usize;
    loop {
        match from.read_exact(&mut buf) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }
        if to.write_all(&buf).is_err() {
            break;
        }
        done += 1;
        progress((done as f32 / total as f32).min(1.0));
    }
    drop(to);
    let rst = rend.wait()?;
    let rerr = rerr.join().unwrap_or_default();
    let enc = enc.wait_with_output()?;
    if !rst.success() {
        return Err(format!(
            "the graphic \"{}\" did not render: {}",
            o.name,
            String::from_utf8_lossy(&rerr).trim()
        )
        .into());
    }
    if !enc.status.success() {
        return Err(String::from_utf8_lossy(&enc.stderr)
            .trim()
            .to_string()
            .into());
    }
    Ok(out)
}

/// One frame of a graphic at `t` as PNG (the agent's self-check with a
/// vision model, and the timeline's thumbnail).
pub fn snapshot(p: &Project, o: &Overlay, t: f64) -> Result<Vec<u8>> {
    let r = renderer().ok_or("the graphics renderer is not installed")?;
    let w = (((o.rect.w * p.width as f64).round() as u32).max(16)) & !1;
    let h = (((o.rect.h * p.height as f64).round() as u32).max(16)) & !1;
    let dir = std::env::temp_dir().join(format!("liteaura-capture-snap-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir)?;
    let html = dir.join("g.html");
    std::fs::write(&html, document(p, o, w, h))?;
    let out = Command::new(r)
        .args(["--html".into(), html.display().to_string()])
        .args([
            "--width".into(),
            w.to_string(),
            "--height".into(),
            h.to_string(),
        ])
        .args(["--at".into(), format!("{t:.3}"), "--png".into()])
        .stdin(Stdio::null())
        .output()?;
    let _ = std::fs::remove_dir_all(&dir);
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr)
            .trim()
            .to_string()
            .into());
    }
    let mut v = Vec::new();
    v.write_all(&out.stdout)?;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libraries_are_inlined_by_name() {
        let html = r#"<head><script src="lib/three.js"></script><script src='x.js'></script><script>let a=1</script></head>"#;
        let out = inline_libraries(html);
        assert!(out.contains("THREE"), "three.js is in the page");
        assert!(!out.contains(r#"src="lib/three.js""#));
        assert!(
            out.contains("<script src='x.js'></script>"),
            "other scripts are left alone"
        );
        assert!(out.contains("<script>let a=1</script>"));
        assert_eq!(inline_libraries("<p>no scripts</p>"), "<p>no scripts</p>");
    }

    #[test]
    fn style_tokens_work_as_properties_or_as_a_rule() {
        let mut p = Project::default();
        p.style.css = "--accent:#2f6bff;".into();
        assert!(style_block(&p).contains(":root{--accent:#2f6bff;}"));
        p.style.css = ":root{--ease-out:cubic-bezier(.16,1,.3,1)}".into();
        let b = style_block(&p);
        assert!(b.contains(":root{--ease-out"));
        assert!(
            !b.contains(":root{:root"),
            "a whole rule is not wrapped again"
        );
    }

    #[test]
    fn runtime_goes_first() {
        let p = Project::default();
        let o = Overlay {
            html: "<html><head><title>x</title></head><body>hi</body></html>".into(),
            ..Overlay::default()
        };
        let d = document(&p, &o, 100, 50);
        let rt = d.find("window.capture").unwrap();
        assert!(rt < d.find("<title>").unwrap());
        let frag = document(
            &p,
            &Overlay {
                html: "<div>x</div>".into(),
                ..Overlay::default()
            },
            10,
            10,
        );
        assert!(frag.starts_with("<!doctype html><html><head><meta charset"));
    }
}
