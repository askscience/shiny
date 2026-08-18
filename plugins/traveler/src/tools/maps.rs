//! Map tools: geocode search, reverse geocode, route preview, nearby POI.

use async_trait::async_trait;
use serde_json::json;
use uuid::Uuid;

use shiny_plugin_sdk::artifacts::{Artifact, ArtifactSection, Coordinates, RouteMeta};
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::osm::{GeoPlace, OsmClient};

/* ── map_search ─────────────────────────────────────────────── */

pub struct MapSearch;

#[async_trait]
impl Tool for MapSearch {
    fn name(&self) -> &str { "map_search" }
    fn step_label(&self) -> &str { "Searching the map…" }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let q = req.params.require_str("q")?;
        let limit = req.params.get("limit").and_then(|v| v.as_u64()).map(|n| n as usize);
        let places = OsmClient::new().geocode(&q, limit).await?;
        Ok(ActionOutcome::ok("map_search", json!({ "places": places })))
    }
}

/* ── map_reverse ────────────────────────────────────────────── */

pub struct MapReverse;

#[async_trait]
impl Tool for MapReverse {
    fn name(&self) -> &str { "map_reverse" }
    fn step_label(&self) -> &str { "Looking up address…" }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let lat = req
            .params
            .param_f64("lat")
            .or(req.ctx.lat)
            .ok_or_else(|| AppError::BadRequest("lat required".into()))?;
        let lon = req
            .params
            .param_f64("lon")
            .or(req.ctx.lon)
            .ok_or_else(|| AppError::BadRequest("lon required".into()))?;
        let place = OsmClient::new().reverse_geocode(lat, lon).await?;
        Ok(ActionOutcome::ok("map_reverse", json!({ "place": place })))
    }
}

/* ── map_route ──────────────────────────────────────────────── */

pub struct MapRoute;

#[async_trait]
impl Tool for MapRoute {
    fn name(&self) -> &str { "map_route" }
    fn step_label(&self) -> &str { "Computing route…" }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let from_lat = req
            .params
            .param_f64("from_lat")
            .or(req.ctx.lat)
            .ok_or_else(|| AppError::BadRequest("from_lat required".into()))?;
        let from_lon = req
            .params
            .param_f64("from_lon")
            .or(req.ctx.lon)
            .ok_or_else(|| AppError::BadRequest("from_lon required".into()))?;
        let to_lat = req
            .params
            .param_f64("to_lat")
            .ok_or_else(|| AppError::BadRequest("to_lat required".into()))?;
        let to_lon = req
            .params
            .param_f64("to_lon")
            .ok_or_else(|| AppError::BadRequest("to_lon required".into()))?;
        let profile = req.params.param_str("profile").unwrap_or_else(|| "car".into());

        let route = OsmClient::new()
            .route(from_lat, from_lon, to_lat, to_lon, &profile)
            .await?;

        let artifact = Artifact {
            id: Uuid::new_v4().to_string(),
            artifact_type: "route_preview".into(),
            title: "Route".into(),
            subtitle: Some(format!(
                "{:.1} km, {:.0} min",
                route.total_distance_meters / 1000.0,
                route.total_duration_seconds / 60.0
            )),
            coordinates: Some(Coordinates { lat: to_lat, lon: to_lon }),
            sections: route
                .steps
                .iter()
                .take(5)
                .map(|s| ArtifactSection {
                    label: format!("{:.0}m", s.distance),
                    value: s.instruction.clone(),
                })
                .collect(),
            actions: vec![],
            days: vec![],
            route: Some(RouteMeta {
                distance_km: route.total_distance_meters / 1000.0,
                duration_min: route.total_duration_seconds / 60.0,
            }),
            geometry: route.geometry.clone(),
            narrative: None,
            theme: None,
            destination: None,
        };

        Ok(ActionOutcome::ok("map_route", json!({ "route": route })).with_artifact(artifact))
    }
}

/* ── map_poi ────────────────────────────────────────────────── */

pub struct MapPoi;

#[async_trait]
impl Tool for MapPoi {
    fn name(&self) -> &str { "map_poi" }
    fn step_label(&self) -> &str { "Finding places nearby…" }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let lat = req
            .params
            .param_f64("lat")
            .or(req.ctx.lat)
            .ok_or_else(|| AppError::BadRequest("lat required".into()))?;
        let lon = req
            .params
            .param_f64("lon")
            .or(req.ctx.lon)
            .ok_or_else(|| AppError::BadRequest("lon required".into()))?;
        let radius = req.params.param_f64("radius").unwrap_or(1000.0);
        let amenity = req.params.param_str("amenity");

        let places = OsmClient::new()
            .nearby_poi(lat, lon, radius, amenity.as_deref())
            .await?;

        Ok(ActionOutcome::ok("map_poi", json!({ "places": places }))
            .with_artifact(poi_list(&places)))
    }
}

fn poi_list(places: &[GeoPlace]) -> Artifact {
    let sections: Vec<ArtifactSection> = places
        .iter()
        .take(8)
        .map(|p| ArtifactSection {
            label: p.display_name.chars().take(40).collect(),
            value: format!("{:.4}, {:.4}", p.lat, p.lon),
        })
        .collect();

    Artifact {
        id: Uuid::new_v4().to_string(),
        artifact_type: "poi_list".into(),
        title: "Nearby places".into(),
        subtitle: None,
        coordinates: places.first().map(|p| Coordinates { lat: p.lat, lon: p.lon }),
        sections,
        actions: vec![],
        days: vec![],
        route: None,
        geometry: vec![],
        narrative: None,
        theme: None,
        destination: None,
    }
}
