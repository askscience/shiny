//! Value type for an artifact card — the dock payload the AI sphere shows.
//!
//! `Artifact` is intentionally generic: new artifact types can be introduced
//! by plugins without a schema migration; the front-end just renders what
//! fields it understands.

use serde::{Deserialize, Serialize};
use serde_json::Value;

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

impl PlanDay {
    pub fn new(day: u32) -> Self {
        Self {
            day,
            title: String::new(),
            items: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanDayItem {
    pub label: String,
    pub value: String,
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