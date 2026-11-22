//! The checks a plugin's sources pass before it is packed.
//!
//! Each check reads the workspace and returns findings with a file and, where
//! it has one, a line: the assistant fixes what is named and runs the check
//! again. They are static: nothing of the plugin is executed.
//!
//! A plugin the assistant builds follows two rules that make clashes with
//! other plugins impossible by construction, and the checks hold it to them:
//! its tools are named `<name>_…` and its routes live under `/api/<name>/`.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use super::workspace::Workspace;

/// The checks, in the order they run.
pub const CHECKS: &[&str] = &["manifest", "web", "i18n", "migrations", "conflicts"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub check: &'static str,
    /// An error blocks packing; a warning does not.
    pub error: bool,
    pub file: String,
    pub line: usize,
    pub message: String,
}

impl Finding {
    fn to_json(&self) -> Value {
        let mut value = json!({ "check": self.check, "message": self.message });
        if !self.file.is_empty() {
            value["file"] = json!(self.file);
        }
        if self.line > 0 {
            value["line"] = json!(self.line);
        }
        value
    }
}

/// What the checks need to know about the rest of the system.
#[derive(Debug, Clone, Default)]
pub struct Surroundings {
    /// Tools other plugins register, with their owner.
    pub foreign_tools: BTreeMap<String, String>,
    /// The installed copy of this plugin, when there is one.
    pub installed_dir: Option<std::path::PathBuf>,
    /// The installed plugin of this name is the read-only system one.
    pub overrides_system: bool,
    /// The name is a Craft app's.
    pub is_craft_app: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub ran: Vec<&'static str>,
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn ok(&self) -> bool {
        !self.findings.iter().any(|finding| finding.error)
    }

    pub fn to_json(&self) -> Value {
        let list = |error: bool| {
            self.findings
                .iter()
                .filter(|finding| finding.error == error)
                .map(Finding::to_json)
                .collect::<Vec<_>>()
        };
        json!({ "ok": self.ok(), "checked": self.ran, "errors": list(true), "warnings": list(false) })
    }
}

/// Run one check, or all of them.
pub fn run(workspace: &Workspace, only: Option<&str>, around: &Surroundings) -> Result<Report, String> {
    let wanted: Vec<&'static str> = match only {
        None => CHECKS.to_vec(),
        Some(name) => match CHECKS.iter().find(|check| **check == name) {
            Some(check) => vec![check],
            None => return Err(format!("unknown check '{name}': use one of {}", CHECKS.join(", "))),
        },
    };
    let mut report = Report { ran: wanted.clone(), findings: Vec::new() };
    let sources = Sources::read(workspace);
    for check in wanted {
        let mut found = Found { check, findings: &mut report.findings };
        match check {
            "manifest" => manifest(workspace, &sources, &mut found),
            "web" => web(&sources, &mut found),
            "i18n" => i18n(workspace, &sources, &mut found),
            "migrations" => migrations(&sources, around, &mut found),
            _ => conflicts(workspace, &sources, around, &mut found),
        }
    }
    Ok(report)
}

/// The workspace's text files, read once.
struct Sources {
    files: BTreeMap<String, String>,
}

impl Sources {
    fn read(workspace: &Workspace) -> Self {
        let files = workspace
            .files()
            .into_iter()
            .filter_map(|entry| {
                let text = std::fs::read_to_string(workspace.dir.join(&entry.path)).ok()?;
                Some((entry.path, text))
            })
            .collect();
        Self { files }
    }

    fn get(&self, path: &str) -> Option<&str> {
        self.files.get(path).map(String::as_str)
    }

    fn under<'a>(&'a self, prefix: &'a str, extension: &'a str) -> impl Iterator<Item = (&'a str, &'a str)> {
        self.files
            .iter()
            .filter(move |(path, _)| path.starts_with(prefix) && path.ends_with(extension))
            .map(|(path, text)| (path.as_str(), text.as_str()))
    }
}

struct Found<'a> {
    check: &'static str,
    findings: &'a mut Vec<Finding>,
}

impl Found<'_> {
    fn error(&mut self, file: &str, line: usize, message: impl Into<String>) {
        self.push(true, file, line, message.into());
    }

    fn warning(&mut self, file: &str, line: usize, message: impl Into<String>) {
        self.push(false, file, line, message.into());
    }

    fn push(&mut self, error: bool, file: &str, line: usize, message: String) {
        self.findings.push(Finding { check: self.check, error, file: file.to_string(), line, message });
    }
}

/// The line (1-based) a byte offset falls on.
fn line_of(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].bytes().filter(|byte| *byte == b'\n').count() + 1
}

/// Every string literal that directly follows `marker` (after optional
/// spaces), with the offset of the marker. Quotes: `'`, `"` and `` ` ``.
fn literals_after<'a>(text: &'a str, marker: &str) -> Vec<(usize, &'a str)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(found) = text[from..].find(marker) {
        let at = from + found;
        let rest = &text[at + marker.len()..];
        let trimmed = rest.trim_start();
        if let Some(quote) = trimmed.chars().next().filter(|c| matches!(c, '\'' | '"' | '`')) {
            let body = &trimmed[1..];
            if let Some(end) = body.find(quote) {
                out.push((at, &body[..end]));
            }
        }
        from = at + marker.len();
    }
    out
}

// ── manifest ─────────────────────────────────────────────────────────────

fn manifest(workspace: &Workspace, sources: &Sources, found: &mut Found<'_>) {
    const FILE: &str = "plugin.toml";
    let Some(text) = sources.get(FILE) else {
        found.error(FILE, 0, "plugin.toml is missing: every plugin has one at its root");
        return;
    };
    let table: toml::Value = match toml::from_str(text) {
        Ok(table) => table,
        Err(error) => {
            let line = error.span().map(|span| line_of(text, span.start)).unwrap_or(0);
            found.error(FILE, line, format!("not valid TOML: {}", error.message()));
            return;
        }
    };
    let line = |key: &str| {
        text.lines()
            .position(|line| line.trim_start().starts_with(key) && line.contains('='))
            .map_or(0, |index| index + 1)
    };
    let string = |key: &str| table.get(key).and_then(toml::Value::as_str).map(str::trim).unwrap_or_default();

    match string("name") {
        "" => found.error(FILE, 0, "name is missing"),
        name if name != workspace.name => found.error(
            FILE,
            line("name"),
            format!("name is \"{name}\" but this plugin is \"{}\": they must be the same", workspace.name),
        ),
        _ => {}
    }
    match string("version") {
        "" => found.error(FILE, 0, "version is missing (for example \"0.1.0\")"),
        version if semver::Version::parse(version).is_err() => found.error(
            FILE,
            line("version"),
            format!("version \"{version}\" is not a version number like \"0.1.0\""),
        ),
        _ => {}
    }
    match table.get("api_level").and_then(toml::Value::as_integer) {
        None => found.error(FILE, line("api_level"), "api_level is missing: it must be 1"),
        Some(level) if level < 1 || level > i64::from(shiny_plugin_sdk::CORE_API_LEVEL) => found.error(
            FILE,
            line("api_level"),
            format!("api_level {level} is not supported: it must be {}", shiny_plugin_sdk::CORE_API_LEVEL),
        ),
        Some(_) => {}
    }
    if string("runtime") != "process" {
        found.error(
            FILE,
            line("runtime"),
            "runtime must be \"process\": a plugin built here runs as its own process",
        );
    }
    if !matches!(string("kind"), "" | "native") {
        found.error(FILE, line("kind"), "kind must be left out: a plugin built here has an engine and a window");
    }
    if string("description").is_empty() {
        found.error(FILE, line("description"), "description is missing: one line saying what the plugin does");
    }
    if string("category").is_empty() {
        found.warning(FILE, 0, "category is missing: the plugin tray groups plugins by it (Office, Media, System…)");
    }
    for key in ["migrations_dir", "skills_dir", "web_dir"] {
        if let Some(value) = table.get(key).and_then(toml::Value::as_str) {
            if let Err(error) = crate::plugins::loader::validate_relative_path(key, value) {
                found.error(FILE, line(key), error.to_string());
            }
        }
    }
    // What the server itself will parse.
    if let Err(error) = toml::from_str::<shiny_plugin_sdk::manifest::Manifest>(text) {
        if found.findings.iter().all(|finding| finding.check != "manifest" || !finding.error) {
            found.error(FILE, 0, format!("the server cannot read this manifest: {}", error.message()));
        }
    }
}

// ── web ──────────────────────────────────────────────────────────────────

fn web(sources: &Sources, found: &mut Found<'_>) {
    const ENTRY: &str = "web/plugin.js";
    let Some(entry) = sources.get(ENTRY) else {
        found.error(ENTRY, 0, "web/plugin.js is missing: it is the plugin's window");
        return;
    };
    if !entry.contains("export default") {
        found.error(
            ENTRY,
            0,
            "no `export default`: the window is the module's default export, with mount, unmount, getElement and wireEvents",
        );
    } else {
        for member in ["mount", "unmount", "getElement"] {
            if !entry.contains(member) {
                found.error(ENTRY, 0, format!("the default export has no `{member}`"));
            }
        }
    }
    if !entry.contains("dataset.plugin") && !entry.contains("data-plugin") {
        found.error(
            ENTRY,
            0,
            "the window element never gets `data-plugin`: set `tile.dataset.plugin` to the plugin's name in mount()",
        );
    }
    if !sources.files.contains_key("web/icon.svg") && !sources.files.contains_key("web/icon.png") {
        found.warning("web/icon.svg", 0, "no web/icon.svg: the tray and the launcher show a generic icon");
    }

    for (path, text) in sources.under("web/", ".js") {
        for marker in [" from ", "import(", "import "] {
            for (at, specifier) in literals_after(text, marker) {
                // `import x from './y.js'` is found by " from "; a bare
                // `import './y.js'` by "import ". Anything else is not a path.
                let relative = specifier.starts_with("./") || specifier.starts_with("../");
                let looks_like_module = specifier.ends_with(".js") || specifier.ends_with(".mjs")
                    || specifier.starts_with('/') || specifier.starts_with("http");
                if !relative && looks_like_module {
                    found.error(
                        path,
                        line_of(text, at),
                        format!(
                            "import \"{specifier}\" is not relative: import core modules as '../../ui/index.js' or \
                             '../../js/…', and the plugin's own as './…'"
                        ),
                    );
                }
            }
        }
        for (needle, why) in [
            ("eval(", "`eval` is refused by the app's Content-Security-Policy"),
            ("new Function(", "`new Function` is refused by the app's Content-Security-Policy"),
            ("<script", "a `<script>` built in a string never runs: the Content-Security-Policy forbids inline scripts"),
            ("document.write(", "`document.write` would replace the whole desktop"),
        ] {
            if let Some(at) = text.find(needle) {
                found.error(path, line_of(text, at), why);
            }
        }
        for needle in ["onclick=", "onload=", "onerror=", "oninput="] {
            if let Some(at) = text.find(needle) {
                found.error(
                    path,
                    line_of(text, at),
                    format!("inline handler `{needle}…` never runs under the Content-Security-Policy: use addEventListener"),
                );
            }
        }
        if let Some(at) = text.find(".innerHTML") {
            found.warning(
                path,
                line_of(text, at),
                "innerHTML with data from the server or the user injects markup: build elements and set textContent",
            );
        }
        for (at, url) in literals_after(text, "fetch(") {
            if url.starts_with("http://") || url.starts_with("https://") {
                found.error(
                    path,
                    line_of(text, at),
                    format!(
                        "the window fetches \"{url}\" directly: the Content-Security-Policy only allows this server. \
                         Fetch it in the engine and expose a route."
                    ),
                );
            }
        }
    }
    for (path, text) in sources.under("web/", ".css") {
        if let Some(at) = text.find("@import") {
            found.error(path, line_of(text, at), "@import pulls a stylesheet the app does not ship: keep the rules in this file");
        }
        let hard_coded = text
            .lines()
            .enumerate()
            .find(|(_, line)| !line.trim_start().starts_with("/*") && (line.contains("#") && is_hex_colour(line)));
        if let Some((index, _)) = hard_coded {
            found.warning(
                path,
                index + 1,
                "a hard-coded colour: use the theme's tokens (var(--text), var(--muted), var(--accent)…) so both themes work",
            );
        }
    }
}

/// Whether a CSS line holds a `#rgb` / `#rrggbb` colour (not an id selector).
fn is_hex_colour(line: &str) -> bool {
    let Some(colon) = line.find(':') else { return false };
    line[colon..].split('#').skip(1).any(|rest| {
        let digits = rest.chars().take_while(char::is_ascii_hexdigit).count();
        matches!(digits, 3 | 4 | 6 | 8)
    })
}

// ── i18n ─────────────────────────────────────────────────────────────────

/// Whether the line a byte offset falls on is a comment line.
fn in_comment(text: &str, offset: usize) -> bool {
    let start = text[..offset.min(text.len())].rfind('\n').map_or(0, |index| index + 1);
    let line = text[start..].lines().next().unwrap_or_default().trim_start();
    line.starts_with("//") || line.starts_with('*') || line.starts_with("/*")
}

fn i18n(workspace: &Workspace, sources: &Sources, found: &mut Found<'_>) {
    const REFERENCE: &str = "web/i18n/en.json";
    let prefix = format!("plugin.{}.", workspace.name);
    let mut reference: Option<BTreeSet<String>> = None;
    let mut catalogues: Vec<(&str, BTreeSet<String>)> = Vec::new();
    for (path, text) in sources.under("web/i18n/", ".json") {
        let parsed: Value = match serde_json::from_str(text) {
            Ok(parsed) => parsed,
            Err(error) => {
                found.error(path, error.line(), format!("not valid JSON: {error}"));
                continue;
            }
        };
        let Some(map) = parsed.as_object() else {
            found.error(path, 1, "a catalogue is one flat object of \"key\": \"text\" pairs");
            continue;
        };
        for (key, value) in map {
            if !value.is_string() {
                found.error(path, 0, format!("\"{key}\" is not a string: a catalogue is flat, no nested objects"));
            }
            if !key.starts_with(&prefix) {
                found.error(path, 0, format!("\"{key}\" must start with \"{prefix}\""));
            }
        }
        let keys: BTreeSet<String> = map.keys().cloned().collect();
        if path == REFERENCE {
            reference = Some(keys);
        } else {
            catalogues.push((path, keys));
        }
    }

    let used: Vec<(&str, usize, &str)> = sources
        .under("web/", ".js")
        .flat_map(|(path, text)| {
            ["t(", "tp("]
                .into_iter()
                .flat_map(move |marker| literals_after(text, marker))
                .filter(|(_, key)| key.starts_with("plugin."))
                // A key quoted in a comment is not a key in use.
                .filter(move |(at, _)| !in_comment(text, *at))
                .map(move |(at, key)| (path, line_of(text, at), key))
        })
        .collect();

    let Some(reference) = reference else {
        if !used.is_empty() {
            found.error(REFERENCE, 0, "web/i18n/en.json is missing: it is the reference catalogue the window's keys are read from");
        }
        return;
    };
    let mut reported: BTreeSet<&str> = BTreeSet::new();
    for (path, line, key) in used {
        if !reference.contains(key) && reported.insert(key) {
            found.error(path, line, format!("the key \"{key}\" is used here but is not in {REFERENCE}"));
        }
    }
    for (path, keys) in catalogues {
        for key in keys.difference(&reference) {
            found.error(path, 0, format!("\"{key}\" is not in {REFERENCE}: a translation only translates the reference's keys"));
        }
    }
}

// ── migrations ───────────────────────────────────────────────────────────

/// `001_init.sql` → 1. `None` when the name is not `NNN_name.sql`.
fn migration_number(file: &str) -> Option<u32> {
    let stem = file.strip_suffix(".sql")?;
    let (number, name) = stem.split_once('_')?;
    let named = !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    (number.len() == 3 && number.chars().all(|c| c.is_ascii_digit()) && named).then(|| number.parse().ok())?
}

fn migrations(sources: &Sources, around: &Surroundings, found: &mut Found<'_>) {
    let mut numbers: BTreeMap<u32, &str> = BTreeMap::new();
    for (path, text) in sources.under("migrations/", "") {
        let file = path.trim_start_matches("migrations/");
        if file.contains('/') {
            found.error(path, 0, "migrations are files directly inside migrations/, not in a subfolder");
            continue;
        }
        let Some(number) = migration_number(file) else {
            found.error(path, 0, "a migration is named NNN_what_it_does.sql (for example 002_add_tags.sql)");
            continue;
        };
        if let Some(other) = numbers.insert(number, path) {
            found.error(path, 0, format!("number {number:03} is already used by {other}"));
        }
        if text.trim().is_empty() {
            found.error(path, 0, "this migration is empty");
        }
        // A migration that has run is recorded by its file name and never run
        // again: editing it changes nothing on a machine that already has it.
        if let Some(installed) = around.installed_dir.as_deref() {
            if let Ok(applied) = std::fs::read_to_string(installed.join("migrations").join(file)) {
                if applied != text {
                    found.error(
                        path,
                        0,
                        "this migration is already installed with other content and will not run again: restore it \
                         and put the change in a new file with the next number",
                    );
                }
            }
        }
    }
    if let Some(installed) = around.installed_dir.as_deref() {
        let Ok(entries) = std::fs::read_dir(installed.join("migrations")) else { return };
        for entry in entries.flatten() {
            let file = entry.file_name().to_string_lossy().into_owned();
            let path = format!("migrations/{file}");
            if file.ends_with(".sql") && sources.get(&path).is_none() {
                found.error(&path, 0, "this migration is installed but missing here: a migration that has run is never removed");
            }
        }
    }
}

// ── conflicts ────────────────────────────────────────────────────────────

/// Something an engine declares: the file and line it is declared at, and its
/// name (a tool) or path (a route).
type Declared = (String, usize, String);

/// The tool names and route paths a plugin's engine declares, read from its
/// sources: `fn name(&self) -> &str { "…" }` and `RouteSpec::get("…", …)` in
/// Rust, `@plugin.tool("…")` and `@plugin.route("GET", "…")` in Python.
fn declared(sources: &Sources) -> (Vec<Declared>, Vec<Declared>) {
    let mut tools = Vec::new();
    let mut routes = Vec::new();
    for (path, text) in sources.under("src/", ".rs") {
        for (at, _) in text.match_indices("fn name(&self)") {
            let rest = &text[at..(at + 200).min(text.len())];
            if let Some(body) = rest.find('{') {
                if let Some((_, name)) = literals_after(&rest[body..], "{").first() {
                    tools.push((path.to_string(), line_of(text, at), name.to_string()));
                }
            }
        }
        for method in ["get", "post", "put", "delete", "patch"] {
            for (at, route) in literals_after(text, &format!("RouteSpec::{method}(")) {
                routes.push((path.to_string(), line_of(text, at), route.to_string()));
            }
        }
    }
    for (path, text) in sources.under("engine/", ".py") {
        // The kit's own docstring shows an example: it declares nothing.
        if path == crate::services::plugin_dev::python::KIT_PATH {
            continue;
        }
        for (at, name) in literals_after(text, "plugin.tool(") {
            tools.push((path.to_string(), line_of(text, at), name.to_string()));
        }
        // `@plugin.route("GET", "/api/…")`: the path is the second literal.
        for (at, method) in literals_after(text, "plugin.route(") {
            let rest = &text[at..];
            let after_method = rest.find(method).map(|index| &rest[index + method.len() + 1..]).unwrap_or("");
            if let Some((_, route)) = literals_after(after_method, ",").first() {
                routes.push((path.to_string(), line_of(text, at), route.to_string()));
            }
        }
    }
    (tools, routes)
}

fn conflicts(workspace: &Workspace, sources: &Sources, around: &Surroundings, found: &mut Found<'_>) {
    let name = &workspace.name;
    if around.is_craft_app {
        found.error("plugin.toml", 0, format!("\"{name}\" is the name of a Craft app: choose another name"));
    }
    if around.overrides_system {
        found.warning(
            "plugin.toml",
            0,
            format!("installing this replaces the system plugin \"{name}\" for this account until it is uninstalled"),
        );
    }
    let ident = name.replace('-', "_");
    let (tools, routes) = declared(sources);
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for (file, line, tool) in &tools {
        if !tool.starts_with(&format!("{ident}_")) {
            found.error(file, *line, format!("the tool \"{tool}\" must be named \"{ident}_…\" so it cannot clash with another plugin's"));
        }
        if let Some(owner) = around.foreign_tools.get(tool) {
            found.error(file, *line, format!("the tool \"{tool}\" already belongs to the plugin \"{owner}\""));
        }
        if !seen.insert(tool.as_str()) {
            found.error(file, *line, format!("the tool \"{tool}\" is declared twice"));
        }
    }
    let base = format!("/api/{name}");
    for (file, line, route) in &routes {
        if route != &base && !route.starts_with(&format!("{base}/")) {
            found.error(file, *line, format!("the route \"{route}\" must be under \"{base}/\""));
        }
    }

    // The skill is the only thing that tells the assistant the tools exist.
    let skills: Vec<&str> = sources.under("skills/", ".md").map(|(_, text)| text).collect();
    if !tools.is_empty() {
        if skills.iter().all(|text| text.trim().is_empty()) {
            found.error("skills/", 0, "there is no skill: without skills/<name>.md the assistant never learns these tools exist");
        } else {
            for (_, _, tool) in &tools {
                if !skills.iter().any(|text| text.contains(tool.as_str())) {
                    found.error(
                        &format!("skills/{name}.md"),
                        0,
                        format!("the skill never names the tool \"{tool}\": document it with its parameters"),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::plugin_dev::workspace::{scaffold_common, NewPlugin, TemplateValues, Workspaces};
    use crate::services::plugin_dev::Language;

    fn scaffolded() -> (tempfile::TempDir, Workspace) {
        let root = tempfile::tempdir().unwrap();
        let workspace = Workspaces::at(root.path().join("plugin-dev")).create("notes", Language::Rust).unwrap();
        let values = TemplateValues::of(&NewPlugin {
            name: "notes".into(),
            title: "Notes".into(),
            description: "Quick notes".into(),
            category: "Office".into(),
        });
        scaffold_common(&workspace, &values).unwrap();
        crate::services::plugin_dev::rust::scaffold(&workspace, &values).unwrap();
        (root, workspace)
    }

    fn errors(workspace: &Workspace, only: &str, around: &Surroundings) -> Vec<String> {
        let report = run(workspace, Some(only), around).unwrap();
        report.findings.iter().filter(|f| f.error).map(|f| format!("{}:{} {}", f.file, f.line, f.message)).collect()
    }

    #[test]
    fn the_template_passes_every_check() {
        let (_root, workspace) = scaffolded();
        let report = run(&workspace, None, &Surroundings::default()).unwrap();
        assert!(report.ok(), "{:#?}", report.findings);
        assert_eq!(report.ran, CHECKS);
        assert!(report.findings.is_empty(), "no warnings either: {:#?}", report.findings);
        assert!(run(&workspace, Some("spelling"), &Surroundings::default()).is_err());
    }

    #[test]
    fn the_manifest_must_match_what_the_server_loads() {
        let (_root, workspace) = scaffolded();
        workspace
            .write(
                "plugin.toml",
                "name = \"other\"\nversion = \"one\"\napi_level = 7\nruntime = \"dylib\"\nweb_dir = \"../web\"\n",
            )
            .unwrap();
        let found = errors(&workspace, "manifest", &Surroundings::default());
        let has = |text: &str| found.iter().any(|line| line.contains(text));
        assert!(has("plugin.toml:1 name is \"other\" but this plugin is \"notes\""), "{found:#?}");
        assert!(has("plugin.toml:2 version \"one\""), "{found:#?}");
        assert!(has("plugin.toml:3 api_level 7"), "{found:#?}");
        assert!(has("plugin.toml:4 runtime must be \"process\""), "{found:#?}");
        assert!(has("plugin.toml:5") && has("web_dir"), "{found:#?}");
        assert!(has("description is missing"), "{found:#?}");

        workspace.write("plugin.toml", "name = \n").unwrap();
        let found = errors(&workspace, "manifest", &Surroundings::default());
        assert!(found[0].starts_with("plugin.toml:1 not valid TOML"), "{found:#?}");
        workspace.delete("plugin.toml").unwrap();
        assert!(errors(&workspace, "manifest", &Surroundings::default())[0].contains("plugin.toml is missing"));
    }

    #[test]
    fn the_window_must_fit_the_desktop_and_its_security_policy() {
        let (_root, workspace) = scaffolded();
        workspace
            .write(
                "web/plugin.js",
                "import { button } from '/ui/index.js';\nimport x from 'https://cdn.example/x.js';\nimport './local.js';\n\
                 const html = '<script>alert(1)</script>';\nel.innerHTML = html;\nfetch('https://api.example.com/x');\n\
                 eval('1');\nexport default { mount, unmount };\n",
            )
            .unwrap();
        let found = errors(&workspace, "web", &Surroundings::default());
        let has = |text: &str| found.iter().any(|line| line.contains(text));
        assert!(has("web/plugin.js:1 import \"/ui/index.js\" is not relative"), "{found:#?}");
        assert!(has("web/plugin.js:2 import \"https://cdn.example/x.js\""), "{found:#?}");
        assert!(!has("./local.js"), "a relative import is fine: {found:#?}");
        assert!(has("web/plugin.js:4") && has("inline scripts"), "{found:#?}");
        assert!(has("web/plugin.js:6 the window fetches"), "{found:#?}");
        assert!(has("web/plugin.js:7 `eval`"), "{found:#?}");
        assert!(has("no `getElement`"), "{found:#?}");
        assert!(has("data-plugin"), "{found:#?}");
        let report = run(&workspace, Some("web"), &Surroundings::default()).unwrap();
        assert!(report.findings.iter().any(|f| !f.error && f.line == 5 && f.message.contains("innerHTML")));

        workspace.write("web/plugin.css", ".notes-tile { color: #ff0000; }\n#notes { color: var(--text); }\n").unwrap();
        let report = run(&workspace, Some("web"), &Surroundings::default()).unwrap();
        let colours: Vec<usize> =
            report.findings.iter().filter(|f| f.message.contains("hard-coded colour")).map(|f| f.line).collect();
        assert_eq!(colours, vec![1], "an id selector is not a colour");
    }

    #[test]
    fn every_key_the_window_uses_is_in_the_reference_catalogue() {
        let (_root, workspace) = scaffolded();
        workspace.patch("web/plugin.js", "'plugin.notes.add'", "'plugin.notes.addNew'", false).unwrap();
        workspace
            .patch("web/plugin.js", "const PLUGIN =", "// see t('plugin.notes.commented')\nconst PLUGIN =", false)
            .unwrap();
        workspace
            .write("web/i18n/it.json", "{\n  \"plugin.notes.title\": \"Note\",\n  \"plugin.notes.extra\": \"x\"\n}\n")
            .unwrap();
        workspace.write("web/i18n/fr.json", "{ \"plugin.notes.title\": \"Notes\", }").unwrap();
        let found = errors(&workspace, "i18n", &Surroundings::default());
        let has = |text: &str| found.iter().any(|line| line.contains(text));
        assert!(has("the key \"plugin.notes.addNew\" is used here but is not in web/i18n/en.json"), "{found:#?}");
        assert!(has("web/i18n/it.json") && has("\"plugin.notes.extra\" is not in web/i18n/en.json"), "{found:#?}");
        assert!(has("web/i18n/fr.json:1 not valid JSON"), "{found:#?}");
        assert!(!has("plugin.notes.commented"), "a key in a comment is not in use: {found:#?}");

        workspace.write("web/i18n/en.json", "{ \"title\": \"Notes\", \"plugin.notes.nested\": {} }").unwrap();
        let found = errors(&workspace, "i18n", &Surroundings::default());
        assert!(found.iter().any(|l| l.contains("\"title\" must start with \"plugin.notes.\"")), "{found:#?}");
        assert!(found.iter().any(|l| l.contains("is not a string")), "{found:#?}");
    }

    #[test]
    fn a_migration_that_has_run_is_never_edited_or_removed() {
        let (root, workspace) = scaffolded();
        workspace.write("migrations/2_tags.sql", "ALTER TABLE x ADD y;").unwrap();
        workspace.write("migrations/001_again.sql", "SELECT 1;").unwrap();
        workspace.write("migrations/003_empty.sql", "  ").unwrap();
        let found = errors(&workspace, "migrations", &Surroundings::default());
        let has = |text: &str| found.iter().any(|line| line.contains(text));
        assert!(has("migrations/2_tags.sql") && has("named NNN_what_it_does.sql"), "{found:#?}");
        assert!(has("number 001 is already used"), "{found:#?}");
        assert!(has("migrations/003_empty.sql:0 this migration is empty"), "{found:#?}");
        for file in ["2_tags.sql", "001_again.sql", "003_empty.sql"] {
            workspace.delete(&format!("migrations/{file}")).unwrap();
        }

        // The installed copy has 001 with other content, and a 002 this
        // workspace does not have.
        let installed = root.path().join("installed/notes");
        std::fs::create_dir_all(installed.join("migrations")).unwrap();
        std::fs::write(installed.join("migrations/001_init.sql"), "CREATE TABLE old (id TEXT);").unwrap();
        std::fs::write(installed.join("migrations/002_more.sql"), "SELECT 1;").unwrap();
        let around = Surroundings { installed_dir: Some(installed.clone()), ..Surroundings::default() };
        let found = errors(&workspace, "migrations", &around);
        assert!(found.iter().any(|l| l.contains("migrations/001_init.sql") && l.contains("already installed")), "{found:#?}");
        assert!(found.iter().any(|l| l.contains("migrations/002_more.sql") && l.contains("never removed")), "{found:#?}");

        // Unchanged and complete: adding a new one is how a schema changes.
        std::fs::copy(workspace.dir.join("migrations/001_init.sql"), installed.join("migrations/001_init.sql")).unwrap();
        std::fs::remove_file(installed.join("migrations/002_more.sql")).unwrap();
        workspace.write("migrations/002_tags.sql", "ALTER TABLE notes_items ADD COLUMN tag TEXT;").unwrap();
        assert!(errors(&workspace, "migrations", &around).is_empty());
    }

    #[test]
    fn tools_and_routes_carry_the_plugins_name() {
        let (_root, workspace) = scaffolded();
        let around = Surroundings::default();
        assert!(errors(&workspace, "conflicts", &around).is_empty());

        workspace.patch("src/tools.rs", "\"notes_add\"", "\"file_write\"", false).unwrap();
        workspace.patch("src/plugin.rs", "RouteSpec::get(\"/api/notes/items\"", "RouteSpec::get(\"/api/files/list\"", false).unwrap();
        let around = Surroundings {
            foreign_tools: BTreeMap::from([("file_write".to_string(), "files".to_string())]),
            is_craft_app: true,
            ..Surroundings::default()
        };
        let found = errors(&workspace, "conflicts", &around);
        let has = |text: &str| found.iter().any(|line| line.contains(text));
        assert!(has("src/tools.rs") && has("the tool \"file_write\" must be named \"notes_…\""), "{found:#?}");
        assert!(has("already belongs to the plugin \"files\""), "{found:#?}");
        assert!(has("the route \"/api/files/list\" must be under \"/api/notes/\""), "{found:#?}");
        assert!(has("the skill never names the tool \"file_write\""), "{found:#?}");
        assert!(has("is the name of a Craft app"), "{found:#?}");

        let report = run(&workspace, Some("conflicts"), &Surroundings { overrides_system: true, ..Surroundings::default() }).unwrap();
        assert!(report.findings.iter().any(|f| !f.error && f.message.contains("replaces the system plugin")));
    }

    #[test]
    fn a_python_engine_is_held_to_the_same_names() {
        let root = tempfile::tempdir().unwrap();
        let workspace = Workspaces::at(root.path().join("plugin-dev")).create("notes", Language::Python).unwrap();
        let values = TemplateValues::of(&NewPlugin {
            name: "notes".into(),
            title: "Notes".into(),
            description: "Quick notes".into(),
            category: "Office".into(),
        });
        scaffold_common(&workspace, &values).unwrap();
        crate::services::plugin_dev::python::scaffold(&workspace, &values).unwrap();
        let report = run(&workspace, None, &Surroundings::default()).unwrap();
        assert!(report.findings.is_empty(), "the Python template passes every check: {:#?}", report.findings);

        workspace.patch("engine/plugin.py", "@plugin.tool(\"notes_add\"", "@plugin.tool(\"purge\"", false).unwrap();
        workspace
            .patch("engine/plugin.py", "@plugin.route(\"POST\", \"/api/notes/items\")", "@plugin.route(\"POST\", \"/api/other\")", false)
            .unwrap();
        let found = errors(&workspace, "conflicts", &Surroundings::default());
        assert!(found.iter().any(|l| l.contains("engine/plugin.py") && l.contains("the tool \"purge\" must be named \"notes_…\"")), "{found:#?}");
        assert!(found.iter().any(|l| l.contains("the route \"/api/other\" must be under \"/api/notes/\"")), "{found:#?}");
        assert!(found.iter().any(|l| l.contains("the skill never names the tool \"purge\"")), "{found:#?}");
    }

    #[test]
    fn a_report_lists_errors_and_warnings_apart() {
        let (_root, workspace) = scaffolded();
        workspace.delete("web/icon.svg").unwrap();
        workspace.delete("plugin.toml").unwrap();
        let report = run(&workspace, None, &Surroundings::default()).unwrap();
        let shown = report.to_json();
        assert_eq!(shown["ok"], false);
        assert_eq!(shown["errors"][0]["check"], "manifest");
        assert_eq!(shown["errors"][0]["file"], "plugin.toml");
        assert_eq!(shown["warnings"][0]["check"], "web");
        assert_eq!(shown["checked"].as_array().unwrap().len(), CHECKS.len());
    }
}
