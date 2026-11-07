//! Trip lifecycle tools: create, list, get, active, start, end, stats.

use async_trait::async_trait;
use chrono::{NaiveDateTime, Utc};
use serde_json::json;
use sqlx::SqlitePool;

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::diary;
use crate::models::{Location, Trip};
use crate::osm::haversine_km;

/* ── shared helpers ─────────────────────────────────────────── */

pub(crate) async fn fetch_trips(pool: &SqlitePool, traveler_id: &str) -> Result<Vec<Trip>, AppError> {
    Ok(sqlx::query_as::<_, Trip>(
        "SELECT * FROM trips WHERE traveler_id = ?1 ORDER BY created_at DESC",
    )
    .bind(traveler_id)
    .fetch_all(pool)
    .await?)
}

pub(crate) async fn fetch_trip(pool: &SqlitePool, traveler_id: &str, id: &str) -> Result<Trip, AppError> {
    sqlx::query_as::<_, Trip>("SELECT * FROM trips WHERE id = ?1 AND traveler_id = ?2")
        .bind(id)
        .bind(traveler_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Trip not found".into()))
}

pub(crate) async fn fetch_active_trip(pool: &SqlitePool, traveler_id: &str) -> Result<Option<Trip>, AppError> {
    Ok(sqlx::query_as::<_, Trip>(
        "SELECT * FROM trips WHERE traveler_id = ?1 AND status = 'active' LIMIT 1",
    )
    .bind(traveler_id)
    .fetch_optional(pool)
    .await?)
}

async fn start_trip_internal(pool: &SqlitePool, traveler_id: &str, id: &str) -> Result<Trip, AppError> {
    let trip = fetch_trip(pool, traveler_id, id).await?;
    if trip.status == "active" {
        return Err(AppError::BadRequest("Trip is already active".into()));
    }
    let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    sqlx::query("UPDATE trips SET status = 'active', start_time = ?1 WHERE id = ?2")
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
    fetch_trip(pool, traveler_id, id).await
}

/* ── create_trip ────────────────────────────────────────────── */

pub struct CreateTrip;

#[async_trait]
impl Tool for CreateTrip {
    fn name(&self) -> &str { "create_trip" }
    fn aliases(&self) -> &[&str] { &["new_trip"] }
    fn step_label(&self) -> &str { "Creating trip…" }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let name = req.params.require_str("name")?;
        let description = req.params.param_str("description");

        let trip = Trip::new(req.traveler_id.to_string(), name, description);
        sqlx::query(
            "INSERT INTO trips (id, traveler_id, name, description, status, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
        )
        .bind(&trip.id)
        .bind(&trip.traveler_id)
        .bind(&trip.name)
        .bind(&trip.description)
        .bind(&trip.status)
        .execute(ctx.pool().await)
        .await?;

        let had_active = fetch_active_trip(ctx.pool().await, req.traveler_id).await?.is_some();
        let trip = if had_active {
            fetch_trip(ctx.pool().await, req.traveler_id, &trip.id).await?
        } else {
            start_trip_internal(ctx.pool().await, req.traveler_id, &trip.id).await?
        };

        Ok(ActionOutcome::ok(
            "create_trip",
            json!({ "trip": trip, "auto_started": !had_active }),
        ))
    }
}

/* ── list_trips ─────────────────────────────────────────────── */

pub struct ListTrips;

#[async_trait]
impl Tool for ListTrips {
    fn name(&self) -> &str { "list_trips" }
    fn step_label(&self) -> &str { "Listing trips…" }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let trips = fetch_trips(ctx.pool().await, req.traveler_id).await?;
        Ok(ActionOutcome::ok("list_trips", json!({ "trips": trips })))
    }
}

/* ── get_trip ───────────────────────────────────────────────── */

pub struct GetTrip;

#[async_trait]
impl Tool for GetTrip {
    fn name(&self) -> &str { "get_trip" }
    fn step_label(&self) -> &str { "Loading trip…" }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let id = req.params.require_str("trip_id")?;
        let trip = fetch_trip(ctx.pool().await, req.traveler_id, &id).await?;
        Ok(ActionOutcome::ok("get_trip", json!({ "trip": trip })))
    }
}

/* ── get_active_trip ────────────────────────────────────────── */

pub struct GetActiveTrip;

#[async_trait]
impl Tool for GetActiveTrip {
    fn name(&self) -> &str { "get_active_trip" }
    fn step_label(&self) -> &str { "Checking active trip…" }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let trip = fetch_active_trip(ctx.pool().await, req.traveler_id).await?;
        Ok(ActionOutcome::ok("get_active_trip", json!({ "trip": trip })))
    }
}

/* ── start_trip ─────────────────────────────────────────────── */

pub struct StartTrip;

#[async_trait]
impl Tool for StartTrip {
    fn name(&self) -> &str { "start_trip" }
    fn step_label(&self) -> &str { "Starting trip…" }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let id = req.params.require_str("trip_id")?;
        let trip = start_trip_internal(ctx.pool().await, req.traveler_id, &id).await?;
        Ok(ActionOutcome::ok("start_trip", json!({ "trip": trip })))
    }
}

/* ── end_trip ───────────────────────────────────────────────── */

pub struct EndTrip;

#[async_trait]
impl Tool for EndTrip {
    fn name(&self) -> &str { "end_trip" }
    fn step_label(&self) -> &str { "Ending trip…" }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let id = req.params.require_str("trip_id")?;
        let trip = fetch_trip(ctx.pool().await, req.traveler_id, &id).await?;
        if trip.status != "active" {
            return Err(AppError::BadRequest("Trip is not active".into()));
        }
        let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        sqlx::query("UPDATE trips SET status = 'completed', end_time = ?1 WHERE id = ?2")
            .bind(&now)
            .bind(&id)
            .execute(ctx.pool().await)
            .await?;

        // Best-effort diary for today (mirrors core's end-of-trip diarize).
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let _ = diary::generate_for_date(ctx, req.traveler_id, &today).await;

        let trip = fetch_trip(ctx.pool().await, req.traveler_id, &id).await?;
        Ok(ActionOutcome::ok("end_trip", json!({ "trip": trip })))
    }
}

/* ── trip_stats ─────────────────────────────────────────────── */

pub struct TripStats;

#[async_trait]
impl Tool for TripStats {
    fn name(&self) -> &str { "trip_stats" }
    fn step_label(&self) -> &str { "Computing trip stats…" }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let id = req.params.require_str("trip_id")?;
        let trip = fetch_trip(ctx.pool().await, req.traveler_id, &id).await?;
        let locations = sqlx::query_as::<_, Location>(
            "SELECT * FROM locations WHERE trip_id = ?1 ORDER BY timestamp ASC",
        )
        .bind(&id)
        .fetch_all(ctx.pool().await)
        .await?;

        Ok(ActionOutcome::ok(
            "trip_stats",
            json!({ "stats": compute_trip_stats(&trip, &locations, Utc::now().naive_utc()) }),
        ))
    }
}

/// Parse a stored timestamp: SQLite's `YYYY-MM-DD HH:MM:SS` (what the plugin
/// writes) or RFC 3339.
fn parse_ts(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M:%S")
        .ok()
        .or_else(|| chrono::DateTime::parse_from_rfc3339(s.trim()).ok().map(|d| d.naive_utc()))
}

fn point_json(loc: &Location) -> serde_json::Value {
    json!({ "lat": loc.latitude, "lon": loc.longitude, "timestamp": loc.timestamp })
}

/// Stats for one trip. Duration runs from the trip's `start_time` (else its
/// first GPS point) to its `end_time` (else `now` while it is active, else its
/// last GPS point); `null` when either end is unknown.
pub(crate) fn compute_trip_stats(
    trip: &Trip,
    locations: &[Location],
    now: NaiveDateTime,
) -> serde_json::Value {
    let total_distance: f64 = locations
        .windows(2)
        .map(|w| haversine_km(w[0].latitude, w[0].longitude, w[1].latitude, w[1].longitude))
        .sum();
    // Average over ALL recorded speeds (m/s → km/h).
    let speeds: Vec<f64> = locations.iter().filter_map(|l| l.speed).collect();
    let avg_speed = (!speeds.is_empty())
        .then(|| speeds.iter().sum::<f64>() / speeds.len() as f64 * 3.6);

    let first = locations.first();
    let last = locations.last();
    let start = trip
        .start_time
        .as_deref()
        .and_then(parse_ts)
        .or_else(|| first.and_then(|l| l.timestamp.as_deref()).and_then(parse_ts));
    let end = trip.end_time.as_deref().and_then(parse_ts).or_else(|| {
        if trip.status == "active" {
            Some(now)
        } else {
            last.and_then(|l| l.timestamp.as_deref()).and_then(parse_ts)
        }
    });
    let duration_hours = match (start, end) {
        (Some(s), Some(e)) if e >= s => Some((e - s).num_seconds() as f64 / 3600.0),
        _ => None,
    };

    json!({
        "total_distance_km": total_distance,
        "total_duration_hours": duration_hours,
        "point_count": locations.len() as i64,
        "avg_speed_kmh": avg_speed,
        "start_location": first.map(point_json),
        "end_location": last.map(point_json),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trip(status: &str, start: Option<&str>, end: Option<&str>) -> Trip {
        let mut t = Trip::new("me".into(), "t".into(), None);
        t.status = status.into();
        t.start_time = start.map(String::from);
        t.end_time = end.map(String::from);
        t
    }

    fn loc(lat: f64, lon: f64, ts: &str, speed: Option<f64>) -> Location {
        let mut l = Location::new("me".into(), None, lat, lon, None, speed, None, "gps".into());
        l.timestamp = Some(ts.into());
        l
    }

    fn now() -> NaiveDateTime {
        NaiveDateTime::parse_from_str("2026-01-01 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap()
    }

    #[test]
    fn stats_use_trip_times_and_first_last_points() {
        let t = trip("completed", Some("2026-01-01 08:00:00"), Some("2026-01-01 10:30:00"));
        let locs = vec![
            loc(45.0, 9.0, "2026-01-01 08:05:00", Some(10.0)),
            loc(45.1, 9.0, "2026-01-01 10:00:00", Some(20.0)),
        ];
        let s = compute_trip_stats(&t, &locs, now());
        assert_eq!(s["total_duration_hours"], 2.5);
        assert_eq!(s["start_location"]["lat"], 45.0);
        assert_eq!(s["end_location"]["lat"], 45.1);
        assert_eq!(s["point_count"], 2);
        assert!((s["avg_speed_kmh"].as_f64().unwrap() - 54.0).abs() < 1e-9);
        assert!(s["total_distance_km"].as_f64().unwrap() > 11.0);
    }

    #[test]
    fn stats_fall_back_to_points_and_now() {
        let locs = vec![
            loc(45.0, 9.0, "2026-01-01 09:00:00", None),
            loc(45.0, 9.1, "2026-01-01 10:00:00", None),
        ];
        // Ended trip without stored times: first → last point.
        let s = compute_trip_stats(&trip("completed", None, None), &locs, now());
        assert_eq!(s["total_duration_hours"], 1.0);
        // Active trip: start → now.
        let s = compute_trip_stats(&trip("active", Some("2026-01-01 11:00:00"), None), &locs, now());
        assert_eq!(s["total_duration_hours"], 1.0);
        // Nothing known: nulls, never fake zeros.
        let s = compute_trip_stats(&trip("planned", None, None), &[], now());
        assert!(s["total_duration_hours"].is_null());
        assert!(s["start_location"].is_null());
        assert!(s["avg_speed_kmh"].is_null());
    }
}
