use chrono::Utc;
use serde_json::{json, Value};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::{DiaryEntry, Location, Traveler, Trip};
use crate::services::artifacts::{self, Artifact, PlanDay, RouteMeta};
use crate::services::web_search::SearchResult;

#[derive(Debug, Clone)]
pub struct AgentContext {
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub heading: Option<f64>,
    pub lang: String,
}

#[derive(Debug, Clone)]
pub struct ActionOutcome {
    pub action: String,
    pub result: String,
    pub data: Value,
    pub artifact: Option<Artifact>,
    pub extra_artifacts: Vec<Artifact>,
}

pub async fn execute_action(
    state: &AppState,
    traveler: &Traveler,
    ctx: &AgentContext,
    action: &str,
    params: &Value,
) -> Result<ActionOutcome, AppError> {
    let outcome = match action {
        "create_trip" => {
            let name = param_str(params, "name").ok_or_else(|| AppError::BadRequest("name required".into()))?;
            let description = params.get("description").and_then(|v| v.as_str()).map(String::from);
            let trip = Trip::new(traveler.id.clone(), name, description);
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
            .await?;
            let had_active = fetch_active_trip(&state.pool, &traveler.id).await?.is_some();
            let trip = if had_active {
                fetch_trip(&state.pool, &traveler.id, &trip.id).await?
            } else {
                start_trip_internal(&state.pool, &traveler.id, &trip.id).await?
            };
            ok(
                action,
                json!({
                    "trip": trip,
                    "auto_started": !had_active,
                }),
            )
        }
        "list_trips" => {
            let trips = fetch_trips(&state.pool, &traveler.id).await?;
            ok(action, json!({ "trips": trips }))
        }
        "get_trip" => {
            let id = param_str(params, "trip_id").ok_or_else(|| AppError::BadRequest("trip_id required".into()))?;
            let trip = fetch_trip(&state.pool, &traveler.id, &id).await?;
            ok(action, json!({ "trip": trip }))
        }
        "get_active_trip" => {
            let trip = fetch_active_trip(&state.pool, &traveler.id).await?;
            ok(action, json!({ "trip": trip }))
        }
        "start_trip" => {
            let id = param_str(params, "trip_id").ok_or_else(|| AppError::BadRequest("trip_id required".into()))?;
            let trip = start_trip_internal(&state.pool, &traveler.id, &id).await?;
            ok(action, json!({ "trip": trip }))
        }
        "end_trip" => {
            let id = param_str(params, "trip_id").ok_or_else(|| AppError::BadRequest("trip_id required".into()))?;
            let trip = end_trip_internal(state, &traveler.id, &id).await?;
            ok(action, json!({ "trip": trip }))
        }
        "trip_stats" => {
            let id = param_str(params, "trip_id").ok_or_else(|| AppError::BadRequest("trip_id required".into()))?;
            let stats = trip_stats_internal(&state.pool, &traveler.id, &id).await?;
            ok(action, json!({ "stats": stats }))
        }
        "submit_location" => {
            let lat = param_f64(params, "latitude").ok_or_else(|| AppError::BadRequest("latitude required".into()))?;
            let lon = param_f64(params, "longitude").ok_or_else(|| AppError::BadRequest("longitude required".into()))?;
            let trip_id = params.get("trip_id").and_then(|v| v.as_str()).map(String::from);
            let location = Location::new(
                traveler.id.clone(),
                trip_id,
                lat,
                lon,
                param_f64(params, "altitude"),
                param_f64(params, "speed"),
                param_f64(params, "heading"),
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
            .await?;
            ok(action, json!({ "location": location }))
        }
        "list_locations" => {
            let limit = params.get("limit").and_then(|v| v.as_i64()).unwrap_or(50);
            let rows = if let Some(trip_id) = params.get("trip_id").and_then(|v| v.as_str()) {
                sqlx::query_as::<_, Location>(
                    "SELECT * FROM locations WHERE traveler_id = ?1 AND trip_id = ?2 ORDER BY timestamp DESC LIMIT ?3",
                )
                .bind(&traveler.id)
                .bind(trip_id)
                .bind(limit)
                .fetch_all(&state.pool)
                .await?
            } else {
                sqlx::query_as::<_, Location>(
                    "SELECT * FROM locations WHERE traveler_id = ?1 ORDER BY timestamp DESC LIMIT ?2",
                )
                .bind(&traveler.id)
                .bind(limit)
                .fetch_all(&state.pool)
                .await?
            };
            ok(action, json!({ "locations": rows, "count": rows.len() }))
        }
        "trip_route" => {
            let id = param_str(params, "trip_id").ok_or_else(|| AppError::BadRequest("trip_id required".into()))?;
            let _ = fetch_trip(&state.pool, &traveler.id, &id).await?;
            let rows = sqlx::query_as::<_, Location>(
                "SELECT * FROM locations WHERE trip_id = ?1 ORDER BY timestamp ASC",
            )
            .bind(&id)
            .fetch_all(&state.pool)
            .await?;
            let route: Vec<Value> = rows
                .iter()
                .map(|l| json!({ "lat": l.latitude, "lon": l.longitude, "timestamp": l.timestamp, "speed": l.speed }))
                .collect();
            ok(action, json!({ "route": route }))
        }
        "map_search" => {
            let q = param_str(params, "q").ok_or_else(|| AppError::BadRequest("q required".into()))?;
            let limit = params.get("limit").and_then(|v| v.as_u64()).map(|n| n as usize);
            let places = state.osm.geocode(&q, limit).await?;
            ok(action, json!({ "places": places }))
        }
        "map_reverse" => {
            let lat = param_f64(params, "lat").or(ctx.lat).ok_or_else(|| AppError::BadRequest("lat required".into()))?;
            let lon = param_f64(params, "lon").or(ctx.lon).ok_or_else(|| AppError::BadRequest("lon required".into()))?;
            let place = state.osm.reverse_geocode(lat, lon).await?;
            ok(action, json!({ "place": place }))
        }
        "map_route" => {
            let from_lat = param_f64(params, "from_lat").or(ctx.lat).ok_or_else(|| AppError::BadRequest("from_lat required".into()))?;
            let from_lon = param_f64(params, "from_lon").or(ctx.lon).ok_or_else(|| AppError::BadRequest("from_lon required".into()))?;
            let to_lat = param_f64(params, "to_lat").ok_or_else(|| AppError::BadRequest("to_lat required".into()))?;
            let to_lon = param_f64(params, "to_lon").ok_or_else(|| AppError::BadRequest("to_lon required".into()))?;
            let profile = params.get("profile").and_then(|v| v.as_str()).unwrap_or("car");
            let route = state.osm.route(from_lat, from_lon, to_lat, to_lon, profile).await?;
            let artifact = Artifact {
                id: Uuid::new_v4().to_string(),
                artifact_type: "route_preview".into(),
                title: "Route".into(),
                subtitle: Some(format!("{:.1} km, {:.0} min", route.total_distance_meters / 1000.0, route.total_duration_seconds / 60.0)),
                coordinates: Some(artifacts::Coordinates { lat: to_lat, lon: to_lon }),
                sections: route.steps.iter().take(5).map(|s| artifacts::ArtifactSection {
                    label: format!("{:.0}m", s.distance),
                    value: s.instruction.clone(),
                }).collect(),
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
            ActionOutcome {
                action: action.to_string(),
                result: "ok".into(),
                data: json!({ "route": route }),
                artifact: Some(artifact),
                extra_artifacts: vec![],
            }
        }
        "map_poi" => {
            let lat = param_f64(params, "lat").or(ctx.lat).ok_or_else(|| AppError::BadRequest("lat required".into()))?;
            let lon = param_f64(params, "lon").or(ctx.lon).ok_or_else(|| AppError::BadRequest("lon required".into()))?;
            let radius = param_f64(params, "radius").unwrap_or(1000.0);
            let amenity = params.get("amenity").and_then(|v| v.as_str());
            let places = state.osm.nearby_poi(lat, lon, radius, amenity).await?;
            let artifact = artifacts::poi_list(&places);
            ActionOutcome {
                action: action.to_string(),
                result: "ok".into(),
                data: json!({ "places": places }),
                artifact: Some(artifact),
                extra_artifacts: vec![],
            }
        }
        "list_diary" => {
            let limit = params.get("limit").and_then(|v| v.as_i64()).unwrap_or(20);
            let entries = sqlx::query_as::<_, DiaryEntry>(
                "SELECT * FROM diary_entries WHERE traveler_id = ?1 ORDER BY date DESC LIMIT ?2",
            )
            .bind(&traveler.id)
            .bind(limit)
            .fetch_all(&state.pool)
            .await?;
            ok(action, json!({ "entries": entries }))
        }
        "get_diary" => {
            let date = param_str(params, "date").ok_or_else(|| AppError::BadRequest("date required".into()))?;
            let entry = sqlx::query_as::<_, DiaryEntry>(
                "SELECT * FROM diary_entries WHERE traveler_id = ?1 AND date = ?2",
            )
            .bind(&traveler.id)
            .bind(&date)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("No diary entry for {}", date)))?;
            ok(action, json!({ "entry": entry }))
        }
        "search_diary" => {
            let q = param_str(params, "q").ok_or_else(|| AppError::BadRequest("q required".into()))?;
            let limit = params.get("limit").and_then(|v| v.as_i64()).unwrap_or(20);
            let pattern = format!("%{}%", q);
            let entries = sqlx::query_as::<_, DiaryEntry>(
                "SELECT * FROM diary_entries WHERE traveler_id = ?1 AND \
                 (content_markdown LIKE ?2 OR title LIKE ?2 OR summary LIKE ?2 OR tags LIKE ?2) \
                 ORDER BY date DESC LIMIT ?3",
            )
            .bind(&traveler.id)
            .bind(&pattern)
            .bind(limit)
            .fetch_all(&state.pool)
            .await?;
            ok(action, json!({ "entries": entries }))
        }
        "generate_diary" => {
            let date = params
                .get("date")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
            let entry = state.diary_gen.generate_for_date(&traveler.id, &date).await?;
            ok(action, json!({ "entry": entry }))
        }
        "web_search" => {
            let query = param_str(params, "query").ok_or_else(|| AppError::BadRequest("query required".into()))?;
            let results = state.search.search(&query).await?;
            let summary = if state.ollama.is_available().await {
                let prompt = format!(
                    "Summarize these search results in 2-3 sentences in language '{}': {:?}",
                    ctx.lang, results
                );
                state.ollama.generate(&prompt, None).await.ok()
            } else {
                None
            };
            let artifact = Artifact {
                id: Uuid::new_v4().to_string(),
                artifact_type: "site_info".into(),
                title: query.clone(),
                subtitle: summary.clone(),
                coordinates: None,
                sections: results.iter().take(4).map(|r| artifacts::ArtifactSection {
                    label: r.title.clone(),
                    value: r.snippet.clone(),
                }).collect(),
                actions: vec![],
                days: vec![],
                route: None,
                geometry: vec![],
                narrative: None,
                theme: None,
                destination: None,
            };
            ActionOutcome {
                action: action.to_string(),
                result: "ok".into(),
                data: json!({ "results": results, "summary": summary }),
                artifact: Some(artifact),
                extra_artifacts: vec![],
            }
        }
        "plan_trip" => {
            let destination = param_str(params, "destination")
                .ok_or_else(|| AppError::BadRequest("destination required".into()))?;
            let num_days = params.get("days").and_then(|v| v.as_u64()).unwrap_or(3).max(1) as u32;
            let profile = params
                .get("profile")
                .and_then(|v| v.as_str())
                .unwrap_or("car");

            let places = state.osm.geocode(&destination, Some(1)).await?;
            let place = places.first().ok_or_else(|| {
                AppError::BadRequest(format!("Could not find destination: {}", destination))
            })?;

            let from_lat = ctx.lat.unwrap_or(place.lat);
            let from_lon = ctx.lon.unwrap_or(place.lon);
            let route_opt = state
                .osm
                .route(from_lat, from_lon, place.lat, place.lon, profile)
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

            let overview_search = state
                .search
                .search(&format!(
                    "{} {} day travel guide itinerary what to do",
                    destination, num_days
                ))
                .await?;

            let (narrative, day_sections) = build_overview_story(
                state,
                &destination,
                num_days,
                &ctx.lang,
                &overview_search,
            )
            .await;

            let origin_hint = match (ctx.lat, ctx.lon) {
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
                coordinates: Some(artifacts::Coordinates {
                    lat: place.lat,
                    lon: place.lon,
                }),
                narrative: Some(narrative),
                sections: day_sections,
                days: vec![],
                route: route_meta.clone(),
                geometry: geometry.clone(),
                actions: vec![artifacts::ArtifactAction {
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
                (
                    "nightlife",
                    "site_info",
                    "nightlife bars evening clubs where to go at night",
                ),
                (
                    "food",
                    "poi_list",
                    "best restaurants food markets local cuisine must eat",
                ),
                (
                    "culture",
                    "monument_info",
                    "museums art culture history hidden gems",
                ),
            ];

            for (theme, artifact_type, query_suffix) in themes {
                let query = format!("{} {}", destination, query_suffix);
                if let Ok(results) = state.search.search(&query).await {
                    if !results.is_empty() {
                        guides.push(
                            build_theme_guide(
                                state,
                                &destination,
                                &ctx.lang,
                                theme,
                                artifact_type,
                                &results,
                                place.lat,
                                place.lon,
                            )
                            .await,
                        );
                    }
                }
            }

            outcome_with_artifacts(
                action,
                json!({
                    "destination": place,
                    "guides_created": guides.len() + 1,
                    "route": route_opt,
                }),
                Some(main),
                guides,
            )
        }
        "show_artifact" => {
            let artifact = artifacts::build_from_params(params);
            ActionOutcome {
                action: action.to_string(),
                result: "ok".into(),
                data: json!({ "artifact": artifact }),
                artifact: Some(artifact),
                extra_artifacts: vec![],
            }
        }
        "update_artifact" => {
            let artifact_id = param_str(params, "artifact_id")
                .ok_or_else(|| AppError::BadRequest("artifact_id required".into()))?;
            let update = artifacts::ArtifactUpdate {
                title: params.get("title").and_then(|v| v.as_str()).map(String::from),
                subtitle: params.get("subtitle").and_then(|v| v.as_str()).map(String::from),
                sections: params.get("sections").and_then(|s| {
                    serde_json::from_value::<Vec<artifacts::ArtifactSection>>(s.clone()).ok()
                }),
                actions: params.get("actions").and_then(|s| {
                    serde_json::from_value::<Vec<artifacts::ArtifactAction>>(s.clone()).ok()
                }),
                coordinates: params.get("coordinates").and_then(|c| {
                    Some(artifacts::Coordinates {
                        lat: c.get("lat")?.as_f64()?,
                        lon: c.get("lon")?.as_f64()?,
                    })
                }),
            };
            let artifact = artifacts::merge_update(&state.pool, &traveler.id, &artifact_id, update).await?;
            ActionOutcome {
                action: action.to_string(),
                result: "ok".into(),
                data: json!({ "artifact": artifact }),
                artifact: Some(artifact),
                extra_artifacts: vec![],
            }
        }
        other => {
            return Err(AppError::BadRequest(format!("Unknown action: {}", other)));
        }
    };

    Ok(outcome)
}

async fn build_overview_story(
    state: &AppState,
    destination: &str,
    num_days: u32,
    lang: &str,
    results: &[SearchResult],
) -> (String, Vec<artifacts::ArtifactSection>) {
    let snippets: Vec<String> = results
        .iter()
        .take(8)
        .map(|r| format!("- {}: {}", r.title, r.snippet))
        .collect();

    if state.ollama.is_available().await {
        let prompt = format!(
            "You are a passionate travel writer. Write for language '{}' about visiting {} for {} days.\n\
             Use these web search findings:\n{}\n\n\
             Reply with ONLY valid JSON (no markdown):\n\
             {{\"intro\":\"2-3 evocative paragraphs as one string with line breaks\",\"days\":[{{\"title\":\"Day 1 theme\",\"story\":\"One rich paragraph: what to feel, where to wander, practical rhythm — no bullet lists\"}}]}}\n\
             Include exactly {} day objects in days array.",
            lang, destination, num_days, snippets.join("\n"), num_days
        );
        if let Ok(raw) = state.ollama.generate(&prompt, None).await {
            if let Ok(v) = serde_json::from_str::<Value>(&extract_json_object(&raw)) {
                let intro = v
                    .get("intro")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let sections: Vec<artifacts::ArtifactSection> = v
                    .get("days")
                    .and_then(|d| d.as_array())
                    .map(|arr| {
                        arr.iter()
                            .enumerate()
                            .filter_map(|(i, item)| {
                                Some(artifacts::ArtifactSection {
                                    label: item
                                        .get("title")
                                        .and_then(|t| t.as_str())
                                        .unwrap_or("Day")
                                        .to_string(),
                                    value: item
                                        .get("story")
                                        .and_then(|s| s.as_str())
                                        .unwrap_or("")
                                        .to_string(),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if !intro.is_empty() {
                    return (intro, sections);
                }
            }
        }
    }

    let intro = format!(
        "{destination} unfolds over {num_days} days — a city best met on foot, between café stops and golden-hour streets.\n\n\
         Start from what the web turned up: {}",
        snippets.join(" ")
    );
    let sections = (1..=num_days)
        .map(|d| artifacts::ArtifactSection {
            label: format!("Day {}", d),
            value: format!(
                "Let the {} day lead you through {} — follow curiosity, leave room for a long lunch and an unplanned detour.",
                d, destination
            ),
        })
        .collect();
    (intro, sections)
}

async fn build_theme_guide(
    state: &AppState,
    destination: &str,
    lang: &str,
    theme: &str,
    artifact_type: &str,
    results: &[SearchResult],
    lat: f64,
    lon: f64,
) -> Artifact {
    let snippets: Vec<String> = results
        .iter()
        .take(6)
        .map(|r| format!("- {}: {}", r.title, r.snippet))
        .collect();

    let (title, narrative) = if state.ollama.is_available().await {
        let theme_label = match theme {
            "nightlife" => "after dark",
            "food" => "food & drink",
            "culture" => "culture & art",
            _ => theme,
        };
        let prompt = format!(
            "Write a vivid tourist guide about {} in {} for language '{}'. Theme: {}.\n\
             Web research:\n{}\n\n\
             Reply with ONLY JSON: {{\"title\":\"short catchy title\",\"narrative\":\"2-3 paragraphs, sensory and practical, no bullet lists\"}}",
            theme_label, destination, lang, theme_label, snippets.join("\n")
        );
        if let Ok(raw) = state.ollama.generate(&prompt, None).await {
            if let Ok(v) = serde_json::from_str::<Value>(&extract_json_object(&raw)) {
                let t = v.get("title").and_then(|x| x.as_str()).unwrap_or(theme).to_string();
                let n = v
                    .get("narrative")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                if !n.is_empty() {
                    (t, n)
                } else {
                    default_theme_copy(theme, destination, &snippets)
                }
            } else {
                default_theme_copy(theme, destination, &snippets)
            }
        } else {
            default_theme_copy(theme, destination, &snippets)
        }
    } else {
        default_theme_copy(theme, destination, &snippets)
    };

    Artifact {
        id: Uuid::new_v4().to_string(),
        artifact_type: artifact_type.to_string(),
        theme: Some(theme.to_string()),
        destination: Some(destination.to_string()),
        title,
        subtitle: Some(destination.to_string()),
        coordinates: Some(artifacts::Coordinates { lat, lon }),
        narrative: Some(narrative),
        sections: vec![],
        days: vec![],
        route: None,
        geometry: vec![],
        actions: vec![],
    }
}

fn default_theme_copy(theme: &str, destination: &str, snippets: &[String]) -> (String, String) {
    let title = match theme {
        "nightlife" => format!("{} after dark", destination),
        "food" => format!("Eat like a local in {}", destination),
        "culture" => format!("Culture & soul of {}", destination),
        _ => format!("{} — {}", destination, theme),
    };
    let body = if snippets.is_empty() {
        format!("Explore {} through {} — take your time, ask locals, follow the mood.", destination, theme)
    } else {
        format!(
            "{}\n\nWhat travelers mention: {}",
            match theme {
                "nightlife" => format!("When the sun sets, {} changes pace — wine bars, live music, and late walks reward those who stay out.", destination),
                "food" => format!("{} is a city tasted slowly — markets in the morning, long lunches, and neighborhood trattorias worth the detour.", destination),
                "culture" => format!("{} wears its history openly — museums, churches, and street corners each tell a layer of the story.", destination),
                _ => String::new(),
            },
            snippets.join(" ")
        )
    };
    (title, body)
}

fn extract_json_object(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            return trimmed[start..=end].to_string();
        }
    }
    trimmed.to_string()
}

async fn build_plan_days(
    state: &AppState,
    destination: &str,
    num_days: u32,
    lang: &str,
    results: &[SearchResult],
) -> Vec<PlanDay> {
    if state.ollama.is_available().await {
        let snippets: Vec<String> = results
            .iter()
            .take(6)
            .map(|r| format!("{}: {}", r.title, r.snippet))
            .collect();
        let prompt = format!(
            "Create a detailed {}-day travel itinerary for {}. Reply in language code '{}'. \
             Use this research:\n{}\n\n\
             Reply with ONLY a JSON array (no markdown):\n\
             [{{\"day\":1,\"title\":\"Short theme\",\"items\":[\"09:00 — Place — 2h — tip\",\"12:30 — Lunch spot\",\"15:00 — Museum\"]}}]\n\
             Rules: exactly {} days; 4–6 items per day; each item should include time (24h), place name, duration, and a short practical tip.",
            num_days, destination, lang, snippets.join("\n"), num_days
        );
        if let Ok(raw) = state.ollama.generate(&prompt, None).await {
            if let Some(days) = parse_plan_days_json(&raw) {
                if !days.is_empty() {
                    return days;
                }
            }
        }
    }
    fallback_plan_days(num_days, destination, results)
}

fn parse_plan_days_json(raw: &str) -> Option<Vec<PlanDay>> {
    let trimmed = raw.trim();
    let start = trimmed.find('[')?;
    let end = trimmed.rfind(']')?;
    serde_json::from_str::<Vec<PlanDay>>(&trimmed[start..=end]).ok()
}

fn fallback_plan_days(num_days: u32, destination: &str, results: &[SearchResult]) -> Vec<PlanDay> {
    let mut items_pool: Vec<String> = results
        .iter()
        .flat_map(|r| {
            r.snippet
                .split(|c| c == '.' || c == ';')
                .map(|s| s.trim().to_string())
                .filter(|s| s.len() > 12)
        })
        .take((num_days as usize) * 3)
        .collect();
    if items_pool.is_empty() {
        items_pool = vec![
            format!("Explore downtown {}", destination),
            "Local food and culture".into(),
            "Evening stroll".into(),
        ];
    }
    (1..=num_days)
        .map(|d| {
            let start = ((d - 1) as usize) * 4;
            let day_items: Vec<String> = items_pool.iter().skip(start).take(5).cloned().collect();
            PlanDay {
                day: d,
                title: format!("Day {} — {}", d, destination),
                items: if day_items.is_empty() {
                    vec![
                        format!("09:00 — Explore {}", destination),
                        "12:30 — Local lunch".into(),
                        "15:00 — Main sight".into(),
                        "19:00 — Evening walk".into(),
                    ]
                } else {
                    day_items
                },
            }
        })
        .collect()
}

fn ok(action: &str, data: Value) -> ActionOutcome {
    ActionOutcome {
        action: action.to_string(),
        result: "ok".into(),
        data,
        artifact: None,
        extra_artifacts: vec![],
    }
}

fn outcome_with_artifacts(
    action: &str,
    data: Value,
    primary: Option<Artifact>,
    extra: Vec<Artifact>,
) -> ActionOutcome {
    ActionOutcome {
        action: action.to_string(),
        result: "ok".into(),
        data,
        artifact: primary,
        extra_artifacts: extra,
    }
}

fn param_str(params: &Value, key: &str) -> Option<String> {
    params.get(key).and_then(|v| v.as_str()).map(String::from)
}

fn param_f64(params: &Value, key: &str) -> Option<f64> {
    params.get(key).and_then(|v| v.as_f64())
}

pub async fn fetch_trips(pool: &SqlitePool, traveler_id: &str) -> Result<Vec<Trip>, AppError> {
    Ok(sqlx::query_as::<_, Trip>(
        "SELECT * FROM trips WHERE traveler_id = ?1 ORDER BY created_at DESC",
    )
    .bind(traveler_id)
    .fetch_all(pool)
    .await?)
}

pub async fn fetch_trip(pool: &SqlitePool, traveler_id: &str, id: &str) -> Result<Trip, AppError> {
    sqlx::query_as::<_, Trip>("SELECT * FROM trips WHERE id = ?1 AND traveler_id = ?2")
        .bind(id)
        .bind(traveler_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Trip not found".into()))
}

pub async fn fetch_active_trip(pool: &SqlitePool, traveler_id: &str) -> Result<Option<Trip>, AppError> {
    Ok(sqlx::query_as::<_, Trip>(
        "SELECT * FROM trips WHERE traveler_id = ?1 AND status = 'active' LIMIT 1",
    )
    .bind(traveler_id)
    .fetch_optional(pool)
    .await?)
}

pub async fn start_trip_internal(pool: &SqlitePool, traveler_id: &str, id: &str) -> Result<Trip, AppError> {
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

pub async fn end_trip_internal(state: &AppState, traveler_id: &str, id: &str) -> Result<Trip, AppError> {
    let trip = fetch_trip(&state.pool, traveler_id, id).await?;
    if trip.status != "active" {
        return Err(AppError::BadRequest("Trip is not active".into()));
    }
    let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    sqlx::query("UPDATE trips SET status = 'completed', end_time = ?1 WHERE id = ?2")
        .bind(&now)
        .bind(id)
        .execute(&state.pool)
        .await?;
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let _ = state.diary_gen.generate_for_date(traveler_id, &today).await;
    fetch_trip(&state.pool, traveler_id, id).await
}

pub async fn trip_stats_internal(
    pool: &SqlitePool,
    traveler_id: &str,
    id: &str,
) -> Result<crate::models::TripStats, AppError> {
    let _ = fetch_trip(pool, traveler_id, id).await?;
    let locations = sqlx::query_as::<_, Location>(
        "SELECT * FROM locations WHERE trip_id = ?1 ORDER BY timestamp ASC",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

    let mut total_distance = 0.0;
    let mut total_speed = 0.0;
    let mut speed_count = 0;
    for window in locations.windows(2) {
        total_distance += haversine(
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
    Ok(crate::models::TripStats {
        total_distance_km: total_distance,
        total_duration_hours: 0.0,
        point_count: locations.len() as i64,
        avg_speed_kmh: avg_speed,
        start_location: None,
        end_location: None,
    })
}

fn haversine(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6371.0;
    let d_lat = (lat2 - lat1).to_radians();
    let d_lon = (lon2 - lon1).to_radians();
    let a = (d_lat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (d_lon / 2.0).sin().powi(2);
    2.0 * r * a.sqrt().asin()
}

pub fn parse_actions(text: &str) -> Vec<(String, Value)> {
    let normalized = text
        .replace("```json", "")
        .replace("```JSON", "")
        .replace("```", "");

    let mut actions = Vec::new();
    let mut search_from = 0;
    while let Some(start) = normalized[search_from..].find('{') {
        let abs_start = search_from + start;
        if let Some(end) = find_json_end(&normalized[abs_start..]) {
            let slice = &normalized[abs_start..abs_start + end + 1];
            if let Ok(v) = serde_json::from_str::<Value>(slice) {
                if let Some(action) = v.get("action").and_then(|a| a.as_str()) {
                    let params = v
                        .get("params")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    actions.push((action.to_string(), params));
                }
            }
            search_from = abs_start + end + 1;
        } else {
            break;
        }
    }
    actions
}

fn find_json_end(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn strip_action_blocks(text: &str) -> String {
    let mut result = text.to_string();
    for (action, params) in parse_actions(text) {
        let compact = json!({ "action": action, "params": params }).to_string();
        result = result.replace(&compact, "");

        let pretty = serde_json::to_string_pretty(&json!({ "action": action, "params": params }))
            .unwrap_or_default();
        result = result.replace(&pretty, "");
    }

    result = result
        .replace("```json", "")
        .replace("```JSON", "")
        .replace("```", "");

    result
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}
