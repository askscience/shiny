//! Mail plugin tools: `mail_status`, `mail_list`, `mail_read`, `mail_send`.
//! Every tool is registered through `bridged(...)` (§15) and does DB work via
//! `ctx.db()` plus network work via the `mail` module's blocking helpers.

use async_trait::async_trait;
use serde_json::{json, Value};
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::mail;

pub struct MailStatus;
pub struct MailList;
pub struct MailRead;
pub struct MailSend;

fn str_array(v: &Value) -> Vec<String> {
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
        Some("- `mail_list` — List messages in a folder (default INBOX). params: `{ account?: string, folder?: string, page?: number }`. Returns up to 60 envelopes with subject, sender, date, seen.")
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
        let messages = mail::list_envelopes(a, folder.clone(), page).await?;
        Ok(ActionOutcome::ok("mail_list", json!({ "folder": folder, "messages": messages })))
    }
}

#[async_trait]
impl Tool for MailRead {
    fn name(&self) -> &str { "mail_read" }
    fn step_label(&self) -> &str { "Reading message…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `mail_read` — Fetch one full message. params: `{ account?: string, folder?: string, id: string }` where `id` comes from `mail_list`. Returns subject, from, to, date, body.")
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
        let message = mail::get_message(a, folder, id).await?;
        Ok(ActionOutcome::ok("mail_read", message))
    }
}

#[async_trait]
impl Tool for MailSend {
    fn name(&self) -> &str { "mail_send" }
    fn step_label(&self) -> &str { "Sending email…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `mail_send` — Compose and send an email. params: `{ account?: string, to: string[], cc?: string[], bcc?: string[], subject: string, body: string }`. The sender is the account's email.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let to = data.get("to").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
        format!("Sent email to {to}")
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
        let a = mail::resolve_account(ctx.db(), req.user_id, account.as_deref())?;
        mail::send(a, to.clone(), cc, bcc, subject, body, None).await?;
        Ok(ActionOutcome::ok("mail_send", json!({ "to": to })))
    }
}
