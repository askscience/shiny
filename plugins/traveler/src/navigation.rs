//! Navigation session builder — geocode (if needed), route from GPS to the
//! destination, return the navigator payload the frontend turn-by-turn UI
//! consumes. Field names must stay in sync with `data.navigator` parsing in
//! core's agent runner.

use serde_json::Value;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::navigation::{NavigationSession, RouteStepDto};
use shiny_plugin_sdk::tools::ParamHelpers;

use crate::osm::{self, OsmClient};

pub async fn build_navigation_session(
    osm_client: &OsmClient,
    from_lat: f64,
    from_lon: f64,
    params: &Value,
) -> Result<NavigationSession, AppError> {
    let profile = params.param_str("profile").unwrap_or_else(|| "car".into());

    let (to_lat, to_lon, dest_name) = if let Some(dest) = params.param_str("destination") {
        let place = osm_client.geocode_near(&dest, from_lat, from_lon, Some(8)).await?;
        (place.lat, place.lon, osm::place_label(&place))
    } else {
        let to_lat = params
            .param_f64("to_lat")
            .ok_or_else(|| AppError::BadRequest("destination or to_lat/to_lon required".into()))?;
        let to_lon = params
            .param_f64("to_lon")
            .ok_or_else(|| AppError::BadRequest("to_lon required".into()))?;
        let name = params
            .param_str("name")
            .unwrap_or_else(|| format!("{:.4}, {:.4}", to_lat, to_lon));
        (to_lat, to_lon, name)
    };

    let route = osm_client
        .route(from_lat, from_lon, to_lat, to_lon, &profile)
        .await?;

    Ok(NavigationSession {
        destination: dest_name,
        to_lat,
        to_lon,
        geometry: route.geometry,
        steps: route
            .steps
            .into_iter()
            .map(|s| RouteStepDto {
                distance: s.distance,
                duration: s.duration,
                instruction: s.instruction,
            })
            .collect(),
        distance_km: route.total_distance_meters / 1000.0,
        duration_min: route.total_duration_seconds / 60.0,
        profile,
    })
}
