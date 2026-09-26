//! Files plugin filesystem helpers: virtual home layout, sandboxed path
//! resolution, directory listing, trash and search. Everything here is real
//! filesystem I/O through `tokio::fs` — the plugin owns no database.

use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use shiny_plugin_sdk::errors::AppError;

use crate::preview;

/// The classic Linux home folders every account is provisioned with.
pub const CLASSIC_DIRS: &[&str] = &[
    "Desktop",
    "Documents",
    "Downloads",
    "Music",
    "Pictures",
    "Public",
    "Templates",
    "Videos",
];

pub const TRASH_DIR: &str = ".Trash";
pub const CACHE_DIR: &str = ".cache";

/// The OS home directory (real `$HOME`, falling back to `USERPROFILE`, then
/// the process working directory).
pub fn os_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Root of every Shiny user's virtual home. Hidden app-data path so it never
/// collides with a project folder (the repo itself is often `~/shiny`).
pub fn shiny_root() -> PathBuf {
    os_home().join(".shiny").join("home")
}

/// Keep an account id usable as a single directory name.
pub fn sanitize_user(user_id: &str) -> String {
    let cleaned: String = user_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(64)
        .collect();
    if cleaned.is_empty() {
        "default".into()
    } else {
        cleaned
    }
}

pub fn user_home(user_id: &str) -> PathBuf {
    shiny_root().join(sanitize_user(user_id))
}

/// The home directory a request should operate on.
///
/// With Linux-user binding the caller's real OS home (`os_home`) is used;
/// otherwise the account keeps its virtual `~/.shiny/home/<id>`.
pub fn home_for(user_id: &str, os_home: Option<&str>) -> PathBuf {
    match os_home.map(str::trim).filter(|h| !h.is_empty()) {
        Some(home) => PathBuf::from(home),
        None => user_home(user_id),
    }
}

/// Display form of the home path (used by the window's breadcrumb root).
/// Virtual homes read as `~/.shiny/home/<id>`; a real OS home is shown by its
/// absolute path.
pub fn home_display_for(home: &Path) -> String {
    let root = shiny_root();
    if let Ok(rel) = home.strip_prefix(&root) {
        let rel = rel.to_string_lossy();
        if rel.is_empty() {
            return "~/.shiny/home".into();
        }
        return format!("~/.shiny/home/{rel}");
    }
    home.to_string_lossy().into_owned()
}

/// Thumbnail cache. Namespaced under `shiny/` so a real `~/.cache` is never
/// clobbered by the desktop's own thumbnails.
pub fn thumbs_dir(home: &Path) -> PathBuf {
    home.join(".cache").join("shiny").join("thumbnails")
}

/// Create any missing classic home folders (Desktop, Documents, …). Idempotent
/// and never clobbers an existing directory — used for both the virtual home
/// and a real OS home that predates Shiny.
async fn provision_classic_dirs(home: &Path) -> Result<(), AppError> {
    for dir in CLASSIC_DIRS {
        tokio::fs::create_dir_all(home.join(dir)).await?;
    }
    Ok(())
}

/// Create the user's virtual home plus the classic folders. Idempotent; called
/// for a brand-new account by `on_user_registered` and lazily by every
/// route/tool so existing accounts are backfilled on first use.
pub async fn ensure_home(user_id: &str) -> Result<PathBuf, AppError> {
    let home = user_home(user_id);
    tokio::fs::create_dir_all(&home).await?;
    provision_classic_dirs(&home).await?;
    tokio::fs::create_dir_all(home.join(CACHE_DIR)).await?;
    Ok(home)
}

/// Resolve the home for a request and make sure it exists. With an `os_home`
/// the real directory must already exist (it belongs to a Linux account); only
/// missing classic folders are added.
pub async fn ensure_home_for(user_id: &str, os_home: Option<&str>) -> Result<PathBuf, AppError> {
    match os_home.map(str::trim).filter(|h| !h.is_empty()) {
        Some(h) => {
            let home = PathBuf::from(h);
            let meta = tokio::fs::metadata(&home).await.map_err(|_| {
                AppError::Internal(format!("home folder {h} is unavailable"))
            })?;
            if !meta.is_dir() {
                return Err(AppError::Internal(format!("home path {h} is not a folder")));
            }
            provision_classic_dirs(&home).await?;
            Ok(home)
        }
        None => ensure_home(user_id).await,
    }
}

/// Validate a user-supplied relative path and return its cleaned form.
fn clean_rel(rel: &str) -> Result<PathBuf, AppError> {
    if rel.contains('\0') {
        return Err(AppError::BadRequest("invalid path".into()));
    }
    let rel = rel.trim();
    let rel = rel.strip_prefix("./").unwrap_or(rel);
    let p = Path::new(rel);
    if p.is_absolute() {
        return Err(AppError::BadRequest("absolute paths are not allowed".into()));
    }
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::Normal(seg) => out.push(seg),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(AppError::BadRequest("'..' is not allowed".into()))
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(AppError::BadRequest("absolute paths are not allowed".into()))
            }
        }
    }
    Ok(out)
}

/// Resolve `rel` inside `home`, refusing anything that escapes the sandbox.
/// Existing symlinks are canonicalized and checked; for not-yet-existing
/// targets the nearest existing ancestor is checked instead.
pub async fn resolve(home: &Path, rel: &str) -> Result<PathBuf, AppError> {
    let rel = clean_rel(rel)?;
    let canon_home = tokio::fs::canonicalize(home)
        .await
        .map_err(|_| AppError::Internal("home folder is unavailable".into()))?;

    if rel.as_os_str().is_empty() {
        return Ok(canon_home);
    }
    let target = canon_home.join(&rel);

    if let Ok(canon) = tokio::fs::canonicalize(&target).await {
        if !canon.starts_with(&canon_home) {
            return Err(AppError::BadRequest("path escapes the home folder".into()));
        }
        return Ok(canon);
    }

    // Target does not exist yet — walk up to the nearest existing ancestor and
    // confirm that is still inside the home sandbox (guards symlinked dirs).
    let mut ancestor = target
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| canon_home.clone());
    loop {
        if tokio::fs::symlink_metadata(&ancestor).await.is_ok() {
            break;
        }
        match ancestor.parent() {
            Some(p) => ancestor = p.to_path_buf(),
            None => break,
        }
    }
    let canon_ancestor = tokio::fs::canonicalize(&ancestor)
        .await
        .map_err(|_| AppError::BadRequest("invalid path".into()))?;
    if !canon_ancestor.starts_with(&canon_home) {
        return Err(AppError::BadRequest("path escapes the home folder".into()));
    }
    Ok(target)
}

/// Home-relative path with `/` separators, for the API surface.
pub fn rel_display(home: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(home).unwrap_or(path);
    let mut out = String::new();
    for comp in rel.components() {
        if let Component::Normal(seg) = comp {
            if !out.is_empty() {
                out.push('/');
            }
            out.push_str(&seg.to_string_lossy());
        }
    }
    out
}

fn kind_str(ft: &std::fs::FileType) -> &'static str {
    if ft.is_dir() {
        "dir"
    } else if ft.is_symlink() {
        "symlink"
    } else {
        "file"
    }
}

/// One directory entry as the JSON the window consumes.
pub async fn entry_json(home: &Path, path: &Path) -> Option<Value> {
    let meta = tokio::fs::symlink_metadata(path).await.ok()?;
    let ft = meta.file_type();
    let kind = kind_str(&ft);
    let size = if ft.is_file() { meta.len() } else { 0 };
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Some(json!({
        "name": name,
        "path": rel_display(home, path),
        "kind": kind,
        "ext": preview::ext_of(path),
        "size": size,
        "modified": modified,
        "hidden": name.starts_with('.'),
        "is_image": preview::is_image(path),
        "is_text": preview::is_text(path),
        "is_pdf": preview::is_pdf(path),
        "is_office": preview::is_office(path),
        "is_video": preview::is_video(path),
        "is_audio": preview::is_audio(path),
        "is_archive": preview::is_archive(path),
    }))
}

/// Immediate child count for a directory (capped). Powers GNOME's "N items".
pub async fn dir_count(path: &Path) -> u64 {
    let mut n = 0u64;
    if let Ok(mut rd) = tokio::fs::read_dir(path).await {
        while let Ok(Some(_)) = rd.next_entry().await {
            n += 1;
            if n >= 9999 {
                break;
            }
        }
    }
    n
}

/// List a directory, folders first then case-insensitive by name.
pub async fn list_dir(home: &Path, dir: &Path) -> Result<Vec<Value>, AppError> {
    let mut entries: Vec<Value> = Vec::new();
    let mut rd = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = rd.next_entry().await? {
        let path = entry.path();
        let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
        if let Some(mut v) = entry_json(home, &path).await {
            if is_dir {
                let count = dir_count(&path).await;
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("count".into(), json!(count));
                }
            }
            entries.push(v);
        }
    }
    entries.sort_by(|a, b| {
        let ad = a.get("kind").and_then(|k| k.as_str()) == Some("dir");
        let bd = b.get("kind").and_then(|k| k.as_str()) == Some("dir");
        bd.cmp(&ad).then_with(|| {
            let an = a.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let bn = b.get("name").and_then(|n| n.as_str()).unwrap_or("");
            an.to_lowercase().cmp(&bn.to_lowercase())
        })
    });
    Ok(entries)
}

fn split_ext(name: &str) -> (String, String) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => (stem.to_string(), ext.to_string()),
        _ => (name.to_string(), String::new()),
    }
}

async fn exists(path: &Path) -> bool {
    tokio::fs::symlink_metadata(path).await.is_ok()
}

/// Pick a non-colliding destination inside `dir` for `name`.
async fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !exists(&first).await {
        return first;
    }
    let (stem, ext) = split_ext(name);
    for n in 1..10_000u32 {
        let fname = if ext.is_empty() {
            format!("{stem}.{n}")
        } else {
            format!("{stem}.{n}.{ext}")
        };
        let candidate = dir.join(&fname);
        if !exists(&candidate).await {
            return candidate;
        }
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    dir.join(format!("{stem}-{ts}"))
}

/// Move an entry into the user's `.Trash`, returning its new home-relative path.
pub async fn move_to_trash(home: &Path, path: &Path) -> Result<String, AppError> {
    let trash = home.join(TRASH_DIR);
    tokio::fs::create_dir_all(&trash).await?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "item".into());
    let dest = unique_path(&trash, &name).await;
    tokio::fs::rename(path, &dest).await?;
    Ok(rel_display(home, &dest))
}

/// Restore a trashed entry back to the home root (or into `dest` under home).
pub async fn restore_from_trash(home: &Path, name: &str) -> Result<String, AppError> {
    let trash = home.join(TRASH_DIR);
    let src = resolve(&trash, name).await?;
    if !exists(&src).await {
        return Err(AppError::NotFound("not in trash".into()));
    }
    let file_name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "restored".into());
    let dest = unique_path(home, &file_name).await;
    tokio::fs::rename(&src, &dest).await?;
    Ok(rel_display(home, &dest))
}

/// Permanently remove a file or directory (used when deleting from Trash).
pub async fn delete_permanent(path: &Path) -> Result<(), AppError> {
    let meta = tokio::fs::symlink_metadata(path).await?;
    if meta.is_dir() {
        tokio::fs::remove_dir_all(path).await?;
    } else {
        tokio::fs::remove_file(path).await?;
    }
    Ok(())
}

/// Empty the trash. Idempotent.
pub async fn empty_trash(home: &Path) -> Result<(), AppError> {
    let trash = home.join(TRASH_DIR);
    if exists(&trash).await {
        tokio::fs::remove_dir_all(&trash).await?;
    }
    tokio::fs::create_dir_all(&trash).await?;
    Ok(())
}

/// Recursively copy a file or directory tree.
pub async fn copy_entry(src: &Path, dest: &Path) -> Result<(), AppError> {
    let meta = tokio::fs::symlink_metadata(src).await?;
    if meta.is_dir() {
        tokio::fs::create_dir_all(dest).await?;
        let mut rd = tokio::fs::read_dir(src).await?;
        while let Some(entry) = rd.next_entry().await? {
            let child = dest.join(entry.file_name());
            Box::pin(copy_entry(&entry.path(), &child)).await?;
        }
    } else {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::copy(src, dest).await?;
    }
    Ok(())
}

/// Recursively search `start` for names containing `query` (case-insensitive).
/// Bounded by depth and node count; skips `.Trash` and `.cache`.
pub async fn search(
    home: &Path,
    start: &Path,
    query: &str,
    limit: usize,
) -> Result<Vec<Value>, AppError> {
    const MAX_DEPTH: usize = 8;
    const MAX_NODES: usize = 20_000;
    let needle = query.to_lowercase();
    let mut out: Vec<Value> = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(start.to_path_buf(), 0)];
    let mut visited = 0usize;

    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_DEPTH || out.len() >= limit || visited >= MAX_NODES {
            break;
        }
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        while let Some(entry) = rd.next_entry().await.ok().flatten() {
            visited += 1;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == TRASH_DIR || name == CACHE_DIR {
                continue;
            }
            if name.to_lowercase().contains(&needle) {
                if let Some(v) = entry_json(home, &path).await {
                    out.push(v);
                }
                if out.len() >= limit {
                    break;
                }
            }
            let is_dir = entry
                .file_type()
                .await
                .map(|t| t.is_dir())
                .unwrap_or(false);
            if is_dir {
                stack.push((path, depth + 1));
            }
        }
    }
    Ok(out)
}
