//! REST routes for link sharing (`/api/files/share*`).
//!
//! Host-level setup lives in Settings → Remote (`web/js/settings/fileSharing.js`).
//! Install and enable are refused from remote clients (`require_local`); any
//! signed-in local user may use them, since core has no admin role yet
//! (`RouteSpec::admin` is treated as auth). Shares themselves are per-account
//! through `file_shares.owner`.

use std::sync::Arc;

use axum::extract::{FromRequest, Request};
use serde::Deserialize;
use serde_json::{json, Value as Json};
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, ok, user_id, RouteHandler};
use shiny_plugin_sdk::services::PluginCtx;

use super::server::{self, Instance};
use super::{cli, install, rest, store, GOKAPI_VERSION};
use crate::fs_util;
use crate::routes::{request_home, take_query};

/// Upper bounds for the share options (Gokapi itself accepts any int).
const MAX_EXPIRY_DAYS: u32 = 3650;
const MAX_DOWNLOADS: u32 = 100_000;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    Some(match tag {
        "files_share_status" => status(ctx),
        "files_share_install" => install_route(ctx),
        "files_share_setup" => setup(ctx),
        "files_share_disable" => disable(ctx),
        "files_share_create" => create(ctx),
        "files_share_job" => job(ctx),
        "files_share_list" => list(ctx),
        "files_share_revoke" => revoke(ctx),
        _ => return None,
    })
}

fn internal(e: String) -> AppError {
    AppError::Internal(e)
}

async fn json_body<T: serde::de::DeserializeOwned>(req: Request) -> Result<T, AppError> {
    let axum::Json(body) = axum::Json::<T>::from_request(req, &())
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
    Ok(body)
}

/// Installing and turning sharing on publish this machine to the internet, so
/// (like core's Tailscale enable) they are refused from remote clients.
/// Turning it off stays allowed from anywhere.
fn require_local(req: &Request) -> Result<(), AppError> {
    if shiny_plugin_sdk::is_remote_request(req) {
        return Err(AppError::Unauthorized(
            "link sharing can only be set up from this machine".into(),
        ));
    }
    Ok(())
}

/// The instance, when sharing has been set up and enabled.
fn enabled_instance() -> Result<Instance, AppError> {
    server::load_instance()
        .filter(|i| i.enabled)
        .ok_or_else(|| AppError::BadRequest("link sharing is not set up".into()))
}

fn status_json() -> Json {
    let inst = server::load_instance();
    json!({
        "installed": install::server_bin().is_some() && install::cli_bin().is_some(),
        "bundled": install::is_bundled(),
        "version": GOKAPI_VERSION,
        "install": install::state_json(),
        "configured": inst.is_some(),
        "enabled": inst.as_ref().map(|i| i.enabled).unwrap_or(false),
        "running": server::is_running(),
        "mode": inst.as_ref().map(|i| i.mode.clone()),
        "public_url": inst.as_ref().map(|i| i.public_url.clone()),
        // What Funnel / a custom reverse proxy must forward to.
        "forward_port": inst.as_ref().map(|i| i.gate_port).filter(|p| *p != 0),
        "e2e": inst.as_ref().map(|i| i.e2e).unwrap_or(false),
        "tailscale": server::tailscale_installed(),
        "error": server::last_error(),
    })
}

/* ── setup ────────────────────────────────────────────────── */

fn status(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            user_id(&req)?;
            Ok(ok(status_json()))
        }
    })
}

fn install_route(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            user_id(&req)?;
            require_local(&req)?;
            let started = install::start().map_err(AppError::BadRequest)?;
            Ok(ok(json!({ "started": started, "status": status_json() })))
        }
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SetupBody {
    /// `"funnel"` (default) or `"custom"`.
    mode: Option<String>,
    /// Public base URL for `"custom"`.
    url: Option<String>,
}

fn setup(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            user_id(&req)?;
            require_local(&req)?;
            let body: SetupBody = json_body(req).await?;
            let mode = body.mode.unwrap_or_else(|| "funnel".into());
            if install::server_bin().is_none() || install::cli_bin().is_none() {
                return Err(AppError::BadRequest("Gokapi is not installed yet".into()));
            }
            let url = match mode.as_str() {
                "funnel" => {
                    if !server::tailscale_installed() {
                        return Err(AppError::BadRequest("Tailscale is not installed".into()));
                    }
                    server::funnel_url(
                        &server::tailnet_hostname()
                            .await
                            .map_err(AppError::BadRequest)?,
                    )
                }
                "custom" => server::normalize_url(body.url.as_deref().unwrap_or(""))
                    .map_err(AppError::BadRequest)?,
                _ => return Err(AppError::BadRequest("mode must be funnel or custom".into())),
            };

            let previous = server::load_instance();
            let mut inst = server::bootstrap(&url, &mode).await.map_err(internal)?;
            let url_changed = inst.public_url != url;
            if url_changed {
                server::set_server_url(&url).map_err(internal)?;
                inst.public_url = url;
            }
            server::ensure_gate(&mut inst).map_err(internal)?;
            if mode == "funnel" {
                server::funnel(inst.gate_port, true)
                    .await
                    .map_err(AppError::BadRequest)?;
            } else if previous.map(|p| p.mode == "funnel").unwrap_or(false) {
                let _ = server::funnel(inst.gate_port, false).await;
            }
            inst.mode = mode;
            inst.enabled = true;
            server::save_instance(&inst).map_err(internal)?;

            // Gokapi reads ServerUrl only at start.
            if url_changed {
                server::stop_server().await;
            }
            server::ensure_running(&inst).await.map_err(internal)?;
            Ok(ok(status_json()))
        }
    })
}

fn disable(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            user_id(&req)?;
            server::stop_server().await;
            if let Some(mut inst) = server::load_instance() {
                if inst.mode == "funnel" {
                    let _ = server::funnel(inst.gate_port, false).await;
                }
                inst.enabled = false;
                server::save_instance(&inst).map_err(internal)?;
            }
            Ok(ok(status_json()))
        }
    })
}

/* ── shares ───────────────────────────────────────────────── */

#[derive(Deserialize, Default)]
#[serde(default)]
struct CreateBody {
    path: Option<String>,
    expiry_days: Option<u32>,
    max_downloads: Option<u32>,
    password: Option<String>,
    /// Shown on the download page: "<from> wants to share a file with you".
    from: Option<String>,
    message: Option<String>,
    /// End-to-end encrypt this link (turns E2E on the first time).
    e2e: Option<bool>,
}

fn create(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let home = request_home(&uid, shiny_plugin_sdk::os_home_from_request(&req)).await?;
            let body: CreateBody = json_body(req).await?;
            let rel = body.path.unwrap_or_default();
            if rel.trim().is_empty() {
                return Err(AppError::BadRequest("path is required".into()));
            }
            let abs = fs_util::resolve(&home, &rel).await?;
            if abs == home {
                return Err(AppError::BadRequest(
                    "the home folder itself cannot be shared".into(),
                ));
            }
            let meta = tokio::fs::metadata(&abs)
                .await
                .map_err(|_| AppError::NotFound("file not found".into()))?;
            let expiry_days = body.expiry_days.unwrap_or(0);
            let max_downloads = body.max_downloads.unwrap_or(0);
            if expiry_days > MAX_EXPIRY_DAYS || max_downloads > MAX_DOWNLOADS {
                return Err(AppError::BadRequest(
                    "expiry or download limit out of range".into(),
                ));
            }
            let password = body.password.filter(|p| !p.is_empty());
            if password.as_deref().map(|p| p.len() > 256).unwrap_or(false) {
                return Err(AppError::BadRequest("password too long".into()));
            }

            let e2e = body.e2e.unwrap_or(false);
            let mut inst = enabled_instance()?;
            server::ensure_running(&inst).await.map_err(internal)?;
            if e2e {
                super::e2e::enable(&mut inst).await.map_err(internal)?;
            }
            let job = cli::start(
                ctx,
                &uid,
                cli::UploadRequest {
                    rel_path: fs_util::rel_display(&home, &abs),
                    abs_path: abs,
                    is_dir: meta.is_dir(),
                    expiry_days,
                    max_downloads,
                    password,
                    from: body.from.unwrap_or_default(),
                    message: body.message.unwrap_or_default(),
                    e2e,
                },
            )
            .map_err(internal)?;
            Ok(ok(json!({ "job": job })))
        }
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct JobQuery {
    id: Option<String>,
}

fn job(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            let uid = user_id(&req)?;
            let (q, _req): (JobQuery, _) = take_query(req).await?;
            let id = q.id.unwrap_or_default();
            cli::job_json(&uid, &id)
                .map(ok)
                .ok_or_else(|| AppError::NotFound("upload job not found".into()))
        }
    })
}

fn list(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let shares = store::list_for(ctx.db(), &uid)?;
            let inst = server::load_instance().filter(|i| i.enabled);
            let live = match &inst {
                Some(i) if server::is_running() => Some(i),
                _ => None,
            };
            let mut out = Vec::with_capacity(shares.len());
            for s in shares {
                let mut row = serde_json::to_value(&s).unwrap_or(Json::Null);
                if let Some(inst) = live {
                    match rest::file_info(inst, &s.id).await {
                        // Expired or used up: Gokapi has already dropped it.
                        Ok(None) => {
                            let _ = store::delete(ctx.db(), &uid, &s.id);
                            super::theme::delete_share_meta(&s.id);
                            continue;
                        }
                        Ok(Some(f)) => {
                            row["download_count"] = json!(f.download_count);
                            row["downloads_remaining"] = if f.unlimited_downloads {
                                Json::Null
                            } else {
                                json!(f.downloads_remaining)
                            };
                        }
                        Err(_) => {}
                    }
                }
                out.push(row);
            }
            Ok(ok(json!({ "shares": out, "live": live.is_some() })))
        }
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RevokeBody {
    id: Option<String>,
}

fn revoke(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let body: RevokeBody = json_body(req).await?;
            let id = body.id.unwrap_or_default();
            if !store::owns(ctx.db(), &uid, &id)? {
                return Err(AppError::NotFound("share not found".into()));
            }
            let inst = enabled_instance()?;
            server::ensure_running(&inst).await.map_err(internal)?;
            rest::delete(&inst, &id).await.map_err(internal)?;
            store::delete(ctx.db(), &uid, &id)?;
            super::theme::delete_share_meta(&id);
            Ok(ok(json!({ "revoked": id })))
        }
    })
}
