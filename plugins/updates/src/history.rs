//! The `updates_history` audit trail.
//!
//! Must run on the plugin runtime thread (it uses the synchronous `ctx.db()`),
//! so job threads never touch the database — the route/tool records history
//! after the job completes.

use std::sync::Arc;
use std::time::Duration;

use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::services::PluginCtx;

use crate::jobs::Job;

const OUTPUT_LIMIT: usize = 8000;

fn truncate(s: &str) -> String {
    if s.len() <= OUTPUT_LIMIT {
        return s.to_string();
    }
    let mut end = OUTPUT_LIMIT;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… (truncated)", &s[..end])
}

pub fn record(
    ctx: &PluginCtx,
    user_id: &str,
    action: &str,
    manager: &str,
    detail: &str,
    success: bool,
    output: &str,
) {
    let params = [
        Value::text(user_id),
        Value::text(action),
        Value::text(manager),
        Value::text(detail),
        Value::text(if success { "ok" } else { "error" }),
        Value::text(truncate(output)),
    ];
    if let Err(e) = ctx.db().execute(
        "INSERT INTO updates_history (user_id, action, manager, detail, status, output) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        &params,
    ) {
        tracing::warn!("updates: history insert failed: {e}");
    }
}

/// Record a job's outcome once it finishes, without blocking the caller.
///
/// Used by the write routes/tools so a long upgrade never occupies the shared
/// plugin runtime; the waiter is queued on that runtime only when the job has
/// actually finished.
pub fn record_when_done(
    ctx: PluginCtx,
    user_id: String,
    action: &'static str,
    manager: String,
    detail: String,
    job: Arc<Job>,
) {
    shiny_plugin_sdk::rt::spawn(async move {
        let deadline = std::time::Instant::now() + Duration::from_secs(7200);
        loop {
            let (done, success, _err) = job.state();
            if done {
                record(
                    &ctx,
                    &user_id,
                    action,
                    &manager,
                    &detail,
                    success.unwrap_or(false),
                    &job.full_log(),
                );
                break;
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1000)).await;
        }
    });
}

pub fn list(ctx: &PluginCtx, user_id: &str, limit: usize) -> Vec<serde_json::Value> {
    let limit = limit.clamp(1, 200) as i64;
    let rows = ctx
        .db()
        .query(
            "SELECT id, action, manager, detail, status, substr(output, 1, 2000), at \
             FROM updates_history WHERE user_id = ?1 ORDER BY id DESC LIMIT ?2",
            &[Value::text(user_id), Value::Int(limit)],
        )
        .unwrap_or_default();

    rows.into_iter()
        .map(|row| {
            let mut it = row.into_iter();
            let id = int_of(it.next());
            let action = text_of(it.next());
            let manager = text_of(it.next());
            let detail = text_of(it.next());
            let status = text_of(it.next());
            let output = text_of(it.next());
            let at = text_of(it.next());
            serde_json::json!({
                "id": id,
                "action": action,
                "manager": manager,
                "detail": detail,
                "status": status,
                "output": output,
                "at": at,
            })
        })
        .collect()
}

fn text_of(v: Option<Value>) -> String {
    match v {
        Some(Value::Text(s)) => s,
        Some(Value::Int(i)) => i.to_string(),
        Some(Value::Blob(b)) => String::from_utf8_lossy(&b).into_owned(),
        _ => String::new(),
    }
}

fn int_of(v: Option<Value>) -> i64 {
    match v {
        Some(Value::Int(i)) => i,
        Some(Value::Text(s)) => s.parse().unwrap_or(0),
        _ => 0,
    }
}
