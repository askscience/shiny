//! Mail plugin tools: `mail_status`, `mail_list`, `mail_read`, `mail_search`,
//! `mail_sync`, `mail_send`, `mail_save_attachment`.
//!
//! List/read/search read from the LOCAL cache (`crate::cache`) so the AI never
//! opens a fresh IMAP connection for mail that's already been downloaded;
//! `mail_sync` (and the automatic first-use backfill) is the only thing that
//! talks to the provider. Sending stays live via SMTP.

use async_trait::async_trait;
use serde_json::{json, Value};
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::{attach, cache, mail};

pub struct MailStatus;
pub struct MailList;
pub struct MailRead;
pub struct MailSearch;
pub struct MailSync;
pub struct MailSend;
pub struct MailSaveAttachment;

/// A list of strings, or one string (comma- or semicolon-separated): models
/// often pass `"to":"a@x.com"` for a single recipient.
fn str_array(v: &Value) -> Vec<String> {
    if let Some(s) = v.as_str() {
        return s
            .split([',', ';'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
    }
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

#[async_trait]
impl Tool for MailStatus {
    fn name(&self) -> &str { "mail_status" }
    fn step_label(&self) -> &str { "Checking mail accounts…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `mail_status` — Summarise the user's mail setup: which accounts are configured, verified and connected. params: `{}`")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let accounts = data.get("accounts").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        let verified = data
            .get("accounts")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter(|x| x.get("verified").and_then(|b| b.as_bool()).unwrap_or(false)).count())
            .unwrap_or(0);
        format!("Mail: {verified}/{accounts} account(s) verified")
    }
    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let accounts = mail::list_accounts(ctx.db(), req.user_id)?;
        Ok(ActionOutcome::ok("mail_status", json!({
            "configured": accounts.iter().any(|a| a.verified),
            "accounts": accounts.iter().map(|a| a.to_json(false)).collect::<Vec<_>>(),
        })))
    }
}

#[async_trait]
impl Tool for MailList {
    fn name(&self) -> &str { "mail_list" }
    fn step_label(&self) -> &str { "Listing messages…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `mail_list` — List messages in a folder (default INBOX) from the local cache. params: `{ account?: string, folder?: string, page?: number }`. Returns up to 60 envelopes with subject, sender, date, seen.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("messages").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        let folder = data.get("folder").and_then(|v| v.as_str()).unwrap_or("INBOX");
        format!("Listed {n} message(s) in {folder}")
    }
    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let account = req.params.param_str("account");
        let folder = req.params.param_str("folder").unwrap_or_else(|| "INBOX".into());
        let page = req.params.param_u32("page").unwrap_or(0);
        let a = mail::resolve_account(ctx.db(), req.user_id, account.as_deref())?;

        // First use backfills the cache; after that it's a local read.
        if mail::needs_backfill(ctx.db(), req.user_id, &a.id, &folder)? {
            mail::sync_folder(ctx.db(), req.user_id, a.clone(), folder.clone()).await?;
        }

        let offset = (page as usize) * 60;
        let messages = cache::list(ctx.db(), req.user_id, &a.id, &folder, 60, offset)?;
        let total = cache::total(ctx.db(), req.user_id, &a.id, &folder)?;
        Ok(ActionOutcome::ok("mail_list", json!({ "folder": folder, "messages": messages, "total": total, "page": page })))
    }
}

#[async_trait]
impl Tool for MailRead {
    fn name(&self) -> &str { "mail_read" }
    fn step_label(&self) -> &str { "Reading message…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `mail_read` — Fetch one full message from the local cache. params: `{ account?: string, folder?: string, id: string }` where `id` comes from `mail_list`/`mail_search`. Returns subject, from, to, date, body.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let subject = data.get("subject").and_then(|v| v.as_str()).unwrap_or("(no subject)");
        format!("Read message: {subject}")
    }
    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let account = req.params.param_str("account");
        let folder = req.params.param_str("folder").unwrap_or_else(|| "INBOX".into());
        let id = req.params.require_str("id")?;
        let a = mail::resolve_account(ctx.db(), req.user_id, account.as_deref())?;
        let account_email = a.email.clone();

        // Prefer the cache; refetch (then re-cache) when the row is missing OR
        // is envelope-only. A cache hit without a body used to be returned as
        //-is, so the model (and the window) saw an empty message.
        let cached = cache::get(ctx.db(), req.user_id, &a.id, &folder, &id)?;
        let mut message = match cached {
            Some(m) if cache::has_body(&m) => m,
            stale => match mail::get_message(a.clone(), folder.clone(), id.clone()).await {
                Ok(m) => {
                    cache::upsert_message(ctx.db(), req.user_id, &a.id, &folder, &id, &m)?;
                    m
                }
                Err(e) => match stale {
                    Some(m) => m,
                    None => return Err(e),
                },
            },
        };

        // Include the source folder + account email so the frontend can open
        // the right message in the right folder, and the AI knows its origin.
        if let Some(obj) = message.as_object_mut() {
            obj.insert("folder".into(), json!(folder));
            obj.insert("account".into(), json!(account_email));
        }
        Ok(ActionOutcome::ok("mail_read", message))
    }
}

#[async_trait]
impl Tool for MailSearch {
    fn name(&self) -> &str { "mail_search" }
    fn step_label(&self) -> &str { "Searching mail…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `mail_search` — Search cached messages by subject, sender or body text (local, no IMAP). params: `{ account?: string, folder?: string, query: string }`. Omit `folder` to search all folders. Returns up to 60 matching envelopes.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("messages").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        format!("Found {n} matching message(s)")
    }
    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let account = req.params.param_str("account");
        let folder = req.params.param_str("folder");
        let query = req.params.require_str("query")?;
        let a = mail::resolve_account(ctx.db(), req.user_id, account.as_deref())?;

        // Backfill the default/requested folder on first use; then search local.
        let eff_folder = folder.clone().unwrap_or_else(|| "INBOX".into());
        if mail::needs_backfill(ctx.db(), req.user_id, &a.id, &eff_folder)? {
            mail::sync_folder(ctx.db(), req.user_id, a.clone(), eff_folder).await?;
        }

        let messages = cache::search(ctx.db(), req.user_id, Some(&a.id), folder.as_deref(), &query, 60)?;
        Ok(ActionOutcome::ok("mail_search", json!({ "query": query, "messages": messages })))
    }
}

#[async_trait]
impl Tool for MailSync {
    fn name(&self) -> &str { "mail_sync" }
    fn aliases(&self) -> &[&str] { &["refresh_mail", "sync_mail"] }
    fn step_label(&self) -> &str { "Syncing mail…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `mail_sync` — Download new mail into the local cache so later list/read/search are instant and offline. params: `{ account?: string, folder?: string = \"INBOX\" }`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("new").and_then(|v| v.as_u64()).unwrap_or(0);
        let total = data.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Mail synced: {n} new, {total} cached")
    }
    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let account = req.params.param_str("account");
        let folder = req.params.param_str("folder").unwrap_or_else(|| "INBOX".into());
        let a = mail::resolve_account(ctx.db(), req.user_id, account.as_deref())?;
        let summary = mail::sync_folder(ctx.db(), req.user_id, a, folder).await?;
        Ok(ActionOutcome::ok("mail_sync", summary))
    }
}

#[async_trait]
impl Tool for MailSend {
    fn name(&self) -> &str { "mail_send" }
    fn step_label(&self) -> &str { "Sending email…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `mail_send` — Compose and send an email. params: `{ account?: string, to: string[], cc?: string[], bcc?: string[], subject: string, body: string, attachments?: string[] }`. The sender is the account's email. `attachments` are files in the user's home, as home-relative paths (e.g. `Documents/report.pdf`, the paths the Files tools return); 18 MB total at most.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let to = data.get("to").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
        let attached = data.get("attachments").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        if data.get("sent").and_then(|v| v.as_bool()) == Some(false) {
            format!("Email to {to} was already sent (skipped duplicate)")
        } else if attached > 0 {
            format!("Sent email to {to} with {attached} attachment(s)")
        } else {
            format!("Sent email to {to}")
        }
    }
    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let account = req.params.param_str("account");
        let to = str_array(req.params.get("to").unwrap_or(&Value::Null));
        if to.is_empty() {
            return Err(AppError::BadRequest("mail_send requires at least one recipient in 'to'".into()));
        }
        let cc = str_array(req.params.get("cc").unwrap_or(&Value::Null));
        let bcc = str_array(req.params.get("bcc").unwrap_or(&Value::Null));
        let subject = req.params.param_str("subject").unwrap_or_default();
        let body = req.params.param_str("body").unwrap_or_default();
        // A single path is accepted as well as a list; objects may carry a
        // display `filename` next to the `path`.
        let refs: Vec<attach::AttachmentRef> = match req.params.get("attachments") {
            Some(Value::String(p)) => vec![attach::AttachmentRef { path: p.clone(), filename: None }],
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|item| match item {
                    Value::String(p) => Some(attach::AttachmentRef { path: p.clone(), filename: None }),
                    Value::Object(_) => serde_json::from_value(item.clone()).ok(),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let attachments = attach::load_refs(req.user_id, req.os_home, &refs, 0)?;
        let names: Vec<String> = attachments.iter().map(|a| a.filename.clone()).collect();
        let a = mail::resolve_account(ctx.db(), req.user_id, account.as_deref())?;
        let sent = mail::send(a, req.user_id, to.clone(), cc, bcc, subject, body, None, attachments).await?;
        Ok(ActionOutcome::ok("mail_send", json!({ "to": to, "sent": sent, "attachments": names })))
    }
}

#[async_trait]
impl Tool for MailSaveAttachment {
    fn name(&self) -> &str { "mail_save_attachment" }
    fn step_label(&self) -> &str { "Saving attachment…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `mail_save_attachment` — Save an attachment of a received message into the user's home. params: `{ account?: string, folder?: string, id: string, index?: number, dir?: string }`; `index` is the attachment's position in `mail_read`'s `attachments` (default 0), `dir` a home-relative folder (default `Downloads`). Returns the saved path — use it to open the file, or to attach it to a `mail_send`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let path = data.get("path").and_then(|v| v.as_str()).unwrap_or("");
        format!("Saved attachment to {path}")
    }
    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let account = req.params.param_str("account");
        let folder = req.params.param_str("folder").unwrap_or_else(|| "INBOX".into());
        let id = req.params.require_str("id")?;
        let index = req.params.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let dir = req.params.param_str("dir").filter(|d| !d.trim().is_empty()).unwrap_or_else(|| "Downloads".into());
        let a = mail::resolve_account(ctx.db(), req.user_id, account.as_deref())?;
        let att = attach::fetch_attachment(a, folder, id, index).await?;
        let path = attach::save_into_home(req.user_id, req.os_home, &dir, &att.filename, &att.bytes)?;
        Ok(ActionOutcome::ok(
            "mail_save_attachment",
            json!({ "path": path, "name": att.filename, "size": att.bytes.len() }),
        ))
    }
}
