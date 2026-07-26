use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NavigationSession {
    pub destination: String,
    pub to_lat: f64,
    pub to_lon: f64,
    pub geometry: Vec<[f64; 2]>,
    // Deconstructed `RouteStep` to keep SDK self-contained.
    pub steps: Vec<RouteStepDto>,
    pub distance_km: f64,
    pub duration_min: f64,
    pub profile: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RouteStepDto {
    pub distance: f64,
    pub duration: f64,
    pub instruction: String,
}