//! The plugin builder's Rust module: what the machine needs, the Rust half
//! of a new plugin, and the build.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt};

use super::workspace::{fill, TemplateValues, Workspace, Workspaces};
use super::Readiness;
use crate::errors::AppError;
use crate::services::agent_cancel::CancelHandle;
use crate::services::ask::AskLine;

/// Where the plugin SDK bundle is looked for, in order: an explicit
/// directory, the packaged copy, then a source checkout the server runs in.
///
/// The bundle is a directory laid out like the repository: the SDK crate and
/// the protocol crate it depends on under `crates/`, the project's copy of
/// `libsqlite3-sys` under `vendor/`, and the server's `Cargo.lock`. A plugin
/// builds against it, so it is the bundle of the server that will run it.
pub const SDK_DIR_ENV: &str = "SHINY_PLUGIN_SDK_DIR";
const PACKAGED_SDK_DIR: &str = "/usr/share/shiny/plugin-sdk";
const CHECKOUT_SDK_DIR: &str = ".";
/// What makes a directory a usable bundle.
const BUNDLE_FILES: &[&str] = &[
    "crates/shiny-plugin-sdk/Cargo.toml",
    "crates/shiny-plugin-protocol/Cargo.toml",
    "vendor/libsqlite3-sys/Cargo.toml",
];

/// What a Rust plugin needs to compile on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Found {
    /// `cargo` and `rustc`.
    compiler: bool,
    /// A C linker (`cc`): rustc links every executable through it.
    linker: bool,
    /// The plugin SDK bundle: the SDK is not published, a plugin builds
    /// against the copy that matches this server.
    sdk: bool,
    /// The system SQLite library the SDK links: the one the server uses.
    sqlite: bool,
}

pub fn lacks_compiler() -> bool {
    !(on_path("cargo", &cargo_bin()) && on_path("rustc", &cargo_bin()))
}

pub fn lacks_linker() -> bool {
    !on_path("cc", &[])
}

pub fn readiness() -> Readiness {
    assess(Found {
        compiler: !lacks_compiler(),
        linker: !lacks_linker(),
        sdk: sdk_root().is_some(),
        sqlite: sqlite_library().is_some(),
    })
}

fn assess(found: Found) -> Readiness {
    let mut missing = Vec::new();
    if !found.compiler {
        missing.push(AskLine::keyed(
            "chat.ask.missing.rustCompiler",
            "Rust compiler (downloaded into your home folder)",
        ));
    }
    if !found.linker {
        missing.push(AskLine::keyed(
            "chat.ask.missing.linker",
            "C compiler gcc (system package, asks for the administrator password)",
        ));
    }
    if !found.sqlite {
        missing.push(AskLine::keyed("chat.ask.missing.sqlite", "SQLite library (part of the LiteAura package)"));
    }
    if !found.sdk {
        missing.push(AskLine::keyed("chat.ask.missing.sdk", "Plugin SDK (part of the LiteAura package)"));
    }
    // The compiler and the linker are installed for the user. The SDK and
    // SQLite come with the package: missing, the installation is broken and
    // nothing here repairs it.
    Readiness { missing, fetchable: found.sdk && found.sqlite }
}

/// The SDK bundle's directory, absolute, when this machine has one.
pub fn sdk_root() -> Option<PathBuf> {
    let explicit = std::env::var(SDK_DIR_ENV).ok().filter(|dir| !dir.trim().is_empty());
    for dir in [explicit.as_deref(), Some(PACKAGED_SDK_DIR), Some(CHECKOUT_SDK_DIR)].into_iter().flatten() {
        let dir = PathBuf::from(dir);
        if BUNDLE_FILES.iter().all(|file| dir.join(file).is_file()) {
            return std::fs::canonicalize(&dir).ok();
        }
    }
    None
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|home| home.is_absolute())
}

/// rustup installs into the user's home, which a session's `PATH` may not list.
fn cargo_bin() -> Vec<PathBuf> {
    home_dir().map(|home| home.join(".cargo/bin")).into_iter().collect()
}

/// Whether `program` is an executable file in `PATH` or in one of `extra`.
pub(super) fn on_path(program: &str, extra: &[PathBuf]) -> bool {
    find_program(program, extra).is_some()
}

fn find_program(program: &str, extra: &[PathBuf]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(extra.iter().cloned())
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Where the system's SQLite library is, and whether the directory also has
/// the unversioned `libsqlite3.so` the linker resolves `-lsqlite3` with.
///
/// Only the development package installs the unversioned name; the runtime
/// library, which the LiteAura package depends on, is `libsqlite3.so.0`. The
/// build links against that through a name of its own ([`sqlite_link_dir`]),
/// so the development package is not needed.
fn sqlite_library() -> Option<(PathBuf, bool)> {
    let multiarch = format!("/usr/lib/{}-linux-gnu", std::env::consts::ARCH);
    [multiarch.as_str(), "/usr/lib64", "/usr/lib", "/usr/local/lib"].iter().find_map(|dir| {
        let dir = Path::new(dir);
        if dir.join("libsqlite3.so").exists() {
            Some((dir.join("libsqlite3.so"), true))
        } else if dir.join("libsqlite3.so.0").exists() {
            Some((dir.join("libsqlite3.so.0"), false))
        } else {
            None
        }
    })
}

/// A directory to add to the linker's search path so `-lsqlite3` resolves on
/// a machine without SQLite's development package: it holds one symbolic link,
/// `libsqlite3.so`, to the runtime library. `None` when the system already
/// has the unversioned name.
fn sqlite_link_dir(workspaces: &Workspaces) -> Option<PathBuf> {
    let (library, unversioned) = sqlite_library()?;
    if unversioned {
        return None;
    }
    let dir = workspaces.root().join(".lib");
    let link = dir.join("libsqlite3.so");
    if std::fs::read_link(&link).ok().as_deref() != Some(library.as_path()) {
        std::fs::create_dir_all(&dir).ok()?;
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&library, &link).ok()?;
    }
    Some(dir)
}

// ── scaffold ─────────────────────────────────────────────────────────────

const FILES: &[(&str, &str)] = &[
    ("Cargo.toml", include_str!("templates/rust/Cargo.toml")),
    ("src/main.rs", include_str!("templates/rust/src/main.rs")),
    ("src/lib.rs", include_str!("templates/rust/src/lib.rs")),
    ("src/plugin.rs", include_str!("templates/rust/src/plugin.rs")),
    ("src/store.rs", include_str!("templates/rust/src/store.rs")),
    ("src/tools.rs", include_str!("templates/rust/src/tools.rs")),
    ("src/routes.rs", include_str!("templates/rust/src/routes.rs")),
];

/// The name of a plugin's library crate (`my-notes` → `shiny_my_notes_plugin`).
fn crate_name(ident: &str) -> String {
    format!("shiny_{ident}_plugin")
}

/// The name of the executable cargo builds for a plugin.
fn binary_name(name: &str) -> String {
    format!("shiny-plugin-{name}")
}

/// Write the Rust half of a new plugin: the crate, with one example tool
/// pair, two routes and their store, built against this machine's SDK bundle.
pub fn scaffold(workspace: &Workspace, values: &TemplateValues) -> Result<(), AppError> {
    let root = sdk_root().ok_or_else(|| {
        AppError::BadRequest(
            "the plugin SDK sources are not on this machine, so a Rust plugin cannot be created yet".into(),
        )
    })?;
    let root_text = root.to_string_lossy().into_owned();
    let crate_name = crate_name(&values.ident);
    let mut pairs = values.pairs();
    pairs.push(("CRATE", crate_name.as_str()));
    pairs.push(("SDK_ROOT", root_text.as_str()));
    for (path, template) in FILES {
        workspace.write(path, &fill(template, &pairs))?;
    }
    // Start from the versions the server was built with: the build then needs
    // nothing newer from the network, and a plugin links what the server links.
    // Written directly: the lock file is larger than a hand-written source.
    if let Ok(lock) = std::fs::read(root.join("Cargo.lock")) {
        std::fs::write(workspace.dir.join("Cargo.lock"), lock)?;
    }
    Ok(())
}

// ── build ────────────────────────────────────────────────────────────────

/// Longest a build may run: a first build compiles the SDK and everything it
/// depends on.
const BUILD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(40 * 60);
const MAX_ERRORS: usize = 20;
const MAX_WARNINGS: usize = 8;

/// One compiler message, where the assistant can act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub level: String,
    pub message: String,
    pub code: String,
    pub file: String,
    pub line: u64,
    pub column: u64,
    /// The compiler's own suggestion, when it has one.
    pub help: String,
}

impl Diagnostic {
    fn to_json(&self) -> Value {
        let mut value = json!({ "message": self.message });
        if !self.file.is_empty() {
            value["file"] = json!(self.file);
            value["line"] = json!(self.line);
            value["column"] = json!(self.column);
        }
        if !self.code.is_empty() {
            value["code"] = json!(self.code);
        }
        if !self.help.is_empty() {
            value["help"] = json!(self.help);
        }
        value
    }
}

/// Read one line of `cargo --message-format=json` output. Only messages about
/// the plugin's own crate matter: a warning inside a dependency is not the
/// assistant's to fix.
fn diagnostic(line: &str, workspace_dir: &Path) -> Option<Diagnostic> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("reason")?.as_str()? != "compiler-message" {
        return None;
    }
    let message = value.get("message")?;
    let level = message.get("level")?.as_str()?;
    if level != "error" && level != "warning" {
        return None;
    }
    let text = message.get("message")?.as_str()?.to_string();
    // "aborting due to 2 previous errors" and the like only repeat the count.
    if text.starts_with("aborting due to") || text.contains("warning emitted") || text.contains("warnings emitted")
    {
        return None;
    }
    let spans = message.get("spans").and_then(Value::as_array);
    let span = spans.and_then(|spans| {
        spans
            .iter()
            .find(|span| span.get("is_primary").and_then(Value::as_bool) == Some(true))
            .or(spans.first())
    });
    let file = span.and_then(|span| span.get("file_name")).and_then(Value::as_str).unwrap_or_default();
    // Paths are relative to the crate; an absolute one inside it is made so.
    let file = Path::new(file)
        .strip_prefix(workspace_dir)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| file.to_string());
    if Path::new(&file).is_absolute() {
        return None;
    }
    let number = |key: &str| span.and_then(|span| span.get(key)).and_then(Value::as_u64).unwrap_or(0);
    let help = message
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|child| matches!(child.get("level").and_then(Value::as_str), Some("help" | "note")))
        .filter_map(|child| {
            let text = child.get("message")?.as_str()?;
            let replacement = child
                .get("spans")
                .and_then(Value::as_array)
                .and_then(|spans| spans.first())
                .and_then(|span| span.get("suggested_replacement"))
                .and_then(Value::as_str)
                .filter(|replacement| !replacement.trim().is_empty());
            Some(match replacement {
                Some(replacement) => format!("{text}: `{}`", replacement.trim()),
                None => text.to_string(),
            })
        })
        .take(2)
        .collect::<Vec<_>>()
        .join("; ");
    Some(Diagnostic {
        level: level.to_string(),
        message: text,
        code: message.pointer("/code/code").and_then(Value::as_str).unwrap_or_default().to_string(),
        file,
        line: number("line_start"),
        column: number("column_start"),
        help,
    })
}

/// How a build ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildEnd {
    Built,
    /// The compiler refused the sources.
    Failed,
    /// Cargo could not start the build at all (a dependency it cannot fetch,
    /// a broken manifest): its own words are in `detail`.
    Broken,
    Stopped,
    TimedOut,
}

#[derive(Debug, Clone)]
pub struct BuildReport {
    pub end: BuildEnd,
    pub errors: Vec<Diagnostic>,
    pub warnings: Vec<Diagnostic>,
    pub error_count: usize,
    pub warning_count: usize,
    /// Cargo's own last lines when the build did not produce diagnostics.
    pub detail: String,
    pub seconds: u64,
}

impl BuildReport {
    pub fn to_json(&self) -> Value {
        let mut value = json!({
            "built": self.end == BuildEnd::Built,
            "errors": self.errors.iter().map(Diagnostic::to_json).collect::<Vec<_>>(),
            "error_count": self.error_count,
            "warnings": self.warnings.iter().map(Diagnostic::to_json).collect::<Vec<_>>(),
            "warning_count": self.warning_count,
            "seconds": self.seconds,
        });
        if !self.detail.is_empty() {
            value["detail"] = json!(self.detail);
        }
        value
    }
}

/// Build the plugin's executable and stage it as `bin/<name>`.
///
/// The command is fixed: `cargo build --release` in the workspace, into the
/// target directory every workspace shares. Nothing of it comes from the
/// assistant.
pub async fn build(
    workspaces: &Workspaces,
    workspace: &mut Workspace,
    cancel: Option<&CancelHandle>,
) -> Result<BuildReport, AppError> {
    let cargo = find_program("cargo", &cargo_bin()).ok_or_else(|| {
        AppError::BadRequest("the Rust compiler is not on this machine, so the plugin cannot be built".into())
    })?;
    if !workspace.dir.join("Cargo.toml").is_file() {
        return Err(AppError::BadRequest(
            "this plugin has no Cargo.toml: it is not a Rust plugin, or the file was deleted".into(),
        ));
    }
    let target = workspaces.shared_target_dir();
    std::fs::create_dir_all(&target)?;
    let sources = workspace.sources_digest();

    // rustc and the linker are found next to cargo.
    let mut path: Vec<PathBuf> = cargo_bin();
    path.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
    let started = std::time::Instant::now();
    let mut command = tokio::process::Command::new(&cargo);
    if let Some(dir) = sqlite_link_dir(workspaces) {
        // gcc reads LIBRARY_PATH for `-l` lookups.
        let mut search = vec![dir];
        search.extend(std::env::split_paths(&std::env::var_os("LIBRARY_PATH").unwrap_or_default()));
        command.env("LIBRARY_PATH", std::env::join_paths(search).unwrap_or_default());
    }
    let mut child = command
        .args(["build", "--release", "--message-format=json", "--color=never"])
        .current_dir(&workspace.dir)
        .env("CARGO_TARGET_DIR", &target)
        .env("PATH", std::env::join_paths(path).unwrap_or_default())
        .env("CARGO_TERM_PROGRESS_WHEN", "never")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| AppError::Internal(format!("could not start the build: {e}")))?;

    let stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let dir = workspace.dir.clone();
    let messages = tokio::spawn(async move {
        let mut found: Vec<Diagnostic> = Vec::new();
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(diagnostic) = diagnostic(&line, &dir) {
                if !found.contains(&diagnostic) {
                    found.push(diagnostic);
                }
            }
        }
        found
    });
    let stderr_text = tokio::spawn(async move {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text).await;
        text
    });

    let waited = async {
        match cancel {
            Some(handle) => tokio::select! {
                biased;
                _ = handle.cancelled() => None,
                status = child.wait() => Some(status),
            },
            None => Some(child.wait().await),
        }
    };
    let (status, end_override) = match tokio::time::timeout(BUILD_TIMEOUT, waited).await {
        Ok(Some(status)) => (status.ok(), None),
        Ok(None) => (None, Some(BuildEnd::Stopped)),
        Err(_) => (None, Some(BuildEnd::TimedOut)),
    };
    if end_override.is_some() {
        let _ = child.kill().await;
    }
    let found = messages.await.unwrap_or_default();
    let stderr_text = stderr_text.await.unwrap_or_default();

    let (errors, warnings): (Vec<Diagnostic>, Vec<Diagnostic>) =
        found.into_iter().partition(|diagnostic| diagnostic.level == "error");
    let succeeded = status.is_some_and(|status| status.success());
    let end = match end_override {
        Some(end) => end,
        None if succeeded => BuildEnd::Built,
        None if !errors.is_empty() => BuildEnd::Failed,
        None => BuildEnd::Broken,
    };
    // Cargo's own trouble (it could not resolve or fetch something) is on
    // stderr, as its last lines; compiler messages are already structured.
    let detail = if end == BuildEnd::Broken {
        let lines: Vec<&str> = stderr_text.lines().filter(|line| !line.trim().is_empty()).collect();
        lines[lines.len().saturating_sub(12)..].join("\n")
    } else {
        String::new()
    };

    if end == BuildEnd::Built {
        let built = target.join("release").join(binary_name(&workspace.name));
        let staged = crate::plugins::process::executable_path(&workspace.dir, &workspace.name);
        if let Some(parent) = staged.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Written beside and renamed: `bin/<name>` is never a half-copied file.
        let partial = staged.with_extension("partial");
        std::fs::copy(&built, &partial)
            .map_err(|e| AppError::Internal(format!("the build finished but its executable is missing: {e}")))?;
        crate::plugins::process::ensure_executable(&partial)?;
        std::fs::rename(&partial, &staged)?;
        workspace.meta.built = Some(sources);
    } else {
        workspace.meta.built = None;
    }
    workspace.save_meta()?;

    Ok(BuildReport {
        end,
        error_count: errors.len(),
        warning_count: warnings.len(),
        errors: errors.into_iter().take(MAX_ERRORS).collect(),
        warnings: warnings.into_iter().take(MAX_WARNINGS).collect(),
        detail,
        seconds: started.elapsed().as_secs(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Found = Found { compiler: true, linker: true, sdk: true, sqlite: true };

    #[test]
    fn a_complete_machine_is_ready() {
        assert!(assess(ALL).ready());
    }

    #[test]
    fn each_missing_piece_is_named() {
        let none = assess(Found { compiler: false, linker: false, sdk: false, sqlite: false });
        assert_eq!(none.missing.len(), 4);
        assert!(none.missing.iter().all(|line| line.key.is_some()), "every line is translatable");

        // The compiler and the linker are installed for the user.
        let tools = assess(Found { compiler: false, linker: false, ..ALL });
        assert_eq!(tools.missing.len(), 2);
        assert!(tools.fetchable);
        assert!(tools.missing_text()[0].starts_with("Rust compiler"));
        // What ships with the package is not: a broken installation.
        let no_sdk = assess(Found { sdk: false, ..ALL });
        assert!(no_sdk.missing_text()[0].starts_with("Plugin SDK"));
        assert!(!no_sdk.fetchable);
        assert!(!assess(Found { sqlite: false, ..ALL }).fetchable);
    }

    #[test]
    fn a_program_is_found_only_as_an_executable_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let extra = vec![dir.path().to_path_buf()];
        assert!(!on_path("shiny-test-no-such-program", &extra));
        let file = dir.path().join("shiny-test-program");
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!on_path("shiny-test-program", &extra), "not executable");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(on_path("shiny-test-program", &extra));
    }

    #[test]
    fn the_checkout_the_tests_run_in_is_a_bundle() {
        // Tests run in the repository root, which has the SDK, the protocol
        // crate and the vendored SQLite crate.
        let root = sdk_root().expect("the repository is an SDK bundle");
        assert!(root.is_absolute());
        assert!(root.join("crates/shiny-plugin-sdk/src/lib.rs").is_file());
    }

    #[test]
    fn a_compiler_error_becomes_a_place_and_a_message() {
        let dir = Path::new("/work/notes");
        let line = json!({
            "reason": "compiler-message",
            "message": {
                "level": "error",
                "message": "mismatched types",
                "code": { "code": "E0308" },
                "spans": [
                    { "file_name": "src/other.rs", "line_start": 1, "column_start": 1, "is_primary": false },
                    { "file_name": "/work/notes/src/tools.rs", "line_start": 31, "column_start": 9, "is_primary": true },
                ],
                "children": [
                    { "level": "help", "message": "try using a conversion method",
                      "spans": [{ "suggested_replacement": ".to_string()" }] },
                    { "level": "note", "message": "expected `String`, found `&str`", "spans": [] },
                ],
            },
        })
        .to_string();
        let found = diagnostic(&line, dir).unwrap();
        assert_eq!((found.file.as_str(), found.line, found.column), ("src/tools.rs", 31, 9));
        assert_eq!(found.code, "E0308");
        assert_eq!(found.help, "try using a conversion method: `.to_string()`; expected `String`, found `&str`");
        let shown = found.to_json();
        assert_eq!(shown["file"], "src/tools.rs");
        assert_eq!(shown["line"], 31);
    }

    #[test]
    fn noise_and_other_crates_are_not_reported() {
        let dir = Path::new("/work/notes");
        let message = |level: &str, text: &str, file: &str| {
            json!({
                "reason": "compiler-message",
                "message": { "level": level, "message": text, "spans": [{ "file_name": file, "is_primary": true }] },
            })
            .to_string()
        };
        assert!(diagnostic(&message("error", "aborting due to 2 previous errors", "src/lib.rs"), dir).is_none());
        assert!(diagnostic(&message("warning", "1 warning emitted", "src/lib.rs"), dir).is_none());
        assert!(diagnostic(&message("note", "something", "src/lib.rs"), dir).is_none());
        assert!(
            diagnostic(&message("warning", "unused", "/home/u/.cargo/registry/src/x/lib.rs"), dir).is_none(),
            "a dependency's warning is not the plugin's"
        );
        assert!(diagnostic(r#"{"reason":"compiler-artifact"}"#, dir).is_none());
        assert!(diagnostic("not json", dir).is_none());
        assert!(diagnostic(&message("warning", "unused variable", "src/lib.rs"), dir).is_some());
    }
}
