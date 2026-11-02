//! Mail plugin REST routes — served through the plugin's `RouteSpec`s.
//!
//! Account rows live in the plugin-owned `mail_accounts` table (per user);
//! mail I/O goes through `io-email` on a blocking thread (see `mail.rs`).

use std::sync::Arc;

use axum::extract::{FromRequest, FromRequestParts};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, os_home_from_request, RouteHandler, path_params_from_request, ok, user_id};
use shiny_plugin_sdk::services::PluginCtx;

use crate::attach;
use crate::cache;
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
        "mail_search" => mail_search(ctx),
        "mail_sync" => mail_sync(ctx),
        "mail_message" => mail_message(ctx),
        "mail_send" => mail_send(ctx),
        "mail_attachment" => mail_attachment(ctx),
        "mail_attachment_save" => mail_attachment_save(ctx),
        "mail_flag" => mail_flag(ctx),
        "mail_delete" => mail_delete(ctx),
        _ => return None,
    })
}

/* ── helpers ────────────────────────────────────────────────── */
async fn take_query<T: DeserializeOwned + Send + 'static>(
    req: axum::extract::Request,
) -> Result<(T, axum::extract::Request), AppError> {
    let (mut parts, body) = req.into_parts();
    let query = axum::extract::Query::<T>::from_request_parts(&mut parts, &())
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid query: {e}")))?;
    Ok((query.0, axum::extract::Request::from_parts(parts, body)))
}

fn take_path(req: &axum::extract::Request) -> Result<String, AppError> {
    let mut params = path_params_from_request(req)
        .ok_or_else(|| AppError::BadRequest("no path parameter found".into()))?;
    if params.len() != 1 {
        return Err(AppError::BadRequest("expected exactly one path parameter".into()));
    }
    Ok(params.remove(0).1)
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
            Ok(ok(a.to_json(false)))
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

            Ok(ok(json!({ "ok": a.verified, "error": a.last_error, "account": a.to_json(false) })))
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
            let id = take_path(&req)?;
            let axum::Json(body) = axum::Json::<UpdateAccount>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid body: {e}")))?;

            let mut a = mail::load_account(ctx.db(), &uid, &id)?;
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
            Ok(ok(a.to_json(false)))
        }
    })
}

fn accounts_delete(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            mail::delete_account(ctx.db(), &uid, &id)?;
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

            // First use backfills the cache; later list calls are local.
            if mail::needs_backfill(ctx.db(), &uid, &a.id, &folder)? {
                mail::sync_folder(ctx.db(), &uid, a.clone(), folder.clone()).await?;
            }
            let offset = (page as usize) * 60;
            let messages = cache::list(ctx.db(), &uid, &a.id, &folder, 60, offset)?;
            let total = cache::total(ctx.db(), &uid, &a.id, &folder)?;
            Ok(ok(json!({ "messages": messages, "total": total, "page": page })))
        }
    })
}

#[derive(Deserialize)]
struct SearchQuery {
    account: Option<String>,
    folder: Option<String>,
    query: Option<String>,
}

fn mail_search(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (q, _req) = take_query::<SearchQuery>(req).await?;
            let query = q
                .query
                .ok_or_else(|| AppError::BadRequest("search query required".into()))?;
            let a = mail::resolve_account(ctx.db(), &uid, q.account.as_deref())?;

            let eff_folder = q.folder.clone().unwrap_or_else(|| "INBOX".into());
            if mail::needs_backfill(ctx.db(), &uid, &a.id, &eff_folder)? {
                mail::sync_folder(ctx.db(), &uid, a.clone(), eff_folder).await?;
            }

            let messages = cache::search(ctx.db(), &uid, Some(&a.id), q.folder.as_deref(), &query, 60)?;
            Ok(ok(json!({ "messages": messages, "query": query })))
        }
    })
}

#[derive(Deserialize)]
struct SyncBody {
    account: Option<String>,
    folder: Option<String>,
}

fn mail_sync(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let axum::Json(body) = axum::Json::<SyncBody>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid body: {e}")))?;
            let a = mail::resolve_account(ctx.db(), &uid, body.account.as_deref())?;
            let folder = body.folder.unwrap_or_else(|| "INBOX".into());
            let summary = mail::sync_folder(ctx.db(), &uid, a, folder).await?;
            Ok(ok(summary))
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

            // A cache hit is only usable when it carries a body: rows cached
            // before bodies were downloaded (or whose body fetch failed) are
            // envelope-only and used to render as "(empty message)". Refetch
            // those once, then serve the cached headers if the provider is down.
            let cached = cache::get(ctx.db(), &uid, &a.id, &folder, &id)?;
            let message = match cached {
                Some(m) if cache::has_body(&m) => m,
                stale => match mail::get_message(a.clone(), folder.clone(), id.clone()).await {
                    Ok(m) => {
                        cache::upsert_message(ctx.db(), &uid, &a.id, &folder, &id, &m)?;
                        m
                    }
                    Err(e) => match stale {
                        Some(m) => {
                            tracing::warn!("mail: live fetch of message {} failed: {e}", id);
                            m
                        }
                        None => return Err(e),
                    },
                },
            };
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
    /// Files in the user's home to attach (home-relative paths).
    #[serde(default)]
    attachments: Vec<attach::AttachmentRef>,
}

/// Send a message. Two body shapes:
///
/// - `application/json` — `SendBody`; attachments as home paths.
/// - `multipart/form-data` — a `message` field holding the same JSON, plus one
///   `file` field per uploaded attachment (the window's compose).
fn mail_send(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let os_home = os_home_from_request(&req);
            let multipart_boundary = req
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .filter(|ct| ct.starts_with("multipart/"))
                .and_then(|ct| multer::parse_boundary(ct).ok());
            let (body, mut uploaded) = match multipart_boundary {
                Some(boundary) => read_multipart_send(req, boundary).await?,
                None => {
                    let axum::Json(body) = axum::Json::<SendBody>::from_request(req, &())
                        .await
                        .map_err(|e| AppError::BadRequest(format!("invalid body: {e}")))?;
                    (body, Vec::new())
                }
            };
            if body.to.is_empty() {
                return Err(AppError::BadRequest("at least one recipient required".into()));
            }
            let already: usize = uploaded.iter().map(|a| a.bytes.len()).sum();
            let mut attachments =
                attach::load_refs(&uid, os_home.as_deref(), &body.attachments, already)?;
            attachments.append(&mut uploaded);
            let count = attachments.len();
            let a = mail::resolve_account(ctx.db(), &uid, body.account_id.as_deref())?;
            let sent = mail::send(
                a,
                &uid,
                body.to,
                body.cc.unwrap_or_default(),
                body.bcc.unwrap_or_default(),
                body.subject,
                body.body,
                body.html,
                attachments,
            )
            .await?;
            Ok(ok(json!({ "sent": sent, "attachments": count })))
        }
    })
}

/// Parse the compose's multipart body: the `message` JSON and every `file`.
/// The size ceiling is enforced on the streamed chunks (see the SDK's
/// `upload` module for why axum's own extractor is not used).
async fn read_multipart_send(
    req: axum::extract::Request,
    boundary: String,
) -> Result<(SendBody, Vec<attach::OutAttachment>), AppError> {
    let mut multipart = multer::Multipart::new(req.into_body().into_data_stream(), boundary);
    let mut body: Option<SendBody> = None;
    let mut files = Vec::new();
    let mut total = 0usize;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("upload error: {e}")))?
    {
        match field.name() {
            Some("message") => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("upload error: {e}")))?;
                body = Some(
                    serde_json::from_str(&text)
                        .map_err(|e| AppError::BadRequest(format!("invalid message: {e}")))?,
                );
            }
            Some("file") => {
                let filename = attach::safe_filename(field.file_name().unwrap_or("attachment"));
                let content_type = field
                    .content_type()
                    .map(|m| m.essence_str().to_string())
                    .filter(|m| m != "application/octet-stream")
                    .unwrap_or_else(|| attach::guess_content_type(&filename).to_string());
                let mut bytes = Vec::new();
                while let Some(chunk) = field
                    .chunk()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("upload error: {e}")))?
                {
                    total += chunk.len();
                    if total > attach::MAX_ATTACHMENTS_BYTES {
                        return Err(attach::too_large());
                    }
                    bytes.extend_from_slice(&chunk);
                }
                files.push(attach::OutAttachment { filename, content_type, bytes });
            }
            _ => {}
        }
    }
    let body = body.ok_or_else(|| AppError::BadRequest("missing 'message' field".into()))?;
    Ok((body, files))
}

#[derive(Deserialize)]
struct AttachmentQuery {
    account: Option<String>,
    folder: Option<String>,
    id: Option<String>,
    index: Option<usize>,
    /// `1` = `Content-Disposition: attachment` (download) instead of inline.
    download: Option<String>,
}

/// The bytes of one attachment of a received message.
fn mail_attachment(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (q, _req) = take_query::<AttachmentQuery>(req).await?;
            let id = q.id.ok_or_else(|| AppError::BadRequest("message id required".into()))?;
            let index = q.index.unwrap_or(0);
            let a = mail::resolve_account(ctx.db(), &uid, q.account.as_deref())?;
            let folder = q.folder.unwrap_or_else(|| "INBOX".into());
            let att = attach::fetch_attachment(a, folder, id, index).await?;
            let disposition = if q.download.as_deref() == Some("1") { "attachment" } else { "inline" };
            // RFC 6266: an ASCII fallback plus the UTF-8 name.
            let ascii: String = att
                .filename
                .chars()
                .map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' { c } else { '_' })
                .collect();
            let encoded = url::form_urlencoded::byte_serialize(att.filename.as_bytes())
                .collect::<String>()
                .replace('+', "%20");
            let response = axum::http::Response::builder()
                .header(axum::http::header::CONTENT_TYPE, att.content_type)
                .header(
                    axum::http::header::CONTENT_DISPOSITION,
                    format!("{disposition}; filename=\"{ascii}\"; filename*=UTF-8''{encoded}"),
                )
                .header("X-Content-Type-Options", "nosniff")
                // Mail content is untrusted: never let an HTML attachment run
                // with the app's origin.
                .header("Content-Security-Policy", "sandbox")
                .header(axum::http::header::CACHE_CONTROL, "no-store")
                .body(axum::body::Body::from(att.bytes))
                .map_err(|e| AppError::Internal(format!("response error: {e}")))?;
            Ok(response)
        }
    })
}

#[derive(Deserialize)]
struct SaveAttachmentBody {
    account_id: Option<String>,
    folder: Option<String>,
    id: String,
    index: Option<usize>,
    /// Home-relative folder; default `Downloads`.
    dir: Option<String>,
}

/// Save one attachment of a received message into the user's home.
fn mail_attachment_save(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let os_home = os_home_from_request(&req);
            let axum::Json(body) = axum::Json::<SaveAttachmentBody>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid body: {e}")))?;
            let a = mail::resolve_account(ctx.db(), &uid, body.account_id.as_deref())?;
            let folder = body.folder.unwrap_or_else(|| "INBOX".into());
            let att = attach::fetch_attachment(a, folder, body.id, body.index.unwrap_or(0)).await?;
            let dir = body.dir.unwrap_or_else(|| "Downloads".into());
            let path = attach::save_into_home(&uid, os_home.as_deref(), &dir, &att.filename, &att.bytes)?;
            Ok(ok(json!({ "path": path, "name": att.filename, "dir": dir, "size": att.bytes.len() })))
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
            mail::set_seen(ctx.db(), &uid, a, folder, body.ids, body.seen).await?;
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
