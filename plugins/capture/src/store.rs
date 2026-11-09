//! Per-user settings and live destinations (the stream keys sealed).
//!
//! Keys are sealed with AES-256-GCM (`shiny-vault`) under a machine key kept
//! in `~/.config/liteaura/capture.key` (mode 0600) and bound to their owner
//! and row: the database alone, copied anywhere, reveals no key. A key is
//! opened only to build the ingest URL when a stream starts, and never
//! leaves the server again (the window sees `••••1234`).

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::services::PluginCtx;
use std::path::PathBuf;

use crate::platforms;

const MAX_SETTINGS: usize = 32 * 1024;

pub fn ensure_schema(ctx: &PluginCtx) {
    for sql in include_str!("../migrations/001_init.sql").split(';') {
        let sql = sql.trim();
        if !sql.is_empty() && !sql.lines().all(|l| l.trim_start().starts_with("--")) {
            let _ = ctx.db().execute(sql, &[]);
        }
    }
}

fn key_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp".into());
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"))
        .join("liteaura")
        .join("capture.key")
}

fn machine_key() -> Result<shiny_vault::Key, AppError> {
    shiny_vault::machine_key(&key_path()).map_err(|e| AppError::Internal(e.0))
}

fn aad(user: &str, id: &str) -> String {
    format!("capture:v1:key:{user}:{id}")
}

pub fn get_settings(ctx: &PluginCtx, user: &str) -> Result<Json, AppError> {
    let rows = ctx.db().query(
        "SELECT settings FROM capture_settings WHERE traveler_id = ?1",
        &[Value::text(user)],
    )?;
    Ok(rows
        .into_iter()
        .next()
        .and_then(|r| r.into_iter().next())
        .and_then(|v| match v {
            Value::Text(s) => serde_json::from_str(&s).ok(),
            _ => None,
        })
        .unwrap_or_else(|| serde_json::json!({})))
}

pub fn put_settings(ctx: &PluginCtx, user: &str, s: &Json) -> Result<(), AppError> {
    if !s.is_object() {
        return Err(AppError::BadRequest("settings must be an object".into()));
    }
    let text = s.to_string();
    if text.len() > MAX_SETTINGS {
        return Err(AppError::BadRequest("settings are too large".into()));
    }
    ctx.db().execute(
        "INSERT INTO capture_settings (traveler_id, settings, updated_at) VALUES (?1, ?2, datetime('now')) \
         ON CONFLICT(traveler_id) DO UPDATE SET settings = excluded.settings, updated_at = datetime('now')",
        &[Value::text(user), Value::text(text)],
    )?;
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
pub struct Destination {
    pub id: String,
    pub platform: String,
    pub name: String,
    pub server: String,
    /// `••••1234`, never the key.
    pub key_hint: String,
    pub has_key: bool,
    pub enabled: bool,
    pub kbps: u32,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct DestinationInput {
    pub platform: Option<String>,
    pub name: Option<String>,
    pub server: Option<String>,
    /// A new key; absent keeps the stored one.
    pub key: Option<String>,
    pub enabled: Option<bool>,
    pub kbps: Option<u32>,
}

fn text(v: Option<Value>) -> String {
    match v {
        Some(Value::Text(s)) => s,
        _ => String::new(),
    }
}

fn int(v: Option<Value>) -> i64 {
    match v {
        Some(Value::Int(n)) => n,
        _ => 0,
    }
}

fn row_to_dest(user: &str, row: Vec<Value>) -> Destination {
    let mut it = row.into_iter();
    let id = text(it.next());
    let platform = text(it.next());
    let name = text(it.next());
    let server = text(it.next());
    let sealed = text(it.next());
    let enabled = int(it.next()) != 0;
    let kbps = int(it.next()).max(0) as u32;
    let key = open_key(user, &id, &sealed).unwrap_or_default();
    Destination {
        key_hint: platforms::mask_key(&key),
        has_key: !key.is_empty(),
        id,
        platform,
        name,
        server,
        enabled,
        kbps,
    }
}

fn open_key(user: &str, id: &str, sealed: &str) -> Option<String> {
    if sealed.is_empty() {
        return Some(String::new());
    }
    let key = machine_key().ok()?;
    let raw = shiny_vault::open(&key, sealed, &aad(user, id))?;
    String::from_utf8(raw.to_vec()).ok()
}

const COLS: &str = "id, platform, name, server, key_sealed, enabled, kbps";

pub fn list_destinations(ctx: &PluginCtx, user: &str) -> Result<Vec<Destination>, AppError> {
    let rows = ctx.db().query(
        &format!(
            "SELECT {COLS} FROM capture_destinations WHERE traveler_id = ?1 ORDER BY created_at"
        ),
        &[Value::text(user)],
    )?;
    Ok(rows.into_iter().map(|r| row_to_dest(user, r)).collect())
}

fn get_row(ctx: &PluginCtx, user: &str, id: &str) -> Result<Option<Vec<Value>>, AppError> {
    Ok(ctx
        .db()
        .query(
            &format!("SELECT {COLS} FROM capture_destinations WHERE traveler_id = ?1 AND id = ?2"),
            &[Value::text(user), Value::text(id)],
        )?
        .into_iter()
        .next())
}

fn clean(s: &str, max: usize) -> String {
    s.trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect()
}

pub fn save_destination(
    ctx: &PluginCtx,
    user: &str,
    id: Option<&str>,
    input: &DestinationInput,
) -> Result<Destination, AppError> {
    let existing = match id {
        Some(id) => Some(
            get_row(ctx, user, id)?
                .map(|r| row_to_dest(user, r))
                .ok_or_else(|| AppError::NotFound("destination not found".into()))?,
        ),
        None => None,
    };
    let platform = input
        .platform
        .clone()
        .or_else(|| existing.as_ref().map(|d| d.platform.clone()))
        .unwrap_or_else(|| "custom".into());
    let p =
        platforms::get(&platform).ok_or_else(|| AppError::BadRequest("unknown platform".into()))?;
    let server = clean(
        &input
            .server
            .clone()
            .or_else(|| existing.as_ref().map(|d| d.server.clone()))
            .unwrap_or_default(),
        512,
    );
    let server = if server.is_empty() {
        p.server.to_string()
    } else {
        server
    };
    if server.is_empty() {
        return Err(AppError::BadRequest(
            "this platform needs the server address from its dashboard".into(),
        ));
    }
    platforms::ingest_url(&server, "").map_err(AppError::BadRequest)?;
    let name = clean(
        &input
            .name
            .clone()
            .or_else(|| existing.as_ref().map(|d| d.name.clone()))
            .unwrap_or_else(|| p.name.to_string()),
        80,
    );
    let enabled = input
        .enabled
        .or_else(|| existing.as_ref().map(|d| d.enabled))
        .unwrap_or(true);
    let kbps = input
        .kbps
        .or_else(|| existing.as_ref().map(|d| d.kbps))
        .unwrap_or(0)
        .min(p.max_kbps);
    let id = id
        .map(String::from)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let sealed = match &input.key {
        Some(k) => {
            let k = k.trim();
            if k.chars().any(|c| c.is_whitespace() || c.is_control()) || k.len() > 512 {
                return Err(AppError::BadRequest("the stream key is not valid".into()));
            }
            if k.is_empty() {
                String::new()
            } else {
                shiny_vault::seal(&*machine_key()?, k.as_bytes(), &aad(user, &id))
                    .map_err(|e| AppError::Internal(e.0))?
            }
        }
        None => get_row(ctx, user, &id)?
            .and_then(|r| r.into_iter().nth(4))
            .map(|v| text(Some(v)))
            .unwrap_or_default(),
    };
    ctx.db().execute(
        "INSERT INTO capture_destinations (id, traveler_id, platform, name, server, key_sealed, enabled, kbps) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT(id) DO UPDATE SET platform = excluded.platform, name = excluded.name, \
         server = excluded.server, key_sealed = excluded.key_sealed, enabled = excluded.enabled, \
         kbps = excluded.kbps, updated_at = datetime('now') WHERE traveler_id = excluded.traveler_id",
        &[
            Value::text(&id),
            Value::text(user),
            Value::text(&platform),
            Value::text(&name),
            Value::text(&server),
            Value::text(sealed),
            Value::Int(enabled as i64),
            Value::Int(kbps as i64),
        ],
    )?;
    get_row(ctx, user, &id)?
        .map(|r| row_to_dest(user, r))
        .ok_or_else(|| AppError::Internal("destination not saved".into()))
}

pub fn delete_destination(ctx: &PluginCtx, user: &str, id: &str) -> Result<bool, AppError> {
    Ok(ctx.db().execute(
        "DELETE FROM capture_destinations WHERE traveler_id = ?1 AND id = ?2",
        &[Value::text(user), Value::text(id)],
    )? > 0)
}

/// Ingest URLs (with keys), labels and platforms, in the same order.
pub type Ingest = (Vec<String>, Vec<String>, Vec<&'static platforms::Platform>);

/// Ingest URLs (with keys) for the chosen destinations, plus their labels
/// and platforms. Unknown ids are an error, not silently skipped.
pub fn ingest_for(ctx: &PluginCtx, user: &str, ids: &[String]) -> Result<Ingest, AppError> {
    let (mut urls, mut labels, mut plats) = (Vec::new(), Vec::new(), Vec::new());
    for id in ids {
        let row = get_row(ctx, user, id)?
            .ok_or_else(|| AppError::NotFound(format!("destination {id} not found")))?;
        let sealed = text(row.get(4).cloned());
        let d = row_to_dest(user, row);
        let key = open_key(user, id, &sealed)
            .ok_or_else(|| AppError::Internal("the stream key could not be opened".into()))?;
        if key.is_empty() && d.platform != "custom" {
            return Err(AppError::BadRequest(format!(
                "{} has no stream key",
                d.name
            )));
        }
        urls.push(platforms::ingest_url(&d.server, &key).map_err(AppError::BadRequest)?);
        labels.push(d.name.clone());
        plats.push(platforms::get(&d.platform).unwrap_or(&platforms::PLATFORMS[4]));
    }
    Ok((urls, labels, plats))
}
