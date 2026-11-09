//! Editor routes: projects, versions, proposals, export jobs and the
//! sandboxed overlay documents the timeline previews.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::response::Response;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, ok, user_id, RouteHandler};

use super::export::{self, Preset};
use super::project::{self, Project};
use crate::routes::{home, param, take_json};

fn not_found() -> AppError {
    AppError::NotFound("project not found".into())
}

pub fn list() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let h = home(&req);
        let v = tokio::task::spawn_blocking(move || project::list(&h))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        Ok(ok(json!(v)))
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CreateBody {
    title: Option<String>,
    /// Files to import (paths in the home, or recording ids).
    media: Vec<String>,
}

/// A file the caller named: a recording id or a media path in their home.
pub fn resolve_media(home: &Path, s: &str) -> Option<PathBuf> {
    crate::library::resolve(&crate::library::capture_dir(home), s)
        .or_else(|| super::media::allowed(home, Path::new(s)))
}

/// Probe and add files, placing each after the previous one.
pub fn import(
    home: &Path,
    p: &mut Project,
    files: &[String],
    place: bool,
) -> Result<Vec<String>, AppError> {
    let mut ids = Vec::new();
    for f in files.iter().take(64) {
        let path = resolve_media(home, f)
            .ok_or_else(|| AppError::BadRequest(format!("{f}: not a media file in your home")))?;
        let info =
            super::media::probe(&path).map_err(|e| AppError::BadRequest(format!("{f}: {e}")))?;
        let tracks = recording_tracks(&path);
        if p.media.is_empty() && info.has_video && info.width > 0 {
            // The first video decides the canvas.
            p.width = info.width & !1;
            p.height = info.height & !1;
            if info.fps >= 10.0 {
                p.fps = (info.fps.round() as u32).clamp(10, 60);
            }
        }
        let id = p.add_media(&path, info, tracks);
        if place {
            let at = p.duration();
            p.place_media(&id, at);
        }
        ids.push(id);
    }
    p.sanitize();
    Ok(ids)
}

/// Audio stream names of a Capture recording (`mix` first when there are
/// several sources).
fn recording_tracks(path: &Path) -> Vec<String> {
    let meta: Option<Value> = std::fs::read(crate::library::sidecar(path))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let names: Vec<String> = meta
        .and_then(|m| m.get("audio").cloned())
        .and_then(|a| serde_json::from_value(a).ok())
        .unwrap_or_default();
    if names.len() > 1 {
        std::iter::once("mix".to_string()).chain(names).collect()
    } else {
        names
    }
}

pub fn create() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let h = home(&req);
        let b: CreateBody = take_json(req).await?;
        let p = tokio::task::spawn_blocking(move || -> Result<Project, AppError> {
            let mut p = project::create(&h, b.title.as_deref()).map_err(AppError::Internal)?;
            if !b.media.is_empty() {
                import(&h, &mut p, &b.media, true)?;
                project::save(&h, &mut p).map_err(AppError::Internal)?;
            }
            Ok(p)
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
        Ok(ok(json!(p)))
    })
}

pub fn get() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let h = home(&req);
        let id = param(&req, "id")?;
        let p = project::load(&h, &id).ok_or_else(not_found)?;
        let versions = project::versions(&h, &id);
        let mut v = json!(p);
        v["versions"] = json!(versions);
        v["duration"] = json!(p.duration());
        Ok(ok(v))
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PutBody {
    /// The whole document (the editor saves it as the user works).
    project: Option<Project>,
    /// Or operations to apply.
    ops: Vec<Value>,
    /// Files to import.
    import: Vec<String>,
    place: Option<bool>,
    /// Restore this version.
    restore: Option<u64>,
    /// Accept, reject or refine a proposal.
    accept: Option<String>,
    reject: Option<String>,
    /// Expected revision; a mismatch means someone else (the agent) saved.
    revision: Option<u64>,
}

pub fn put() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let h = home(&req);
        let id = param(&req, "id")?;
        let b: PutBody = take_json(req).await?;
        let v = tokio::task::spawn_blocking(move || -> Result<Value, AppError> {
            let mut p = project::load(&h, &id).ok_or_else(not_found)?;
            if let Some(r) = b.revision {
                if r != p.revision {
                    return Err(AppError::BadRequest(format!(
                        "conflict: the project changed (revision {})",
                        p.revision
                    )));
                }
            }
            let mut created = Vec::new();
            if let Some(ts) = b.restore {
                let old = project::load_version(&h, &id, ts)
                    .ok_or_else(|| AppError::NotFound("version not found".into()))?;
                let rev = p.revision;
                p = old;
                p.id = id.clone();
                p.revision = rev;
            }
            if let Some(mut doc) = b.project {
                doc.id = id.clone();
                doc.revision = p.revision;
                doc.created = p.created;
                // Media paths come from the server only.
                doc.media = p.media.clone();
                doc.sanitize();
                p = doc;
            }
            if !b.import.is_empty() {
                created.extend(import(&h, &mut p, &b.import, b.place.unwrap_or(true))?);
            }
            if !b.ops.is_empty() {
                created.extend(project::apply_all(&mut p, &b.ops).map_err(AppError::BadRequest)?);
            }
            if let Some(pid) = b.accept {
                created.extend(accept(&mut p, &pid)?);
            }
            if let Some(pid) = b.reject {
                reject(&mut p, &pid);
            }
            project::save(&h, &mut p).map_err(AppError::Internal)?;
            let mut v = json!(p);
            v["created"] = json!(created);
            v["duration"] = json!(p.duration());
            Ok(v)
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
        Ok(ok(v))
    })
}

/// Accept a proposal: its ghost items become real, its pending ops apply.
pub fn accept(p: &mut Project, pid: &str) -> Result<Vec<String>, AppError> {
    let i = p
        .proposals
        .iter()
        .position(|x| x.id == pid)
        .ok_or_else(|| AppError::NotFound("proposal not found".into()))?;
    let prop = p.proposals.remove(i);
    let ops: Vec<Value> = prop
        .ops
        .iter()
        .map(|o| {
            let mut o = o.clone();
            if let Some(m) = o.as_object_mut() {
                m.remove("ghost");
            }
            o
        })
        .collect();
    // Ghost previews of this proposal are replaced by the real thing.
    let ghosts: Vec<String> = prop
        .ops
        .iter()
        .filter_map(|o| {
            o.get("_ghost_id")
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .collect();
    p.clips.retain(|c| !ghosts.contains(&c.id));
    p.overlays.retain(|o| !ghosts.contains(&o.id));
    project::apply_all(p, &ops).map_err(AppError::BadRequest)
}

pub fn reject(p: &mut Project, pid: &str) {
    if let Some(i) = p.proposals.iter().position(|x| x.id == pid) {
        let prop = p.proposals.remove(i);
        let ghosts: Vec<String> = prop
            .ops
            .iter()
            .filter_map(|o| {
                o.get("_ghost_id")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .collect();
        p.clips.retain(|c| !ghosts.contains(&c.id));
        p.overlays.retain(|o| !ghosts.contains(&o.id));
    }
}

pub fn delete() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let h = home(&req);
        let id = param(&req, "id")?;
        let path = project::path_of(&h, &id).ok_or_else(not_found)?;
        std::fs::remove_file(&path).map_err(|_| not_found())?;
        let _ = std::fs::remove_dir_all(project::projects_dir(&h).join(".versions").join(&id));
        Ok(ok(json!({ "deleted": id })))
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ExportBody {
    preset: Option<String>,
    options: Option<Preset>,
}

pub fn export() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let h = home(&req);
        let id = param(&req, "id")?;
        let b: ExportBody = take_json(req).await?;
        let p = project::load(&h, &id).ok_or_else(not_found)?;
        if p.duration() <= 0.0 {
            return Err(AppError::BadRequest("the timeline is empty".into()));
        }
        let preset = b
            .options
            .unwrap_or_else(|| Preset::named(b.preset.as_deref().unwrap_or("source")));
        let job = export::start(h, p, preset);
        Ok(ok(json!({ "job": job })))
    })
}

pub fn job() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let id = param(&req, "id")?;
        let j = export::job(&id).ok_or_else(|| AppError::NotFound("job not found".into()))?;
        let mut v = json!(j);
        v["output_name"] = json!(j
            .output
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string()));
        Ok(ok(v))
    })
}

/// The overlay as a page for the timeline's preview `<iframe>`. Served with
/// a CSP sandbox: the page gets an opaque origin, no network and no access
/// to the app, whatever the HTML tries.
pub fn overlay_html() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let h = home(&req);
        let pid = param(&req, "project")?;
        let oid = param(&req, "id")?;
        let p = project::load(&h, &pid).ok_or_else(not_found)?;
        let o = p
            .overlays
            .iter()
            .find(|o| o.id == oid)
            .ok_or_else(|| AppError::NotFound("graphic not found".into()))?;
        let w = ((o.rect.w * p.width as f64).round() as u32).max(16);
        let hh = ((o.rect.h * p.height as f64).round() as u32).max(16);
        let doc = super::render::document(&p, o, w, hh);
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-store")
            .header(
                "content-security-policy",
                "sandbox allow-scripts; default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:",
            )
            .header("x-content-type-options", "nosniff")
            .body(Body::from(doc))
            .map_err(|e| AppError::Internal(e.to_string()))
    })
}
