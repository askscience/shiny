//! Local mail cache — persists envelopes + parsed bodies so the AI and the
//! window read mail from SQLite instead of re-fetching from IMAP every request.
//!
//! Rows are keyed by `account_id:folder:uid` (the IMAP UID). The AI-facing
//! JSON shapes mirror what the live `mail.rs` helpers used to return, so the
//! tools and the window don't need to change their parsing.

use std::collections::HashSet;

use serde_json::{json, Value as Json};
use shiny_plugin_sdk::db::{Db, Value};
use shiny_plugin_sdk::errors::AppError;

/// Upper bound on how many messages one folder sync will backfill.
pub const MAX_SYNC: usize = 300;

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

fn parse_json_array(s: &str) -> Json {
    serde_json::from_str::<Json>(s).unwrap_or_else(|_| json!([]))
}

/// Turn a stored `[{name, email}, ...]` JSON array into a `["Name <email>", ...]`
/// list, matching the shape the live message parser produced for `from`/`to`/`cc`.
fn display_list(json_arr: &Json) -> Vec<String> {
    json_arr
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|o| {
                    let name = o.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let email = o.get("email").and_then(|v| v.as_str()).unwrap_or("");
                    match (name.is_empty(), email.is_empty()) {
                        (false, false) => Some(format!("{name} <{email}>")),
                        (false, true) => Some(name.to_string()),
                        (true, false) => Some(email.to_string()),
                        (true, true) => None,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn msg_id(account_id: &str, folder: &str, uid: &str) -> String {
    format!("{account_id}:{folder}:{uid}")
}

/// Insert or replace one cached message. `row` carries the merged fields:
/// uid, message_id, subject, from_addr, to_addr, cc_addr, from_json, to_json,
/// cc_json, sent_at, body_text, body_html, attachments_json, seen,
/// has_attachment, size.
pub fn upsert(
    db: &Db,
    user_id: &str,
    account_id: &str,
    folder: &str,
    row: &Json,
) -> Result<(), AppError> {
    let uid = row.get("uid").and_then(|v| v.as_str()).unwrap_or("");
    if uid.is_empty() {
        return Ok(());
    }
    let id = msg_id(account_id, folder, uid);
    let message_id = row.get("message_id").and_then(|v| v.as_str()).map(String::from);
    let subject = row.get("subject").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let from_addr = row.get("from_addr").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let to_addr = row.get("to_addr").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let cc_addr = row.get("cc_addr").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let from_json = row.get("from_json").map(|v| v.to_string()).unwrap_or_else(|| "[]".into());
    let to_json = row.get("to_json").map(|v| v.to_string()).unwrap_or_else(|| "[]".into());
    let cc_json = row.get("cc_json").map(|v| v.to_string()).unwrap_or_else(|| "[]".into());
    let sent_at = row.get("sent_at").and_then(|v| v.as_str()).map(String::from);
    let body_text = row.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let body_html = row.get("html").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let attachments_json = row.get("attachments").map(|v| v.to_string()).unwrap_or_else(|| "[]".into());
    let seen = row.get("seen").and_then(|v| v.as_bool()).unwrap_or(false);
    let has_attachment = row.get("has_attachment").and_then(|v| v.as_bool()).unwrap_or(false);
    let size = row.get("size").and_then(|v| v.as_i64()).unwrap_or(0);

    db.execute(
        "INSERT OR REPLACE INTO mail_messages \
         (id, user_id, account_id, folder, uid, message_id, subject, from_addr, to_addr, cc_addr, \
          from_json, to_json, cc_json, sent_at, body_text, body_html, attachments_json, \
          seen, has_attachment, size, fetched_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, datetime('now'), datetime('now'))",
        &[
            Value::text(&id),
            Value::text(user_id),
            Value::text(account_id),
            Value::text(folder),
            Value::text(uid),
            Value::text(message_id.unwrap_or_default()),
            Value::text(&subject),
            Value::text(&from_addr),
            Value::text(&to_addr),
            Value::text(&cc_addr),
            Value::text(&from_json),
            Value::text(&to_json),
            Value::text(&cc_json),
            Value::text(sent_at.unwrap_or_default()),
            Value::text(&body_text),
            Value::text(&body_html),
            Value::text(&attachments_json),
            Value::Int(if seen { 1 } else { 0 }),
            Value::Int(if has_attachment { 1 } else { 0 }),
            Value::Int(size),
        ],
    )?;
    Ok(())
}

/// Cache a single message from a live parse (`parse_message` shape). Used as a
/// fallback when a requested id isn't in the cache yet: fetch once, store it.
pub fn upsert_message(
    db: &Db,
    user_id: &str,
    account_id: &str,
    folder: &str,
    uid: &str,
    msg: &Json,
) -> Result<(), AppError> {
    let join = |key: &str| -> String {
        msg.get(key)
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", "))
            .unwrap_or_default()
    };
    let has_attachment = msg
        .get("attachments")
        .and_then(|v| v.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);

    let row = json!({
        "uid": uid,
        "message_id": msg.get("message_id").and_then(|v| v.as_str()).map(String::from),
        "subject": msg.get("subject").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        "from_addr": join("from"),
        "to_addr": join("to"),
        "cc_addr": join("cc"),
        "from_json": msg.get("from_addresses").cloned().unwrap_or_else(|| json!([])),
        "to_json": msg.get("to_addresses").cloned().unwrap_or_else(|| json!([])),
        "cc_json": msg.get("cc_addresses").cloned().unwrap_or_else(|| json!([])),
        "sent_at": msg.get("date").and_then(|v| v.as_str()).map(String::from),
        "text": msg.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        "html": msg.get("html").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        "attachments": msg.get("attachments").cloned().unwrap_or_else(|| json!([])),
        "seen": false,
        "has_attachment": has_attachment,
        "size": 0,
    });
    upsert(db, user_id, account_id, folder, &row)
}

/// UIDs already cached for one account + folder (scoped to the user like
/// every other cache query).
pub fn cached_uids(
    db: &Db,
    user_id: &str,
    account_id: &str,
    folder: &str,
) -> Result<HashSet<String>, AppError> {
    let rows = db.query(
        "SELECT uid FROM mail_messages WHERE user_id = ?1 AND account_id = ?2 AND folder = ?3",
        &[Value::text(user_id), Value::text(account_id), Value::text(folder)],
    )?;
    Ok(rows.into_iter().filter_map(|r| r.into_iter().next().map(|v| as_text(&v))).collect())
}

/// Refresh only the envelope-derived flags of an already-cached message.
/// Re-syncs must NOT full-upsert cached rows: bodies are only fetched for
/// new UIDs, and a full replace would overwrite the stored body with "".
pub fn update_flags(
    db: &Db,
    user_id: &str,
    account_id: &str,
    folder: &str,
    uid: &str,
    seen: bool,
    has_attachment: bool,
) -> Result<(), AppError> {
    db.execute(
        "UPDATE mail_messages SET seen = ?4, has_attachment = ?5 \
         WHERE user_id = ?1 AND account_id = ?2 AND folder = ?3 AND uid = ?6",
        &[
            Value::text(user_id),
            Value::text(account_id),
            Value::text(folder),
            Value::Int(seen as i64),
            Value::Int(has_attachment as i64),
            Value::text(uid),
        ],
    )?;
    Ok(())
}

/// Mark cached messages seen/unseen without an IMAP round-trip (used right
/// after the server-side flag store so list/read stay consistent).
pub fn mark_seen(
    db: &Db,
    user_id: &str,
    account_id: &str,
    folder: &str,
    ids: &[String],
    seen: bool,
) -> Result<(), AppError> {
    for id in ids {
        db.execute(
            "UPDATE mail_messages SET seen = ?4 \
             WHERE user_id = ?1 AND account_id = ?2 AND folder = ?3 AND uid = ?5",
            &[
                Value::text(user_id),
                Value::text(account_id),
                Value::text(folder),
                Value::Int(seen as i64),
                Value::text(id),
            ],
        )?;
    }
    Ok(())
}

fn envelope_json(uid: &str, subject: &str, from_addr: &str, from_json: &str, sent_at: Option<&str>, size: i64, seen: bool, has_attachment: bool) -> Json {
    json!({
        "id": uid,
        "subject": subject,
        "from": from_addr,
        "from_addresses": parse_json_array(from_json),
        "date": sent_at,
        "size": size,
        "seen": seen,
        "has_attachment": has_attachment,
    })
}

/// List cached envelopes for one account + folder (newest first).
pub fn list(
    db: &Db,
    user_id: &str,
    account_id: &str,
    folder: &str,
    limit: usize,
    offset: usize,
) -> Result<Vec<Json>, AppError> {
    let rows = db.query(
        "SELECT uid, subject, from_addr, from_json, sent_at, size, seen, has_attachment \
         FROM mail_messages \
         WHERE user_id = ?1 AND account_id = ?2 AND folder = ?3 \
         ORDER BY sent_at DESC LIMIT ?4 OFFSET ?5",
        &[
            Value::text(user_id),
            Value::text(account_id),
            Value::text(folder),
            Value::Int(limit as i64),
            Value::Int(offset as i64),
        ],
    )?;

    Ok(rows
        .iter()
        .map(|r| {
            let uid = as_text(&r[0]);
            let subject = as_text(&r[1]);
            let from_addr = as_text(&r[2]);
            let from_json = as_text(&r[3]);
            let sent_at = as_opt_text(&r[4]);
            let size = as_i64(&r[5]);
            let seen = as_i64(&r[6]) != 0;
            let has_attachment = as_i64(&r[7]) != 0;
            envelope_json(&uid, &subject, &from_addr, &from_json, sent_at.as_deref(), size, seen, has_attachment)
        })
        .collect())
}

/// Fetch one full cached message by UID, reconstructed to the live-parser shape.
pub fn get(
    db: &Db,
    user_id: &str,
    account_id: &str,
    folder: &str,
    uid: &str,
) -> Result<Option<Json>, AppError> {
    let rows = db.query(
        "SELECT uid, message_id, subject, from_json, to_json, cc_json, \
                sent_at, body_text, body_html, attachments_json \
         FROM mail_messages WHERE user_id = ?1 AND account_id = ?2 AND folder = ?3 AND uid = ?4",
        &[
            Value::text(user_id),
            Value::text(account_id),
            Value::text(folder),
            Value::text(uid),
        ],
    )?;
    let Some(r) = rows.first() else {
        return Ok(None);
    };
    let uid = as_text(&r[0]);
    let message_id = as_opt_text(&r[1]);
    let subject = as_text(&r[2]);
    let from_json = parse_json_array(&as_text(&r[3]));
    let to_json = parse_json_array(&as_text(&r[4]));
    let cc_json = parse_json_array(&as_text(&r[5]));
    let sent_at = as_opt_text(&r[6]);
    let body_text = as_text(&r[7]);
    let body_html = as_text(&r[8]);
    let attachments = parse_json_array(&as_text(&r[9]));

    Ok(Some(json!({
        "id": uid,
        "subject": subject,
        "from": display_list(&from_json),
        "to": display_list(&to_json),
        "cc": display_list(&cc_json),
        "from_addresses": from_json,
        "to_addresses": to_json,
        "cc_addresses": cc_json,
        "date": sent_at,
        "message_id": message_id,
        "text": body_text,
        "html": body_html,
        "attachments": attachments,
    })))
}

/// Full-text search over the local cache (subject / sender / body).
/// `folder` restricts to one folder when provided; otherwise searches all.
pub fn search(
    db: &Db,
    user_id: &str,
    account_id: Option<&str>,
    folder: Option<&str>,
    query: &str,
    limit: usize,
) -> Result<Vec<Json>, AppError> {
    let like = format!("%{}%", query.to_lowercase());
    let (sql, params): (String, Vec<Value>) = if let Some(acc) = account_id {
        if let Some(f) = folder {
            (
                "SELECT uid, subject, from_addr, from_json, sent_at, size, seen, has_attachment \
                 FROM mail_messages \
                 WHERE user_id = ?1 AND account_id = ?2 AND folder = ?3 \
                   AND (lower(subject) LIKE ?4 OR lower(from_addr) LIKE ?4 OR lower(body_text) LIKE ?4) \
                 ORDER BY sent_at DESC LIMIT ?5".into(),
                vec![Value::text(user_id), Value::text(acc), Value::text(f), Value::text(&like), Value::Int(limit as i64)],
            )
        } else {
            (
                "SELECT uid, subject, from_addr, from_json, sent_at, size, seen, has_attachment \
                 FROM mail_messages \
                 WHERE user_id = ?1 AND account_id = ?2 \
                   AND (lower(subject) LIKE ?3 OR lower(from_addr) LIKE ?3 OR lower(body_text) LIKE ?3) \
                 ORDER BY sent_at DESC LIMIT ?4".into(),
                vec![Value::text(user_id), Value::text(acc), Value::text(&like), Value::Int(limit as i64)],
            )
        }
    } else {
        (
            "SELECT uid, subject, from_addr, from_json, sent_at, size, seen, has_attachment \
             FROM mail_messages \
             WHERE user_id = ?1 \
               AND (lower(subject) LIKE ?2 OR lower(from_addr) LIKE ?2 OR lower(body_text) LIKE ?2) \
             ORDER BY sent_at DESC LIMIT ?3".into(),
            vec![Value::text(user_id), Value::text(&like), Value::Int(limit as i64)],
        )
    };

    let rows = db.query(&sql, &params)?;
    Ok(rows
        .iter()
        .map(|r| {
            let uid = as_text(&r[0]);
            let subject = as_text(&r[1]);
            let from_addr = as_text(&r[2]);
            let from_json = as_text(&r[3]);
            let sent_at = as_opt_text(&r[4]);
            let size = as_i64(&r[5]);
            let seen = as_i64(&r[6]) != 0;
            let has_attachment = as_i64(&r[7]) != 0;
            envelope_json(&uid, &subject, &from_addr, &from_json, sent_at.as_deref(), size, seen, has_attachment)
        })
        .collect())
}

/// (last_synced_at, total) for one account + folder, scoped to the user.
pub fn sync_state(
    db: &Db,
    user_id: &str,
    account_id: &str,
    folder: &str,
) -> Result<Option<(String, i64)>, AppError> {
    let rows = db.query(
        "SELECT last_synced_at, total FROM mail_sync_state \
         WHERE user_id = ?1 AND account_id = ?2 AND folder = ?3",
        &[Value::text(user_id), Value::text(account_id), Value::text(folder)],
    )?;
    Ok(rows.first().map(|r| (as_text(&r[0]), as_i64(&r[1]))))
}

/// True when the folder was synced within the last `max_age_secs`. Lets the
/// list/search backfill skip empty folders it just synced instead of hitting
/// IMAP on every call.
pub fn synced_recently(
    db: &Db,
    user_id: &str,
    account_id: &str,
    folder: &str,
    max_age_secs: i64,
) -> Result<bool, AppError> {
    let rows = db.query(
        "SELECT COUNT(*) FROM mail_sync_state \
         WHERE user_id = ?1 AND account_id = ?2 AND folder = ?3 \
           AND datetime(last_synced_at) >= datetime('now', ?4)",
        &[
            Value::text(user_id),
            Value::text(account_id),
            Value::text(folder),
            Value::text(format!("-{max_age_secs} seconds")),
        ],
    )?;
    Ok(rows.first().map(|r| as_i64(&r[0])).unwrap_or(0) > 0)
}

/// How many messages are cached for one account + folder.
pub fn total(db: &Db, user_id: &str, account_id: &str, folder: &str) -> Result<i64, AppError> {
    let rows = db.query(
        "SELECT COUNT(*) FROM mail_messages WHERE user_id = ?1 AND account_id = ?2 AND folder = ?3",
        &[Value::text(user_id), Value::text(account_id), Value::text(folder)],
    )?;
    Ok(rows.first().map(|r| as_i64(&r[0])).unwrap_or(0))
}

/// Record a completed sync (upsert sync state + optionally prune a stale row).
pub fn set_sync_state(
    db: &Db,
    user_id: &str,
    account_id: &str,
    folder: &str,
    total: i64,
) -> Result<(), AppError> {
    db.execute(
        "INSERT OR REPLACE INTO mail_sync_state (account_id, folder, user_id, last_synced_at, total) \
         VALUES (?1, ?2, ?3, datetime('now'), ?4)",
        &[Value::text(account_id), Value::text(folder), Value::text(user_id), Value::Int(total)],
    )?;
    Ok(())
}
