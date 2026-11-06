//! Import from another browser: history and bookmarks straight from the
//! profiles of browsers installed on this machine, passwords and bookmarks
//! from the files browsers export.
//!
//! # Profiles
//!
//! [`detect`] looks in the user's home for Firefox-family profiles (a
//! `places.sqlite`) and Chromium-family ones (a `History` / `Bookmarks` file),
//! in their usual, Snap and Flatpak places. The window only ever names a
//! profile by the id `detect` gave it — never a path — and the server
//! re-detects to find it. A profile database is **copied** (with its `-wal`)
//! before it is read: a running Firefox holds `places.sqlite` locked, and the
//! original is never written.
//!
//! Passwords are not read from profiles: Firefox and Chrome seal them with
//! the profile's or the desktop keyring's key. Every browser exports them as
//! CSV instead, and [`parse_passwords_csv`] reads the formats of Firefox,
//! Chrome/Edge/Brave, Safari, Bitwarden, KeePassXC and 1Password by their
//! header. [`parse_bookmarks_html`] reads the Netscape bookmark file every
//! browser exports.
//!
//! History brings each address once, at its last visit, newest first and at
//! most [`HISTORY_LIMIT`] of them; importing again adds nothing twice.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value as Json;

use shiny_plugin_sdk::db::{Db, Value};
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::services::PluginCtx;

use crate::{bookmarks, history, vault};

/// The most addresses one profile's history brings.
pub const HISTORY_LIMIT: i64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    Firefox,
    Chromium,
}

/// One browser profile found on this machine.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Source {
    /// What the window sends back to import it.
    pub id: String,
    /// "Firefox", "Google Chrome", …
    pub browser: String,
    /// The profile's own name ("default-release", "Person 1", …).
    pub profile: String,
    pub family: Family,
    pub history: bool,
    pub bookmarks: bool,
    #[serde(skip)]
    pub dir: PathBuf,
}

/// Where each browser keeps its profiles, relative to the home.
const FIREFOX_ROOTS: &[(&str, &str)] = &[
    ("Firefox", ".mozilla/firefox"),
    ("Firefox", "snap/firefox/common/.mozilla/firefox"),
    ("Firefox", ".var/app/org.mozilla.firefox/.mozilla/firefox"),
    ("LibreWolf", ".librewolf"),
    (
        "LibreWolf",
        ".var/app/io.gitlab.librewolf-community/.librewolf",
    ),
];
const CHROMIUM_ROOTS: &[(&str, &str)] = &[
    ("Google Chrome", ".config/google-chrome"),
    (
        "Google Chrome",
        ".var/app/com.google.Chrome/config/google-chrome",
    ),
    ("Chromium", ".config/chromium"),
    ("Chromium", "snap/chromium/common/chromium"),
    ("Chromium", ".var/app/org.chromium.Chromium/config/chromium"),
    ("Brave", ".config/BraveSoftware/Brave-Browser"),
    (
        "Brave",
        ".var/app/com.brave.Browser/config/BraveSoftware/Brave-Browser",
    ),
    ("Microsoft Edge", ".config/microsoft-edge"),
    ("Vivaldi", ".config/vivaldi"),
    ("Opera", ".config/opera"),
];

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// The profile names Chromium keeps in `Local State`, by directory.
fn chromium_names(root: &Path) -> serde_json::Map<String, Json> {
    std::fs::read_to_string(root.join("Local State"))
        .ok()
        .and_then(|text| serde_json::from_str::<Json>(&text).ok())
        .and_then(|v| {
            v.pointer("/profile/info_cache")
                .and_then(Json::as_object)
                .cloned()
        })
        .unwrap_or_default()
}

/// Every browser profile under `home` with something to import.
pub fn detect(home: &Path) -> Vec<Source> {
    let mut out = Vec::new();
    for (browser, rel) in FIREFOX_ROOTS {
        for dir in subdirs(&home.join(rel)) {
            if !dir.join("places.sqlite").is_file() {
                continue;
            }
            let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            // `abcd1234.default-release` → `default-release`.
            let profile = name
                .split_once('.')
                .map_or(name, |(_, rest)| rest)
                .to_string();
            out.push(Source {
                id: String::new(),
                browser: (*browser).into(),
                profile,
                family: Family::Firefox,
                history: true,
                bookmarks: true,
                dir,
            });
        }
    }
    for (browser, rel) in CHROMIUM_ROOTS {
        let root = home.join(rel);
        if !root.is_dir() {
            continue;
        }
        let names = chromium_names(&root);
        // Opera keeps its one profile in the root itself.
        let mut dirs = vec![root.clone()];
        dirs.extend(subdirs(&root));
        for dir in dirs {
            let history = dir.join("History").is_file();
            let bookmarks = dir.join("Bookmarks").is_file();
            if !history && !bookmarks {
                continue;
            }
            let key = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            let profile = names
                .get(&key)
                .and_then(|p| p.get("name"))
                .and_then(Json::as_str)
                .map(str::to_string)
                .unwrap_or(if dir == root { String::new() } else { key });
            out.push(Source {
                id: String::new(),
                browser: (*browser).into(),
                profile,
                family: Family::Chromium,
                history,
                bookmarks,
                dir,
            });
        }
    }
    for (i, source) in out.iter_mut().enumerate() {
        source.id = format!(
            "{}-{i}",
            match source.family {
                Family::Firefox => "firefox",
                Family::Chromium => "chromium",
            }
        );
    }
    out
}

/// The home the per-user server runs in.
pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/* ── Reading a profile ───────────────────────────────────────────── */

/// A profile database copied aside (with its write-ahead log), removed on drop.
struct Copied {
    dir: PathBuf,
    db: Db,
}

impl Copied {
    fn open(file: &Path) -> Result<Self, AppError> {
        let dir = std::env::temp_dir().join(format!("liteaura-import-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).map_err(|e| AppError::Internal(format!("import: {e}")))?;
        let copy = dir.join("profile.sqlite");
        let copied = std::fs::copy(file, &copy).and_then(|_| {
            let wal = PathBuf::from(format!("{}-wal", file.display()));
            if wal.is_file() {
                std::fs::copy(&wal, dir.join("profile.sqlite-wal"))?;
            }
            Ok(())
        });
        if let Err(e) = copied {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(AppError::Internal(format!(
                "import: could not read {}: {e}",
                file.display()
            )));
        }
        match Db::open(&format!("sqlite://{}", copy.display())) {
            Ok(db) => Ok(Self { dir, db }),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                Err(e)
            }
        }
    }
}

impl Drop for Copied {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// One visited address: `(url, last visit as YYYY-MM-DD HH:MM:SS UTC)`.
pub type Visit = (String, String);
/// One bookmark: `(url, title)`.
pub type Mark = (String, Option<String>);

fn text(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::Text(t)) if !t.is_empty() => Some(t.clone()),
        _ => None,
    }
}

fn web_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// History and bookmarks of one profile.
pub fn read_profile(source: &Source) -> Result<(Vec<Visit>, Vec<Mark>), AppError> {
    match source.family {
        Family::Firefox => {
            let copy = Copied::open(&source.dir.join("places.sqlite"))?;
            let visits = copy.db.query(
                "SELECT p.url, datetime(MAX(v.visit_date) / 1000000, 'unixepoch') \
                 FROM moz_places p JOIN moz_historyvisits v ON v.place_id = p.id \
                 WHERE p.url LIKE 'http%' GROUP BY p.id ORDER BY MAX(v.visit_date) DESC LIMIT ?1",
                &[Value::Int(HISTORY_LIMIT)],
            )?;
            let marks = copy.db.query(
                "SELECT p.url, b.title FROM moz_bookmarks b JOIN moz_places p ON p.id = b.fk \
                 WHERE b.type = 1 AND p.url LIKE 'http%' ORDER BY b.dateAdded",
                &[],
            )?;
            Ok((rows_to_visits(visits), rows_to_marks(marks)))
        }
        Family::Chromium => {
            let visits = if source.history {
                let copy = Copied::open(&source.dir.join("History"))?;
                // Chromium counts microseconds from 1601-01-01.
                rows_to_visits(copy.db.query(
                    "SELECT url, datetime(last_visit_time / 1000000 - 11644473600, 'unixepoch') \
                     FROM urls WHERE url LIKE 'http%' AND last_visit_time > 0 \
                     ORDER BY last_visit_time DESC LIMIT ?1",
                    &[Value::Int(HISTORY_LIMIT)],
                )?)
            } else {
                Vec::new()
            };
            let marks = if source.bookmarks {
                std::fs::read_to_string(source.dir.join("Bookmarks"))
                    .ok()
                    .map(|t| parse_chromium_bookmarks(&t))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            Ok((visits, marks))
        }
    }
}

fn rows_to_visits(rows: Vec<Vec<Value>>) -> Vec<Visit> {
    rows.into_iter()
        .filter_map(|row| Some((text(row.first())?, text(row.get(1))?)))
        .filter(|(url, _)| web_url(url))
        .collect()
}

fn rows_to_marks(rows: Vec<Vec<Value>>) -> Vec<Mark> {
    rows.into_iter()
        .filter_map(|row| Some((text(row.first())?, text(row.get(1)))))
        .filter(|(url, _)| web_url(url))
        .collect()
}

/// The bookmarks in Chromium's `Bookmarks` JSON, every folder flattened.
pub fn parse_chromium_bookmarks(text: &str) -> Vec<Mark> {
    fn walk(node: &Json, out: &mut Vec<Mark>) {
        if node.get("type").and_then(Json::as_str) == Some("url") {
            if let Some(url) = node
                .get("url")
                .and_then(Json::as_str)
                .filter(|u| web_url(u))
            {
                let title = node
                    .get("name")
                    .and_then(Json::as_str)
                    .filter(|t| !t.is_empty());
                out.push((url.to_string(), title.map(str::to_string)));
            }
        }
        if let Some(kids) = node.get("children").and_then(Json::as_array) {
            for kid in kids {
                walk(kid, out);
            }
        }
    }
    let mut out = Vec::new();
    if let Some(roots) = serde_json::from_str::<Json>(text)
        .ok()
        .and_then(|v| v.get("roots").cloned())
    {
        if let Some(map) = roots.as_object() {
            for root in map.values() {
                walk(root, &mut out);
            }
        }
    }
    out
}

/* ── Files ───────────────────────────────────────────────────────── */

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let Some(semi) = tail.find(';').filter(|i| *i <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[1..semi];
        let ch = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The bookmarks in a Netscape bookmark file (what every browser exports as
/// "bookmarks.html"), every folder flattened.
pub fn parse_bookmarks_html(text: &str) -> Vec<Mark> {
    let lower = text.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = lower[from..].find("<a ") {
        let open = from + i;
        let Some(gt) = lower[open..].find('>').map(|g| open + g) else {
            break;
        };
        let tag = &text[open..gt];
        let close = lower[gt..].find("</a").map_or(lower.len(), |c| gt + c);
        from = close.max(gt + 1);
        let tag_lower = &lower[open..gt];
        let Some(h) = tag_lower.find("href=") else {
            continue;
        };
        let value = tag[h + 5..].trim_start();
        let href = match value.chars().next() {
            Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or_default(),
            _ => value
                .split(|c: char| c.is_whitespace())
                .next()
                .unwrap_or_default(),
        };
        let url = decode_entities(href.trim());
        if !web_url(&url) {
            continue;
        }
        // The title, without any markup inside the link.
        let mut title = String::new();
        let mut in_tag = false;
        for c in text[gt + 1..close].chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                c if !in_tag => title.push(c),
                _ => {}
            }
        }
        let title = decode_entities(title.trim());
        out.push((url, (!title.is_empty()).then_some(title)));
    }
    out
}

/// Whether `text` is a Netscape bookmark file rather than a CSV.
pub fn looks_like_bookmarks_html(text: &str) -> bool {
    let head: String = text
        .chars()
        .take(2048)
        .collect::<String>()
        .to_ascii_lowercase();
    head.contains("netscape-bookmark-file") || head.contains("<dl") || head.contains("<!doctype")
}

/// The rows of a CSV file (RFC 4180: quoted fields with commas, quotes and
/// line breaks inside). A leading byte-order mark is dropped.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                c => field.push(c),
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                if row.iter().any(|f| !f.is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            c => field.push(c),
        }
    }
    row.push(field);
    if row.iter().any(|f| !f.is_empty()) {
        rows.push(row);
    }
    rows
}

/// The logins in a password CSV, by its header: Firefox (`url, username,
/// password, …`), Chrome/Edge/Brave (`name, url, username, password, note`),
/// Safari (`Title, URL, Username, Password, Notes, …`), Bitwarden
/// (`login_uri, login_username, login_password, notes`), KeePassXC and
/// 1Password. `None` when the file has no address and password columns.
pub fn parse_passwords_csv(text: &str) -> Option<Vec<vault::SaveRequest>> {
    let mut rows = parse_csv(text).into_iter();
    let header: Vec<String> = rows
        .next()?
        .iter()
        .map(|h| h.trim().to_ascii_lowercase())
        .collect();
    let col = |names: &[&str]| header.iter().position(|h| names.contains(&h.as_str()));
    let url = col(&[
        "url",
        "login_uri",
        "website",
        "web site",
        "origin",
        "uri",
        "login url",
        "site",
    ])?;
    let password = col(&["password", "login_password"])?;
    let username = col(&[
        "username",
        "login_username",
        "user name",
        "login",
        "email",
        "user",
    ]);
    let note = col(&["note", "notes", "comments"]);
    let get =
        |row: &[String], i: Option<usize>| i.and_then(|i| row.get(i)).cloned().unwrap_or_default();
    Some(
        rows.map(|row| vault::SaveRequest {
            id: None,
            site: get(&row, Some(url)),
            username: get(&row, username),
            password: get(&row, Some(password)),
            note: get(&row, note),
        })
        .collect(),
    )
}

/* ── Writing ─────────────────────────────────────────────────────── */

/// What an import of history and bookmarks added.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Added {
    pub history: usize,
    pub bookmarks: usize,
}

/// Add visits to the user's history: one transaction, each `(url, time)`
/// once however often it is imported.
pub fn add_history(ctx: &PluginCtx, user_id: &str, visits: &[Visit]) -> Result<usize, AppError> {
    let table = history::TABLE;
    let db = ctx.db();
    db.execute("BEGIN IMMEDIATE", &[])?;
    let mut added = 0;
    for (url, at) in visits {
        let n = db.execute(
            &format!(
                "INSERT INTO {table} (id, traveler_id, url, mode, created_at) \
                 SELECT ?1, ?2, ?3, 'page', ?4 \
                 WHERE EXISTS (SELECT 1 FROM travelers WHERE id = ?2) \
                 AND NOT EXISTS (SELECT 1 FROM {table} WHERE traveler_id = ?2 AND url = ?3 AND created_at = ?4)"
            ),
            &[
                Value::text(uuid::Uuid::new_v4().to_string()),
                Value::text(user_id.to_string()),
                Value::text(url.clone()),
                Value::text(at.clone()),
            ],
        );
        match n {
            Ok(n) => added += n,
            Err(e) => {
                let _ = db.execute("ROLLBACK", &[]);
                return Err(e);
            }
        }
    }
    db.execute("COMMIT", &[])?;
    Ok(added)
}

/// Add bookmarks (an address already saved keeps its row).
pub fn add_bookmarks(ctx: &Arc<PluginCtx>, user_id: &str, marks: &[Mark]) -> usize {
    let before = bookmarks::count(ctx, user_id);
    for (url, title) in marks {
        let _ = bookmarks::add(ctx, user_id, url, title.as_deref());
    }
    bookmarks::count(ctx, user_id).saturating_sub(before)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_handles_quotes_commas_and_line_breaks() {
        let rows = parse_csv("\u{feff}a,b,c\r\n\"x, y\",\"he said \"\"hi\"\"\",\"two\nlines\"\n\n");
        assert_eq!(
            rows,
            vec![
                vec!["a".to_string(), "b".into(), "c".into()],
                vec![
                    "x, y".to_string(),
                    "he said \"hi\"".into(),
                    "two\nlines".into()
                ],
            ]
        );
    }

    #[test]
    fn password_files_of_every_browser_are_read() {
        let firefox = "\"url\",\"username\",\"password\",\"httpRealm\",\"formActionOrigin\",\"guid\",\"timeCreated\",\"timeLastUsed\",\"timePasswordChanged\"\n\"https://example.com\",\"me\",\"p,w\"\"1\",,\"https://example.com\",\"{x}\",\"1\",\"2\",\"3\"\n";
        let chrome = "name,url,username,password,note\nexample.com,https://example.com/login,me,pw2,a note\n";
        let bitwarden = "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n,,login,Ex,n,,0,https://example.com,me,pw3,\n";
        let safari =
            "Title,URL,Username,Password,Notes,OTPAuth\nEx,https://example.com/,me,pw4,,\n";
        for (file, password) in [
            (firefox, "p,w\"1"),
            (chrome, "pw2"),
            (bitwarden, "pw3"),
            (safari, "pw4"),
        ] {
            let logins = parse_passwords_csv(file).expect("a password file");
            assert_eq!(logins.len(), 1, "{file}");
            assert!(logins[0].site.starts_with("https://example.com"));
            assert_eq!(logins[0].username, "me");
            assert_eq!(logins[0].password, password);
        }
        assert_eq!(parse_passwords_csv(chrome).unwrap()[0].note, "a note");
        assert!(
            parse_passwords_csv("a,b\n1,2\n").is_none(),
            "no address/password columns"
        );
    }

    #[test]
    fn netscape_bookmarks_are_read() {
        let html = "<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<DL><p>\n<DT><H3>Folder</H3>\n<DL><p>\n\
            <DT><A HREF=\"https://example.com/?a=1&amp;b=2\" ADD_DATE=\"1\">Ex &amp; <b>co</b></A>\n\
            <DT><A HREF='https://two.example'>Two</A>\n<DT><A HREF=\"place:sort=8\">Recent</A>\n\
            <DT><A HREF=\"javascript:alert(1)\">x</A>\n</DL><p>\n</DL>";
        assert!(looks_like_bookmarks_html(html));
        assert!(!looks_like_bookmarks_html("url,username,password\n"));
        assert_eq!(
            parse_bookmarks_html(html),
            vec![
                (
                    "https://example.com/?a=1&b=2".to_string(),
                    Some("Ex & co".to_string())
                ),
                ("https://two.example".to_string(), Some("Two".to_string())),
            ]
        );
    }

    #[test]
    fn chromium_bookmarks_are_flattened() {
        let json = r#"{"roots":{"bookmark_bar":{"type":"folder","children":[
            {"type":"url","name":"A","url":"https://a.example/"},
            {"type":"folder","children":[{"type":"url","name":"","url":"https://b.example/"}]}]},
            "other":{"type":"folder","children":[{"type":"url","name":"C","url":"chrome://flags"}]}}}"#;
        assert_eq!(
            parse_chromium_bookmarks(json),
            vec![
                ("https://a.example/".to_string(), Some("A".to_string())),
                ("https://b.example/".to_string(), None),
            ]
        );
    }

    #[test]
    fn profiles_are_found_and_read() {
        let home =
            std::env::temp_dir().join(format!("liteaura-import-home-{}", uuid::Uuid::new_v4()));
        // Firefox, with a visit and a bookmark.
        let ff = home.join(".mozilla/firefox/ab12cd34.default-release");
        std::fs::create_dir_all(&ff).unwrap();
        let places = Db::open(&format!("sqlite://{}", ff.join("places.sqlite").display())).unwrap();
        for sql in [
            "CREATE TABLE moz_places (id INTEGER PRIMARY KEY, url TEXT, title TEXT)",
            "CREATE TABLE moz_historyvisits (id INTEGER PRIMARY KEY, place_id INTEGER, visit_date INTEGER)",
            "CREATE TABLE moz_bookmarks (id INTEGER PRIMARY KEY, type INTEGER, fk INTEGER, title TEXT, dateAdded INTEGER)",
            "INSERT INTO moz_places VALUES (1, 'https://ff.example/', 'FF'), (2, 'about:config', NULL)",
            "INSERT INTO moz_historyvisits VALUES (1, 1, 1700000000000000), (2, 1, 1600000000000000), (3, 2, 1700000000000000)",
            "INSERT INTO moz_bookmarks VALUES (1, 1, 1, 'FF mark', 1), (2, 2, NULL, 'folder', 1)",
        ] {
            places.execute(sql, &[]).unwrap();
        }
        drop(places);
        // Chrome, with a named profile.
        let chrome = home.join(".config/google-chrome");
        std::fs::create_dir_all(chrome.join("Default")).unwrap();
        std::fs::write(
            chrome.join("Local State"),
            r#"{"profile":{"info_cache":{"Default":{"name":"Work"}}}}"#,
        )
        .unwrap();
        std::fs::write(chrome.join("Default/Bookmarks"),
            r#"{"roots":{"bookmark_bar":{"children":[{"type":"url","name":"G","url":"https://g.example/"}]}}}"#).unwrap();
        let hist = Db::open(&format!(
            "sqlite://{}",
            chrome.join("Default/History").display()
        ))
        .unwrap();
        hist.execute("CREATE TABLE urls (id INTEGER PRIMARY KEY, url TEXT, title TEXT, last_visit_time INTEGER)", &[]).unwrap();
        hist.execute(
            "INSERT INTO urls VALUES (1, 'https://g.example/', 'G', 13340000000000000)",
            &[],
        )
        .unwrap();
        drop(hist);

        let sources = detect(&home);
        assert_eq!(sources.len(), 2, "{sources:?}");
        let ff = sources
            .iter()
            .find(|s| s.family == Family::Firefox)
            .unwrap();
        assert_eq!(
            (ff.browser.as_str(), ff.profile.as_str()),
            ("Firefox", "default-release")
        );
        let gc = sources
            .iter()
            .find(|s| s.family == Family::Chromium)
            .unwrap();
        assert_eq!(
            (gc.browser.as_str(), gc.profile.as_str()),
            ("Google Chrome", "Work")
        );

        let (visits, marks) = read_profile(ff).unwrap();
        assert_eq!(
            visits,
            vec![(
                "https://ff.example/".to_string(),
                "2023-11-14 22:13:20".to_string()
            )]
        );
        assert_eq!(
            marks,
            vec![(
                "https://ff.example/".to_string(),
                Some("FF mark".to_string())
            )]
        );
        let (visits, marks) = read_profile(gc).unwrap();
        assert_eq!(
            visits,
            vec![(
                "https://g.example/".to_string(),
                "2023-09-24 03:33:20".to_string()
            )]
        );
        assert_eq!(marks.len(), 1);
        assert!(
            !std::fs::read_dir(std::env::temp_dir())
                .unwrap()
                .flatten()
                .any(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with("liteaura-import-")
                        && e.path().join("profile.sqlite").exists()
                        && e.path() != home
                }),
            "the copies are removed"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn imported_history_is_added_once() {
        let path = "/tmp/browser-import-history-test.db";
        let _ = std::fs::remove_file(path);
        let ctx = crate::history::tests_support::ctx_for(path);
        let db = ctx.db();
        db.execute(&format!(
            "CREATE TABLE {} (id TEXT PRIMARY KEY, traveler_id TEXT NOT NULL, url TEXT NOT NULL, \
             mode TEXT NOT NULL DEFAULT 'page', query TEXT, created_at TEXT NOT NULL DEFAULT (datetime('now')))",
            history::TABLE
        ), &[]).unwrap();
        db.execute("CREATE TABLE travelers (id TEXT PRIMARY KEY)", &[])
            .unwrap();
        db.execute("INSERT INTO travelers VALUES ('u1')", &[])
            .unwrap();
        let visits = vec![
            (
                "https://a.example/".to_string(),
                "2024-01-01 10:00:00".to_string(),
            ),
            (
                "https://b.example/".to_string(),
                "2024-01-02 10:00:00".to_string(),
            ),
        ];
        assert_eq!(add_history(&ctx, "u1", &visits).unwrap(), 2);
        assert_eq!(
            add_history(&ctx, "u1", &visits).unwrap(),
            0,
            "importing again adds nothing"
        );
        assert_eq!(
            add_history(&ctx, "nobody", &visits).unwrap(),
            0,
            "no account, no rows"
        );
        let _ = std::fs::remove_file(path);
    }
}
