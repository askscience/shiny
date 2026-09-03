//! Calendar plugin tools: create/list/get/update/delete events.
//!
//! Events live in the plugin-owned `calendar_events` table; the tools write
//! through the plugin's own SQLite pool (`ctx.pool()`), exactly like the
//! traveler/calc plugins.

use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::SqlitePool;

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::date::{current_month, month_bounds, normalize_date, normalize_time};

const EVENT_COLS: &str = "id, title, date, start_time, end_time, description, location, all_day";
type EventRow = (String, String, String, String, String, String, String, i64);

fn event_json(r: &EventRow) -> Value {
    json!({
        "event_id": r.0,
        "title": r.1,
        "date": r.2,
        "start_time": r.3,
        "end_time": r.4,
        "description": r.5,
        "location": r.6,
        "all_day": r.7 != 0,
    })
}

/// Resolve an `event_id` param to a real event id. Accepts the UUID, or the
/// event's exact title (case-insensitive). Empty/missing → `None`.
async fn resolve_event_id(
    pool: &SqlitePool,
    user_id: &str,
    id_or_title: Option<String>,
) -> Result<Option<String>, AppError> {
    let Some(v) = id_or_title.filter(|s| !s.trim().is_empty()) else {
        return Ok(None);
    };
    let v = v.trim();

    let by_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM calendar_events WHERE id = ?1 AND user_id = ?2",
    )
    .bind(v)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    if by_id.is_some() {
        return Ok(by_id);
    }

    let by_title: Option<String> = sqlx::query_scalar(
        "SELECT id FROM calendar_events WHERE lower(title) = lower(?1) AND user_id = ?2 \
         ORDER BY date DESC, updated_at DESC LIMIT 1",
    )
    .bind(v)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(by_title)
}

fn require_title(req: &ToolRequest<'_>) -> Result<String, AppError> {
    req.params
        .param_str("title")
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| AppError::BadRequest("title required".into()))
}

/* ── calendar_create ────────────────────────────────────────── */

pub struct CalendarCreate;

#[async_trait]
impl Tool for CalendarCreate {
    fn name(&self) -> &str { "calendar_create" }
    fn aliases(&self) -> &[&str] { &["create_event", "add_event", "schedule", "new_event"] }
    fn step_label(&self) -> &str { "Scheduling event…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `calendar_create` — Schedule an event. params: `{ title: string, date: string, start_time?: string, end_time?: string, all_day?: boolean, description?: string, location?: string }` — `title` and `date` required (date = \"YYYY-MM-DD\", \"today\", \"tomorrow\" or \"yesterday\"; `time` is an alias for `start_time`). Returns the new `event_id`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("Event");
        let date = data.get("date").and_then(|v| v.as_str()).unwrap_or("");
        format!("Scheduled \"{title}\" on {date}")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let title = require_title(&req)?;
        let date_raw = req
            .params
            .param_str("date")
            .or_else(|| req.params.param_str("day"))
            .unwrap_or_default();
        let date = normalize_date(&date_raw).map_err(AppError::BadRequest)?;
        let start_raw = req
            .params
            .param_str("start_time")
            .or_else(|| req.params.param_str("time"))
            .unwrap_or_default();
        let start_time = normalize_time(&start_raw).map_err(AppError::BadRequest)?;
        let end_time = normalize_time(&req.params.param_str("end_time").unwrap_or_default())
            .map_err(AppError::BadRequest)?;
        let all_day = req.params.param_bool("all_day").unwrap_or(false);
        let description = req.params.param_str("description").unwrap_or_default();
        let location = req.params.param_str("location").unwrap_or_default();

        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO calendar_events \
             (id, user_id, title, description, location, date, start_time, end_time, all_day, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'), datetime('now'))",
        )
        .bind(&id)
        .bind(req.traveler_id)
        .bind(&title)
        .bind(&description)
        .bind(&location)
        .bind(&date)
        .bind(&start_time)
        .bind(&end_time)
        .bind(if all_day { 1i64 } else { 0i64 })
        .execute(ctx.pool().await)
        .await?;

        Ok(ActionOutcome::ok(
            "calendar_create",
            json!({
                "event_id": id,
                "title": title,
                "date": date,
                "start_time": start_time,
                "end_time": end_time,
                "all_day": all_day,
                "description": description,
                "location": location,
            }),
        ))
    }
}

/* ── calendar_list ──────────────────────────────────────────── */

pub struct CalendarList;

#[async_trait]
impl Tool for CalendarList {
    fn name(&self) -> &str { "calendar_list" }
    fn aliases(&self) -> &[&str] { &["list_events", "events", "get_events", "calendar"] }
    fn step_label(&self) -> &str { "Listing events…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `calendar_list` — List events in a range. params: `{ month?: string, from?: string, to?: string, limit?: number }` — `month` = \"YYYY-MM\"; defaults to the current month. Returns `events` (each with `event_id`, `title`, `date`, `start_time`, `end_time`, `description`, `location`, `all_day`) and `count`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Found {n} events")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let (from, to) = match (req.params.param_str("from"), req.params.param_str("to")) {
            (Some(f), Some(t)) => (
                normalize_date(&f).map_err(AppError::BadRequest)?,
                normalize_date(&t).map_err(AppError::BadRequest)?,
            ),
            _ => {
                let month = req.params.param_str("month").unwrap_or_else(current_month);
                let (f, t) = month_bounds(&month).map_err(AppError::BadRequest)?;
                (f, t)
            }
        };

        let limit = req.params.param_u32("limit").unwrap_or(200).clamp(1, 500) as i64;
        let rows = sqlx::query_as::<_, EventRow>(&format!(
            "SELECT {EVENT_COLS} FROM calendar_events \
             WHERE user_id = ?1 AND date BETWEEN ?2 AND ?3 \
             ORDER BY date ASC, start_time ASC, title ASC LIMIT ?4"
        ))
        .bind(req.traveler_id)
        .bind(&from)
        .bind(&to)
        .bind(limit)
        .fetch_all(ctx.pool().await)
        .await?;

        let events: Vec<Value> = rows.iter().map(event_json).collect();
        Ok(ActionOutcome::ok(
            "calendar_list",
            json!({ "events": events, "count": events.len(), "from": from, "to": to }),
        ))
    }
}

/* ── calendar_get ───────────────────────────────────────────── */

pub struct CalendarGet;

#[async_trait]
impl Tool for CalendarGet {
    fn name(&self) -> &str { "calendar_get" }
    fn aliases(&self) -> &[&str] { &["get_day", "events_on", "day_events", "calendar_day"] }
    fn step_label(&self) -> &str { "Reading the day's events…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `calendar_get` — List one day's events. params: `{ date: string }` — date = \"YYYY-MM-DD\" (or \"today\"/\"tomorrow\"). Returns that day's `events` and `count`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        let date = data.get("date").and_then(|v| v.as_str()).unwrap_or("");
        format!("Found {n} events on {date}")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let date_raw = req.params.param_str("date").unwrap_or_default();
        let date = normalize_date(&date_raw).map_err(AppError::BadRequest)?;

        let rows = sqlx::query_as::<_, EventRow>(&format!(
            "SELECT {EVENT_COLS} FROM calendar_events WHERE user_id = ?1 AND date = ?2 \
             ORDER BY start_time ASC, title ASC"
        ))
        .bind(req.traveler_id)
        .bind(&date)
        .fetch_all(ctx.pool().await)
        .await?;

        let events: Vec<Value> = rows.iter().map(event_json).collect();
        Ok(ActionOutcome::ok(
            "calendar_get",
            json!({ "date": date, "events": events, "count": events.len() }),
        ))
    }
}

/* ── calendar_update ────────────────────────────────────────── */

pub struct CalendarUpdate;

#[async_trait]
impl Tool for CalendarUpdate {
    fn name(&self) -> &str { "calendar_update" }
    fn aliases(&self) -> &[&str] { &["update_event", "edit_event", "reschedule", "move_event"] }
    fn step_label(&self) -> &str { "Updating event…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `calendar_update` — Change an event. params: `{ event_id: string, title?, date?, start_time?, end_time?, all_day?, description?, location? }` — only the fields you pass are changed. `time` is an alias for `start_time`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("Event");
        format!("Updated \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let event_id = resolve_event_id(ctx.pool().await, req.traveler_id, req.params.param_str("event_id"))
            .await?
            .ok_or_else(|| AppError::NotFound("Event not found".into()))?;

        let row = sqlx::query_as::<_, EventRow>(&format!(
            "SELECT {EVENT_COLS} FROM calendar_events WHERE id = ?1 AND user_id = ?2"
        ))
        .bind(&event_id)
        .bind(req.traveler_id)
        .fetch_optional(ctx.pool().await)
        .await?
        .ok_or_else(|| AppError::NotFound("Event not found".into()))?;

        let title = match req.params.param_str("title").map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
            Some(t) => t,
            None => row.1.clone(),
        };
        let date = match req.params.param_str("date").or_else(|| req.params.param_str("day")) {
            Some(d) if !d.trim().is_empty() => normalize_date(&d).map_err(AppError::BadRequest)?,
            _ => row.2.clone(),
        };
        let start_raw = req
            .params
            .param_str("start_time")
            .or_else(|| req.params.param_str("time"));
        let start_time = match start_raw {
            Some(s) => normalize_time(&s).map_err(AppError::BadRequest)?,
            None => row.3.clone(),
        };
        let end_time = match req.params.param_str("end_time") {
            Some(s) => normalize_time(&s).map_err(AppError::BadRequest)?,
            None => row.4.clone(),
        };
        let description = req.params.param_str("description").unwrap_or_else(|| row.5.clone());
        let location = req.params.param_str("location").unwrap_or_else(|| row.6.clone());
        let all_day = req.params.param_bool("all_day").unwrap_or(row.7 != 0);

        sqlx::query(
            "UPDATE calendar_events SET title = ?1, date = ?2, start_time = ?3, end_time = ?4, \
             description = ?5, location = ?6, all_day = ?7, updated_at = datetime('now') \
             WHERE id = ?8 AND user_id = ?9",
        )
        .bind(&title)
        .bind(&date)
        .bind(&start_time)
        .bind(&end_time)
        .bind(&description)
        .bind(&location)
        .bind(if all_day { 1i64 } else { 0i64 })
        .bind(&event_id)
        .bind(req.traveler_id)
        .execute(ctx.pool().await)
        .await?;

        Ok(ActionOutcome::ok(
            "calendar_update",
            json!({
                "event_id": event_id,
                "title": title,
                "date": date,
                "start_time": start_time,
                "end_time": end_time,
                "all_day": all_day,
                "description": description,
                "location": location,
            }),
        ))
    }
}

/* ── calendar_delete ────────────────────────────────────────── */

pub struct CalendarDelete;

#[async_trait]
impl Tool for CalendarDelete {
    fn name(&self) -> &str { "calendar_delete" }
    fn aliases(&self) -> &[&str] { &["delete_event", "remove_event", "cancel_event"] }
    fn step_label(&self) -> &str { "Deleting event…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `calendar_delete` — Permanently remove an event. params: `{ event_id: string, confirm: true }` — requires `confirm:true`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("Event");
        format!("Deleted \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let event_id = resolve_event_id(ctx.pool().await, req.traveler_id, req.params.param_str("event_id"))
            .await?
            .ok_or_else(|| AppError::NotFound("Event not found".into()))?;

        if !req.params.param_bool("confirm").unwrap_or(false) {
            return Ok(ActionOutcome::error(
                "calendar_delete",
                "refusing: deleting an event is permanent. Only call calendar_delete with \
                 {\"confirm\":true} when the user explicitly asks to cancel or remove the event.",
            ));
        }

        // Fetch the title first so the humanized step can name it.
        let title: Option<String> = sqlx::query_scalar(
            "SELECT title FROM calendar_events WHERE id = ?1 AND user_id = ?2",
        )
        .bind(&event_id)
        .bind(req.traveler_id)
        .fetch_optional(ctx.pool().await)
        .await?;

        let result = sqlx::query("DELETE FROM calendar_events WHERE id = ?1 AND user_id = ?2")
            .bind(&event_id)
            .bind(req.traveler_id)
            .execute(ctx.pool().await)
            .await?;
        if result.rows_affected() == 0 {
            return Ok(ActionOutcome::error("calendar_delete", "Event not found"));
        }
        Ok(ActionOutcome::ok(
            "calendar_delete",
            json!({ "event_id": event_id, "title": title.unwrap_or_default() }),
        ))
    }
}
