//! Plugin workspaces: where a plugin's sources live while the assistant
//! builds it.
//!
//! One directory per plugin under `<data>/plugin-dev/`, next to the installed
//! plugins and never inside them: the server watches the installed directory
//! and loads again whatever changes there, so half-written sources must not
//! be in it. The assistant reads and writes a workspace only through the
//! functions here, which keep every path inside the workspace, accept text
//! files of known kinds, and bound their size and number.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::Language;
use crate::errors::AppError;

/// Largest file the assistant may write, and the most it reads in one call.
pub const MAX_FILE_BYTES: usize = 256 * 1024;
pub const READ_CHUNK_BYTES: usize = 48 * 1024;
/// Most source files a workspace may hold.
pub const MAX_FILES: usize = 300;

/// Directories that hold what the build produces, not what the assistant
/// writes: compiled output, the staged executable, packed archives.
const GENERATED_DIRS: &[&str] = &["target", "bin", "dist"];
const META_FILE: &str = ".workspace.json";
/// The `language` of a workspace whose engine is an executable without sources.
pub const PREBUILT: &str = "prebuilt";

/// File kinds a plugin's sources are made of. Anything else (an executable, an
/// archive, a script for another interpreter) is refused.
const TEXT_EXTENSIONS: &[&str] = &[
    "rs", "toml", "lock", "py", "js", "mjs", "css", "json", "md", "sql", "svg", "html", "txt",
];

/// What is recorded about a workspace between commands.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Meta {
    pub language: String,
    /// Digest of the sources the checks last passed on.
    #[serde(default)]
    pub checked: Option<String>,
    /// Digest of the sources the executable in `bin/` was built from.
    #[serde(default)]
    pub built: Option<String>,
    /// The archive made by the last pack, and what it was made from.
    #[serde(default)]
    pub packed: Option<Packed>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Packed {
    pub sources: String,
    pub file: String,
    pub sha256: String,
    pub version: String,
}

/// What a new plugin is created with.
#[derive(Debug, Clone)]
pub struct NewPlugin {
    pub name: String,
    pub title: String,
    pub description: String,
    pub category: String,
}

/// One file of a workspace, as listed.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub bytes: u64,
}

/// The directory that holds every workspace.
#[derive(Debug, Clone)]
pub struct Workspaces {
    root: PathBuf,
}

impl Workspaces {
    /// Next to the installed plugins: `data/plugins` → `data/plugin-dev`.
    pub fn beside_plugins(plugins_dir: &str) -> Self {
        let plugins = Path::new(plugins_dir);
        let parent = plugins.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        Self { root: parent.join("plugin-dev") }
    }

    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Compiled output shared by every workspace, so a second plugin does not
    /// compile the SDK and its dependencies again.
    pub fn shared_target_dir(&self) -> PathBuf {
        self.root.join(".target")
    }

    pub fn exists(&self, name: &str) -> bool {
        validate_name(name).is_ok() && self.root.join(name).join(META_FILE).is_file()
    }

    pub fn open(&self, name: &str) -> Result<Workspace, AppError> {
        validate_name(name)?;
        let dir = self.root.join(name);
        let meta = read_meta(&dir).ok_or_else(|| {
            AppError::NotFound(format!(
                "no plugin workspace named '{name}'. Use plugin_dev_list to see the ones that exist, \
                 or plugin_dev_new to create one."
            ))
        })?;
        Ok(Workspace { name: name.to_string(), dir, meta })
    }

    /// Create the workspace directory for a new plugin. The caller fills it
    /// from the language's template.
    pub fn create(&self, name: &str, language: Language) -> Result<Workspace, AppError> {
        validate_name(name)?;
        let dir = self.root.join(name);
        if dir.exists() {
            return Err(AppError::BadRequest(format!(
                "a plugin workspace named '{name}' already exists. Work on it, or choose another name."
            )));
        }
        std::fs::create_dir_all(&dir)?;
        let workspace = Workspace {
            name: name.to_string(),
            dir,
            meta: Meta { language: language.as_str().to_string(), ..Meta::default() },
        };
        workspace.save_meta()?;
        Ok(workspace)
    }

    /// Make a workspace out of an installed plugin: its manifest, window,
    /// skill, migrations, a Python engine when it has one, and its executable.
    /// Files are copied as they are, binary ones included (an icon, a font):
    /// the assistant reads and writes only the text ones.
    pub fn adopt(&self, name: &str, installed: &Path) -> Result<Workspace, AppError> {
        validate_name(name)?;
        let dir = self.root.join(name);
        if dir.exists() {
            return Err(AppError::BadRequest(format!("a plugin workspace named '{name}' already exists")));
        }
        let copied = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(&dir)?;
            std::fs::copy(installed.join("plugin.toml"), dir.join("plugin.toml"))?;
            for folder in ["web", "skills", "migrations", "engine", "bin"] {
                copy_tree(&installed.join(folder), &dir.join(folder))?;
            }
            Ok(())
        })();
        if let Err(error) = copied {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(AppError::Internal(format!("could not copy the installed plugin: {error}")));
        }
        let python = dir.join("engine/plugin.py").is_file();
        let mut workspace = Workspace {
            name: name.to_string(),
            dir,
            meta: Meta { language: if python { "python".into() } else { PREBUILT.into() }, ..Meta::default() },
        };
        // What was installed was built from exactly these files.
        workspace.meta.built = Some(workspace.sources_digest());
        workspace.save_meta()?;
        Ok(workspace)
    }

    /// Every workspace, by name, with its language.
    pub fn list(&self) -> Vec<(String, String)> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut found: Vec<(String, String)> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let meta = read_meta(&entry.path())?;
                validate_name(&name).ok()?;
                Some((name, meta.language))
            })
            .collect();
        found.sort();
        found
    }
}

/// Copy a directory's regular files, recursively. Symbolic links and hidden
/// entries are left behind; a missing source is nothing to copy.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    let Ok(entries) = std::fs::read_dir(from) else { return Ok(()) };
    std::fs::create_dir_all(to)?;
    for entry in entries.flatten() {
        let kind = entry.file_type()?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') || kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            copy_tree(&entry.path(), &to.join(&name))?;
        } else {
            std::fs::copy(entry.path(), to.join(&name))?;
        }
    }
    Ok(())
}

fn read_meta(dir: &Path) -> Option<Meta> {
    serde_json::from_str(&std::fs::read_to_string(dir.join(META_FILE)).ok()?).ok()
}

/// A plugin the assistant creates is named like a small identifier: it is a
/// directory name, a URL segment, a table prefix and a tool prefix at once.
pub fn validate_name(name: &str) -> Result<(), AppError> {
    let valid = (2..=32).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && !name.ends_with(['-', '_']);
    if !valid {
        return Err(AppError::BadRequest(format!(
            "invalid plugin name '{}': 2 to 32 lowercase letters, digits, '-' or '_', starting with a letter",
            name.escape_debug()
        )));
    }
    crate::plugins::loader::validate_plugin_name(name)
}

/// One plugin's workspace.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub name: String,
    pub dir: PathBuf,
    pub meta: Meta,
}

impl Workspace {
    pub fn language(&self) -> Option<Language> {
        Language::parse(&self.meta.language)
    }

    /// A workspace opened from an installed plugin whose engine came without
    /// its sources: the executable is kept as it is, and what can change is
    /// the window, the skill, the translations and the manifest.
    pub fn prebuilt(&self) -> bool {
        self.meta.language == PREBUILT
    }

    pub fn save_meta(&self) -> Result<(), AppError> {
        let text = serde_json::to_string_pretty(&self.meta).unwrap_or_default();
        std::fs::write(self.dir.join(META_FILE), text)?;
        Ok(())
    }

    /// The absolute path of a source file, once it is known to stay inside
    /// the workspace: a plain relative path, no hidden or generated
    /// directory, a known text extension, and no symlink on the way.
    pub fn source_path(&self, relative: &str) -> Result<PathBuf, AppError> {
        let relative = relative.trim().trim_start_matches("./");
        crate::plugins::loader::validate_relative_path("path", relative)?;
        let path = Path::new(relative);
        let mut parts: Vec<&str> = Vec::new();
        for component in path.components() {
            match component {
                Component::Normal(part) => parts.push(part.to_str().ok_or_else(|| {
                    AppError::BadRequest("invalid path: not valid UTF-8".into())
                })?),
                Component::CurDir => {}
                _ => return Err(AppError::BadRequest(format!("invalid path '{relative}'"))),
            }
        }
        let Some(file) = parts.last() else {
            return Err(AppError::BadRequest("path required: a file inside the plugin".into()));
        };
        if parts.iter().any(|part| part.starts_with('.')) {
            return Err(AppError::BadRequest(format!(
                "invalid path '{relative}': hidden files and directories are not part of a plugin"
            )));
        }
        if GENERATED_DIRS.contains(&parts[0]) {
            return Err(AppError::BadRequest(format!(
                "'{}/' holds what the build produces: it is not written or read by hand",
                parts[0]
            )));
        }
        let extension = Path::new(file).extension().and_then(|e| e.to_str()).unwrap_or_default();
        if !TEXT_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()) {
            return Err(AppError::BadRequest(format!(
                "'{relative}' is not a kind of file a plugin's sources are made of ({})",
                TEXT_EXTENSIONS.join(", ")
            )));
        }
        // No component that already exists may be a symlink: a link inside the
        // workspace would let a write land outside it.
        let mut walked = self.dir.clone();
        for part in &parts {
            walked.push(part);
            if std::fs::symlink_metadata(&walked).is_ok_and(|meta| meta.file_type().is_symlink()) {
                return Err(AppError::BadRequest(format!("invalid path '{relative}': symbolic link")));
            }
        }
        Ok(walked)
    }

    /// Every source file, sorted. Generated directories and hidden files are
    /// not sources.
    pub fn files(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        self.walk(&self.dir, &mut out);
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    fn walk(&self, dir: &Path, out: &mut Vec<Entry>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else { continue };
            if name.starts_with('.') || kind.is_symlink() {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() {
                if dir == self.dir && GENERATED_DIRS.contains(&name.as_str()) {
                    continue;
                }
                self.walk(&path, out);
            } else if let Ok(relative) = path.strip_prefix(&self.dir) {
                out.push(Entry {
                    path: relative.to_string_lossy().into_owned(),
                    bytes: entry.metadata().map(|meta| meta.len()).unwrap_or(0),
                });
            }
        }
    }

    /// Read a source file as text, from byte `offset`, at most one chunk.
    /// Returns the text, the file's size and whether more follows.
    pub fn read(&self, relative: &str, offset: usize) -> Result<(String, usize, bool), AppError> {
        let path = self.source_path(relative)?;
        let bytes = std::fs::read(&path)
            .map_err(|_| AppError::NotFound(format!("'{relative}' does not exist in this plugin")))?;
        let total = bytes.len();
        let text = String::from_utf8(bytes)
            .map_err(|_| AppError::BadRequest(format!("'{relative}' is not text")))?;
        let mut start = offset.min(total);
        while !text.is_char_boundary(start) {
            start += 1;
        }
        let mut end = (start + READ_CHUNK_BYTES).min(total);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        Ok((text[start..end].to_string(), total, end < total))
    }

    /// Write a source file, replacing it. Returns whether it existed.
    pub fn write(&self, relative: &str, content: &str) -> Result<bool, AppError> {
        let path = self.source_path(relative)?;
        if content.len() > MAX_FILE_BYTES {
            return Err(AppError::BadRequest(format!(
                "'{relative}' would be {} bytes: a source file is at most {MAX_FILE_BYTES}. Split it.",
                content.len()
            )));
        }
        let existed = path.is_file();
        if !existed && self.files().len() >= MAX_FILES {
            return Err(AppError::BadRequest(format!(
                "this plugin already has {MAX_FILES} files, the most a workspace holds"
            )));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
        Ok(existed)
    }

    /// Replace `find` with `replace` in a source file. `find` must occur
    /// exactly once unless `all` is set; returns how many were replaced.
    pub fn patch(&self, relative: &str, find: &str, replace: &str, all: bool) -> Result<usize, AppError> {
        if find.is_empty() {
            return Err(AppError::BadRequest("find required: the exact text to replace".into()));
        }
        let path = self.source_path(relative)?;
        let text = std::fs::read_to_string(&path)
            .map_err(|_| AppError::NotFound(format!("'{relative}' does not exist in this plugin")))?;
        let count = text.matches(find).count();
        match count {
            0 => Err(AppError::BadRequest(format!(
                "the text to replace was not found in '{relative}'. Read the file again: it must match exactly, \
                 spaces and line breaks included."
            ))),
            n if n > 1 && !all => Err(AppError::BadRequest(format!(
                "the text to replace occurs {n} times in '{relative}'. Include more of the surrounding lines so it \
                 is unique, or pass \"all\": true to replace every occurrence."
            ))),
            n => {
                let updated = if all { text.replace(find, replace) } else { text.replacen(find, replace, 1) };
                if updated.len() > MAX_FILE_BYTES {
                    return Err(AppError::BadRequest(format!(
                        "'{relative}' would exceed {MAX_FILE_BYTES} bytes. Split it."
                    )));
                }
                std::fs::write(&path, updated)?;
                Ok(n)
            }
        }
    }

    /// Delete one source file.
    pub fn delete(&self, relative: &str) -> Result<(), AppError> {
        let path = self.source_path(relative)?;
        if !path.is_file() {
            return Err(AppError::NotFound(format!("'{relative}' does not exist in this plugin")));
        }
        std::fs::remove_file(&path)?;
        Ok(())
    }

    /// A fingerprint of the sources: every file's path and content. Checks,
    /// builds and packs record the fingerprint they ran on, so a later step
    /// can tell whether the sources changed since.
    pub fn sources_digest(&self) -> String {
        let mut hasher = Sha256::new();
        for entry in self.files() {
            hasher.update(entry.path.as_bytes());
            hasher.update([0]);
            if let Ok(bytes) = std::fs::read(self.dir.join(&entry.path)) {
                hasher.update(Sha256::digest(&bytes));
            }
            hasher.update([0]);
        }
        hex::encode(hasher.finalize())
    }
}

/// Fill a template: every `@@KEY@@` replaced by its value.
pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut text = template.to_string();
    for (key, value) in values {
        text = text.replace(&format!("@@{key}@@"), value);
    }
    text
}

/// The values every template is filled with for one plugin.
pub struct TemplateValues {
    pub name: String,
    /// The name as a Rust / SQL identifier (`my-notes` → `my_notes`).
    pub ident: String,
    /// The plugin's type name (`my-notes` → `MyNotesPlugin`).
    pub type_name: String,
    pub title: String,
    pub description: String,
    pub category: String,
}

impl TemplateValues {
    pub fn of(plugin: &NewPlugin) -> Self {
        let ident = plugin.name.replace('-', "_");
        let camel: String = plugin
            .name
            .split(['-', '_'])
            .filter(|part| !part.is_empty())
            .map(|part| {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect();
        Self {
            name: plugin.name.clone(),
            ident,
            type_name: format!("{camel}Plugin"),
            title: one_line(&plugin.title, 60),
            description: one_line(&plugin.description, 200),
            category: one_line(&plugin.category, 24),
        }
    }

    pub fn pairs(&self) -> Vec<(&str, &str)> {
        vec![
            ("NAME", self.name.as_str()),
            ("IDENT", self.ident.as_str()),
            ("TYPE", self.type_name.as_str()),
            ("TITLE", self.title.as_str()),
            ("DESCRIPTION", self.description.as_str()),
            ("CATEGORY", self.category.as_str()),
            ("AUTHOR", "LiteAura assistant"),
        ]
    }
}

/// One line of text that is safe inside a quoted TOML / Rust / JSON string:
/// no quotes, backslashes or control characters.
fn one_line(value: &str, max: usize) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !matches!(c, '"' | '\\' | '`' | '$') && !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

const COMMON_FILES: &[(&str, &str)] = &[
    ("plugin.toml", include_str!("templates/common/plugin.toml")),
    ("migrations/001_init.sql", include_str!("templates/common/migrations/001_init.sql")),
    ("web/plugin.js", include_str!("templates/common/web/plugin.js")),
    ("web/plugin.css", include_str!("templates/common/web/plugin.css")),
    ("web/icon.svg", include_str!("templates/common/web/icon.svg")),
    ("web/i18n/en.json", include_str!("templates/common/web/i18n/en.json")),
];
const SKILL_TEMPLATE: &str = include_str!("templates/common/skills/skill.md");

/// Write the part of a new plugin every language shares: the manifest, the
/// first migration, the window, its translations, the skill.
pub fn scaffold_common(workspace: &Workspace, values: &TemplateValues) -> Result<(), AppError> {
    let pairs = values.pairs();
    for (path, template) in COMMON_FILES {
        workspace.write(path, &fill(template, &pairs))?;
    }
    workspace.write(&format!("skills/{}.md", values.name), &fill(SKILL_TEMPLATE, &pairs))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> (tempfile::TempDir, Workspaces, Workspace) {
        let root = tempfile::tempdir().unwrap();
        let all = Workspaces::at(root.path().join("plugin-dev"));
        let workspace = all.create("notes", Language::Rust).unwrap();
        (root, all, workspace)
    }

    #[test]
    fn workspaces_live_beside_the_installed_plugins() {
        assert_eq!(Workspaces::beside_plugins("data/plugins").root(), Path::new("data/plugin-dev"));
        assert_eq!(
            Workspaces::beside_plugins("/home/u/.local/share/shiny/plugins").root(),
            Path::new("/home/u/.local/share/shiny/plugin-dev")
        );
        assert_eq!(Workspaces::beside_plugins("plugins").root(), Path::new("./plugin-dev"));
    }

    #[test]
    fn a_plugin_name_is_a_small_identifier() {
        for name in ["notes", "my-notes", "todo_2"] {
            assert!(validate_name(name).is_ok(), "{name}");
        }
        for name in ["", "n", "Notes", "2do", "notes-", "no tes", "../x", "notes.bak", "a/b", &"x".repeat(33)] {
            assert!(validate_name(name).is_err(), "{name}");
        }
    }

    #[test]
    fn a_workspace_is_created_once_and_listed_with_its_language() {
        let (_root, all, workspace) = workspace();
        assert_eq!(workspace.language(), Some(Language::Rust));
        assert!(all.exists("notes"));
        assert!(all.create("notes", Language::Python).is_err(), "already there");
        all.create("todo", Language::Python).unwrap();
        assert_eq!(all.list(), vec![("notes".into(), "rust".into()), ("todo".into(), "python".into())]);
        assert!(all.open("missing").is_err());
        assert_eq!(all.open("notes").unwrap().meta.language, "rust");
    }

    #[test]
    fn an_installed_plugin_is_adopted_with_everything_it_ships() {
        let (root, all, _workspace) = workspace();
        let installed = root.path().join("installed/radio");
        for dir in ["web/i18n", "skills", "bin", "migrations"] {
            std::fs::create_dir_all(installed.join(dir)).unwrap();
        }
        std::fs::write(installed.join("plugin.toml"), "name = \"radio\"\nversion = \"1.0.0\"\n").unwrap();
        std::fs::write(installed.join("web/plugin.js"), "export default {};").unwrap();
        std::fs::write(installed.join("web/icon.png"), [0x89u8, b'P', b'N', b'G', 0, 1, 2]).unwrap();
        std::fs::write(installed.join("skills/radio.md"), "# Radio").unwrap();
        std::fs::write(installed.join("bin/radio"), [0x7fu8, b'E', b'L', b'F']).unwrap();
        std::fs::write(installed.join("web/.hidden"), "x").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", installed.join("web/link.js")).unwrap();

        let adopted = all.adopt("radio", &installed).unwrap();
        assert!(adopted.prebuilt());
        assert_eq!(adopted.language(), None);
        let paths: Vec<String> = adopted.files().into_iter().map(|entry| entry.path).collect();
        assert_eq!(paths, ["plugin.toml", "skills/radio.md", "web/icon.png", "web/plugin.js"]);
        assert_eq!(std::fs::read(adopted.dir.join("bin/radio")).unwrap(), [0x7f, b'E', b'L', b'F']);
        assert!(!adopted.dir.join("web/link.js").exists(), "links are not followed");
        // The assistant edits the text files; the binary ones only travel.
        assert!(adopted.source_path("web/plugin.js").is_ok());
        assert!(adopted.source_path("web/icon.png").is_err());
        assert_eq!(adopted.meta.built, Some(adopted.sources_digest()));
        assert!(all.adopt("radio", &installed).is_err(), "already adopted");

        // A plugin with a Python engine keeps its language.
        let python = root.path().join("installed/tasks");
        std::fs::create_dir_all(python.join("engine")).unwrap();
        std::fs::write(python.join("plugin.toml"), "name = \"tasks\"\n").unwrap();
        std::fs::write(python.join("engine/plugin.py"), "x = 1\n").unwrap();
        assert_eq!(all.adopt("tasks", &python).unwrap().language(), Some(Language::Python));
    }

    #[test]
    fn paths_stay_inside_the_workspace() {
        let (root, _all, workspace) = workspace();
        for path in ["src/lib.rs", "./plugin.toml", "web/i18n/en.json", "skills/notes.md"] {
            assert!(workspace.source_path(path).is_ok(), "{path}");
        }
        for path in [
            "", "/etc/passwd", "../other/plugin.toml", "src/../../x.rs", ".workspace.json", "src/.hidden.rs",
            ".git/config", "target/release/x.rs", "bin/notes", "dist/a.json", "run.sh", "notes", "lib.so",
            "src\\lib.rs",
        ] {
            assert!(workspace.source_path(path).is_err(), "{path}");
        }

        // A symlink planted in the workspace is never followed.
        let outside = root.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, workspace.dir.join("web")).unwrap();
        assert!(workspace.source_path("web/plugin.js").is_err());
        assert!(workspace.write("web/plugin.js", "x").is_err());
        assert!(!outside.join("plugin.js").exists());
    }

    #[test]
    fn files_are_written_read_patched_and_deleted() {
        let (_root, _all, workspace) = workspace();
        assert!(!workspace.write("src/lib.rs", "fn a() {}\nfn b() {}\n").unwrap());
        assert!(workspace.write("src/lib.rs", "fn a() {}\nfn b() {}\n").unwrap(), "replaced");
        assert_eq!(workspace.read("src/lib.rs", 0).unwrap(), ("fn a() {}\nfn b() {}\n".into(), 20, false));

        assert_eq!(workspace.patch("src/lib.rs", "fn a()", "fn alpha()", false).unwrap(), 1);
        assert!(workspace.patch("src/lib.rs", "fn zzz()", "x", false).is_err(), "not found");
        assert!(workspace.patch("src/lib.rs", "fn ", "pub fn ", false).is_err(), "ambiguous");
        assert_eq!(workspace.patch("src/lib.rs", "fn ", "pub fn ", true).unwrap(), 2);
        assert_eq!(workspace.read("src/lib.rs", 0).unwrap().0, "pub fn alpha() {}\npub fn b() {}\n");

        assert_eq!(workspace.files(), vec![Entry { path: "src/lib.rs".into(), bytes: 32 }]);
        workspace.delete("src/lib.rs").unwrap();
        assert!(workspace.delete("src/lib.rs").is_err());
        assert!(workspace.files().is_empty());
    }

    #[test]
    fn a_file_has_a_size_limit_and_long_files_read_in_chunks() {
        let (_root, _all, workspace) = workspace();
        assert!(workspace.write("src/big.rs", &"x".repeat(MAX_FILE_BYTES + 1)).is_err());
        let long = "é".repeat(READ_CHUNK_BYTES); // two bytes each
        workspace.write("src/long.rs", &long).unwrap();
        let (first, total, more) = workspace.read("src/long.rs", 0).unwrap();
        assert_eq!(total, READ_CHUNK_BYTES * 2);
        assert!(more);
        let (second, _, more) = workspace.read("src/long.rs", first.len()).unwrap();
        assert!(!more);
        assert_eq!(format!("{first}{second}"), long);
        // An offset inside a character moves to the next boundary.
        assert!(workspace.read("src/long.rs", 1).is_ok());
    }

    #[test]
    fn the_digest_follows_the_sources_and_ignores_what_the_build_makes() {
        let (_root, _all, workspace) = workspace();
        workspace.write("src/lib.rs", "a").unwrap();
        let first = workspace.sources_digest();
        std::fs::create_dir_all(workspace.dir.join("target/release")).unwrap();
        std::fs::write(workspace.dir.join("target/release/out"), "binary").unwrap();
        std::fs::create_dir_all(workspace.dir.join("bin")).unwrap();
        std::fs::write(workspace.dir.join("bin/notes"), "binary").unwrap();
        assert_eq!(workspace.sources_digest(), first, "generated files are not sources");
        assert_eq!(workspace.files().len(), 1);
        workspace.write("src/lib.rs", "b").unwrap();
        assert_ne!(workspace.sources_digest(), first);
    }

    #[test]
    fn the_common_template_fills_every_placeholder() {
        let (_root, _all, workspace) = workspace();
        let values = TemplateValues::of(&NewPlugin {
            name: "my-notes".into(),
            title: "My \"Notes\"".into(),
            description: "Quick notes\nwith a list".into(),
            category: "Office".into(),
        });
        assert_eq!((values.ident.as_str(), values.type_name.as_str()), ("my_notes", "MyNotesPlugin"));
        assert_eq!(values.title, "My Notes", "quotes cannot break a manifest string");
        assert_eq!(values.description, "Quick notes with a list");
        scaffold_common(&workspace, &values).unwrap();
        let paths: Vec<String> = workspace.files().into_iter().map(|entry| entry.path).collect();
        assert_eq!(
            paths,
            [
                "migrations/001_init.sql", "plugin.toml", "skills/my-notes.md", "web/i18n/en.json",
                "web/icon.svg", "web/plugin.css", "web/plugin.js",
            ]
        );
        for path in &paths {
            let text = std::fs::read_to_string(workspace.dir.join(path)).unwrap();
            assert!(!text.contains("@@"), "{path} still has a placeholder");
        }
        let manifest: toml::Value =
            toml::from_str(&std::fs::read_to_string(workspace.dir.join("plugin.toml")).unwrap()).unwrap();
        assert_eq!(manifest["name"].as_str(), Some("my-notes"));
        assert_eq!(manifest["runtime"].as_str(), Some("process"));
        let i18n: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(workspace.dir.join("web/i18n/en.json")).unwrap()).unwrap();
        assert_eq!(i18n["plugin.my-notes.title"], "My Notes");
    }
}
