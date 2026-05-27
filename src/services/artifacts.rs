use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;

use crate::errors::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Coordinates {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactSection {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactAction {
    pub label: String,
    pub tool: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanDay {
    #[serde(default = "default_plan_day")]
    pub day: u32,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub items: Vec<String>,
}

fn default_plan_day() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteMeta {
    pub distance_km: f64,
    pub duration_min: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    #[serde(rename = "type")]
    pub artifact_type: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coordinates: Option<Coordinates>,
    #[serde(default)]
    pub sections: Vec<ArtifactSection>,
    #[serde(default)]
    pub actions: Vec<ArtifactAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub days: Vec<PlanDay>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<RouteMeta>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub geometry: Vec<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub narrative: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
}

pub fn build_from_params(params: &Value) -> Artifact {
    let id = uuid::Uuid::new_v4().to_string();
    let artifact_type = params
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("site_info")
        .to_string();
    let title = params
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Place")
        .to_string();
    let subtitle = params
        .get("subtitle")
        .and_then(|v| v.as_str())
        .map(String::from);

    let coordinates = params.get("coordinates").and_then(|c| {
        Some(Coordinates {
            lat: c.get("lat")?.as_f64()?,
            lon: c.get("lon")?.as_f64()?,
        })
    });

    let sections: Vec<ArtifactSection> = params
        .get("sections")
        .and_then(|s| s.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    Some(ArtifactSection {
                        label: item.get("label")?.as_str()?.to_string(),
                        value: item.get("value")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let actions: Vec<ArtifactAction> = params
        .get("actions")
        .and_then(|s| s.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    Some(ArtifactAction {
                        label: item.get("label")?.as_str()?.to_string(),
                        tool: item.get("tool")?.as_str()?.to_string(),
                        params: item.get("params").cloned().unwrap_or(Value::Object(Default::default())),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let days: Vec<PlanDay> = params
        .get("days")
        .and_then(|s| s.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    Some(PlanDay {
                        day: item.get("day")?.as_u64()? as u32,
                        title: item
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        items: item
                            .get("items")
                            .and_then(|v| v.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|i| i.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let route = params.get("route").and_then(|r| {
        Some(RouteMeta {
            distance_km: r.get("distance_km")?.as_f64()?,
            duration_min: r.get("duration_min")?.as_f64()?,
        })
    });

    let geometry: Vec<[f64; 2]> = params
        .get("geometry")
        .and_then(|g| g.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|pair| {
                    let a = pair.as_array()?;
                    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
                })
                .collect()
        })
        .unwrap_or_default();

    Artifact {
        id,
        artifact_type,
        title,
        subtitle,
        coordinates,
        sections,
        actions,
        days,
        route,
        geometry,
        narrative: params
            .get("narrative")
            .and_then(|v| v.as_str())
            .map(String::from),
        theme: params
            .get("theme")
            .and_then(|v| v.as_str())
            .map(String::from),
        destination: params
            .get("destination")
            .and_then(|v| v.as_str())
            .map(String::from),
    }
}

pub fn monument_from_place(
    title: &str,
    place: &crate::services::osm::GeoPlace,
    extra: Option<&str>,
) -> Artifact {
    let mut sections = vec![ArtifactSection {
        label: "Location".into(),
        value: place.display_name.clone(),
    }];
    if let Some(e) = extra {
        sections.push(ArtifactSection {
            label: "Info".into(),
            value: e.to_string(),
        });
    }
    Artifact {
        id: uuid::Uuid::new_v4().to_string(),
        artifact_type: "monument_info".into(),
        title: title.to_string(),
        subtitle: Some(place.display_name.clone()),
        coordinates: Some(Coordinates {
            lat: place.lat,
            lon: place.lon,
        }),
        sections,
        actions: vec![ArtifactAction {
            label: "Navigate".into(),
            tool: "map_route".into(),
            params: serde_json::json!({
                "to_lat": place.lat,
                "to_lon": place.lon,
            }),
        }],
        days: vec![],
        route: None,
        geometry: vec![],
        narrative: None,
        theme: None,
        destination: None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactSummary {
    pub id: String,
    #[serde(rename = "type")]
    pub artifact_type: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct ArtifactUpdate {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub sections: Option<Vec<ArtifactSection>>,
    pub actions: Option<Vec<ArtifactAction>>,
    pub coordinates: Option<Coordinates>,
}

#[derive(Debug, sqlx::FromRow)]
struct SavedArtifactRow {
    payload_json: String,
}

pub async fn list_summaries(
    pool: &SqlitePool,
    traveler_id: &str,
) -> Result<Vec<ArtifactSummary>, AppError> {
    let rows = sqlx::query_as::<_, (String, String, String, Option<String>, Option<String>, String)>(
        "SELECT id, artifact_type, title, json_extract(payload_json, '$.theme'), \
         json_extract(payload_json, '$.destination'), updated_at \
         FROM saved_artifacts WHERE traveler_id = ?1 ORDER BY updated_at DESC",
    )
    .bind(traveler_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(id, artifact_type, title, theme, destination, updated_at)| ArtifactSummary {
            id,
            artifact_type,
            title,
            theme,
            destination,
            updated_at,
        })
        .collect())
}

pub async fn load_artifact(
    pool: &SqlitePool,
    traveler_id: &str,
    id: &str,
) -> Result<Artifact, AppError> {
    let row = sqlx::query_as::<_, SavedArtifactRow>(
        "SELECT payload_json FROM saved_artifacts WHERE id = ?1 AND traveler_id = ?2",
    )
    .bind(id)
    .bind(traveler_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Artifact not found".into()))?;

    let mut artifact: Artifact = serde_json::from_str(&row.payload_json)
        .map_err(|e| AppError::Internal(format!("Invalid artifact JSON: {}", e)))?;
    if artifact.days.is_empty() && !artifact.sections.is_empty() {
        artifact.days = sections_to_days(&artifact.sections);
    }
    Ok(artifact)
}

fn parse_day_label(label: &str) -> Option<u32> {
    let upper = label.to_uppercase();
    for prefix in ["GIORNO ", "DAY ", "JOUR ", "DÍA ", "DIA "] {
        if let Some(rest) = upper.strip_prefix(prefix) {
            if let Ok(n) = rest.trim().parse::<u32>() {
                return Some(n);
            }
        }
    }
    None
}

fn value_looks_like_bullets(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.contains('•') {
        return true;
    }
    let lines: Vec<&str> = trimmed.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    if lines.len() < 2 {
        return false;
    }
    let bulletish = lines
        .iter()
        .filter(|l| l.starts_with('-') || l.starts_with('*') || l.chars().take(3).any(|c| c.is_ascii_digit()))
        .count();
    bulletish * 2 >= lines.len()
}

fn sections_to_days(sections: &[ArtifactSection]) -> Vec<PlanDay> {
    let mut days = Vec::new();
    for sec in sections {
        if let Some(day_num) = parse_day_label(&sec.label) {
            let items = if value_looks_like_bullets(&sec.value) {
                sec.value
                    .split(|c| c == '•' || c == '\n' || c == ';')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            } else {
                vec![sec.value.trim().to_string()]
            };
            days.push(PlanDay {
                day: day_num,
                title: sec.label.clone(),
                items,
            });
        } else if let Some(last) = days.last_mut() {
            last.items.push(format!("{}: {}", sec.label, sec.value));
        }
    }
    days
}

pub async fn save_artifact(
    pool: &SqlitePool,
    traveler_id: &str,
    trip_id: Option<&str>,
    artifact: &Artifact,
) -> Result<Artifact, AppError> {
    let payload = serde_json::to_string(artifact)
        .map_err(|e| AppError::Internal(format!("Failed to serialize artifact: {}", e)))?;

    sqlx::query(
        "INSERT INTO saved_artifacts (id, traveler_id, trip_id, artifact_type, title, payload_json, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), datetime('now')) \
         ON CONFLICT(id) DO UPDATE SET \
           trip_id = excluded.trip_id, \
           artifact_type = excluded.artifact_type, \
           title = excluded.title, \
           payload_json = excluded.payload_json, \
           updated_at = datetime('now')",
    )
    .bind(&artifact.id)
    .bind(traveler_id)
    .bind(trip_id)
    .bind(&artifact.artifact_type)
    .bind(&artifact.title)
    .bind(&payload)
    .execute(pool)
    .await?;

    Ok(artifact.clone())
}

pub async fn merge_update(
    pool: &SqlitePool,
    traveler_id: &str,
    id: &str,
    update: ArtifactUpdate,
) -> Result<Artifact, AppError> {
    let mut artifact = load_artifact(pool, traveler_id, id).await?;

    if let Some(title) = update.title {
        artifact.title = title;
    }
    if update.subtitle.is_some() {
        artifact.subtitle = update.subtitle;
    }
    if let Some(sections) = update.sections {
        artifact.sections = sections;
    }
    if let Some(actions) = update.actions {
        artifact.actions = actions;
    }
    if update.coordinates.is_some() {
        artifact.coordinates = update.coordinates;
    }

    save_artifact(pool, traveler_id, None, &artifact).await
}

pub fn poi_list(places: &[crate::services::osm::GeoPlace]) -> Artifact {
    let sections: Vec<ArtifactSection> = places
        .iter()
        .take(8)
        .map(|p| ArtifactSection {
            label: p.display_name.chars().take(40).collect(),
            value: format!("{:.4}, {:.4}", p.lat, p.lon),
        })
        .collect();

    Artifact {
        id: uuid::Uuid::new_v4().to_string(),
        artifact_type: "poi_list".into(),
        title: "Nearby places".into(),
        subtitle: None,
        coordinates: places.first().map(|p| Coordinates {
            lat: p.lat,
            lon: p.lon,
        }),
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
