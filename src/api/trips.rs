use axum::extract::{Path, State, Extension, Query};
use axum::Json;
use chrono::Utc;
use serde::Serialize;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::{CreateTripRequest, Trip, Traveler, UpdateTripRequest, TripStats, Location};

#[derive(Serialize)]
pub struct TripResponse {
    pub success: bool,
    pub data: Trip,
}

#[derive(Serialize)]
pub struct TripListResponse {
    pub success: bool,
    pub data: Vec<Trip>,
}

#[derive(Serialize)]
pub struct ActiveTripResponse {
    pub success: bool,
    pub data: Option<Trip>,
}

#[derive(Serialize)]
pub struct TripStatsResponse {
    pub success: bool,
    pub data: TripStats,
}

pub async fn create(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(req): Json<CreateTripRequest>,
) -> Result<Json<TripResponse>, AppError> {
    let trip = Trip::new(traveler.id.clone(), req.name, req.description);

    sqlx::query(
        "INSERT INTO trips (id, traveler_id, name, description, status, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
    )
    .bind(&trip.id)
    .bind(&trip.traveler_id)
    .bind(&trip.name)
    .bind(&trip.description)
    .bind(&trip.status)
    .execute(&state.pool)
    .await
    .map_err(AppError::Database)?;

    Ok(Json(TripResponse {
        success: true,
        data: trip,
    }))
}

pub async fn list(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
) -> Result<Json<TripListResponse>, AppError> {
    let trips = sqlx::query_as::<_, Trip>(
        "SELECT * FROM trips WHERE traveler_id = ?1 ORDER BY created_at DESC",
    )
    .bind(&traveler.id)
    .fetch_all(&state.pool)
    .await
    .map_err(AppError::Database)?;

    Ok(Json(TripListResponse {
        success: true,
        data: trips,
    }))
}

pub async fn get_active(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
) -> Result<Json<ActiveTripResponse>, AppError> {
    let trip = crate::services::agent_tools::fetch_active_trip(&state.pool, &traveler.id).await?;
    Ok(Json(ActiveTripResponse {
        success: true,
        data: trip,
    }))
}

pub async fn get_one(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Path(id): Path<String>,
) -> Result<Json<TripResponse>, AppError> {
    let trip = sqlx::query_as::<_, Trip>(
        "SELECT * FROM trips WHERE id = ?1 AND traveler_id = ?2",
    )
    .bind(&id)
    .bind(&traveler.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::Database)?
    .ok_or_else(|| AppError::NotFound("Trip not found".into()))?;

    Ok(Json(TripResponse {
        success: true,
        data: trip,
    }))
}

pub async fn update(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTripRequest>,
) -> Result<Json<TripResponse>, AppError> {
    let trip = sqlx::query_as::<_, Trip>(
        "SELECT * FROM trips WHERE id = ?1 AND traveler_id = ?2",
    )
    .bind(&id)
    .bind(&traveler.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::Database)?
    .ok_or_else(|| AppError::NotFound("Trip not found".into()))?;

    let name = req.name.unwrap_or(trip.name);
    let description = req.description.or(trip.description);
    let status = req.status.unwrap_or(trip.status);

    sqlx::query(
        "UPDATE trips SET name = ?1, description = ?2, status = ?3 WHERE id = ?4",
    )
    .bind(&name)
    .bind(&description)
    .bind(&status)
    .bind(&id)
    .execute(&state.pool)
    .await
    .map_err(AppError::Database)?;

    let updated = sqlx::query_as::<_, Trip>(
        "SELECT * FROM trips WHERE id = ?1",
    )
    .bind(&id)
    .fetch_one(&state.pool)
    .await
    .map_err(AppError::Database)?;

    Ok(Json(TripResponse {
        success: true,
        data: updated,
    }))
}

pub async fn start_trip(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Path(id): Path<String>,
) -> Result<Json<TripResponse>, AppError> {
    let trip = sqlx::query_as::<_, Trip>(
        "SELECT * FROM trips WHERE id = ?1 AND traveler_id = ?2",
    )
    .bind(&id)
    .bind(&traveler.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::Database)?
    .ok_or_else(|| AppError::NotFound("Trip not found".into()))?;

    if trip.status == "active" {
        return Err(AppError::BadRequest("Trip is already active".into()));
    }

    let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

    sqlx::query(
        "UPDATE trips SET status = 'active', start_time = ?1 WHERE id = ?2",
    )
    .bind(&now)
    .bind(&id)
    .execute(&state.pool)
    .await
    .map_err(AppError::Database)?;

    let updated = sqlx::query_as::<_, Trip>(
        "SELECT * FROM trips WHERE id = ?1",
    )
    .bind(&id)
    .fetch_one(&state.pool)
    .await
    .map_err(AppError::Database)?;

    Ok(Json(TripResponse {
        success: true,
        data: updated,
    }))
}

pub async fn end_trip(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Path(id): Path<String>,
) -> Result<Json<TripResponse>, AppError> {
    let trip = sqlx::query_as::<_, Trip>(
        "SELECT * FROM trips WHERE id = ?1 AND traveler_id = ?2",
    )
    .bind(&id)
    .bind(&traveler.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::Database)?
    .ok_or_else(|| AppError::NotFound("Trip not found".into()))?;

    if trip.status != "active" {
        return Err(AppError::BadRequest("Trip is not active".into()));
    }

    let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

    sqlx::query(
        "UPDATE trips SET status = 'completed', end_time = ?1 WHERE id = ?2",
    )
    .bind(&now)
    .bind(&id)
    .execute(&state.pool)
    .await
    .map_err(AppError::Database)?;

    let updated = sqlx::query_as::<_, Trip>(
        "SELECT * FROM trips WHERE id = ?1",
    )
    .bind(&id)
    .fetch_one(&state.pool)
    .await
    .map_err(AppError::Database)?;

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let _ = state
        .diary_gen
        .generate_for_date(&traveler.id, &today)
        .await;

    Ok(Json(TripResponse {
        success: true,
        data: updated,
    }))
}

pub async fn stats(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Path(id): Path<String>,
) -> Result<Json<TripStatsResponse>, AppError> {
    let trip = sqlx::query_as::<_, Trip>(
        "SELECT * FROM trips WHERE id = ?1 AND traveler_id = ?2",
    )
    .bind(&id)
    .bind(&traveler.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::Database)?
    .ok_or_else(|| AppError::NotFound("Trip not found".into()))?;

    let locations = sqlx::query_as::<_, Location>(
        "SELECT * FROM locations WHERE trip_id = ?1 ORDER BY timestamp ASC",
    )
    .bind(&id)
    .fetch_all(&state.pool)
    .await
    .map_err(AppError::Database)?;

    let mut total_distance = 0.0;
    let mut total_speed = 0.0;
    let mut speed_count = 0;

    for window in locations.windows(2) {
        let d = haversine_distance(
            window[0].latitude, window[0].longitude,
            window[1].latitude, window[1].longitude,
        );
        total_distance += d;
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

    let duration = match (&trip.start_time, &trip.end_time) {
        (Some(start), Some(end)) => {
            let s = chrono::NaiveDateTime::parse_from_str(start, "%Y-%m-%d %H:%M:%S").ok();
            let e = chrono::NaiveDateTime::parse_from_str(end, "%Y-%m-%d %H:%M:%S").ok();
            match (s, e) {
                (Some(s), Some(e)) => (e - s).num_minutes() as f64 / 60.0,
                _ => 0.0,
            }
        }
        _ => 0.0,
    };

    Ok(Json(TripStatsResponse {
        success: true,
        data: TripStats {
            total_distance_km: total_distance,
            total_duration_hours: duration,
            point_count: locations.len() as i64,
            avg_speed_kmh: avg_speed,
            start_location: None,
            end_location: None,
        },
    }))
}

pub async fn map_search(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
    Query(params): Query<MapSearchParams>,
) -> Result<Json<MapSearchResponse>, AppError> {
    let results = state
        .osm
        .geocode(&params.q, params.limit)
        .await?;

    Ok(Json(MapSearchResponse {
        success: true,
        data: results,
    }))
}

pub async fn map_reverse(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
    Query(params): Query<MapReverseParams>,
) -> Result<Json<MapReverseResponse>, AppError> {
    let place = state.osm.reverse_geocode(params.lat, params.lon).await?;

    Ok(Json(MapReverseResponse {
        success: true,
        data: place,
    }))
}

pub async fn map_route(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
    Query(params): Query<MapRouteParams>,
) -> Result<Json<MapRouteResponse>, AppError> {
    let route = state
        .osm
        .route(params.from_lat, params.from_lon, params.to_lat, params.to_lon, params.profile.as_deref().unwrap_or("car"))
        .await?;

    Ok(Json(MapRouteResponse {
        success: true,
        data: route,
    }))
}

pub async fn map_poi(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
    Query(params): Query<MapPoiParams>,
) -> Result<Json<MapPoiResponse>, AppError> {
    let places = state
        .osm
        .nearby_poi(params.lat, params.lon, params.radius.unwrap_or(1000.0), params.amenity.as_deref())
        .await?;

    Ok(Json(MapPoiResponse {
        success: true,
        data: places,
    }))
}

fn haversine_distance(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6371.0;
    let d_lat = (lat2 - lat1).to_radians();
    let d_lon = (lon2 - lon1).to_radians();
    let a = (d_lat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (d_lon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();
    r * c
}

use serde::Deserialize;

#[derive(Deserialize)]
pub struct MapSearchParams {
    pub q: String,
    pub limit: Option<usize>,
}

#[derive(Serialize)]
pub struct MapSearchResponse {
    pub success: bool,
    pub data: Vec<crate::services::osm::GeoPlace>,
}

#[derive(Deserialize)]
pub struct MapReverseParams {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Serialize)]
pub struct MapReverseResponse {
    pub success: bool,
    pub data: crate::services::osm::GeoPlace,
}

#[derive(Deserialize)]
pub struct MapRouteParams {
    pub from_lat: f64,
    pub from_lon: f64,
    pub to_lat: f64,
    pub to_lon: f64,
    pub profile: Option<String>,
}

#[derive(Serialize)]
pub struct MapRouteResponse {
    pub success: bool,
    pub data: crate::services::osm::RouteResult,
}

#[derive(Deserialize)]
pub struct MapPoiParams {
    pub lat: f64,
    pub lon: f64,
    pub radius: Option<f64>,
    pub amenity: Option<String>,
}

#[derive(Serialize)]
pub struct MapPoiResponse {
    pub success: bool,
    pub data: Vec<crate::services::osm::GeoPlace>,
}
