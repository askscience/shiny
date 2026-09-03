//! Calendar plugin REST routes — served through the plugin's `RouteSpec`s.
//! The Calendar window lists/creates/updates/deletes events here; DB access
//! via the SDK's synchronous `ctx.db()`.

use std::sync::Arc;

use axum::extract::{FromRequest, FromRequestParts};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value as Json};

use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, path_params_from_request, user_id_from_request, RouteHandler};
use shiny_plugin_sdk::services::PluginCtx;

use crate::date::{current_month, month_bounds, normalize_date, normalize_time};

const EVENT_COLS_SQL: &str = "id, title, date, start_time, end_time, description, location, all_day";

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    Some(match tag {
        "event_list" => event_list(ctx),
        "event_create" => event_create(ctx),
        "event_update" => event_update(ctx),
        "event_delete" => event_delete(ctx),
        _ => return None,
    })
}

fn user_id(req: &axum::extract::Request) -> Result<String, AppError> {
    user_id_from_request(req)
        .ok_or_else(|| AppError::Unauthorized("not authenticated".into()))
}

fn ok(data: Json) -> Response {
    axum::Json(json!({ "success": true, "data": data })).into_response()
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

fn row_json(r: &[Value]) -> Json {
    json!({
        "event_id": as_text(&r[0]),
        "title": as_text(&r[1]),
        "date": as_text(&r[2]),
        "start_time": as_text(&r[3]),
        "end_time": as_text(&r[4]),
        "description": as_text(&r[5]),
        "location": as_text(&r[6]),
        "all_day": as_int(&r[7]) != 0,
    })
}

fn take_path(req: &axum::extract::Request) -> Result<String, AppError> {
    let mut params = path_params_from_request(req)
        .ok_or_else(|| AppError::BadRequest("no path parameter found".into()))?;
    if params.len() != 1 {
        return Err(AppError::BadRequest("expected exactly one path parameter".into()));
    }
    Ok(params.remove(0).1)
}

async fn take_query<T: DeserializeOwned + Send + 'static>(
    req: axum::extract::Request,
) -> Result<(T, axum::extract::Request), AppError> {
    let (mut parts, body) = req.into_parts();
    let query = axum::extract::Query::<T>::from_request_parts(&mut parts, &())
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid query: {e}")))?;
    Ok((query.0, axum::extract::Request::from_parts(parts, body)))
}

fn clean_text(s: Option<String>, max: usize) -> String {
    s.unwrap_or_default().trim().chars().take(max).collect()
}

/* ── GET /api/calendar/events ────────────────────────────────── */

fn event_list(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Q {
        month: Option<String>,
        from: Option<String>,
        to: Option<String>,
        limit: Option<i64>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (q, _req) = take_query::<Q>(req).await?;

            let (from, to) = match (q.from, q.to) {
                (Some(f), Some(t)) => (
                    normalize_date(&f).map_err(AppError::BadRequest)?,
                    normalize_date(&t).map_err(AppError::BadRequest)?,
                ),
                _ => {
                    let month = q.month.unwrap_or_else(current_month);
                    let (f, t) = month_bounds(&month).map_err(AppError::BadRequest)?;
                    (f, t)
                }
            };
            let limit = q.limit.unwrap_or(300).clamp(1, 1000);

            let rows = ctx.db().query(
                &format!(
                    "SELECT {EVENT_COLS_SQL} FROM calendar_events \
                     WHERE user_id = ?1 AND date BETWEEN ?2 AND ?3 \
                     ORDER BY date ASC, start_time ASC, title ASC LIMIT ?4"
                ),
                &[Value::text(&uid), Value::text(&from), Value::text(&to), Value::Int(limit)],
            )?;
            let events: Vec<Json> = rows.iter().map(|r| row_json(r)).collect();
            Ok(ok(json!({ "events": events, "count": events.len(), "from": from, "to": to })))
        }
    })
}

/* ── POST /api/calendar/events ───────────────────────────────── */

fn event_create(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Create {
        title: Option<String>,
        date: Option<String>,
        start_time: Option<String>,
        end_time: Option<String>,
        all_day: Option<bool>,
        description: Option<String>,
        location: Option<String>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let axum::Json(body) = axum::Json::<Create>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;

            let title = clean_text(body.title, 200);
            if title.is_empty() {
                return Err(AppError::BadRequest("title required".into()));
            }
            let date_raw = body.date.unwrap_or_default();
            let date = normalize_date(&date_raw).map_err(AppError::BadRequest)?;
            let start_time = normalize_time(&body.start_time.unwrap_or_default())
                .map_err(AppError::BadRequest)?;
            let end_time = normalize_time(&body.end_time.unwrap_or_default())
                .map_err(AppError::BadRequest)?;
            let all_day = body.all_day.unwrap_or(false);
            let description = clean_text(body.description, 2000);
            let location = clean_text(body.location, 500);

            let id = uuid::Uuid::new_v4().to_string();
            ctx.db().execute(
                "INSERT INTO calendar_events \
                 (id, user_id, title, description, location, date, start_time, end_time, all_day, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'), datetime('now'))",
                &[
                    Value::text(&id),
                    Value::text(&uid),
                    Value::text(&title),
                    Value::text(&description),
                    Value::text(&location),
                    Value::text(&date),
                    Value::text(&start_time),
                    Value::text(&end_time),
                    Value::Int(if all_day { 1 } else { 0 }),
                ],
            )?;

            Ok(ok(json!({
                "event_id": id, "title": title, "date": date,
                "start_time": start_time, "end_time": end_time,
                "all_day": all_day, "description": description, "location": location,
            })))
        }
    })
}

/* ── PUT /api/calendar/events/:id ────────────────────────────── */

fn event_update(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Update {
        title: Option<String>,
        date: Option<String>,
        start_time: Option<String>,
        end_time: Option<String>,
        all_day: Option<bool>,
        description: Option<String>,
        location: Option<String>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let axum::Json(body) = axum::Json::<Update>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;

            let current = ctx.db().query(
                &format!("SELECT {EVENT_COLS_SQL} FROM calendar_events WHERE id = ?1 AND user_id = ?2"),
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let cur = current.first().ok_or_else(|| AppError::NotFound("Event not found".into()))?;

            let title = match body.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
                Some(t) => t.chars().take(200).collect::<String>(),
                None => as_text(&cur[1]),
            };
            let date = match body.date {
                Some(d) if !d.trim().is_empty() => normalize_date(&d).map_err(AppError::BadRequest)?,
                _ => as_text(&cur[2]),
            };
            let start_time = match body.start_time {
                Some(s) => normalize_time(&s).map_err(AppError::BadRequest)?,
                None => as_text(&cur[3]),
            };
            let end_time = match body.end_time {
                Some(s) => normalize_time(&s).map_err(AppError::BadRequest)?,
                None => as_text(&cur[4]),
            };
            let description = match &body.description {
                Some(d) => d.trim().chars().take(2000).collect::<String>(),
                None => as_text(&cur[5]),
            };
            let location = match &body.location {
                Some(l) => l.trim().chars().take(500).collect::<String>(),
                None => as_text(&cur[6]),
            };
            let all_day = body.all_day.unwrap_or(as_int(&cur[7]) != 0);

            let changed = ctx.db().execute(
                "UPDATE calendar_events SET title = ?1, date = ?2, start_time = ?3, end_time = ?4, \
                 description = ?5, location = ?6, all_day = ?7, updated_at = datetime('now') \
                 WHERE id = ?8 AND user_id = ?9",
                &[
                    Value::text(&title),
                    Value::text(&date),
                    Value::text(&start_time),
                    Value::text(&end_time),
                    Value::text(&description),
                    Value::text(&location),
                    Value::Int(if all_day { 1 } else { 0 }),
                    Value::text(&id),
                    Value::text(&uid),
                ],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound("Event not found".into()));
            }

            Ok(ok(json!({
                "event_id": id, "title": title, "date": date,
                "start_time": start_time, "end_time": end_time,
                "all_day": all_day, "description": description, "location": location,
            })))
        }
    })
}

/* ── DELETE /api/calendar/events/:id ─────────────────────────── */

fn event_delete(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let changed = ctx.db().execute(
                "DELETE FROM calendar_events WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound("Event not found".into()));
            }
            Ok(ok(json!({ "event_id": id, "deleted": true })))
        }
    })
}
