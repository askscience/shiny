use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Trip {
    pub id: String,
    pub traveler_id: String,
    pub name: String,
    pub description: Option<String>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub status: String,
    pub created_at: Option<String>,
}

impl Trip {
    pub fn new(traveler_id: String, name: String, description: Option<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            traveler_id,
            name,
            description,
            start_time: None,
            end_time: None,
            status: "planned".into(),
            created_at: None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateTripRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTripRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TripStats {
    pub total_distance_km: f64,
    pub total_duration_hours: f64,
    pub point_count: i64,
    pub avg_speed_kmh: Option<f64>,
    pub start_location: Option<String>,
    pub end_location: Option<String>,
}
