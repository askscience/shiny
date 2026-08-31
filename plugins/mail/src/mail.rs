//! io-email backed mail operations: provider presets, IMAP/SMTP connections,
//! folder/envelope listing, message fetch + parse, compose + send.

use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use io_email::client::{EmailClientStd, EmailClientStdError};
use io_email::envelope::types::Envelope;
use io_email::flag::types::{Flag, FlagOp, IanaFlag};
use io_email::mailbox::types::Mailbox;
use io_smtp::rfc5321::types::domain::Domain;
use io_smtp::rfc5321::types::ehlo_domain::EhloDomain;
use mail_parser::{MessageParser, MimeHeaders};
use pimalaya_stream::sasl::SaslLogin;
use pimalaya_stream::tls::Tls;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use url::Url;

use shiny_plugin_sdk::db::{Db, Value};
use shiny_plugin_sdk::errors::AppError;

/// A mail account as stored per user (v1: credentials in plaintext, single-tenant).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub label: String,
    pub email: String,
    pub provider: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: String, // "ssl" | "starttls" | "none"
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: String,
    pub username: String,
    pub password: String,
    pub verified: bool,
    pub verified_at: Option<String>,
    pub last_error: Option<String>,
}

impl Account {
    /// JSON for the window. The password is only included when explicitly
    /// asked (editing an account); never on list responses.
    pub fn to_json(&self, with_secret: bool) -> Json {
        let mut a = json!({
            "id": self.id,
            "label": self.label,
            "email": self.email,
            "provider": self.provider,
            "imap_host": self.imap_host,
            "imap_port": self.imap_port,
            "imap_security": self.imap_security,
            "smtp_host": self.smtp_host,
            "smtp_port": self.smtp_port,
            "smtp_security": self.smtp_security,
            "username": self.username,
            "verified": self.verified,
            "verified_at": self.verified_at,
            "last_error": self.last_error,
        });
        if with_secret {
            a["password"] = json!(self.password);
        }
        a
    }
}

/// Built-in provider presets so users never have to look up server settings.
pub struct Preset {
    pub provider: &'static str,
    pub label: &'static str,
    pub imap_host: &'static str,
    pub imap_port: u16,
    pub imap_security: &'static str,
    pub smtp_host: &'static str,
    pub smtp_port: u16,
    pub smtp_security: &'static str,
}

static PRESETS: &[Preset] = &[
    Preset { provider: "gmail", label: "Gmail", imap_host: "imap.gmail.com", imap_port: 993, imap_security: "ssl", smtp_host: "smtp.gmail.com", smtp_port: 465, smtp_security: "ssl" },
    Preset { provider: "outlook", label: "Outlook / Microsoft 365", imap_host: "outlook.office365.com", imap_port: 993, imap_security: "ssl", smtp_host: "smtp-mail.outlook.com", smtp_port: 587, smtp_security: "starttls" },
    Preset { provider: "yahoo", label: "Yahoo Mail", imap_host: "imap.mail.yahoo.com", imap_port: 993, imap_security: "ssl", smtp_host: "smtp.mail.yahoo.com", smtp_port: 465, smtp_security: "ssl" },
    Preset { provider: "icloud", label: "iCloud Mail", imap_host: "imap.mail.me.com", imap_port: 993, imap_security: "ssl", smtp_host: "smtp.mail.me.com", smtp_port: 587, smtp_security: "starttls" },
    Preset { provider: "zoho", label: "Zoho Mail", imap_host: "imap.zoho.com", imap_port: 993, imap_security: "ssl", smtp_host: "smtp.zoho.com", smtp_port: 465, smtp_security: "ssl" },
    Preset { provider: "fastmail", label: "Fastmail", imap_host: "imap.fastmail.com", imap_port: 993, imap_security: "ssl", smtp_host: "smtp.fastmail.com", smtp_port: 465, smtp_security: "ssl" },
    Preset { provider: "proton", label: "Proton Mail (via Bridge)", imap_host: "127.0.0.1", imap_port: 1143, imap_security: "none", smtp_host: "127.0.0.1", smtp_port: 1025, smtp_security: "none" },
];

pub fn presets() -> &'static [Preset] {
    PRESETS
}

pub fn preset_for(provider: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.provider == provider)
}

pub fn presets_json() -> Json {
    json!(PRESETS.iter().map(|p| json!({
        "provider": p.provider, "label": p.label,
        "imap_host": p.imap_host, "imap_port": p.imap_port, "imap_security": p.imap_security,
        "smtp_host": p.smtp_host, "smtp_port": p.smtp_port, "smtp_security": p.smtp_security,
    })).collect::<Vec<_>>())
}

// ── connection helpers ────────────────────────────────────────

fn imap_url(a: &Account) -> Result<Url, AppError> {
    let scheme = if a.imap_security == "ssl" { "imaps" } else { "imap" };
    Url::parse(&format!("{scheme}://{}:{}", a.imap_host, a.imap_port))
        .map_err(|e| AppError::BadRequest(format!("invalid IMAP url: {e}")))
}

fn smtp_url(a: &Account) -> Result<Url, AppError> {
    let scheme = if a.smtp_security == "ssl" { "smtps" } else { "smtp" };
    Url::parse(&format!("{scheme}://{}:{}", a.smtp_host, a.smtp_port))
        .map_err(|e| AppError::BadRequest(format!("invalid SMTP url: {e}")))
}

fn login(a: &Account) -> SaslLogin {
    SaslLogin {
        username: a.username.clone(),
        password: SecretString::from(a.password.clone()),
    }
}

fn connect_imap(a: &Account) -> Result<EmailClientStd, AppError> {
    let tls = Tls::default();
    let starttls = a.imap_security == "starttls";
    EmailClientStd::new()
        .connect_imap(&imap_url(a)?, &tls, starttls, Some(login(a)), None)
        .map_err(|e| AppError::BadRequest(format!("IMAP connection failed: {e}")))
}

fn connect_smtp(a: &Account) -> Result<EmailClientStd, AppError> {
    let tls = Tls::default();
    let starttls = a.smtp_security == "starttls";
    EmailClientStd::new()
        .connect_smtp(
            &smtp_url(a)?,
            &tls,
            starttls,
            EhloDomain::Domain(Domain(Cow::Borrowed("shiny.local"))),
            Some(login(a)),
        )
        .map_err(|e| AppError::BadRequest(format!("SMTP connection failed: {e}")))
}

/// Run a blocking io-email operation on a blocking thread so the plugin's
/// single runtime thread stays responsive during network round-trips.
pub async fn blocking<T, F>(f: F) -> Result<T, AppError>
where
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::Internal(format!("mail worker failed: {e}")))?
}

// ── account persistence (shared by routes and tools) ──────────

const COLUMNS: &str = "id, label, email, provider, imap_host, imap_port, imap_security, \
    smtp_host, smtp_port, smtp_security, username, password, verified, verified_at, last_error";

fn as_text(v: &Value) -> String {
    match v {
        Value::Text(s) => s.clone(),
        Value::Int(n) => n.to_string(),
        _ => String::new(),
    }
}

fn as_i64(v: &Value) -> i64 {
    match v {
        Value::Int(n) => *n,
        _ => 0,
    }
}

fn as_opt_text(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

fn as_bool(v: &Value) -> bool {
    as_i64(v) != 0
}

fn account_from_row(r: &[Value]) -> Result<Account, AppError> {
    if r.len() < 15 {
        return Err(AppError::Internal("mail_accounts row shape mismatch".into()));
    }
    Ok(Account {
        id: as_text(&r[0]),
        label: as_text(&r[1]),
        email: as_text(&r[2]),
        provider: as_text(&r[3]),
        imap_host: as_text(&r[4]),
        imap_port: as_i64(&r[5]) as u16,
        imap_security: as_text(&r[6]),
        smtp_host: as_text(&r[7]),
        smtp_port: as_i64(&r[8]) as u16,
        smtp_security: as_text(&r[9]),
        username: as_text(&r[10]),
        password: as_text(&r[11]),
        verified: as_bool(&r[12]),
        verified_at: as_opt_text(&r[13]),
        last_error: as_opt_text(&r[14]),
    })
}

pub fn list_accounts(db: &Db, uid: &str) -> Result<Vec<Account>, AppError> {
    let rows = db.query(
        &format!("SELECT {COLUMNS} FROM mail_accounts WHERE user_id = ?1 ORDER BY created_at"),
        &[Value::text(uid)],
    )?;
    rows.iter().map(|r| account_from_row(r)).collect()
}

pub fn load_account(db: &Db, uid: &str, id: &str) -> Result<Account, AppError> {
    let rows = db.query(
        &format!("SELECT {COLUMNS} FROM mail_accounts WHERE id = ?1 AND user_id = ?2"),
        &[Value::text(id), Value::text(uid)],
    )?;
    let row = rows
        .first()
        .ok_or_else(|| AppError::NotFound("mail account not found".into()))?;
    account_from_row(row)
}

pub fn save_account(db: &Db, uid: &str, a: &Account) -> Result<(), AppError> {
    db.execute(
        "INSERT OR REPLACE INTO mail_accounts \
         (id, user_id, label, email, provider, imap_host, imap_port, imap_security, \
          smtp_host, smtp_port, smtp_security, username, password, verified, verified_at, last_error) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        &[
            Value::text(&a.id),
            Value::text(uid),
            Value::text(&a.label),
            Value::text(&a.email),
            Value::text(&a.provider),
            Value::text(&a.imap_host),
            Value::Int(a.imap_port as i64),
            Value::text(&a.imap_security),
            Value::text(&a.smtp_host),
            Value::Int(a.smtp_port as i64),
            Value::text(&a.smtp_security),
            Value::text(&a.username),
            Value::text(&a.password),
            Value::Int(if a.verified { 1 } else { 0 }),
            Value::text(a.verified_at.clone().unwrap_or_default()),
            Value::text(a.last_error.clone().unwrap_or_default()),
        ],
    )?;
    Ok(())
}

pub fn delete_account(db: &Db, uid: &str, id: &str) -> Result<(), AppError> {
    let changed = db.execute(
        "DELETE FROM mail_accounts WHERE id = ?1 AND user_id = ?2",
        &[Value::text(id), Value::text(uid)],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound("mail account not found".into()));
    }
    Ok(())
}

/// Resolve the target account: an explicit id wins, else the first verified
/// account (the default used by the AI tools).
pub fn resolve_account(db: &Db, uid: &str, id: Option<&str>) -> Result<Account, AppError> {
    if let Some(id) = id {
        return load_account(db, uid, id);
    }
    let mut accounts = list_accounts(db, uid)?;
    accounts.retain(|a| a.verified);
    accounts
        .into_iter()
        .next()
        .ok_or_else(|| AppError::BadRequest("no verified mail account configured".into()))
}

/// Apply a provider preset to any empty server fields.
pub fn apply_preset(a: &mut Account) {
    if let Some(p) = preset_for(&a.provider) {
        if a.imap_host.is_empty() {
            a.imap_host = p.imap_host.into();
            a.imap_port = p.imap_port;
            a.imap_security = p.imap_security.into();
        }
        if a.smtp_host.is_empty() {
            a.smtp_host = p.smtp_host.into();
            a.smtp_port = p.smtp_port;
            a.smtp_security = p.smtp_security.into();
        }
    }
}

// ── operations ────────────────────────────────────────────────

/// Verify an account by connecting to IMAP and listing mailboxes.
pub async fn test_connection(a: Account) -> Result<(), AppError> {
    blocking(move || {
        let mut client = connect_imap(&a)?;
        client
            .list_mailboxes(true)
            .map_err(|e| AppError::BadRequest(format!("IMAP check failed: {e}")))?;
        Ok(())
    })
    .await
}

pub async fn list_folders(a: Account) -> Result<Vec<Json>, AppError> {
    blocking(move || {
        let mut client = connect_imap(&a)?;
        let mailboxes = client
            .list_mailboxes(true)
            .map_err(|e| AppError::BadRequest(format!("list mailboxes failed: {e}")))?;
        Ok(mailboxes.iter().map(mailbox_json).collect())
    })
    .await
}

fn mailbox_json(m: &Mailbox) -> Json {
    json!({ "id": m.id, "name": m.name, "total": m.total, "unread": m.unread })
}

pub async fn list_envelopes(a: Account, folder: String, page: u32) -> Result<Vec<Json>, AppError> {
    blocking(move || {
        let mut client = connect_imap(&a)?;
        let envelopes = client
            .list_envelopes(&folder, Some(page), Some(60), false)
            .map_err(|e| AppError::BadRequest(format!("list messages failed: {e}")))?;
        Ok(envelopes.iter().map(envelope_json).collect())
    })
    .await
}

fn envelope_json(e: &Envelope) -> Json {
    let from = e
        .from
        .iter()
        .map(|a| io_addr_str(a.name.as_ref(), &a.email))
        .collect::<Vec<_>>()
        .join(", ");
    let seen = e.flags.iter().any(|f| f.iana() == Some(IanaFlag::Seen));
    json!({
        "id": e.id,
        "subject": e.subject,
        "from": from,
        "from_addresses": e.from.iter().map(|a| json!({"name": a.name, "email": a.email})).collect::<Vec<_>>(),
        "date": e.date.map(|d| d.to_rfc3339()),
        "size": e.size,
        "seen": seen,
        "has_attachment": e.has_attachment,
    })
}

fn io_addr_str(name: Option<&String>, email: &str) -> String {
    match name {
        Some(n) if !n.trim().is_empty() => format!("{n} <{email}>"),
        _ => email.to_string(),
    }
}

pub async fn get_message(a: Account, folder: String, id: String) -> Result<Json, AppError> {
    blocking(move || {
        let mut client = connect_imap(&a)?;
        let raw = client
            .get_message(&folder, &id)
            .map_err(|e| AppError::BadRequest(format!("fetch message failed: {e}")))?;
        parse_message(&id, &raw)
    })
    .await
}

fn parse_message(id: &str, raw: &[u8]) -> Result<Json, AppError> {
    let msg = MessageParser::new()
        .parse(raw)
        .ok_or_else(|| AppError::BadRequest("could not parse message".into()))?;

    let addr_list = |addr: Option<&mail_parser::Address<'_>>| -> Vec<String> {
        match addr {
            Some(a) => a.iter().map(|x| mp_addr_str(x.name(), x.address())).collect(),
            None => Vec::new(),
        }
    };

    let addr_objs = |addr: Option<&mail_parser::Address<'_>>| -> Vec<Json> {
        match addr {
            Some(a) => a
                .iter()
                .map(|x| json!({ "name": x.name().unwrap_or(""), "email": x.address().unwrap_or("") }))
                .collect(),
            None => Vec::new(),
        }
    };

    let attachments: Vec<Json> = msg
        .attachments()
        .map(|att| {
            json!({
                "filename": att.attachment_name().unwrap_or("attachment"),
                "content_type": att.content_type().map(|ct| ct.ctype().to_string()).unwrap_or_else(|| "application/octet-stream".into()),
                "size": att.len(),
            })
        })
        .collect();

    Ok(json!({
        "id": id,
        "subject": msg.subject().unwrap_or(""),
        "from": addr_list(msg.from()),
        "to": addr_list(msg.to()),
        "cc": addr_list(msg.cc()),
        "from_addresses": addr_objs(msg.from()),
        "to_addresses": addr_objs(msg.to()),
        "cc_addresses": addr_objs(msg.cc()),
        "date": msg.date().map(|d| d.to_rfc3339()),
        "message_id": msg.message_id(),
        "text": msg.body_text(0).map(|c| c.into_owned()).unwrap_or_default(),
        "html": msg.body_html(0).map(|c| c.into_owned()).unwrap_or_default(),
        "attachments": attachments,
    }))
}

fn mp_addr_str(name: Option<&str>, address: Option<&str>) -> String {
    match (name, address) {
        (Some(n), Some(e)) if !n.trim().is_empty() => format!("{n} <{e}>"),
        (_, Some(e)) => e.to_string(),
        (Some(n), None) => n.to_string(),
        _ => String::new(),
    }
}

/// In-memory send dedup: the agent occasionally emits `mail_send` twice in a
/// row (e.g. once as `mail.mail_send`, once as `mail_send`). Skip a repeat of
/// an identical message within a short window so it isn't delivered twice.
static RECENT_SENDS: Mutex<Vec<(i64, u64)>> = Mutex::new(Vec::new());
const DEDUP_WINDOW_MS: i64 = 10_000;

fn send_fingerprint(
    to: &[String],
    cc: &[String],
    bcc: &[String],
    subject: &str,
    body: &str,
) -> u64 {
    let mut h = DefaultHasher::new();
    for v in to { v.hash(&mut h); }
    for v in cc { v.hash(&mut h); }
    for v in bcc { v.hash(&mut h); }
    subject.hash(&mut h);
    body.hash(&mut h);
    h.finish()
}

pub async fn send(
    a: Account,
    to: Vec<String>,
    cc: Vec<String>,
    bcc: Vec<String>,
    subject: String,
    body: String,
    html: Option<String>,
) -> Result<bool, AppError> {
    let fp = send_fingerprint(&to, &cc, &bcc, &subject, &body);
    let now = chrono::Utc::now().timestamp_millis();
    {
        let mut recents = RECENT_SENDS.lock().unwrap();
        recents.retain(|(t, _)| now - t <= DEDUP_WINDOW_MS);
        if recents.iter().any(|(_, f)| *f == fp) {
            return Ok(false); // duplicate within the window — already sent
        }
        recents.push((now, fp));
    }

    let result = blocking(move || {
        let mut builder = mail_builder::MessageBuilder::new()
            .from(a.email.clone())
            .to(to)
            .subject(subject)
            .text_body(body);
        if !cc.is_empty() {
            builder = builder.cc(cc);
        }
        if !bcc.is_empty() {
            builder = builder.bcc(bcc);
        }
        if let Some(h) = html {
            if !h.trim().is_empty() {
                builder = builder.html_body(h);
            }
        }
        let raw = builder
            .write_to_vec()
            .map_err(|e| AppError::BadRequest(format!("build message failed: {e}")))?;
        let mut client = connect_smtp(&a)?;
        client
            .send_message(raw)
            .map_err(|e| AppError::BadRequest(format!("send failed: {e}")))?;
        Ok(())
    })
    .await;

    // If the send failed, drop the fingerprint so a retry is allowed.
    if result.is_err() {
        let mut recents = RECENT_SENDS.lock().unwrap();
        recents.retain(|(_, f)| *f != fp);
    }
    result.map(|_| true)
}

/// Mark a set of message ids seen/unseen (read/unread).
pub async fn set_seen(a: Account, folder: String, ids: Vec<String>, seen: bool) -> Result<(), AppError> {
    blocking(move || {
        let mut client = connect_imap(&a)?;
        let refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
        let op = if seen { FlagOp::Add } else { FlagOp::Remove };
        client
            .store_flags(&folder, &refs, &[Flag::from_raw("\\Seen")], op)
            .map_err(|e| AppError::BadRequest(format!("flag update failed: {e}")))?;
        Ok(())
    })
    .await
}

/// Delete a single message (moves it to the server's Trash).
pub async fn delete_message(a: Account, folder: String, id: String) -> Result<(), AppError> {
    blocking(move || {
        let mut client = connect_imap(&a)?;
        client
            .delete_message(&folder, &id)
            .map_err(|e| AppError::BadRequest(format!("delete failed: {e}")))?;
        Ok(())
    })
    .await
}

/// Map a generic io-email error into a friendly message.
pub fn email_err(e: &EmailClientStdError) -> String {
    e.to_string()
}
