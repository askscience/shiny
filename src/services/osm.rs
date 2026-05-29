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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Shared HTTP client (Overpass, Nominatim, OSRM, etc.).
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("Shiny/0.1 (shiny)")
                .timeout(std::time::Duration::from_secs(30))
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
            .filter_map(|v| parse_geo_place(v))
            .collect();

        // Rate limit: max 1 req/sec for Nominatim
        tokio::time::sleep(tokio::time::Duration::from_millis(1100)).await;

        Ok(places)
    }

    /// Geocode biased toward the user's current position — picks the nearest relevant match.
    pub async fn geocode_near(
        &self,
        query: &str,
        near_lat: f64,
        near_lon: f64,
        limit: Option<usize>,
    ) -> Result<GeoPlace, AppError> {
        let query = normalize_destination_query(query);
        if query.is_empty() {
            return Err(AppError::BadRequest("destination required".into()));
        }

        let limit = limit.unwrap_or(8);
        let url = format!(
            "https://nominatim.openstreetmap.org/search?q={}&format=jsonv2&limit={}&lat={}&lon={}",
            urlencoding(&query),
            limit,
            near_lat,
            near_lon,
        );

        let resp = self.client.get(&url).send().await?;
        let data: Vec<serde_json::Value> = resp.json().await.map_err(|e| {
            AppError::Internal(format!("Failed to parse geocode response: {}", e))
        })?;

        let mut places: Vec<GeoPlace> = data.into_iter().filter_map(parse_geo_place).collect();

        tokio::time::sleep(tokio::time::Duration::from_millis(1100)).await;

        if places.is_empty() {
            return Err(AppError::BadRequest(format!(
                "Could not find \"{}\" on the map",
                query
            )));
        }

        let query_lower = query.to_lowercase();
        places.sort_by(|a, b| {
            score_place(a, &query_lower, near_lat, near_lon)
                .partial_cmp(&score_place(b, &query_lower, near_lat, near_lon))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(places.remove(0))
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
            name: v.get("name").and_then(|n| n.as_str().map(String::from)),
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

        let resp = self.client.get(&url).send().await.map_err(|e| {
            if e.is_connect() {
                AppError::BadRequest(
                    "Driving directions service unreachable. Check your internet connection.".into(),
                )
            } else if e.is_timeout() {
                AppError::BadRequest("Driving directions timed out. Try again.".into())
            } else {
                AppError::Http(e)
            }
        })?;
        if !resp.status().is_success() {
            return Err(AppError::BadRequest(format!(
                "Routing service returned HTTP {}",
                resp.status()
            )));
        }
        let data: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Internal(format!("Failed to parse route response: {}", e))
        })?;

        let code = data["code"].as_str().unwrap_or("Unknown");
        if code != "Ok" {
            let msg = data["message"]
                .as_str()
                .unwrap_or("no route found between these points");
            return Err(AppError::BadRequest(format!("Routing failed: {}", msg)));
        }

        let route = data["routes"]
            .as_array()
            .and_then(|arr| arr.first())
            .ok_or_else(|| AppError::BadRequest("Routing returned no routes".into()))?
            .clone();
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

        if geometry.len() < 2 {
            return Err(AppError::BadRequest(
                "Routing returned empty geometry".into(),
            ));
        }

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
                            name: el["tags"]["name"].as_str().map(String::from),
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

fn parse_geo_place(v: serde_json::Value) -> Option<GeoPlace> {
    Some(GeoPlace {
        display_name: v.get("display_name")?.as_str()?.to_string(),
        lat: v.get("lat")?.as_str()?.parse().ok()?,
        lon: v.get("lon")?.as_str()?.parse().ok()?,
        category: v.get("category").and_then(|c| c.as_str().map(String::from)),
        place_type: v.get("type").and_then(|t| t.as_str().map(String::from)),
        name: v.get("name").and_then(|n| n.as_str().map(String::from)),
    })
}

fn normalize_destination_query(raw: &str) -> String {
    let mut q = raw.trim().trim_matches('"').to_string();
    let lower = q.to_lowercase();
    const PREFIXES: &[&str] = &[
        "please drive me to ",
        "please take me to ",
        "please navigate to ",
        "drive me to ",
        "take me to ",
        "navigate me to ",
        "navigate to ",
        "directions to ",
        "go to ",
        "portami a ",
        "guidami a ",
        "guidami verso ",
        "naviga verso ",
        "naviga a ",
        "indicazioni per ",
        "voglio andare a ",
    ];
    for prefix in PREFIXES {
        if lower.starts_with(prefix) {
            q = q[prefix.len()..].trim().to_string();
            break;
        }
    }
    q.trim_end_matches(|c: char| c.is_ascii_punctuation()).trim().to_string()
}

pub fn place_label(place: &GeoPlace) -> String {
    place
        .name
        .clone()
        .or_else(|| {
            place
                .display_name
                .split(',')
                .next()
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| place.display_name.clone())
}

fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6371.0;
    let d_lat = (lat2 - lat1).to_radians();
    let d_lon = (lon2 - lon1).to_radians();
    let a = (d_lat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (d_lon / 2.0).sin().powi(2);
    2.0 * r * a.sqrt().asin()
}

/// Lower score = better match (distance-weighted, name/type bonuses).
fn score_place(place: &GeoPlace, query: &str, near_lat: f64, near_lon: f64) -> f64 {
    let dist = haversine_km(near_lat, near_lon, place.lat, place.lon);
    let label = place_label(place).to_lowercase();
    let full = place.display_name.to_lowercase();

    let mut score = dist;

    if label == query || full.starts_with(query) {
        score -= 80.0;
    } else if label.contains(query) || full.contains(query) {
        score -= 40.0;
    }

    match place.place_type.as_deref() {
        Some("city" | "town" | "village" | "municipality" | "administrative") => score -= 25.0,
        Some("suburb" | "neighbourhood") => score -= 10.0,
        _ => {}
    }

    if dist > 500.0 {
        score += 200.0;
    }

    score
}
