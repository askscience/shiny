//! Files plugin agent tools. Each one is wrapped with `bridged(...)` at
//! registration so its `tokio::fs` work runs on the plugin-owned runtime.

use async_trait::async_trait;
use serde_json::{json, Value};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::fs_util;
use crate::preview;

const MAX_AGENT_READ: usize = 256 * 1024;

async fn home_for(req: &ToolRequest<'_>) -> Result<std::path::PathBuf, AppError> {
    fs_util::ensure_home(req.traveler_id).await
}

async fn resolved(
    req: &ToolRequest<'_>,
    key: &str,
) -> Result<(std::path::PathBuf, std::path::PathBuf), AppError> {
    let home = home_for(req).await?;
    let rel = req.params.param_str(key).unwrap_or_default();
    let path = fs_util::resolve(&home, &rel).await?;
    Ok((home, path))
}

/* ── file_list ──────────────────────────────────────────────── */

pub struct FileList;

#[async_trait]
impl Tool for FileList {
    fn name(&self) -> &str { "file_list" }
    fn aliases(&self) -> &[&str] { &["list_files", "ls_home", "browse_files"] }
    fn step_label(&self) -> &str { "Listing files…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `file_list` — List a folder in the user's home. params: `{ path?: string }` — path is home-relative (`\"\"` = home root, `\"Downloads\"`). Returns `name`, `path`, `kind`, `size`, `modified` per entry.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        let p = data.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let where_ = if p.is_empty() { "home".to_string() } else { p.to_string() };
        format!("Listed {n} entries in {where_}")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let (home, dir) = resolved(&req, "path").await?;
        let meta = tokio::fs::metadata(&dir)
            .await
            .map_err(|_| AppError::NotFound("folder not found".into()))?;
        if !meta.is_dir() {
            return Err(AppError::BadRequest("not a folder".into()));
        }
        let entries = fs_util::list_dir(&home, &dir).await?;
        Ok(ActionOutcome::ok(
            "file_list",
            json!({
                "path": fs_util::rel_display(&home, &dir),
                "entries": entries,
                "count": entries.len(),
            }),
        ))
    }
}

/* ── file_read ──────────────────────────────────────────────── */

pub struct FileRead;

#[async_trait]
impl Tool for FileRead {
    fn name(&self) -> &str { "file_read" }
    fn aliases(&self) -> &[&str] { &["read_file", "open_file", "cat_file"] }
    fn step_label(&self) -> &str { "Reading file…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `file_read` — Read a text file in the user's home. params: `{ path: string }` — home-relative path. Binary files are rejected; report that instead of retrying.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let p = data.get("path").and_then(|v| v.as_str()).unwrap_or("file");
        format!("Read {p}")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let (home, path) = resolved(&req, "path").await?;
        let meta = tokio::fs::metadata(&path)
            .await
            .map_err(|_| AppError::NotFound("file not found".into()))?;
        if meta.is_dir() {
            return Err(AppError::BadRequest("path is a folder — use file_list".into()));
        }
        let bytes = tokio::fs::read(&path).await?;
        if bytes.iter().take(8192).any(|&b| b == 0) {
            return Err(AppError::BadRequest("binary file — not readable as text".into()));
        }
        let truncated = bytes.len() > MAX_AGENT_READ;
        let content = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_AGENT_READ)]).into_owned();
        Ok(ActionOutcome::ok(
            "file_read",
            json!({
                "path": fs_util::rel_display(&home, &path),
                "content": content,
                "size": bytes.len(),
                "truncated": truncated,
            }),
        ))
    }
}

/* ── file_write ─────────────────────────────────────────────── */

pub struct FileWrite;

#[async_trait]
impl Tool for FileWrite {
    fn name(&self) -> &str { "file_write" }
    fn aliases(&self) -> &[&str] { &["write_file", "save_file", "create_file"] }
    fn step_label(&self) -> &str { "Writing file…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `file_write` — Create or overwrite a text file in the user's home. params: `{ path: string, content: string, append?: boolean }` — with `append: true` the content is added to the end.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let p = data.get("path").and_then(|v| v.as_str()).unwrap_or("file");
        format!("Wrote {p}")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let content = req.params.require_str("content")?;
        let (home, path) = resolved(&req, "path").await?;
        if path == home {
            return Err(AppError::BadRequest("path must name a file".into()));
        }
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let append = req.params.param_bool("append").unwrap_or(false);
        if append {
            use tokio::io::AsyncWriteExt;
            let mut f = tokio::fs::OpenOptions::new().create(true).append(true).open(&path).await?;
            f.write_all(content.as_bytes()).await?;
        } else {
            tokio::fs::write(&path, content.as_bytes()).await?;
        }
        Ok(ActionOutcome::ok(
            "file_write",
            json!({
                "path": fs_util::rel_display(&home, &path),
                "bytes": content.len(),
            }),
        ))
    }
}

/* ── file_mkdir ─────────────────────────────────────────────── */

pub struct FileMkdir;

#[async_trait]
impl Tool for FileMkdir {
    fn name(&self) -> &str { "file_mkdir" }
    fn aliases(&self) -> &[&str] { &["create_folder", "make_directory"] }
    fn step_label(&self) -> &str { "Creating folder…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `file_mkdir` — Create a folder in the user's home. params: `{ path: string }`")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let p = data.get("path").and_then(|v| v.as_str()).unwrap_or("folder");
        format!("Created folder {p}")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let rel = req.params.require_str("path")?;
        if rel.trim().is_empty() {
            return Err(AppError::BadRequest("path required".into()));
        }
        let (home, path) = resolved(&req, "path").await?;
        tokio::fs::create_dir_all(&path).await?;
        Ok(ActionOutcome::ok(
            "file_mkdir",
            json!({ "path": fs_util::rel_display(&home, &path) }),
        ))
    }
}

/* ── file_move ──────────────────────────────────────────────── */

pub struct FileMove;

#[async_trait]
impl Tool for FileMove {
    fn name(&self) -> &str { "file_move" }
    fn aliases(&self) -> &[&str] { &["move_file", "rename_file"] }
    fn step_label(&self) -> &str { "Moving file…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `file_move` — Move or rename an entry. params: `{ from: string, to: string }` — both home-relative.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let to = data.get("to").and_then(|v| v.as_str()).unwrap_or("path");
        format!("Moved to {to}")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let from_rel = req.params.require_str("from")?;
        let to_rel = req.params.require_str("to")?;
        let home = home_for(&req).await?;
        let from = fs_util::resolve(&home, &from_rel).await?;
        let to = fs_util::resolve(&home, &to_rel).await?;
        if tokio::fs::symlink_metadata(&from).await.is_err() {
            return Err(AppError::NotFound("source not found".into()));
        }
        if let Some(parent) = to.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::rename(&from, &to).await?;
        Ok(ActionOutcome::ok(
            "file_move",
            json!({
                "from": fs_util::rel_display(&home, &from),
                "to": fs_util::rel_display(&home, &to),
            }),
        ))
    }
}

/* ── file_copy ──────────────────────────────────────────────── */

pub struct FileCopy;

#[async_trait]
impl Tool for FileCopy {
    fn name(&self) -> &str { "file_copy" }
    fn aliases(&self) -> &[&str] { &["copy_file", "duplicate_file"] }
    fn step_label(&self) -> &str { "Copying…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `file_copy` — Copy a file or folder. params: `{ from: string, to: string }`")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let to = data.get("to").and_then(|v| v.as_str()).unwrap_or("path");
        format!("Copied to {to}")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let from_rel = req.params.require_str("from")?;
        let to_rel = req.params.require_str("to")?;
        let home = home_for(&req).await?;
        let from = fs_util::resolve(&home, &from_rel).await?;
        let to = fs_util::resolve(&home, &to_rel).await?;
        if tokio::fs::symlink_metadata(&from).await.is_err() {
            return Err(AppError::NotFound("source not found".into()));
        }
        fs_util::copy_entry(&from, &to).await?;
        Ok(ActionOutcome::ok(
            "file_copy",
            json!({
                "from": fs_util::rel_display(&home, &from),
                "to": fs_util::rel_display(&home, &to),
            }),
        ))
    }
}

/* ── file_delete ────────────────────────────────────────────── */

pub struct FileDelete;

#[async_trait]
impl Tool for FileDelete {
    fn name(&self) -> &str { "file_delete" }
    fn aliases(&self) -> &[&str] { &["delete_file", "remove_file", "trash_file"] }
    fn step_label(&self) -> &str { "Moving to trash…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `file_delete` — Move an entry to Trash (recoverable with `file_restore`). params: `{ path: string }`")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let p = data.get("trashed").and_then(|v| v.as_str()).unwrap_or("item");
        format!("Moved {p} to trash")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let rel = req.params.require_str("path")?;
        if rel.trim().is_empty() {
            return Err(AppError::BadRequest("cannot delete the home folder".into()));
        }
        let (_, path) = resolved(&req, "path").await?;
        let home = home_for(&req).await?;
        if tokio::fs::symlink_metadata(&path).await.is_err() {
            return Err(AppError::NotFound("file not found".into()));
        }
        if rel == fs_util::TRASH_DIR || rel.starts_with(&format!("{}/", fs_util::TRASH_DIR)) {
            fs_util::delete_permanent(&path).await?;
            return Ok(ActionOutcome::ok(
                "file_delete",
                json!({ "deleted": fs_util::rel_display(&home, &path) }),
            ));
        }
        let trashed = fs_util::move_to_trash(&home, &path).await?;
        Ok(ActionOutcome::ok("file_delete", json!({ "trashed": trashed })))
    }
}

/* ── file_restore ───────────────────────────────────────────── */

pub struct FileRestore;

#[async_trait]
impl Tool for FileRestore {
    fn name(&self) -> &str { "file_restore" }
    fn aliases(&self) -> &[&str] { &["restore_file", "undelete_file"] }
    fn step_label(&self) -> &str { "Restoring…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `file_restore` — Restore something from Trash back to the home folder. params: `{ name: string }` — the entry name inside `.Trash`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let p = data.get("restored").and_then(|v| v.as_str()).unwrap_or("item");
        format!("Restored {p}")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let name = req
            .params
            .param_str("name")
            .or_else(|| req.params.param_str("path"))
            .ok_or_else(|| AppError::BadRequest("name required".into()))?;
        let home = home_for(&req).await?;
        let restored = fs_util::restore_from_trash(&home, &name).await?;
        Ok(ActionOutcome::ok("file_restore", json!({ "restored": restored })))
    }
}

/* ── file_search ────────────────────────────────────────────── */

pub struct FileSearch;

#[async_trait]
impl Tool for FileSearch {
    fn name(&self) -> &str { "file_search" }
    fn aliases(&self) -> &[&str] { &["search_files", "find_file"] }
    fn step_label(&self) -> &str { "Searching files…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `file_search` — Search the user's home for entries whose name contains a string. params: `{ query: string, path?: string }`")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("results").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        format!("Found {n} matching files")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let query = req.params.require_str("query")?;
        if query.trim().len() < 2 {
            return Err(AppError::BadRequest("query must be at least 2 characters".into()));
        }
        let home = home_for(&req).await?;
        let start = fs_util::resolve(&home, &req.params.param_str("path").unwrap_or_default()).await?;
        let results = fs_util::search(&home, &start, query.trim(), 100).await?;
        Ok(ActionOutcome::ok(
            "file_search",
            json!({ "query": query, "results": results, "count": results.len() }),
        ))
    }
}

/* ── file_info ──────────────────────────────────────────────── */

pub struct FileInfo;

#[async_trait]
impl Tool for FileInfo {
    fn name(&self) -> &str { "file_info" }
    fn aliases(&self) -> &[&str] { &["stat_file", "file_details"] }
    fn step_label(&self) -> &str { "Reading file info…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `file_info` — Details about an entry: kind, size, modified time, MIME type. params: `{ path: string }`")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let p = data.get("path").and_then(|v| v.as_str()).unwrap_or("item");
        format!("Inspected {p}")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let (home, path) = resolved(&req, "path").await?;
        let meta = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|_| AppError::NotFound("not found".into()))?;
        let ft = meta.file_type();
        let kind = if ft.is_dir() { "dir" } else if ft.is_symlink() { "symlink" } else { "file" };
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut data = json!({
            "path": fs_util::rel_display(&home, &path),
            "kind": kind,
            "size": meta.len(),
            "modified": modified,
            "mime": preview::mime_for(&path),
        });
        if let Some(obj) = data.as_object_mut() {
            obj.insert("is_image".into(), json!(preview::is_image(&path)));
            obj.insert("is_text".into(), json!(preview::is_text(&path)));
            obj.insert("is_pdf".into(), json!(preview::is_pdf(&path)));
            obj.insert("is_office".into(), json!(preview::is_office(&path)));
        }
        Ok(ActionOutcome::ok("file_info", data))
    }
}
