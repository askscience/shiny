//! In-memory image session cache.
//!
//! The editor's real-time path applies operations to raw RGBA pixels held in
//! memory, so dragging a slider never touches the codec and (during preview)
//! never touches the database. A commit writes the raw pixels back to SQLite.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use shiny_plugin_sdk::db::{Db, Value};
use shiny_plugin_sdk::errors::AppError;

use crate::ops;

pub struct Session {
    pub raw: Vec<u8>,
    pub original: Vec<u8>,
    pub w: u32,
    pub h: u32,
    pub orig_w: u32,
    pub orig_h: u32,
}

static SESSIONS: OnceLock<Mutex<HashMap<String, Session>>> = OnceLock::new();

fn map() -> &'static Mutex<HashMap<String, Session>> {
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn clear(id: &str) {
    map().lock().unwrap().remove(id);
}

fn as_text(v: &Value) -> String {
    match v {
        Value::Text(s) => s.clone(),
        Value::Int(n) => n.to_string(),
        _ => String::new(),
    }
}

fn as_int(v: &Value) -> i64 {
    match v {
        Value::Int(n) => *n,
        _ => 0,
    }
}

fn as_blob(v: &Value) -> Vec<u8> {
    match v {
        Value::Blob(b) => b.clone(),
        _ => Vec::new(),
    }
}

fn load(db: &Db, uid: &str, id: &str) -> Result<Session, AppError> {
    let rows = db.query(
        "SELECT bytes, original, width, height, orig_width, orig_height, format \
         FROM images WHERE id = ?1 AND user_id = ?2",
        &[Value::text(id), Value::text(uid)],
    )?;
    let row = rows.first().ok_or_else(|| AppError::NotFound("Image not found".into()))?;

    let bytes = as_blob(&row[0]);
    let original = as_blob(&row[1]);
    let w = as_int(&row[2]).max(0) as u32;
    let h = as_int(&row[3]).max(0) as u32;
    let ow = as_int(&row[4]).max(0) as u32;
    let oh = as_int(&row[5]).max(0) as u32;
    let format = as_text(&row[6]);

    if format == "rgba" {
        return Ok(Session { raw: bytes, original, w, h, orig_w: ow, orig_h: oh });
    }

    // Legacy encoded row: decode once, then upgrade to raw in place.
    let cur = ops::decode(&bytes)?;
    let raw = cur.get_raw_pixels();
    let cw = cur.get_width();
    let ch = cur.get_height();
    let (original_raw, oow, ooh) = match ops::decode(&original) {
        Ok(o) => (o.get_raw_pixels(), o.get_width(), o.get_height()),
        Err(_) => (raw.clone(), cw, ch),
    };

    db.execute(
        "UPDATE images SET bytes = ?1, original = ?2, width = ?3, height = ?4, \
         orig_width = ?5, orig_height = ?6, format = 'rgba' \
         WHERE id = ?7 AND user_id = ?8",
        &[
            Value::blob(raw.clone()),
            Value::blob(original_raw.clone()),
            Value::Int(cw as i64),
            Value::Int(ch as i64),
            Value::Int(oow as i64),
            Value::Int(ooh as i64),
            Value::text(id),
            Value::text(uid),
        ],
    )?;

    Ok(Session { raw, original: original_raw, w: cw, h: ch, orig_w: oow, orig_h: ooh })
}

fn persist(db: &Db, uid: &str, id: &str, s: &Session) -> Result<(), AppError> {
    db.execute(
        "UPDATE images SET bytes = ?1, width = ?2, height = ?3, format = 'rgba', \
         updated_at = datetime('now') WHERE id = ?4 AND user_id = ?5",
        &[
            Value::blob(s.raw.clone()),
            Value::Int(s.w as i64),
            Value::Int(s.h as i64),
            Value::text(id),
            Value::text(uid),
        ],
    )?;
    Ok(())
}

/// Run `f` against the in-memory session (loading from DB on first touch),
/// persisting back to the DB only when `commit` is true.
pub fn with_session<T>(
    db: &Db,
    uid: &str,
    id: &str,
    commit: bool,
    f: impl FnOnce(&mut Session) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let mut sessions = map().lock().unwrap();
    if !sessions.contains_key(id) {
        let s = load(db, uid, id)?;
        sessions.insert(id.to_string(), s);
    }
    let s = sessions.get_mut(id).expect("session present");
    let out = f(s)?;
    if commit {
        persist(db, uid, id, s)?;
    }
    Ok(out)
}
