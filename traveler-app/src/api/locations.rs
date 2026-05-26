use axum::extract::{Path, State, Extension, Query};
use axum::Json;
use serde::{Serialize, Deserialize};

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::{Location, LocationSubmit, Traveler};

#[derive(Serialize)]
pub struct LocationResponse {
    pub success: bool,
    pub data: Location,
}

#[derive(Serialize)]
pub struct LocationListResponse {
    pub success: bool,
    pub data: Vec<Location>,
    pub count: usize,
}

#[derive(Serialize)]
pub struct RouteResponse {
    pub success: bool,
    pub data: Vec<RoutePoint>,
}

#[derive(Serialize)]
pub struct RoutePoint {
    pub lat: f64,
    pub lon: f64,
    pub timestamp: Option<String>,
    pub speed: Option<f64>,
}

#[derive(Deserialize)]
pub struct LocationQuery {
    pub trip_id: Option<String>,
    pub since: Option<String>,
    pub limit: Option<i64>,
}

pub async fn submit(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(req): Json<LocationSubmit>,
) -> Result<Json<LocationResponse>, AppError> {
    let location = Location::new(
        traveler.id.clone(),
        req.trip_id,
        req.latitude,
        req.longitude,
        req.altitude,
        req.speed,
        req.heading,
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
    .execute(&state.pool)
    .await
    .map_err(AppError::Database)?;

    Ok(Json(LocationResponse {
        success: true,
        data: location,
    }))
}

pub async fn list(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Query(params): Query<LocationQuery>,
) -> Result<Json<LocationListResponse>, AppError> {
    let limit = params.limit.unwrap_or(100);

    let locations = if let Some(trip_id) = &params.trip_id {
        sqlx::query_as::<_, Location>(
            "SELECT * FROM locations WHERE traveler_id = ?1 AND trip_id = ?2 \
             ORDER BY timestamp DESC LIMIT ?3",
        )
        .bind(&traveler.id)
        .bind(trip_id)
        .bind(limit)
        .fetch_all(&state.pool)
        .await
        .map_err(AppError::Database)?
    } else if let Some(since) = &params.since {
        sqlx::query_as::<_, Location>(
            "SELECT * FROM locations WHERE traveler_id = ?1 AND timestamp >= ?2 \
             ORDER BY timestamp DESC LIMIT ?3",
        )
        .bind(&traveler.id)
        .bind(since)
        .bind(limit)
        .fetch_all(&state.pool)
        .await
        .map_err(AppError::Database)?
    } else {
        sqlx::query_as::<_, Location>(
            "SELECT * FROM locations WHERE traveler_id = ?1 \
             ORDER BY timestamp DESC LIMIT ?2",
        )
        .bind(&traveler.id)
        .bind(limit)
        .fetch_all(&state.pool)
        .await
        .map_err(AppError::Database)?
    };

    let count = locations.len();
    Ok(Json(LocationListResponse {
        success: true,
        data: locations,
        count,
    }))
}

pub async fn route(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Path(id): Path<String>,
) -> Result<Json<RouteResponse>, AppError> {
    let locations = sqlx::query_as::<_, Location>(
        "SELECT * FROM locations WHERE trip_id = ?1 AND traveler_id = ?2 \
         ORDER BY timestamp ASC",
    )
    .bind(&id)
    .bind(&traveler.id)
    .fetch_all(&state.pool)
    .await
    .map_err(AppError::Database)?;

    let route: Vec<RoutePoint> = locations
        .into_iter()
        .map(|loc| RoutePoint {
            lat: loc.latitude,
            lon: loc.longitude,
            timestamp: loc.timestamp,
            speed: loc.speed,
        })
        .collect();

    Ok(Json(RouteResponse {
        success: true,
        data: route,
    }))
}
