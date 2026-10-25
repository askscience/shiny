//! HTTP surface for the installer wizard.
//!
//! Every route is local-only and turns into one typed request to the root
//! helper. The plugin never receives a shell, a device handle or a password.

use std::sync::Arc;

use axum::extract::{FromRequestParts, Request};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;

use shiny_installer_protocol::{methods, ApplyRequest, InstallPlan};
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{
    bridged_route, is_remote_request, ok, user_id, RouteHandler, RouteSpec,
};
use shiny_plugin_sdk::services::PluginCtx;

pub fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec::get("/api/installer/health", "health"),
        RouteSpec::get("/api/installer/options", "options"),
        RouteSpec::get("/api/installer/probe", "probe"),
        RouteSpec::post("/api/installer/plan", "plan"),
        RouteSpec::post("/api/installer/apply", "apply"),
        RouteSpec::get("/api/installer/status", "status"),
        RouteSpec::post("/api/installer/cancel", "cancel"),
    ]
}

pub fn handle(_ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    Some(match tag {
        "health" => health_handler(),
        "options" => options_handler(),
        "probe" => probe_handler(),
        "plan" => plan_handler(),
        "apply" => apply_handler(),
        "status" => status_handler(),
        "cancel" => cancel_handler(),
        _ => return None,
    })
}

fn deny_remote(req: &Request) -> Result<(), AppError> {
    if is_remote_request(req) {
        Err(AppError::Unauthorized(
            "The installer is available only from the live machine".into(),
        ))
    } else {
        Ok(())
    }
}

async fn json_body<T: DeserializeOwned>(req: Request) -> Result<T, AppError> {
    let bytes = axum::body::to_bytes(req.into_body(), 256 * 1024)
        .await
        .map_err(|_| AppError::BadRequest("installer request is too large".into()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| AppError::BadRequest(format!("invalid installer JSON: {error}")))
}

async fn take_query<T: DeserializeOwned + Send + 'static>(req: Request) -> Result<T, AppError> {
    let (mut parts, _body) = req.into_parts();
    let query = axum::extract::Query::<T>::from_request_parts(&mut parts, &())
        .await
        .map_err(|error| AppError::BadRequest(format!("invalid query: {error}")))?;
    Ok(query.0)
}

fn health_handler() -> RouteHandler {
    bridged_route(|req: Request| async move {
        let _uid = user_id(&req)?;
        deny_remote(&req)?;
        let data = crate::client::call(methods::HEALTH, serde_json::Value::Null).await?;
        Ok(ok(data))
    })
}

fn probe_handler() -> RouteHandler {
    bridged_route(|req: Request| async move {
        let _uid = user_id(&req)?;
        deny_remote(&req)?;
        let data = crate::client::call(methods::PROBE, serde_json::Value::Null).await?;
        Ok(ok(data))
    })
}

fn options_handler() -> RouteHandler {
    bridged_route(|req: Request| async move {
        let _uid = user_id(&req)?;
        deny_remote(&req)?;
        let data = crate::client::call(methods::OPTIONS, serde_json::Value::Null).await?;
        Ok(ok(data))
    })
}

fn plan_handler() -> RouteHandler {
    bridged_route(|req: Request| async move {
        let _uid = user_id(&req)?;
        deny_remote(&req)?;
        let plan: InstallPlan = json_body(req).await?;
        let data = crate::client::call(methods::VALIDATE, json!({ "plan": plan })).await?;
        Ok(ok(data))
    })
}

fn apply_handler() -> RouteHandler {
    bridged_route(|req: Request| async move {
        let _uid = user_id(&req)?;
        deny_remote(&req)?;
        let apply: ApplyRequest = json_body(req).await?;
        let data = crate::client::call(
            methods::APPLY,
            json!({ "plan": apply.plan, "confirm_serial": apply.confirm_serial }),
        )
        .await?;
        Ok(ok(data))
    })
}

#[derive(Deserialize)]
struct JobQuery {
    job: String,
}

fn status_handler() -> RouteHandler {
    bridged_route(|req: Request| async move {
        let _uid = user_id(&req)?;
        deny_remote(&req)?;
        let query: JobQuery = take_query(req).await?;
        let data = crate::client::call(methods::STATUS, json!({ "job": query.job })).await?;
        Ok(ok(data))
    })
}

#[derive(Deserialize)]
struct JobRef {
    job: String,
}

fn cancel_handler() -> RouteHandler {
    bridged_route(|req: Request| async move {
        let _uid = user_id(&req)?;
        deny_remote(&req)?;
        let body: JobRef = json_body(req).await?;
        let data = crate::client::call(methods::CANCEL, json!({ "job": body.job })).await?;
        Ok(ok(data))
    })
}
