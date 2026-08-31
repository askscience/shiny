//! Mail plugin REST routes — served through the plugin's `RouteSpec`s.
//!
//! Account rows live in the plugin-owned `mail_accounts` table (per user);
//! mail I/O goes through `io-email` on a blocking thread (see `mail.rs`).

use std::sync::Arc;

use axum::extract::{FromRequest, FromRequestParts};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value as Json};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, RouteHandler, UserId};
use shiny_plugin_sdk::services::PluginCtx;

use crate::mail::{self, Account};

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    Some(match tag {
        "mail_status" => mail_status(ctx),
        "mail_accounts_list" => accounts_list(ctx),
        "mail_accounts_create" => accounts_create(ctx),
        "mail_accounts_test" => accounts_test(ctx),
        "mail_accounts_update" => accounts_update(ctx),
        "mail_accounts_delete" => accounts_delete(ctx),
        "mail_folders" => folders(ctx),
        "mail_list" => mail_list(ctx),
        "mail_message" => mail_message(ctx),
        "mail_send" => mail_send(ctx),
        "mail_flag" => mail_flag(ctx),
        "mail_delete" => mail_delete(ctx),
        _ => return None,
    })
}

/* ── helpers ────────────────────────────────────────────────── */

fn user_id(req: &axum::extract::Request) -> Result<String, AppError> {
    req.extensions()
        .get::<UserId>()
        .map(|u| u.0.clone())
        .ok_or_else(|| AppError::Unauthorized("not authenticated".into()))
}

fn ok(data: Json) -> Response {
    axum::Json(json!({ "success": true, "data": data })).into_response()
}

async fn take_query<T: DeserializeOwned + Send + 'static>(
    req: axum::extract::Request,
) -> Result<(T, axum::extract::Request), AppError> {
    let (mut parts, body) = req.into_parts();
    let query = axum::extract::Query::<T>::from_request_parts(&mut parts, &())
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid query: {e}")))?;
    Ok((query.0, axum::extract::Request::from_parts(parts, body)))
}

async fn take_path<T: DeserializeOwned + Send + 'static>(
    req: axum::extract::Request,
) -> Result<(T, axum::extract::Request), AppError> {
    let (mut parts, body) = req.into_parts();
    let path = axum::extract::Path::<T>::from_request_parts(&mut parts, &())
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid path: {e}")))?;
    Ok((path.0, axum::extract::Request::from_parts(parts, body)))
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

/* ── status ─────────────────────────────────────────────────── */

fn mail_status(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let accounts = mail::list_accounts(ctx.db(), &uid)?;
            Ok(ok(json!({
                "configured": accounts.iter().any(|a| a.verified),
                "accounts": accounts.iter().map(|a| a.to_json(false)).collect::<Vec<_>>(),
                "presets": mail::presets_json(),
            })))
        }
    })
}

/* ── accounts ───────────────────────────────────────────────── */

fn accounts_list(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let accounts = mail::list_accounts(ctx.db(), &uid)?;
            Ok(ok(json!(accounts.iter().map(|a| a.to_json(false)).collect::<Vec<_>>())))
        }
    })
}

#[derive(Deserialize)]
struct CreateAccount {
    label: Option<String>,
    email: String,
    provider: Option<String>,
    imap_host: Option<String>,
    imap_port: Option<u16>,
    imap_security: Option<String>,
    smtp_host: Option<String>,
    smtp_port: Option<u16>,
    smtp_security: Option<String>,
    username: Option<String>,
    password: String,
    test: Option<bool>,
}

fn accounts_create(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let axum::Json(body) = axum::Json::<CreateAccount>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid body: {e}")))?;

            let mut a = Account {
                id: uuid::Uuid::new_v4().to_string(),
                label: body.label.clone().unwrap_or_else(|| body.email.clone()),
                email: body.email.clone(),
                provider: body.provider.clone().unwrap_or_else(|| "custom".into()),
                imap_host: body.imap_host.clone().unwrap_or_default(),
                imap_port: body.imap_port.unwrap_or(993),
                imap_security: body.imap_security.clone().unwrap_or_else(|| "ssl".into()),
                smtp_host: body.smtp_host.clone().unwrap_or_default(),
                smtp_port: body.smtp_port.unwrap_or(465),
                smtp_security: body.smtp_security.clone().unwrap_or_else(|| "ssl".into()),
                username: body.username.clone().unwrap_or_else(|| body.email.clone()),
                password: body.password.clone(),
                verified: false,
                verified_at: None,
                last_error: None,
            };
            mail::apply_preset(&mut a);

            if body.test.unwrap_or(false) {
                match mail::test_connection(a.clone()).await {
                    Ok(()) => {
                        a.verified = true;
                        a.verified_at = Some(now_iso());
                        a.last_error = None;
                    }
                    Err(e) => {
                        a.verified = false;
                        a.last_error = Some(e.to_string());
                    }
                }
            }

            mail::save_account(ctx.db(), &uid, &a)?;
            Ok(ok(a.to_json(true)))
        }
    })
}

#[derive(Deserialize)]
struct TestAccount {
    id: Option<String>,
    email: Option<String>,
    provider: Option<String>,
    imap_host: Option<String>,
    imap_port: Option<u16>,
    imap_security: Option<String>,
    smtp_host: Option<String>,
    smtp_port: Option<u16>,
    smtp_security: Option<String>,
    username: Option<String>,
    password: Option<String>,
}

fn accounts_test(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let axum::Json(body) = axum::Json::<TestAccount>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid body: {e}")))?;

            let mut a = if let Some(id) = &body.id {
                mail::load_account(ctx.db(), &uid, id)?
            } else {
                Account {
                    id: uuid::Uuid::new_v4().to_string(),
                    label: body.email.clone().unwrap_or_default(),
                    email: body.email.clone().unwrap_or_default(),
                    provider: body.provider.clone().unwrap_or_else(|| "custom".into()),
                    imap_host: body.imap_host.clone().unwrap_or_default(),
                    imap_port: body.imap_port.unwrap_or(993),
                    imap_security: body.imap_security.clone().unwrap_or_else(|| "ssl".into()),
                    smtp_host: body.smtp_host.clone().unwrap_or_default(),
                    smtp_port: body.smtp_port.unwrap_or(465),
                    smtp_security: body.smtp_security.clone().unwrap_or_else(|| "ssl".into()),
                    username: body.username.clone().unwrap_or_default(),
                    password: body.password.clone().unwrap_or_default(),
                    verified: false,
                    verified_at: None,
                    last_error: None,
                }
            };
            if let Some(pw) = &body.password {
                a.password = pw.clone();
            }
            mail::apply_preset(&mut a);

            match mail::test_connection(a.clone()).await {
                Ok(()) => {
                    a.verified = true;
                    a.verified_at = Some(now_iso());
                    a.last_error = None;
                }
                Err(e) => {
                    a.verified = false;
                    a.last_error = Some(e.to_string());
                }
            }

            if body.id.is_some() {
                mail::save_account(ctx.db(), &uid, &a)?;
            }

            Ok(ok(json!({ "ok": a.verified, "error": a.last_error, "account": a.to_json(true) })))
        }
    })
}

#[derive(Deserialize)]
struct UpdateAccount {
    label: Option<String>,
    password: Option<String>,
    username: Option<String>,
    email: Option<String>,
    imap_host: Option<String>,
    imap_port: Option<u16>,
    imap_security: Option<String>,
    smtp_host: Option<String>,
    smtp_port: Option<u16>,
    smtp_security: Option<String>,
}

fn accounts_update(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (path, req) = take_path::<Json>(req).await?;
            let id = path.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let axum::Json(body) = axum::Json::<UpdateAccount>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid body: {e}")))?;

            let mut a = mail::load_account(ctx.db(), &uid, id)?;
            let mut changed_creds = false;
            if let Some(v) = body.label {
                a.label = v;
            }
            if let Some(v) = body.email {
                a.email = v;
                changed_creds = true;
            }
            if let Some(v) = body.username {
                a.username = v;
                changed_creds = true;
            }
            if let Some(v) = body.password {
                a.password = v;
                changed_creds = true;
            }
            if let Some(v) = body.imap_host {
                a.imap_host = v;
                changed_creds = true;
            }
            if let Some(v) = body.imap_port {
                a.imap_port = v;
                changed_creds = true;
            }
            if let Some(v) = body.imap_security {
                a.imap_security = v;
                changed_creds = true;
            }
            if let Some(v) = body.smtp_host {
                a.smtp_host = v;
                changed_creds = true;
            }
            if let Some(v) = body.smtp_port {
                a.smtp_port = v;
                changed_creds = true;
            }
            if let Some(v) = body.smtp_security {
                a.smtp_security = v;
                changed_creds = true;
            }
            if changed_creds {
                a.verified = false;
                a.last_error = None;
            }
            mail::save_account(ctx.db(), &uid, &a)?;
            Ok(ok(a.to_json(true)))
        }
    })
}

fn accounts_delete(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (path, _req) = take_path::<Json>(req).await?;
            let id = path.get("id").and_then(|v| v.as_str()).unwrap_or("");
            mail::delete_account(ctx.db(), &uid, id)?;
            Ok(ok(json!({ "deleted": true })))
        }
    })
}

/* ── folders / list / message / send / flag ─────────────────── */

#[derive(Deserialize)]
struct AccountQuery {
    account: Option<String>,
}

#[derive(Deserialize)]
struct ListQuery {
    account: Option<String>,
    folder: Option<String>,
    page: Option<u32>,
}

#[derive(Deserialize)]
struct MessageQuery {
    account: Option<String>,
    folder: Option<String>,
    id: Option<String>,
}

fn folders(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (q, _req) = take_query::<AccountQuery>(req).await?;
            let a = mail::resolve_account(ctx.db(), &uid, q.account.as_deref())?;
            let folders = mail::list_folders(a).await?;
            Ok(ok(json!({ "folders": folders })))
        }
    })
}

fn mail_list(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (q, _req) = take_query::<ListQuery>(req).await?;
            let a = mail::resolve_account(ctx.db(), &uid, q.account.as_deref())?;
            let folder = q.folder.unwrap_or_else(|| "INBOX".into());
            let page = q.page.unwrap_or(0);
            let messages = mail::list_envelopes(a, folder, page).await?;
            Ok(ok(json!({ "messages": messages })))
        }
    })
}

fn mail_message(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (q, _req) = take_query::<MessageQuery>(req).await?;
            let id = q
                .id
                .ok_or_else(|| AppError::BadRequest("message id required".into()))?;
            let a = mail::resolve_account(ctx.db(), &uid, q.account.as_deref())?;
            let folder = q.folder.unwrap_or_else(|| "INBOX".into());
            let message = mail::get_message(a, folder, id).await?;
            Ok(ok(message))
        }
    })
}

#[derive(Deserialize)]
struct SendBody {
    account_id: Option<String>,
    to: Vec<String>,
    cc: Option<Vec<String>>,
    bcc: Option<Vec<String>>,
    subject: String,
    body: String,
    html: Option<String>,
}

fn mail_send(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let axum::Json(body) = axum::Json::<SendBody>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid body: {e}")))?;
            if body.to.is_empty() {
                return Err(AppError::BadRequest("at least one recipient required".into()));
            }
            let a = mail::resolve_account(ctx.db(), &uid, body.account_id.as_deref())?;
            let sent = mail::send(
                a,
                body.to,
                body.cc.unwrap_or_default(),
                body.bcc.unwrap_or_default(),
                body.subject,
                body.body,
                body.html,
            )
            .await?;
            Ok(ok(json!({ "sent": sent })))
        }
    })
}

#[derive(Deserialize)]
struct FlagBody {
    account_id: Option<String>,
    folder: Option<String>,
    ids: Vec<String>,
    seen: bool,
}

fn mail_flag(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let axum::Json(body) = axum::Json::<FlagBody>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid body: {e}")))?;
            let a = mail::resolve_account(ctx.db(), &uid, body.account_id.as_deref())?;
            let folder = body.folder.unwrap_or_else(|| "INBOX".into());
            mail::set_seen(a, folder, body.ids, body.seen).await?;
            Ok(ok(json!({ "flagged": true })))
        }
    })
}

#[derive(Deserialize)]
struct DeleteBody {
    account_id: Option<String>,
    folder: Option<String>,
    id: String,
}

fn mail_delete(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let axum::Json(body) = axum::Json::<DeleteBody>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid body: {e}")))?;
            let a = mail::resolve_account(ctx.db(), &uid, body.account_id.as_deref())?;
            let folder = body.folder.unwrap_or_else(|| "INBOX".into());
            mail::delete_message(a, folder, body.id).await?;
            Ok(ok(json!({ "deleted": true })))
        }
    })
}
