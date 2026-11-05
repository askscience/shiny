//! LiteAura look for Gokapi's public pages, and per-share sender cards.
//!
//! Gokapi 2.2.4 loads optional files from a `custom/` folder relative to its
//! working directory (`CustomStaticContent.go`); Shiny runs it from
//! [`super::state_dir`], so that is `~/.shiny/gokapi/custom/`:
//!
//! * `custom.css` / `public.js` — the theme ([`write_theme`], refreshed before
//!   every server start, because Gokapi reads the folder only at start);
//! * `version.txt` — cache-busting version Gokapi appends to those URLs;
//! * `s/<share id>.json` — `{ from, message }` shown on that share's download
//!   page ([`write_share_meta`]). The share gate serves these files but never
//!   a listing of `s/` (`gate::is_custom_file`).

use std::path::PathBuf;

use serde_json::json;

use super::{install::sha256_hex, state_dir};

const CUSTOM_CSS: &str = include_str!("theme/custom.css");
const PUBLIC_JS: &str = include_str!("theme/public.js");
/// The LiteAura mark (the desktop's favicon), used as a CSS mask on the page.
const LOGO_SVG: &str = include_str!("../../../../web/favicon.svg");

/// Fonts the page uses, copied from the host's Debian font packages
/// (fonts-inter, fonts-noto-mono; both SIL OFL) so the page never fetches a
/// third-party font. A missing file only means a system-font fallback.
const FONTS: &[(&str, &str)] = &[
    (
        "pk-display-black.otf",
        "/usr/share/fonts/opentype/inter/InterDisplay-Black.otf",
    ),
    (
        "pk-display-medium.otf",
        "/usr/share/fonts/opentype/inter/InterDisplay-Medium.otf",
    ),
    (
        "pk-text-regular.otf",
        "/usr/share/fonts/opentype/inter/Inter-Regular.otf",
    ),
    (
        "pk-mono-regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
    ),
];

/// Longest sender name and message stored for a share.
pub const MAX_FROM: usize = 64;
pub const MAX_MESSAGE: usize = 500;

fn custom_dir() -> PathBuf {
    state_dir().join("custom")
}

fn meta_path(id: &str) -> Option<PathBuf> {
    let valid = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    valid.then(|| custom_dir().join("s").join(format!("{id}.json")))
}

/// Cache-busting version for `version.txt`. Gokapi parses it as an integer
/// (`strconv.Itoa(readCustomStaticVersion())`) and serves the theme as
/// `custom.v<N>.css` with a 28-hour cache, so N must be a decimal number that
/// changes with the content; anything else silently becomes 0 and browsers
/// keep the previous theme.
fn theme_version() -> String {
    let hash = sha256_hex(format!("{CUSTOM_CSS}\0{PUBLIC_JS}\0{LOGO_SVG}").as_bytes());
    // 7 hex digits fit comfortably in Go's int; never 0.
    (u32::from_str_radix(&hash[..7], 16).unwrap_or(0) + 1).to_string()
}

/// Write the theme files when their content changed. Gokapi must be
/// (re)started afterwards to pick them up.
pub fn write_theme() -> std::io::Result<()> {
    let dir = custom_dir();
    std::fs::create_dir_all(dir.join("s"))?;
    let version = theme_version();
    let version = version.as_str();
    for (name, body) in [
        ("custom.css", CUSTOM_CSS),
        ("public.js", PUBLIC_JS),
        ("liteaura-logo.svg", LOGO_SVG),
        ("version.txt", version),
    ] {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(body) {
            std::fs::write(&path, body)?;
        }
    }
    for (name, source) in FONTS {
        let dest = dir.join(name);
        let same = match (std::fs::metadata(source), std::fs::metadata(&dest)) {
            (Ok(a), Ok(b)) => a.len() == b.len(),
            _ => false,
        };
        if !same && std::path::Path::new(source).is_file() {
            std::fs::copy(source, &dest)?;
        }
    }
    Ok(())
}

/// Keep one line of plain text: no control characters, trimmed, capped.
fn clean(text: &str, max: usize, keep_newlines: bool) -> String {
    text.chars()
        .filter(|c| !c.is_control() || (keep_newlines && *c == '\n'))
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Record who shared `id` (shown on its download page). Nothing is written
/// when both fields are empty.
pub fn write_share_meta(id: &str, from: &str, message: &str) -> std::io::Result<()> {
    let Some(path) = meta_path(id) else {
        return Ok(());
    };
    let from = clean(from, MAX_FROM, false);
    let message = clean(message, MAX_MESSAGE, true);
    if from.is_empty() && message.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        json!({ "from": from, "message": message }).to_string(),
    )
}

/// Forget a share's sender card (revoked or expired).
pub fn delete_share_meta(id: &str) {
    if let Some(path) = meta_path(id) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_path_rejects_unsafe_ids() {
        assert!(meta_path("Ab12_x-9").is_some());
        for bad in ["", "../x", "a/b", "a.b", "a b", &"x".repeat(65)] {
            assert!(meta_path(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn clean_strips_controls_and_caps() {
        assert_eq!(clean("  Ma\u{0}rio\u{1b}[31m ", 64, false), "Mario[31m");
        assert_eq!(clean("a\nb", 10, true), "a\nb");
        assert_eq!(clean("a\nb", 10, false), "ab");
        assert_eq!(clean(&"é".repeat(100), 64, false).chars().count(), 64);
    }

    #[test]
    fn theme_never_builds_html_from_sender_text() {
        // The sender card must stay text-only (XSS on a public page): the one
        // innerHTML assignment is the static chip icon.
        assert_eq!(PUBLIC_JS.matches("innerHTML").count(), 1);
        assert!(PUBLIC_JS.contains("c.innerHTML = svg; // static icon markup only"));
        assert!(!PUBLIC_JS.contains("insertAdjacentHTML"));
        assert!(!PUBLIC_JS.contains("outerHTML"));
        assert!(PUBLIC_JS.contains("el('p', 'pk-message', message)"));
    }

    #[test]
    fn version_is_a_positive_decimal_integer() {
        let v = theme_version();
        assert!(v.chars().all(|c| c.is_ascii_digit()), "{v}");
        assert!(v.parse::<u32>().unwrap() > 0);
    }

    #[test]
    fn theme_assets_pass_the_gate() {
        for name in ["custom.css", "public.js", "liteaura-logo.svg"]
            .into_iter()
            .chain(FONTS.iter().map(|(n, _)| *n))
        {
            assert!(
                super::super::gate::is_allowed("GET", &format!("/custom/{name}")),
                "{name}"
            );
        }
        assert!(LOGO_SVG.starts_with("<svg"));
    }
}
