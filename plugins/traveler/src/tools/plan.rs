//! plan_trip — the full research → narrative pipeline: geocode, route,
//! overview story, themed guides (nightlife/food/culture).

use async_trait::async_trait;
use serde_json::json;
use uuid::Uuid;

use shiny_plugin_sdk::artifacts::{Artifact, ArtifactAction, Coordinates, RouteMeta};
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::osm::OsmClient;
use crate::story;

pub struct PlanTrip;

#[async_trait]
impl Tool for PlanTrip {
    fn name(&self) -> &str { "plan_trip" }
    fn step_label(&self) -> &str { "Planning your trip…" }
    fn humanize(&self, _r: &str, data: &serde_json::Value) -> String {
        let dest = data
            .pointer("/destination/name")
            .and_then(|v| v.as_str())
            .unwrap_or("destination");
        format!("Planned a trip to {dest}")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let destination = req.params.require_str("destination")?;
        let num_days = req
            .params
            .get("days")
            .and_then(|v| v.as_u64())
            .unwrap_or(3)
            .max(1) as u32;
        let profile = req.params.param_str("profile").unwrap_or_else(|| "car".into());
        let model = req.ctx.ollama_model.as_deref();
        let lang = req.ctx.lang.as_str();

        let osm = OsmClient::new();
        let places = osm.geocode(&destination, Some(1)).await?;
        let place = places.first().ok_or_else(|| {
            AppError::BadRequest(format!("Could not find destination: {}", destination))
        })?;

        let from_lat = req.ctx.lat.unwrap_or(place.lat);
        let from_lon = req.ctx.lon.unwrap_or(place.lon);
        let route_opt = osm
            .route(from_lat, from_lon, place.lat, place.lon, &profile)
            .await
            .ok();

        let route_meta = route_opt.as_ref().map(|r| RouteMeta {
            distance_km: r.total_distance_meters / 1000.0,
            duration_min: r.total_duration_seconds / 60.0,
        });
        let geometry = route_opt
            .as_ref()
            .map(|r| r.geometry.clone())
            .unwrap_or_default();

        let overview_search = ctx
            .search()
            .await
            .search(&format!(
                "{} {} day travel guide itinerary what to do",
                destination, num_days
            ))
            .await?;

        let lodging_facts = story::gather_lodging_facts(
            ctx,
            &destination,
            num_days,
            route_meta.as_ref(),
            lang,
            model,
        )
        .await;

        let (narrative, day_sections) = story::build_overview_story(
            ctx,
            &destination,
            num_days,
            lang,
            &overview_search,
            &lodging_facts,
            model,
        )
        .await;

        let origin_hint = match (req.ctx.lat, req.ctx.lon) {
            (Some(_), Some(_)) => "From your location".to_string(),
            _ => "Your journey".to_string(),
        };

        let main = Artifact {
            id: Uuid::new_v4().to_string(),
            artifact_type: "travel_plan".into(),
            theme: Some("overview".into()),
            destination: Some(destination.clone()),
            title: destination.clone(),
            subtitle: Some(format!(
                "{} · {} days{}",
                origin_hint,
                num_days,
                route_meta
                    .as_ref()
                    .map(|r| format!(" · {:.0} km drive", r.distance_km))
                    .unwrap_or_default()
            )),
            coordinates: Some(Coordinates {
                lat: place.lat,
                lon: place.lon,
            }),
            narrative: Some(narrative),
            sections: day_sections,
            days: vec![],
            route: route_meta.clone(),
            geometry: geometry.clone(),
            actions: vec![ArtifactAction {
                label: "Show route on map".into(),
                tool: "map_route".into(),
                params: json!({
                    "to_lat": place.lat,
                    "to_lon": place.lon,
                }),
            }],
        };

        let mut guides = Vec::new();
        let themes: &[(&str, &str, &str)] = &[
            ("nightlife", "site_info", "nightlife bars evening clubs where to go at night"),
            ("food", "poi_list", "best restaurants food markets local cuisine must eat"),
            ("culture", "monument_info", "museums art culture history hidden gems"),
        ];

        for (theme, artifact_type, query_suffix) in themes {
            let query = format!("{} {}", destination, query_suffix);
            // Soft-fail: a failed theme search just skips that guide.
            if let Ok(results) = ctx.search().await.search(&query).await {
                if !results.is_empty() {
                    guides.push(
                        story::build_theme_guide(
                            ctx,
                            &destination,
                            lang,
                            theme,
                            artifact_type,
                            &results,
                            place.lat,
                            place.lon,
                            model,
                        )
                        .await,
                    );
                }
            }
        }

        let outcome = ActionOutcome::ok(
            "plan_trip",
            json!({
                "destination": {
                    "name": place.display_name,
                    "lat": place.lat,
                    "lon": place.lon,
                },
                "guides_created": guides.len() + 1,
                "route": route_opt.as_ref().map(|r| json!({
                    "distance_km": r.total_distance_meters / 1000.0,
                    "duration_min": r.total_duration_seconds / 60.0,
                    "geometry_points": r.geometry.len(),
                })),
            }),
        )
        .with_artifact(main)
        .with_extra_artifacts(guides);

        Ok(outcome)
    }
}
