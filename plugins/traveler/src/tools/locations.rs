//! GPS location tools: submit, list, trip route.

use async_trait::async_trait;
use serde_json::json;

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::models::Location;
use crate::tools::trips::fetch_trip;

/* ── submit_location ────────────────────────────────────────── */

pub struct SubmitLocation;

#[async_trait]
impl Tool for SubmitLocation {
    fn name(&self) -> &str { "submit_location" }
    fn step_label(&self) -> &str { "Saving location…" }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let lat = req
            .params
            .param_f64("latitude")
            .ok_or_else(|| AppError::BadRequest("latitude required".into()))?;
        let lon = req
            .params
            .param_f64("longitude")
            .ok_or_else(|| AppError::BadRequest("longitude required".into()))?;
        let trip_id = req.params.param_str("trip_id");

        let location = Location::new(
            req.traveler_id.to_string(),
            trip_id,
            lat,
            lon,
            req.params.param_f64("altitude"),
            req.params.param_f64("speed"),
            req.params.param_f64("heading"),
            "manual".into(),
        );
        sqlx::query(
            "INSERT INTO locations (id, trip_id, traveler_id, latitude, longitude, altitude, speed, heading, timestamp, source) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'), ?9)",
        )
        .bind(&location.id)
        .bind(&location.trip_id)
        .bind(&location.traveler_id)
        .bind(location.latitude)
        .bind(location.longitude)
        .bind(location.altitude)
        .bind(location.speed)
        .bind(location.heading)
        .bind(&location.source)
        .execute(ctx.pool().await)
        .await?;

        Ok(ActionOutcome::ok("submit_location", json!({ "location": location })))
    }
}

/* ── list_locations ─────────────────────────────────────────── */

pub struct ListLocations;

#[async_trait]
impl Tool for ListLocations {
    fn name(&self) -> &str { "list_locations" }
    fn step_label(&self) -> &str { "Listing locations…" }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let limit = req.params.get("limit").and_then(|v| v.as_i64()).unwrap_or(50);
        let rows = if let Some(trip_id) = req.params.param_str("trip_id") {
            sqlx::query_as::<_, Location>(
                "SELECT * FROM locations WHERE traveler_id = ?1 AND trip_id = ?2 ORDER BY timestamp DESC LIMIT ?3",
            )
            .bind(req.traveler_id)
            .bind(trip_id)
            .bind(limit)
            .fetch_all(ctx.pool().await)
            .await?
        } else {
            sqlx::query_as::<_, Location>(
                "SELECT * FROM locations WHERE traveler_id = ?1 ORDER BY timestamp DESC LIMIT ?2",
            )
            .bind(req.traveler_id)
            .bind(limit)
            .fetch_all(ctx.pool().await)
            .await?
        };

        Ok(ActionOutcome::ok(
            "list_locations",
            json!({ "locations": rows, "count": rows.len() }),
        ))
    }
}

/* ── trip_route ─────────────────────────────────────────────── */

pub struct TripRoute;

#[async_trait]
impl Tool for TripRoute {
    fn name(&self) -> &str { "trip_route" }
    fn step_label(&self) -> &str { "Loading trip route…" }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let id = req.params.require_str("trip_id")?;
        let _ = fetch_trip(ctx.pool().await, req.traveler_id, &id).await?;
        let rows = sqlx::query_as::<_, Location>(
            "SELECT * FROM locations WHERE trip_id = ?1 ORDER BY timestamp ASC",
        )
        .bind(&id)
        .fetch_all(ctx.pool().await)
        .await?;
        let route: Vec<_> = rows
            .iter()
            .map(|l| json!({ "lat": l.latitude, "lon": l.longitude, "timestamp": l.timestamp, "speed": l.speed }))
            .collect();

        Ok(ActionOutcome::ok("trip_route", json!({ "route": route })))
    }
}
