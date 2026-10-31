//! Persistence shared by the routes and the agent tools: committing new bytes
//! (with an undo snapshot), undo / redo, and history counts.
//!
//! Every edit used to overwrite the stored bytes with no way back — one wrong
//! find & replace and the original was gone. Now each commit first copies the
//! bytes it replaces into `pdf_revisions` (SQL-side, no extra round trip of the
//! blob through the plugin).

use serde_json::{json, Value as Json};

use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::services::PluginCtx;

/// Revisions kept per document and direction.
pub const KEEP: i64 = 25;

fn as_int(v: &Value) -> i64 {
    match v {
        Value::Int(n) => *n,
        _ => 0,
    }
}

fn snapshot(ctx: &PluginCtx, uid: &str, id: &str, kind: &str, label: &str) -> Result<(), AppError> {
    ctx.db().execute(
        "INSERT INTO pdf_revisions (pdf_id, user_id, kind, label, bytes, page_count) \
         SELECT id, user_id, ?3, ?4, bytes, page_count FROM pdf_documents WHERE id = ?1 AND user_id = ?2",
        &[Value::text(id), Value::text(uid), Value::text(kind), Value::text(label)],
    )?;
    ctx.db().execute(
        "DELETE FROM pdf_revisions WHERE pdf_id = ?1 AND user_id = ?2 AND kind = ?3 AND id NOT IN \
         (SELECT id FROM pdf_revisions WHERE pdf_id = ?1 AND user_id = ?2 AND kind = ?3 ORDER BY id DESC LIMIT ?4)",
        &[Value::text(id), Value::text(uid), Value::text(kind), Value::Int(KEEP)],
    )?;
    Ok(())
}

fn write(ctx: &PluginCtx, uid: &str, id: &str, bytes: Vec<u8>, count: usize) -> Result<(), AppError> {
    let changed = ctx.db().execute(
        "UPDATE pdf_documents SET bytes = ?1, page_count = ?2, updated_at = datetime('now') \
         WHERE id = ?3 AND user_id = ?4",
        &[Value::blob(bytes), Value::Int(count as i64), Value::text(id), Value::text(uid)],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound("PDF not found".into()));
    }
    Ok(())
}

/// Replace a document's bytes, keeping the previous version for undo.
/// `label` names the edit ("Rotate", "Replace text"…) for the history.
pub fn commit(
    ctx: &PluginCtx,
    uid: &str,
    id: &str,
    bytes: Vec<u8>,
    count: usize,
    label: &str,
) -> Result<usize, AppError> {
    snapshot(ctx, uid, id, "undo", label)?;
    // A new edit forks history: whatever could be redone is gone.
    ctx.db().execute(
        "DELETE FROM pdf_revisions WHERE pdf_id = ?1 AND user_id = ?2 AND kind = 'redo'",
        &[Value::text(id), Value::text(uid)],
    )?;
    write(ctx, uid, id, bytes, count)?;
    Ok(count)
}

/// Step through history: `from` is the stack to pop (`undo` or `redo`), the
/// current bytes go onto the other one. Returns the restored page count and
/// the label of the step taken.
fn step(ctx: &PluginCtx, uid: &str, id: &str, from: &str) -> Result<(usize, String), AppError> {
    let rows = ctx.db().query(
        "SELECT id, bytes, page_count, label FROM pdf_revisions \
         WHERE pdf_id = ?1 AND user_id = ?2 AND kind = ?3 ORDER BY id DESC LIMIT 1",
        &[Value::text(id), Value::text(uid), Value::text(from)],
    )?;
    let Some(row) = rows.first() else {
        let what = if from == "undo" { "undo" } else { "redo" };
        return Err(AppError::BadRequest(format!("Nothing to {what}")));
    };
    let rev_id = as_int(&row[0]);
    let bytes = match &row[1] {
        Value::Blob(b) => b.clone(),
        _ => return Err(AppError::Internal("corrupt revision".into())),
    };
    let count = as_int(&row[2]) as usize;
    let label = shiny_plugin_sdk::util::as_text(&row[3]);
    let to = if from == "undo" { "redo" } else { "undo" };
    snapshot(ctx, uid, id, to, &label)?;
    write(ctx, uid, id, bytes, count)?;
    ctx.db().execute("DELETE FROM pdf_revisions WHERE id = ?1", &[Value::Int(rev_id)])?;
    Ok((count, label))
}

pub fn undo(ctx: &PluginCtx, uid: &str, id: &str) -> Result<(usize, String), AppError> {
    step(ctx, uid, id, "undo")
}

pub fn redo(ctx: &PluginCtx, uid: &str, id: &str) -> Result<(usize, String), AppError> {
    step(ctx, uid, id, "redo")
}

/// `{ undo, redo }` counts plus the label of the next undo, for the window.
pub fn history(ctx: &PluginCtx, uid: &str, id: &str) -> Result<Json, AppError> {
    let rows = ctx.db().query(
        "SELECT kind, COUNT(*) FROM pdf_revisions WHERE pdf_id = ?1 AND user_id = ?2 GROUP BY kind",
        &[Value::text(id), Value::text(uid)],
    )?;
    let mut undo = 0;
    let mut redo = 0;
    for r in &rows {
        match shiny_plugin_sdk::util::as_text(&r[0]).as_str() {
            "undo" => undo = as_int(&r[1]),
            "redo" => redo = as_int(&r[1]),
            _ => {}
        }
    }
    let next = ctx.db().query(
        "SELECT label FROM pdf_revisions WHERE pdf_id = ?1 AND user_id = ?2 AND kind = 'undo' ORDER BY id DESC LIMIT 1",
        &[Value::text(id), Value::text(uid)],
    )?;
    let undo_label = next.first().map(|r| shiny_plugin_sdk::util::as_text(&r[0]));
    Ok(json!({ "undo": undo, "redo": redo, "undo_label": undo_label }))
}

/// Drop a document's history (when the document itself is deleted).
pub fn forget(ctx: &PluginCtx, uid: &str, id: &str) -> Result<(), AppError> {
    ctx.db().execute(
        "DELETE FROM pdf_revisions WHERE pdf_id = ?1 AND user_id = ?2",
        &[Value::text(id), Value::text(uid)],
    )?;
    Ok(())
}

/// The real `updated_at` of a document, so responses never carry a fake one.
pub fn updated_at(ctx: &PluginCtx, uid: &str, id: &str) -> String {
    ctx.db()
        .query(
            "SELECT updated_at FROM pdf_documents WHERE id = ?1 AND user_id = ?2",
            &[Value::text(id), Value::text(uid)],
        )
        .ok()
        .and_then(|r| r.first().map(|row| shiny_plugin_sdk::util::as_text(&row[0])))
        .unwrap_or_default()
}
