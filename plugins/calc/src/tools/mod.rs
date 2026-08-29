//! Calc plugin tools: create/write/read/list/delete spreadsheets.
//!
//! Spreadsheets live in the core-owned `spreadsheets` table as a JSON cell
//! map ("A1" -> value) — same interim pattern as the word plugin writing the
//! core-owned `documents` table. The plugin writes through its own SQLite
//! pool (`ctx.pool()`), exactly like the traveler plugin.

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use sqlx::SqlitePool;
use std::collections::BTreeMap;

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

const MAX_CELLS: usize = 5000;
const MAX_CELL_VALUE_LEN: usize = 10_000;

async fn last_sheet_id(pool: &SqlitePool, user_id: &str) -> Result<Option<String>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM spreadsheets WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?)
}

fn sheet_summary_json(row: &(String, String, i64, i64, String)) -> Value {
    json!({
        "sheet_id": row.0,
        "title": row.1,
        "rows": row.2,
        "cols": row.3,
        "updated_at": row.4,
    })
}

/// A1-style reference: 1–2 uppercase letters followed by a positive number.
fn is_valid_cell_ref(cell_ref: &str) -> bool {
    let bytes = cell_ref.as_bytes();
    let mut i = 0;
    let mut letters = 0;
    while i < bytes.len() && bytes[i].is_ascii_uppercase() {
        letters += 1;
        i += 1;
        if letters > 2 {
            return false;
        }
    }
    if letters == 0 || i >= bytes.len() {
        return false;
    }
    let digits = &cell_ref[i..];
    digits.len() <= 4 && digits.chars().all(|c| c.is_ascii_digit()) && digits != "0"
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Parse the stored cells JSON into a map (defensive: bad data = empty).
fn parse_cells(json: &str) -> BTreeMap<String, String> {
    if json.trim().is_empty() {
        return BTreeMap::new();
    }
    serde_json::from_str::<BTreeMap<String, String>>(json)
        .unwrap_or_default()
        .into_iter()
        .take(MAX_CELLS)
        .collect()
}

/// Merge incoming cells into the stored map: upsert, drop empties, validate.
fn merge_cells(
    stored_json: &str,
    incoming: &Map<String, Value>,
) -> Result<BTreeMap<String, String>, AppError> {
    let mut cells = parse_cells(stored_json);

    for (raw_ref, value) in incoming {
        let cell_ref = raw_ref.trim().to_uppercase();
        if !is_valid_cell_ref(&cell_ref) {
            return Err(AppError::BadRequest(format!(
                "Invalid cell reference \"{raw_ref}\" — expected something like A1 or BC42"
            )));
        }
        let text = value_to_string(value);
        if text.chars().count() > MAX_CELL_VALUE_LEN {
            return Err(AppError::BadRequest(format!(
                "Value in {cell_ref} is too long (max {MAX_CELL_VALUE_LEN} chars)"
            )));
        }
        if text.trim().is_empty() {
            cells.remove(&cell_ref);
        } else {
            cells.insert(cell_ref, text);
        }
    }

    if cells.len() > MAX_CELLS {
        return Err(AppError::BadRequest(format!("Too many cells (max {MAX_CELLS})")));
    }
    Ok(cells)
}

fn cells_param<'a>(req: &'a ToolRequest<'a>) -> Option<&'a Map<String, Value>> {
    req.params.get("cells").and_then(|v| v.as_object())
}

/* ── calc_create ────────────────────────────────────────────── */

pub struct CalcCreate;

#[async_trait]
impl Tool for CalcCreate {
    fn name(&self) -> &str { "calc_create" }
    fn aliases(&self) -> &[&str] { &["create_spreadsheet", "new_sheet", "new_spreadsheet"] }
    fn step_label(&self) -> &str { "Creating spreadsheet…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `calc_create` — Create a new spreadsheet. params: `{ title?: string, rows?: number, cols?: number }` — returns the new `sheet_id`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
        format!("Created spreadsheet \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let title = req
            .params
            .param_str("title")
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "Untitled".into());
        let rows = req.params.param_u32("rows").unwrap_or(100).clamp(1, 500) as i64;
        let cols = req.params.param_u32("cols").unwrap_or(26).clamp(1, 52) as i64;
        let id = uuid::Uuid::new_v4().to_string();

        sqlx::query(
            "INSERT INTO spreadsheets (id, user_id, title, cells, rows, cols, created_at, updated_at) \
             VALUES (?1, ?2, ?3, '{}', ?4, ?5, datetime('now'), datetime('now'))",
        )
        .bind(&id)
        .bind(req.traveler_id)
        .bind(&title)
        .bind(rows)
        .bind(cols)
        .execute(ctx.pool().await)
        .await?;

        Ok(ActionOutcome::ok(
            "calc_create",
            json!({ "sheet_id": id, "title": title, "rows": rows, "cols": cols }),
        ))
    }
}

/* ── calc_write ─────────────────────────────────────────────── */

pub struct CalcWrite;

#[async_trait]
impl Tool for CalcWrite {
    fn name(&self) -> &str { "calc_write" }
    fn aliases(&self) -> &[&str] {
        &["set_cell", "set_cells", "write_sheet", "edit_spreadsheet", "update_spreadsheet"]
    }
    fn step_label(&self) -> &str { "Writing cells…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `calc_write` — Write cell values into a spreadsheet. params: `{ sheet_id?: string, title?: string, cells: { A1: \"value\", ... } }` — upserts only the listed cells; an empty-string value clears a cell. Without `sheet_id` targets the most recently used spreadsheet.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("spreadsheet");
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Wrote {n} cells to \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let incoming = cells_param(&req).ok_or_else(|| {
            AppError::BadRequest("cells required — an object like {\"A1\":\"100\",\"B1\":\"=SUM(A1:A5)\"}".into())
        })?;

        let sheet_id = match req.params.param_str("sheet_id") {
            Some(id) if !id.trim().is_empty() => id,
            _ => last_sheet_id(ctx.pool().await, req.traveler_id)
                .await?
                .ok_or_else(|| {
                    AppError::BadRequest("No spreadsheet yet — call calc_create first".into())
                })?,
        };

        let row = sqlx::query_as::<_, (String, String)>(
            "SELECT title, cells FROM spreadsheets WHERE id = ?1 AND user_id = ?2",
        )
        .bind(&sheet_id)
        .bind(req.traveler_id)
        .fetch_optional(ctx.pool().await)
        .await?
        .ok_or_else(|| AppError::NotFound("Spreadsheet not found".into()))?;
        let (title, stored_json) = row;

        let merged = merge_cells(&stored_json, incoming)?;
        let new_title = match req.params.param_str("title") {
            Some(t) if !t.trim().is_empty() => t.trim().chars().take(120).collect::<String>(),
            _ => title.clone(),
        };
        let cells_json = serde_json::to_string(&merged)?;

        sqlx::query(
            "UPDATE spreadsheets SET title = ?1, cells = ?2, updated_at = datetime('now') \
             WHERE id = ?3 AND user_id = ?4",
        )
        .bind(&new_title)
        .bind(&cells_json)
        .bind(&sheet_id)
        .bind(req.traveler_id)
        .execute(ctx.pool().await)
        .await?;

        Ok(ActionOutcome::ok(
            "calc_write",
            json!({
                "sheet_id": sheet_id,
                "title": new_title,
                "count": incoming.len(),
            }),
        ))
    }
}

/* ── calc_read ──────────────────────────────────────────────── */

pub struct CalcRead;

#[async_trait]
impl Tool for CalcRead {
    fn name(&self) -> &str { "calc_read" }
    fn aliases(&self) -> &[&str] { &["read_sheet", "read_spreadsheet", "get_sheet", "get_spreadsheet"] }
    fn step_label(&self) -> &str { "Reading spreadsheet…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `calc_read` — Read a spreadsheet's cells. params: `{ sheet_id?: string }` — returns `sheet_id`, `title`, `rows`, `cols`, `cells` (map of A1-style ref → value) and `updated_at`. Without `sheet_id` reads the most recently used spreadsheet.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("spreadsheet");
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Read \"{title}\" ({n} cells)")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let sheet_id = match req.params.param_str("sheet_id") {
            Some(id) if !id.trim().is_empty() => id,
            _ => last_sheet_id(ctx.pool().await, req.traveler_id)
                .await?
                .ok_or_else(|| {
                    AppError::BadRequest("No spreadsheet yet — call calc_create first".into())
                })?,
        };

        let row = sqlx::query_as::<_, (String, String, i64, i64, String)>(
            "SELECT title, cells, rows, cols, updated_at FROM spreadsheets \
             WHERE id = ?1 AND user_id = ?2",
        )
        .bind(&sheet_id)
        .bind(req.traveler_id)
        .fetch_optional(ctx.pool().await)
        .await?
        .ok_or_else(|| AppError::NotFound("Spreadsheet not found".into()))?;
        let (title, cells_json, rows, cols, updated_at) = row;
        let cells = parse_cells(&cells_json);
        let cell_map: Map<String, Value> = cells
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();

        Ok(ActionOutcome::ok(
            "calc_read",
            json!({
                "sheet_id": sheet_id,
                "title": title,
                "rows": rows,
                "cols": cols,
                "cells": cell_map,
                "count": cells.len(),
                "updated_at": updated_at,
                "hint": "To change cells use calc_write {cells:{A1:\"...\"}}; to compute from this data do the math yourself and write the result with calc_write, or add a formula like =SUM(B1:B10) which evaluates live in the Calc window.",
            }),
        ))
    }
}

/* ── calc_list ──────────────────────────────────────────────── */

pub struct CalcList;

#[async_trait]
impl Tool for CalcList {
    fn name(&self) -> &str { "calc_list" }
    fn aliases(&self) -> &[&str] { &["list_sheets", "list_spreadsheets", "sheets", "spreadsheets"] }
    fn step_label(&self) -> &str { "Listing spreadsheets…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `calc_list` — List the user's spreadsheets. params: `{}` — returns `sheet_id`, `title`, `rows`, `cols`, `updated_at` per spreadsheet.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Found {n} spreadsheets")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let rows = sqlx::query_as::<_, (String, String, i64, i64, String)>(
            "SELECT id, title, rows, cols, updated_at FROM spreadsheets \
             WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT 100",
        )
        .bind(req.traveler_id)
        .fetch_all(ctx.pool().await)
        .await?;

        let sheets: Vec<Value> = rows.iter().map(sheet_summary_json).collect();
        Ok(ActionOutcome::ok(
            "calc_list",
            json!({ "spreadsheets": sheets, "count": sheets.len() }),
        ))
    }
}

/* ── calc_delete ────────────────────────────────────────────── */

pub struct CalcDelete;

#[async_trait]
impl Tool for CalcDelete {
    fn name(&self) -> &str { "calc_delete" }
    fn aliases(&self) -> &[&str] { &["delete_sheet", "delete_spreadsheet", "remove_sheet"] }
    fn step_label(&self) -> &str { "Deleting spreadsheet…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `calc_delete` — Delete a spreadsheet. params: `{ sheet_id: string }`")
    }
    fn humanize(&self, _r: &str, _d: &Value) -> String {
        "Spreadsheet deleted".into()
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let sheet_id = req.params.require_str("sheet_id")?;
        let result = sqlx::query("DELETE FROM spreadsheets WHERE id = ?1 AND user_id = ?2")
            .bind(&sheet_id)
            .bind(req.traveler_id)
            .execute(ctx.pool().await)
            .await?;
        if result.rows_affected() == 0 {
            return Ok(ActionOutcome::error("calc_delete", "Spreadsheet not found"));
        }
        Ok(ActionOutcome::ok("calc_delete", json!({ "sheet_id": sheet_id })))
    }
}
