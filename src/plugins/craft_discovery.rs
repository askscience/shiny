//! Craft store discovery — new Craft apps join the store on their own.
//!
//! The built-in [`SUITE`](crate::plugins::store::SUITE) is the list as it was
//! when this build was made. The suite keeps growing, so once a day the store
//! looks for apps it does not know yet, in two places:
//!
//! - **The ArtCraft release feed** (`api.storyteller.ai`, the service the
//!   ArtCraft launcher reads): every app it lists whose latest release carries
//!   a `<id>-web-<version>.zip` build. One request, no GitHub rate limit.
//! - **The `storytold` GitHub account**: every repository whose name says
//!   `craft` (not a fork, not archived). A candidate is probed once — its
//!   latest release must carry a web build — and a repository without one is
//!   not asked again for a week. At most [`PROBES_PER_PASS`] probes per pass,
//!   so discovery stays far inside GitHub's 60 unauthenticated requests/hour.
//!
//! What is found is remembered in `<plugins_dir>/.craft-store.json` and
//! registered next to the suite for the life of the process, so an installed
//! discovered app stays known across restarts (its window, its AI control).
//! A discovered entry is leaked into `'static`: there are a handful of them
//! at most, ever, and every caller of the catalog holds `&'static CraftApp`.
//!
//! Everything read here is input: names and descriptions are cleaned and
//! bounded, ids must be plain (`a-z`, `0-9`, `-`), and the release itself is
//! still resolved and verified by the store like any built-in app's.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::plugins::store::{CraftApp, SUITE};

/// The ArtCraft launcher's release feed.
pub const FEED_URL: &str = "https://api.storyteller.ai/v1/craft_apps/release_info";
/// The account the official apps live under.
pub const OWNER: &str = "storytold";
/// How often discovery runs (seconds).
pub const INTERVAL_SECS: i64 = 24 * 3600;
/// A repository without a web build is asked again after this long.
pub const REJECT_SECS: i64 = 7 * 24 * 3600;
/// GitHub release probes per discovery pass.
pub const PROBES_PER_PASS: usize = 6;
/// Ids that are tooling around the suite, never an app for the store.
const NOT_APPS: [&str; 3] = ["launcher", "corpus", "-libs"];

/// One discovered app, as remembered on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Found {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub tagline: String,
    /// GitHub `owner/repo`.
    pub repo: String,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub community: bool,
    #[serde(default)]
    pub color: [u8; 3],
}

/// The discovery file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Saved {
    #[serde(default)]
    pub apps: Vec<Found>,
    /// Repository id → when it was found to carry no web build (Unix secs).
    #[serde(default)]
    pub rejected: BTreeMap<String, i64>,
    /// When the last pass reached at least one source (Unix secs).
    #[serde(default)]
    pub checked_at: i64,
}

impl Saved {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        let Ok(text) = serde_json::to_string_pretty(self) else { return };
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }

    /// Whether a pass is due at `now`.
    pub fn due(&self, now: i64) -> bool {
        now - self.checked_at >= INTERVAL_SECS
    }

    /// Whether `id` was rejected recently enough to skip at `now`.
    pub fn recently_rejected(&self, id: &str, now: i64) -> bool {
        self.rejected.get(id).is_some_and(|at| now - at < REJECT_SECS)
    }
}

// ── The registry ────────────────────────────────────────────────────────────

static FOUND: RwLock<Vec<&'static CraftApp>> = RwLock::new(Vec::new());

pub fn find(id: &str) -> Option<&'static CraftApp> {
    FOUND.read().ok()?.iter().copied().find(|app| app.id == id)
}

pub fn all() -> Vec<&'static CraftApp> {
    FOUND.read().map(|apps| apps.clone()).unwrap_or_default()
}

/// Whether the catalog already has `id` (built in or discovered).
pub fn known(id: &str) -> bool {
    SUITE.iter().any(|app| app.id == id || app.former_ids.contains(&id)) || find(id).is_some()
}

/// Add a discovered app to the catalog. False when it is already there or
/// not a usable entry.
pub fn register(found: &Found) -> bool {
    if !valid_id(&found.id) || known(&found.id) || !valid_repo(&found.repo) {
        return false;
    }
    let leak = |text: &str| -> &'static str { Box::leak(text.to_string().into_boxed_str()) };
    let app: &'static CraftApp = Box::leak(Box::new(CraftApp {
        id: leak(&found.id),
        name: leak(&clean(&found.name, 40)),
        tagline: leak(&clean(&found.tagline, 120)),
        former_ids: &[],
        repo: Some(leak(&found.repo)),
        publisher: leak(if found.publisher.is_empty() { "ArtCraft" } else { &found.publisher }),
        community: found.community,
        color: if found.color == [0, 0, 0] { color_for(&found.id) } else { found.color },
        site: None,
        discovered: true,
    }));
    match FOUND.write() {
        Ok(mut apps) => {
            apps.push(app);
            true
        }
        Err(_) => false,
    }
}

// ── Reading the sources ─────────────────────────────────────────────────────

/// A plain app id: lowercase letters, digits and inner dashes, and something
/// that reads as a Craft app.
pub fn valid_id(id: &str) -> bool {
    (3..=40).contains(&id.len())
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-')
        && !id.ends_with('-')
        && id.contains("craft")
        && !NOT_APPS.iter().any(|word| id.contains(word))
}

/// `owner/repo`, both plain GitHub names.
fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let ok = |part: Option<&str>| {
        part.is_some_and(|p| {
            !p.is_empty()
                && p.len() <= 100
                && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                && !p.starts_with('.')
        })
    };
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

/// Whether a release's asset names carry `<id>-web-<version>.zip`.
pub fn has_web_build<'a>(id: &str, assets: impl IntoIterator<Item = &'a str>) -> bool {
    let prefix = format!("{id}-web-");
    assets.into_iter().any(|name| name.starts_with(&prefix) && name.ends_with(".zip"))
}

/// Apps in the release feed that carry a web build and are not known yet.
pub fn from_feed(feed: &serde_json::Value) -> Vec<Found> {
    let Some(apps) = feed
        .pointer("/release_info/apps")
        .or_else(|| feed.get("apps"))
        .and_then(|apps| apps.as_object())
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, entry) in apps {
        let id = key.to_ascii_lowercase();
        if !valid_id(&id) || known(&id) {
            continue;
        }
        let repo = entry.get("repo").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        if !valid_repo(&repo) {
            continue;
        }
        let assets = entry
            .pointer("/latest_release/assets")
            .and_then(|v| v.as_array())
            .map(|assets| {
                assets
                    .iter()
                    .filter_map(|asset| asset.get("name").and_then(|n| n.as_str()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !has_web_build(&id, assets) {
            continue;
        }
        let owner = repo.split('/').next().unwrap_or_default();
        let name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .map(|name| clean(name, 40))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| name_from_id(&id));
        out.push(Found {
            tagline: String::new(),
            publisher: if owner == OWNER { "ArtCraft".into() } else { owner.to_string() },
            community: owner != OWNER,
            color: color_for(&id),
            id,
            name,
            repo,
        });
    }
    out
}

/// A repository worth probing: `(id, owner/repo, description)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    pub repo: String,
    pub description: String,
}

/// The account's repositories that may be Craft apps not known yet.
pub fn candidates(repos: &serde_json::Value) -> Vec<Candidate> {
    let Some(list) = repos.as_array() else { return Vec::new() };
    let mut out = Vec::new();
    for repo in list {
        let flag = |key: &str| repo.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
        if flag("fork") || flag("archived") || flag("private") {
            continue;
        }
        let Some(name) = repo.get("name").and_then(|v| v.as_str()) else { continue };
        let full = repo
            .get("full_name")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{OWNER}/{name}"));
        let id = name.to_ascii_lowercase();
        if !valid_id(&id) || known(&id) || !valid_repo(&full) {
            continue;
        }
        let description = repo
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        out.push(Candidate { id, repo: full, description });
    }
    out
}

/// A display name and a tagline from a repository description such as
/// `"TextureCraft: An open-source, 3D texture painting workflow written from
/// scratch in pure Rust"` → `("TextureCraft", "3D texture painting workflow")`.
pub fn describe(id: &str, description: &str) -> (String, String) {
    let squash = |text: &str| text.to_ascii_lowercase().replace([' ', '-', '_'], "");
    let description = clean(description, 300);
    let (name, rest) = match description.split_once(':') {
        Some((head, rest)) if squash(head) == squash(id) => (clean(head, 40), rest.trim().to_string()),
        _ => (name_from_id(id), description.clone()),
    };
    // The suite's descriptions repeat the same boilerplate; keep what the
    // app is for.
    let mut tagline = rest.as_str();
    for lead in ["An open-source,", "An open-source", "an open-source,", "an open-source"] {
        if let Some(stripped) = tagline.strip_prefix(lead) {
            tagline = stripped.trim_start();
            break;
        }
    }
    for cut in [" written ", " — ", "; ", ". ", " in pure Rust", " by ArtCraft"] {
        if let Some(at) = tagline.find(cut) {
            tagline = &tagline[..at];
        }
    }
    let tagline = tagline.trim().trim_end_matches(['.', ',', ';']);
    let mut chars = tagline.chars();
    let tagline = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    };
    (name, clean(&tagline, 120))
}

/// `texturecraft` → `TextureCraft`, `craft-anything` → `Craft Anything`.
pub fn name_from_id(id: &str) -> String {
    let cap = |word: &str| {
        let mut chars = word.chars();
        chars.next().map(|c| c.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default()
    };
    id.split('-')
        .map(|word| match word.strip_suffix("craft") {
            Some(stem) if !stem.is_empty() => format!("{}Craft", cap(stem)),
            _ => cap(word),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A stable brand-like colour for an app that does not publish one.
pub fn color_for(id: &str) -> [u8; 3] {
    const PALETTE: [[u8; 3]; 8] = [
        [0x2f, 0x7b, 0xf5],
        [0xe8, 0x57, 0x3f],
        [0x8b, 0x5c, 0xf6],
        [0x1f, 0x9d, 0x55],
        [0xf0, 0x7a, 0x1e],
        [0x17, 0x8a, 0x9c],
        [0xe0, 0x36, 0x8f],
        [0x3e, 0xa6, 0xe8],
    ];
    let hash = id.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    PALETTE[(hash as usize) % PALETTE.len()]
}

/// One line of plain text, bounded.
fn clean(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    flat.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ids_must_read_as_craft_apps() {
        assert!(valid_id("texturecraft"));
        assert!(valid_id("craft-anything"));
        assert!(!valid_id("craft-launcher"));
        assert!(!valid_id("photocraft-corpus"));
        assert!(!valid_id("craft-libs"));
        assert!(!valid_id("egui"));
        assert!(!valid_id("../craft"));
        assert!(!valid_id("Craft"));
    }

    #[test]
    fn descriptions_become_a_name_and_a_tagline() {
        assert_eq!(
            describe(
                "texturecraft",
                "TextureCraft: An open-source, 3D texture painting workflow written from scratch in pure Rust",
            ),
            ("TextureCraft".into(), "3D texture painting workflow".into()),
        );
        assert_eq!(
            describe("anicraft", "AniCraft: An open-source 2D animation editor written from scratch in pure Rust."),
            ("AniCraft".into(), "2D animation editor".into()),
        );
        assert_eq!(
            describe("newcraft", "Spreadsheets for everyone"),
            ("NewCraft".into(), "Spreadsheets for everyone".into()),
        );
        assert_eq!(name_from_id("craft-anything"), "Craft Anything");
    }

    #[test]
    fn the_feed_yields_unknown_apps_with_a_web_build() {
        let feed = json!({
            "release_info": { "apps": {
                "photocraft": { "name": "PhotoCraft", "repo": "storytold/photocraft",
                    "latest_release": { "assets": [{ "name": "photocraft-web-1.0.0.zip" }] } },
                "artcraft-launcher": { "name": "ArtCraft Launcher", "repo": "storytold/craft-launcher",
                    "latest_release": { "assets": [{ "name": "artcraft-launcher-web-0.14.0.zip" }] } },
                "zenocraft": { "name": "ZenoCraft", "repo": "storytold/zenocraft",
                    "latest_release": { "assets": [{ "name": "zenocraft-web-0.1.0.zip" }] } },
                "nowebcraft": { "name": "NoWebCraft", "repo": "storytold/nowebcraft",
                    "latest_release": { "assets": [{ "name": "nowebcraft-0.1.0-linux-x86_64.tar.gz" }] } },
                "othercraft": { "name": "OtherCraft", "repo": "someone/othercraft",
                    "latest_release": { "assets": [{ "name": "othercraft-web-2.0.0.zip" }] } }
            } }
        });
        let found = from_feed(&feed);
        let ids: Vec<_> = found.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["othercraft", "zenocraft"]);
        let other = &found[0];
        assert!(other.community);
        assert_eq!(other.publisher, "someone");
        assert!(!found[1].community);
    }

    #[test]
    fn repositories_are_filtered_to_unknown_craft_apps() {
        let repos = json!([
            { "name": "photocraft", "full_name": "storytold/photocraft" },
            { "name": "AniCraft", "full_name": "storytold/AniCraft", "description": "AniCraft: 2D animation" },
            { "name": "craft-fonts", "full_name": "storytold/craft-fonts" },
            { "name": "oldcraft", "full_name": "storytold/oldcraft", "archived": true },
            { "name": "forkcraft", "full_name": "storytold/forkcraft", "fork": true },
            { "name": "egui", "full_name": "storytold/egui" }
        ]);
        let found = candidates(&repos);
        let ids: Vec<_> = found.iter().map(|c| c.id.as_str()).collect();
        // craft-fonts reads as a candidate; its probe finds no web build.
        assert_eq!(ids, ["anicraft", "craft-fonts"]);
        assert_eq!(found[0].repo, "storytold/AniCraft");
    }

    #[test]
    fn a_registered_app_joins_the_catalog_once() {
        let found = Found {
            id: "testregistercraft".into(),
            name: "TestRegisterCraft".into(),
            tagline: "Testing".into(),
            repo: "storytold/testregistercraft".into(),
            publisher: String::new(),
            community: false,
            color: [0, 0, 0],
        };
        assert!(register(&found));
        assert!(!register(&found), "a second registration is refused");
        let app = CraftApp::by_id("testregistercraft").expect("registered");
        assert!(app.discovered);
        assert_eq!(app.repo(), "storytold/testregistercraft");
        assert_eq!(app.publisher, "ArtCraft");
        assert_ne!(app.color, [0, 0, 0]);
        assert!(CraftApp::all().iter().any(|a| a.id == "testregistercraft"));
        assert!(!register(&Found { id: "photocraft".into(), ..found.clone() }));
        assert!(!register(&Found { id: "badrepocraft".into(), repo: "../x".into(), ..found }));
    }

    #[test]
    fn web_builds_are_named_after_the_app() {
        assert!(has_web_build("texturecraft", ["texturecraft-web-0.3.0.zip"]));
        assert!(!has_web_build("texturecraft", ["texturecraft-0.3.0-linux-x86_64.tar.gz"]));
        assert!(!has_web_build("craft", ["craft-anything-web-0.3.0.zip"]));
    }
}
