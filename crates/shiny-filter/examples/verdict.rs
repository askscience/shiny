//! Throwaway diagnostic: run the live compiled ad-filter engine against a list
//! of URLs and print the verdict for each. Not part of the app.
//!
//!   cargo run --release -p shiny-filter --example verdict -- <url> [<url> ...]
//!
//! With no arguments it checks the YouTube URLs the kiosk log showed while a
//! page was failing to work.

use std::path::PathBuf;

use shiny_filter_core::{AdFilter, CacheMeta, Verdict};

const DEFAULT_URLS: &[(&str, &str)] = &[
    ("document", "https://www.youtube.com/"),
    ("document", "https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
    (
        "script",
        "https://www.youtube.com/s/_/ytmainappweb/_/js/k=ytmainappweb.kevlar_base.en_US.VRNYTIAj20w.es5.O/am=AAAAIAAAOAJI/d=1/br=1/rs=AGKMywFqtowllSXh93PrkLgnlcvTfNsZBA/m=kevlar_base_module,kevlar_main_module,kevlar_base_sync_mod_chunk",
    ),
    ("script", "https://www.youtube.com/s/desktop/8c4c9d2a/jsbin/desktop_polymer.vflset/desktop_polymer.js"),
    ("stylesheet", "https://www.youtube.com/s/desktop/8c4c9d2a/cssbin/www-main-desktop-home-page-skeleton.css"),
    ("xhr", "https://www.youtube.com/youtubei/v1/player?key=AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8"),
    ("xhr", "https://www.youtube.com/youtubei/v1/browse?key=AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8"),
    ("ping", "https://www.youtube.com/api/stats/watchtime?ns=yt&docid=dQw4w9WgXcQ"),
    ("image", "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg"),
    ("image", "https://i.ytimg.com/generate_204"),
    ("media", "https://rr1---sn-x.googlevideo.com/videoplayback?expire=1&itag=18"),
    ("xhr", "https://www.youtube.com/api/stats/ads?ver=2&ns=yt"),
    ("xhr", "https://www.youtube.com/pagead/id"),
    ("xhr", "https://www.youtube.com/ptracking?video_id=abc"),
    ("xhr", "https://www.youtube.com/get_midroll_info?cpn=abc"),
];

fn main() {
    let dir: PathBuf = std::env::var("ADFILTER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").expect("HOME"))
                .join(".local/share/shiny/adfilter")
        });

    let meta: CacheMeta = serde_json::from_str(
        &std::fs::read_to_string(dir.join("engine.json")).expect("engine.json"),
    )
    .expect("engine.json parses");
    let bytes = std::fs::read(dir.join("engine.dat")).expect("engine.dat");
    let filter = AdFilter::from_serialized(&bytes, meta.rules).expect("engine.dat deserializes");
    println!("engine: {} rules from {}", filter.rule_count(), dir.display());

    let args: Vec<String> = std::env::args().skip(1).collect();
    let page = "https://www.youtube.com/";
    let kinds = ["document", "subdocument", "script", "stylesheet", "image", "xhr", "media", "ping", "object", "other"];
    if args.is_empty() {
        for (kind, url) in DEFAULT_URLS {
            let verdict = filter.check_url(url, page, kind, "GET");
            println!("{:>10} {:?}\n           {}", kind, verdict, url);
        }
    } else {
        for url in &args {
            let mut line = format!("{}", url);
            for kind in kinds {
                let verdict = filter.check_url(url, page, kind, "GET");
                if matches!(verdict, Verdict::Block) {
                    line.push_str(&format!("\n      BLOCK as {kind}"));
                }
            }
            println!("{line}");
        }
    }

    let blocked = DEFAULT_URLS
        .iter()
        .filter(|(kind, url)| {
            matches!(filter.check_url(url, page, kind, "GET"), Verdict::Block)
        })
        .count();
    println!("\n{blocked} of the default URLs are blocked");
}
