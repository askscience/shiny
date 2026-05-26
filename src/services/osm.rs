use serde::{Deserialize, Serialize};
use crate::errors::AppError;

#[derive(Clone)]
pub struct OsmService {
    client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoPlace {
    pub display_name: String,
    pub lat: f64,
    pub lon: f64,
    pub category: Option<String>,
    pub place_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteStep {
    pub distance: f64,
    pub duration: f64,
    pub instruction: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteResult {
    pub total_distance_meters: f64,
    pub total_duration_seconds: f64,
    pub steps: Vec<RouteStep>,
    pub geometry: Vec<[f64; 2]>,
}

impl OsmService {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("Shiny/0.1 (shiny)")
                .build()
                .unwrap(),
        }
    }

    pub async fn geocode(&self, query: &str, limit: Option<usize>) -> Result<Vec<GeoPlace>, AppError> {
        let limit = limit.unwrap_or(5);
        let url = format!(
            "https://nominatim.openstreetmap.org/search?q={}&format=jsonv2&limit={}",
            urlencoding(query),
            limit
        );

        let resp = self.client.get(&url).send().await?;
        let data: Vec<serde_json::Value> = resp.json().await.map_err(|e| {
            AppError::Internal(format!("Failed to parse geocode response: {}", e))
        })?;

        let places = data
            .into_iter()
            .filter_map(|v| {
                Some(GeoPlace {
                    display_name: v.get("display_name")?.as_str()?.to_string(),
                    lat: v.get("lat")?.as_str()?.parse().ok()?,
                    lon: v.get("lon")?.as_str()?.parse().ok()?,
                    category: v.get("category").and_then(|c| c.as_str().map(String::from)),
                    place_type: v.get("type").and_then(|t| t.as_str().map(String::from)),
                })
            })
            .collect();

        // Rate limit: max 1 req/sec for Nominatim
        tokio::time::sleep(tokio::time::Duration::from_millis(1100)).await;

        Ok(places)
    }

    pub async fn reverse_geocode(&self, lat: f64, lon: f64) -> Result<GeoPlace, AppError> {
        let url = format!(
            "https://nominatim.openstreetmap.org/reverse?lat={}&lon={}&format=jsonv2",
            lat, lon
        );

        let resp = self.client.get(&url).send().await?;
        let v: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Internal(format!("Failed to parse reverse geocode response: {}", e))
        })?;

        let place = GeoPlace {
            display_name: v
                .get("display_name")
                .and_then(|d| d.as_str())
                .unwrap_or("Unknown")
                .to_string(),
            lat: lat,
            lon: lon,
            category: v.get("category").and_then(|c| c.as_str().map(String::from)),
            place_type: v.get("type").and_then(|t| t.as_str().map(String::from)),
        };

        tokio::time::sleep(tokio::time::Duration::from_millis(1100)).await;

        Ok(place)
    }

    pub async fn route(
        &self,
        from_lat: f64,
        from_lon: f64,
        to_lat: f64,
        to_lon: f64,
        profile: &str,
    ) -> Result<RouteResult, AppError> {
        let url = format!(
            "https://router.project-osrm.org/route/v1/{}/{},{};{},{}?steps=true&geometries=geojson&overview=full",
            profile, from_lon, from_lat, to_lon, to_lat
        );

        let resp = self.client.get(&url).send().await?;
        let data: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Internal(format!("Failed to parse route response: {}", e))
        })?;

        let route = data["routes"][0].clone();
        let legs = route["legs"][0].clone();

        let total_distance = route["distance"].as_f64().unwrap_or(0.0);
        let total_duration = route["duration"].as_f64().unwrap_or(0.0);

        let steps: Vec<RouteStep> = legs["steps"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|s| RouteStep {
                        distance: s["distance"].as_f64().unwrap_or(0.0),
                        duration: s["duration"].as_f64().unwrap_or(0.0),
                        instruction: s["maneuver"]["instruction"]
                            .as_str()
                            .unwrap_or("")
                            .to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let geometry: Vec<[f64; 2]> = route["geometry"]["coordinates"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| {
                        let lon = c[0].as_f64()?;
                        let lat = c[1].as_f64()?;
                        Some([lat, lon])
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(RouteResult {
            total_distance_meters: total_distance,
            total_duration_seconds: total_duration,
            steps,
            geometry,
        })
    }

    pub async fn nearby_poi(
        &self,
        lat: f64,
        lon: f64,
        radius: f64,
        amenity: Option<&str>,
    ) -> Result<Vec<GeoPlace>, AppError> {
        let amenity_filter = match amenity {
            Some(a) => format!("[amenity={}]", a),
            None => String::new(),
        };

        let overpass_query = format!(
            "[out:json];node{}(around:{},{},{});out {};",
            amenity_filter, radius as u64, lat, lon, 10
        );

        let url = "https://overpass-api.de/api/interpreter";
        let resp = self
            .client
            .post(url)
            .body(overpass_query)
            .send()
            .await?;

        let data: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Internal(format!("Failed to parse Overpass response: {}", e))
        })?;

        let places: Vec<GeoPlace> = data["elements"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|el| {
                        Some(GeoPlace {
                            display_name: el["tags"]["name"]
                                .as_str()
                                .unwrap_or("Unnamed POI")
                                .to_string(),
                            lat: el["lat"].as_f64()?,
                            lon: el["lon"].as_f64()?,
                            category: el["tags"]["amenity"]
                                .as_str()
                                .map(String::from)
                                .or_else(|| el["tags"]["tourism"].as_str().map(String::from)),
                            place_type: Some("poi".into()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(places)
    }
}

fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ' ' => "%20".to_string(),
            _ => format!("%{:02X}", c as u8),
        })
        .collect()
}
