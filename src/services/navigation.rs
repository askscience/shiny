use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::AppError;
use crate::services::osm::{OsmService, RouteStep};

#[derive(Serialize, Deserialize, Clone)]
pub struct NavigationSession {
    pub destination: String,
    pub to_lat: f64,
    pub to_lon: f64,
    pub geometry: Vec<[f64; 2]>,
    pub steps: Vec<RouteStep>,
    pub distance_km: f64,
    pub duration_min: f64,
    pub profile: String,
}

fn param_str(params: &Value, key: &str) -> Option<String> {
    params.get(key).and_then(|v| v.as_str()).map(String::from)
}

fn param_f64(params: &Value, key: &str) -> Option<f64> {
    params.get(key).and_then(|v| v.as_f64())
}

/// Geocode (if needed), route from GPS → destination, return navigator payload.
pub async fn build_navigation_session(
    osm: &OsmService,
    from_lat: f64,
    from_lon: f64,
    params: &Value,
) -> Result<NavigationSession, AppError> {
    let profile = params
        .get("profile")
        .and_then(|v| v.as_str())
        .unwrap_or("car");

    let (to_lat, to_lon, dest_name) = if let Some(dest) = param_str(params, "destination") {
        let place = osm.geocode_near(&dest, from_lat, from_lon, Some(8)).await?;
        (place.lat, place.lon, crate::services::osm::place_label(&place))
    } else {
        let to_lat = param_f64(params, "to_lat")
            .ok_or_else(|| AppError::BadRequest("destination or to_lat/to_lon required".into()))?;
        let to_lon = param_f64(params, "to_lon")
            .ok_or_else(|| AppError::BadRequest("to_lon required".into()))?;
        let name = param_str(params, "name")
            .unwrap_or_else(|| format!("{:.4}, {:.4}", to_lat, to_lon));
        (to_lat, to_lon, name)
    };

    let route = osm
        .route(from_lat, from_lon, to_lat, to_lon, profile)
        .await?;

    Ok(NavigationSession {
        destination: dest_name,
        to_lat,
        to_lon,
        geometry: route.geometry,
        steps: route.steps,
        distance_km: route.total_distance_meters / 1000.0,
        duration_min: route.total_duration_seconds / 60.0,
        profile: profile.to_string(),
    })
}
