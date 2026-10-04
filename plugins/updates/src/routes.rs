//! REST surface for the Updates window.
//!
//! Read routes are available to any authenticated user; write routes are
//! **local-only** (the same rule the Terminal uses) because they can run
//! privileged commands. Long operations return a job id that the window polls
//! via `GET /api/updates/jobs`.

use std::sync::Arc;

use axum::extract::{FromRequestParts, Request};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{
    bridged_route, is_remote_request, user_id_from_request, HttpMethod, RouteHandler, RouteSpec,
};
use shiny_plugin_sdk::services::PluginCtx;

use crate::distro;
use crate::exec;
use crate::jobs;
use crate::ops;
use crate::status;

pub fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec {
            method: HttpMethod::Get,
            path: "/api/updates/status".into(),
            auth: "auth".into(),
            handler_tag: "status".into(),
        },
        RouteSpec {
            method: HttpMethod::Get,
            path: "/api/updates/history".into(),
            auth: "auth".into(),
            handler_tag: "history".into(),
        },
        RouteSpec {
            method: HttpMethod::Get,
            path: "/api/updates/jobs".into(),
            auth: "auth".into(),
            handler_tag: "job".into(),
        },
        RouteSpec {
            method: HttpMethod::Post,
            path: "/api/updates/refresh".into(),
            auth: "auth".into(),
            handler_tag: "refresh".into(),
        },
        RouteSpec {
            method: HttpMethod::Post,
            path: "/api/updates/apply".into(),
            auth: "auth".into(),
            handler_tag: "apply".into(),
        },
        RouteSpec {
            method: HttpMethod::Post,
            path: "/api/updates/ollama/update".into(),
            auth: "auth".into(),
            handler_tag: "ollama".into(),
        },
    ]
}

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    Some(match tag {
        "status" => status_handler(ctx),
        "history" => history_handler(ctx),
        "job" => job_handler(),
        "refresh" => refresh_handler(ctx),
        "apply" => apply_handler(ctx),
        "ollama" => ollama_handler(ctx),
        _ => return None,
    })
}

/* ── helpers ─────────────────────────────────────────────────────────────── */

fn ok(data: serde_json::Value) -> Response {
    axum::Json(json!({ "success": true, "data": data })).into_response()
}

fn user_id(req: &Request) -> Result<String, AppError> {
    user_id_from_request(req).ok_or_else(|| AppError::Unauthorized("not authenticated".into()))
}

/// Privileged operations are refused for remote (Iroh / reverse-proxy) clients.
fn deny_remote(req: &Request) -> Result<(), AppError> {
    if is_remote_request(req) {
        Err(AppError::Unauthorized(
            "Updates can only be managed from the local machine".into(),
        ))
    } else {
        Ok(())
    }
}

fn needs_password(password: Option<&str>) -> bool {
    !exec::is_root()
        && password
            .map(|p| p.trim().is_empty())
            .unwrap_or(true)
}

async fn json_body<T: DeserializeOwned + Default>(req: Request) -> T {
    let bytes = match axum::body::to_bytes(req.into_body(), 256 * 1024).await {
        Ok(b) => b,
        Err(_) => return T::default(),
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

async fn take_query<T: DeserializeOwned + Default + Send + 'static>(
    req: Request,
) -> Result<T, AppError> {
    let (mut parts, _body) = req.into_parts();
    let query = axum::extract::Query::<T>::from_request_parts(&mut parts, &())
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid query: {e}")))?;
    Ok(query.0)
}

/* ── handlers ────────────────────────────────────────────────────────────── */

#[derive(Deserialize, Default)]
#[serde(default)]
struct StatusQuery {
    force: Option<bool>,
}

fn status_handler(_ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| async move {
        let _uid = user_id(&req)?;
        let q: StatusQuery = take_query(req).await.unwrap_or_default();
        // `force` (an explicit user action) waits for a fresh list; the plain
        // poll used by the top-bar chip returns the cache immediately and
        // refreshes in the background, so it never occupies the plugin runtime.
        let snapshot = if q.force.unwrap_or(false) {
            status::collect(true)
        } else {
            status::snapshot()
        };
        Ok(ok(serde_json::to_value(&snapshot)?))
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct HistoryQuery {
    limit: Option<usize>,
}

fn history_handler(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let q: HistoryQuery = take_query(req).await.unwrap_or_default();
            let rows = crate::history::list(&ctx, &uid, q.limit.unwrap_or(50));
            Ok(ok(json!({ "history": rows })))
        }
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct JobQuery {
    job: Option<String>,
    from: Option<usize>,
}

fn job_handler() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        let _uid = user_id(&req)?;
        let q: JobQuery = take_query(req).await?;
        let id = q
            .job
            .ok_or_else(|| AppError::BadRequest("job required".into()))?;
        let job = jobs::get(&id).ok_or_else(|| AppError::NotFound("job not found".into()))?;
        let snap = job.snapshot(q.from.unwrap_or(0));
        Ok(ok(json!({
            "lines": snap.lines,
            "done": snap.done,
            "success": snap.success,
            "error": snap.error,
            "total": job.total(),
        })))
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PasswordBody {
    password: Option<String>,
}

fn refresh_handler(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            deny_remote(&req)?;
            let body: PasswordBody = json_body(req).await;

            let manager = distro::detect();
            if !manager.supported() {
                return Err(AppError::BadRequest(
                    "Unsupported distribution: no known package manager was detected".into(),
                ));
            }
            if needs_password(body.password.as_deref()) {
                return Err(AppError::BadRequest(
                    "An administrator (sudo) password is required".into(),
                ));
            }

            let manager_id = manager.id().to_string();
            let detail = format!("refresh {manager_id}");
            let password = body.password;
            let job = jobs::start("refresh", move |job| {
                let ok = ops::refresh_job(&job, manager.as_ref(), password.as_deref());
                job.finish(ok, None);
            });
            crate::history::record_when_done(
                (*ctx).clone(),
                uid,
                "refresh",
                manager_id,
                detail,
                job.clone(),
            );
            Ok(ok(json!({ "job": job.id })))
        }
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ApplyBody {
    password: Option<String>,
    packages: Option<Vec<String>>,
    all: Option<bool>,
    include_ollama: Option<bool>,
}

fn apply_handler(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            deny_remote(&req)?;
            let body: ApplyBody = json_body(req).await;

            let manager = distro::detect();
            let packages = body.packages.unwrap_or_default();
            let include_ollama = body.include_ollama.unwrap_or(false);
            let all = body
                .all
                .unwrap_or(packages.is_empty() && !include_ollama);

            if !all && packages.is_empty() && !include_ollama {
                return Err(AppError::BadRequest(
                    "Nothing to install: pass packages, all=true, or include_ollama=true".into(),
                ));
            }
            if !manager.supported() && !include_ollama {
                return Err(AppError::BadRequest(
                    "Unsupported distribution: no known package manager was detected".into(),
                ));
            }
            if needs_password(body.password.as_deref()) {
                return Err(AppError::BadRequest(
                    "An administrator (sudo) password is required".into(),
                ));
            }

            let manager_id = manager.id().to_string();
            let detail = if all {
                "all".to_string()
            } else if packages.is_empty() {
                "ollama".to_string()
            } else {
                packages.join(", ")
            };
            let password = body.password;
            let pkgs = packages.clone();
            let job = jobs::start("apply", move |job| {
                let mut ok = true;
                if manager.supported() && (all || !pkgs.is_empty()) {
                    ok = ops::system_job(&job, manager.as_ref(), password.as_deref(), all, &pkgs);
                }
                if include_ollama {
                    ok = ops::ollama_job(&job, password.as_deref()) && ok;
                }
                job.finish(ok, None);
            });
            crate::history::record_when_done(
                (*ctx).clone(),
                uid,
                "apply",
                manager_id,
                detail,
                job.clone(),
            );
            Ok(ok(json!({ "job": job.id })))
        }
    })
}

fn ollama_handler(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            deny_remote(&req)?;
            let body: PasswordBody = json_body(req).await;
            if needs_password(body.password.as_deref()) {
                return Err(AppError::BadRequest(
                    "An administrator (sudo) password is required".into(),
                ));
            }
            let password = body.password;
            let job = jobs::start("ollama", move |job| {
                let ok = ops::ollama_job(&job, password.as_deref());
                job.finish(ok, None);
            });
            crate::history::record_when_done(
                (*ctx).clone(),
                uid,
                "ollama",
                "ollama".into(),
                "official installer".into(),
                job.clone(),
            );
            Ok(ok(json!({ "job": job.id })))
        }
    })
}
