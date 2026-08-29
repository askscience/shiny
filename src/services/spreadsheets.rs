//! Spreadsheet storage for the calc plugin — core-owned table (`spreadsheets`),
//! cells stored as a JSON map "A1" -> "value". Same pattern as `documents`:
//! core owns the schema + routes, the plugin owns the domain logic (its tools
//! write through the same table).

use serde::Serialize;
use sqlx::SqlitePool;
use std::collections::BTreeMap;

use crate::errors::AppError;

/// Hard caps so a spreadsheet payload never explodes.
pub const MAX_CELLS: usize = 5000;
pub const MAX_CELL_VALUE_LEN: usize = 10_000;
const MAX_TITLE_LEN: usize = 120;

#[derive(Debug, Clone, Serialize)]
pub struct SpreadsheetSummary {
    pub id: String,
    pub title: String,
    pub rows: i64,
    pub cols: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Spreadsheet {
    pub id: String,
    pub title: String,
    pub rows: i64,
    pub cols: i64,
    pub cells: BTreeMap<String, String>,
    pub updated_at: String,
}

pub async fn list_spreadsheets(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<SpreadsheetSummary>, AppError> {
    let rows = sqlx::query_as::<_, (String, String, i64, i64, String)>(
        "SELECT id, title, rows, cols, updated_at FROM spreadsheets \
         WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT 200",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(id, title, rows, cols, updated_at)| SpreadsheetSummary {
            id,
            title,
            rows,
            cols,
            updated_at,
        })
        .collect())
}

pub async fn create_spreadsheet(
    pool: &SqlitePool,
    user_id: &str,
    title: &str,
    rows: Option<i64>,
    cols: Option<i64>,
) -> Result<Spreadsheet, AppError> {
    let id = uuid::Uuid::new_v4().to_string();
    let title = clean_title(title);
    let rows = clamp_dim(rows.unwrap_or(100), 1, 500);
    let cols = clamp_dim(cols.unwrap_or(26), 1, 52);

    sqlx::query(
        "INSERT INTO spreadsheets (id, user_id, title, cells, rows, cols, created_at, updated_at) \
         VALUES (?1, ?2, ?3, '{}', ?4, ?5, datetime('now'), datetime('now'))",
    )
    .bind(&id)
    .bind(user_id)
    .bind(&title)
    .bind(rows)
    .bind(cols)
    .execute(pool)
    .await?;

    Ok(Spreadsheet {
        id,
        title,
        rows,
        cols,
        cells: BTreeMap::new(),
        updated_at: "now".into(),
    })
}

pub async fn load_spreadsheet(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
) -> Result<Spreadsheet, AppError> {
    let row = sqlx::query_as::<_, (String, String, i64, i64, String)>(
        "SELECT title, cells, rows, cols, updated_at FROM spreadsheets \
         WHERE id = ?1 AND user_id = ?2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Spreadsheet not found".into()))?;

    let (title, cells_json, rows, cols, updated_at) = row;
    let cells = parse_cells(&cells_json)?;

    Ok(Spreadsheet {
        id: id.to_string(),
        title,
        rows,
        cols,
        cells,
        updated_at,
    })
}

/// Save a spreadsheet: replace the full cell map + optional title.
/// Rejects invalid cell refs, oversized values and runaway cell counts so
/// garbage never lands in the table.
pub async fn save_spreadsheet(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
    title: Option<&str>,
    cells: &BTreeMap<String, String>,
) -> Result<(), AppError> {
    let current: (String, String) = sqlx::query_as(
        "SELECT title, cells FROM spreadsheets WHERE id = ?1 AND user_id = ?2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Spreadsheet not found".into()))?;

    let new_title = match title {
        Some(t) => clean_title(t),
        None => current.0,
    };
    let merged = merge_cells(&current.1, cells)?;

    let result = sqlx::query(
        "UPDATE spreadsheets SET title = ?1, cells = ?2, updated_at = datetime('now') \
         WHERE id = ?3 AND user_id = ?4",
    )
    .bind(&new_title)
    .bind(&merged)
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Spreadsheet not found".into()));
    }
    Ok(())
}

pub async fn delete_spreadsheet(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
) -> Result<bool, AppError> {
    let result = sqlx::query("DELETE FROM spreadsheets WHERE id = ?1 AND user_id = ?2")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/* ── helpers ────────────────────────────────────────────────── */

/// Parse the stored cells JSON into a map. Malformed JSON is treated as empty
/// (defensive — the table is written by us, but never trust stored data).
fn parse_cells(json: &str) -> Result<BTreeMap<String, String>, AppError> {
    if json.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    Ok(serde_json::from_str::<BTreeMap<String, String>>(json)
        .unwrap_or_default()
        .into_iter()
        .take(MAX_CELLS)
        .collect())
}

/// Merge incoming cells into the stored map: upsert values, drop cells whose
/// value is empty (a real spreadsheet's "clear cell"), validate refs.
fn merge_cells(
    stored_json: &str,
    incoming: &BTreeMap<String, String>,
) -> Result<String, AppError> {
    let mut cells = parse_cells(stored_json)?;

    for (raw_ref, value) in incoming {
        let cell_ref = raw_ref.trim().to_uppercase();
        if !is_valid_cell_ref(&cell_ref) {
            return Err(AppError::BadRequest(format!(
                "Invalid cell reference \"{raw_ref}\" — expected something like A1 or BC42"
            )));
        }
        if value.chars().count() > MAX_CELL_VALUE_LEN {
            return Err(AppError::BadRequest(format!(
                "Value in {cell_ref} is too long (max {MAX_CELL_VALUE_LEN} chars)"
            )));
        }
        if value.trim().is_empty() {
            cells.remove(&cell_ref);
        } else {
            cells.insert(cell_ref, value.clone());
        }
    }

    if cells.len() > MAX_CELLS {
        return Err(AppError::BadRequest(format!(
            "Too many cells (max {MAX_CELLS})"
        )));
    }
    Ok(serde_json::to_string(&cells)?)
}

/// A1-style reference: 1–2 uppercase letters followed by a positive number.
pub fn is_valid_cell_ref(cell_ref: &str) -> bool {
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

pub fn clean_title(title: &str) -> String {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        "Untitled".into()
    } else {
        trimmed.chars().take(MAX_TITLE_LEN).collect()
    }
}

fn clamp_dim(v: i64, min: i64, max: i64) -> i64 {
    v.clamp(min, max)
}
