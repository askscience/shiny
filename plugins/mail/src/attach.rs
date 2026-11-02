//! Attachments: files the user (or the assistant) attaches to an outgoing mail,
//! and attachments of a received mail read back or saved into the home.
//!
//! Outgoing attachments arrive two ways: as home-relative paths (the assistant's
//! `mail_send`, or the window picking through the shell's file picker) and as
//! uploaded multipart fields (the window's compose). Both are resolved into
//! [`OutAttachment`]s with the same total-size ceiling, so neither path can
//! make the host buffer an unbounded body.

use std::path::{Component, Path, PathBuf};

use mail_parser::{MessageParser, MimeHeaders};
use serde::Deserialize;

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::util::sanitize_user;

use crate::mail::{self, Account};

/// Most providers refuse a message over ~25 MB; base64 adds a third on the
/// wire, so the raw attachments stay under that.
pub const MAX_ATTACHMENTS_BYTES: usize = 18 * 1024 * 1024;

/// One file to attach to an outgoing message.
#[derive(Debug, Clone)]
pub struct OutAttachment {
    pub filename: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// An attachment named in a JSON send body / tool call.
#[derive(Debug, Clone, Deserialize)]
pub struct AttachmentRef {
    /// A file in the user's home: home-relative (`Documents/report.pdf`),
    /// `~/`-prefixed, or absolute inside the home.
    pub path: String,
    /// Name shown to the recipient; defaults to the file's own name.
    pub filename: Option<String>,
}

/// The home directory a request operates on — the same rule as the Files
/// plugin: the caller's real OS home with Linux-user binding, otherwise the
/// account's virtual `~/.shiny/home/<id>`.
pub fn home_for(user_id: &str, os_home: Option<&str>) -> PathBuf {
    match os_home.map(str::trim).filter(|h| !h.is_empty()) {
        Some(home) => PathBuf::from(home),
        None => os_home_dir().join(".shiny").join("home").join(sanitize_user(user_id)),
    }
}

fn os_home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Resolve `path` to an existing regular file inside `home`. Anything that
/// leaves the home (`..`, an absolute path elsewhere, a symlink out) is refused.
pub fn resolve_in_home(home: &Path, path: &str) -> Result<PathBuf, AppError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("attachment path is empty".into()));
    }
    let rel = trimmed.strip_prefix("~/").unwrap_or(trimmed);
    let candidate = if Path::new(rel).is_absolute() {
        PathBuf::from(rel)
    } else {
        if Path::new(rel).components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(AppError::BadRequest(format!("attachment path leaves the home: {trimmed}")));
        }
        home.join(rel)
    };
    let home_real = home
        .canonicalize()
        .map_err(|_| AppError::BadRequest("the home folder is not available".into()))?;
    let real = candidate
        .canonicalize()
        .map_err(|_| AppError::BadRequest(format!("attachment not found: {trimmed}")))?;
    if !real.starts_with(&home_real) {
        return Err(AppError::BadRequest(format!("attachment is outside the home folder: {trimmed}")));
    }
    if !real.is_file() {
        return Err(AppError::BadRequest(format!("attachment is not a file: {trimmed}")));
    }
    Ok(real)
}

/// A content type from the file name (enough for mail clients to pick a
/// viewer; anything unknown is `application/octet-stream`).
pub fn guess_content_type(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "txt" | "md" | "log" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "zip" => "application/zip",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "doc" => "application/msword",
        "xls" => "application/vnd.ms-excel",
        "ppt" => "application/vnd.ms-powerpoint",
        "ics" => "text/calendar",
        _ => "application/octet-stream",
    }
}

/// Keep a file name usable as one path component and as a MIME filename.
pub fn safe_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .trim_start_matches('.')
        .to_string();
    if cleaned.is_empty() { "attachment".into() } else { cleaned }
}

/// Read the referenced home files, enforcing the total-size ceiling.
pub fn load_refs(
    user_id: &str,
    os_home: Option<&str>,
    refs: &[AttachmentRef],
    already: usize,
) -> Result<Vec<OutAttachment>, AppError> {
    if refs.is_empty() {
        return Ok(Vec::new());
    }
    let home = home_for(user_id, os_home);
    let mut total = already;
    let mut out = Vec::with_capacity(refs.len());
    for r in refs {
        let path = resolve_in_home(&home, &r.path)?;
        let size = path.metadata().map(|m| m.len() as usize).unwrap_or(0);
        total = total.saturating_add(size);
        if total > MAX_ATTACHMENTS_BYTES {
            return Err(too_large());
        }
        let bytes = std::fs::read(&path)
            .map_err(|e| AppError::BadRequest(format!("could not read {}: {e}", r.path)))?;
        let own = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let filename = safe_filename(r.filename.as_deref().filter(|n| !n.trim().is_empty()).unwrap_or(&own));
        let content_type = guess_content_type(&filename).to_string();
        out.push(OutAttachment { filename, content_type, bytes });
    }
    Ok(out)
}

pub fn too_large() -> AppError {
    AppError::BadRequest(format!(
        "attachments are too large ({} MB max in total)",
        MAX_ATTACHMENTS_BYTES / (1024 * 1024)
    ))
}

/// One attachment of a received message: its name, type and bytes.
pub struct InAttachment {
    pub filename: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// Fetch a message and return its `index`-th attachment (the order the
/// message's attachment list reports).
pub async fn fetch_attachment(
    a: Account,
    folder: String,
    id: String,
    index: usize,
) -> Result<InAttachment, AppError> {
    let raw = mail::get_raw_message(a, folder, id).await?;
    let msg = MessageParser::new()
        .parse(&raw[..])
        .ok_or_else(|| AppError::BadRequest("could not parse message".into()))?;
    let att = msg
        .attachments()
        .nth(index)
        .ok_or_else(|| AppError::NotFound("attachment not found".into()))?;
    let filename = safe_filename(att.attachment_name().unwrap_or("attachment"));
    let content_type = att
        .content_type()
        .map(|ct| match ct.subtype() {
            Some(sub) => format!("{}/{}", ct.ctype(), sub),
            None => ct.ctype().to_string(),
        })
        .unwrap_or_else(|| guess_content_type(&filename).to_string());
    Ok(InAttachment { filename, content_type, bytes: att.contents().to_vec() })
}

/// Write bytes into `dir` (home-relative, created when missing) under a name
/// that does not overwrite an existing file. Returns the home-relative path.
pub fn save_into_home(
    user_id: &str,
    os_home: Option<&str>,
    dir: &str,
    filename: &str,
    bytes: &[u8],
) -> Result<String, AppError> {
    save_in(&home_for(user_id, os_home), dir, filename, bytes)
}

fn save_in(home: &Path, dir: &str, filename: &str, bytes: &[u8]) -> Result<String, AppError> {
    let dir_rel = dir.trim().trim_start_matches("~/").trim_matches('/');
    if Path::new(dir_rel).is_absolute()
        || Path::new(dir_rel).components().any(|c| matches!(c, Component::ParentDir))
    {
        return Err(AppError::BadRequest("folder must be inside the home".into()));
    }
    let target_dir = if dir_rel.is_empty() { home.to_path_buf() } else { home.join(dir_rel) };
    std::fs::create_dir_all(&target_dir)
        .map_err(|e| AppError::BadRequest(format!("could not create {dir_rel}: {e}")))?;
    let name = safe_filename(filename);
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    let mut candidate = target_dir.join(&name);
    let mut n = 1;
    while candidate.exists() {
        candidate = target_dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    std::fs::write(&candidate, bytes)
        .map_err(|e| AppError::BadRequest(format!("could not save the attachment: {e}")))?;
    let rel = candidate.strip_prefix(home).unwrap_or(&candidate);
    Ok(rel.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filenames_are_single_components() {
        assert_eq!(safe_filename("../../etc/passwd"), "passwd");
        assert_eq!(safe_filename("a\\b.pdf"), "b.pdf");
        assert_eq!(safe_filename(".hidden"), "hidden");
        assert_eq!(safe_filename("  "), "attachment");
    }

    #[test]
    fn paths_must_stay_in_the_home() {
        let root = std::env::temp_dir().join(format!("mail-attach-{}", std::process::id()));
        let home = root.join("home");
        std::fs::create_dir_all(home.join("Documents")).unwrap();
        std::fs::write(home.join("Documents/a.txt"), b"hi").unwrap();
        std::fs::write(root.join("outside.txt"), b"no").unwrap();

        assert!(resolve_in_home(&home, "Documents/a.txt").is_ok());
        assert!(resolve_in_home(&home, "~/Documents/a.txt").is_ok());
        assert!(resolve_in_home(&home, "../outside.txt").is_err());
        assert!(resolve_in_home(&home, root.join("outside.txt").to_str().unwrap()).is_err());
        assert!(resolve_in_home(&home, "Documents").is_err());

        assert_eq!(save_in(&home, "Downloads", "a.txt", b"x").unwrap(), "Downloads/a.txt");
        assert_eq!(save_in(&home, "Downloads", "a.txt", b"y").unwrap(), "Downloads/a (1).txt");
        assert!(save_in(&home, "../x", "a.txt", b"z").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

}
