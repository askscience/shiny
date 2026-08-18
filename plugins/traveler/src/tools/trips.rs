//! Trip lifecycle tools: create, list, get, active, start, end, stats.

use async_trait::async_trait;
use chrono::Utc;
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
        let _ = fetch_trip(ctx.pool().await, req.traveler_id, &id).await?;
        let locations = sqlx::query_as::<_, Location>(
            "SELECT * FROM locations WHERE trip_id = ?1 ORDER BY timestamp ASC",
        )
        .bind(&id)
        .fetch_all(ctx.pool().await)
        .await?;

        let mut total_distance = 0.0;
        let mut total_speed = 0.0;
        let mut speed_count = 0;
        for window in locations.windows(2) {
            total_distance += haversine_km(
                window[0].latitude,
                window[0].longitude,
                window[1].latitude,
                window[1].longitude,
            );
            if let Some(s) = window[0].speed {
                total_speed += s;
                speed_count += 1;
            }
        }
        let avg_speed = if speed_count > 0 {
            Some(total_speed / speed_count as f64 * 3.6)
        } else {
            None
        };

        Ok(ActionOutcome::ok(
            "trip_stats",
            json!({
                "stats": {
                    "total_distance_km": total_distance,
                    "total_duration_hours": 0.0,
                    "point_count": locations.len() as i64,
                    "avg_speed_kmh": avg_speed,
                    "start_location": null,
                    "end_location": null,
                }
            }),
        ))
    }
}
