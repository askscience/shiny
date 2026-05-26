use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Location {
    pub id: String,
    pub trip_id: Option<String>,
    pub traveler_id: String,
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: Option<f64>,
    pub speed: Option<f64>,
    pub heading: Option<f64>,
    pub accuracy: Option<f64>,
    pub timestamp: Option<String>,
    pub source: String,
}

impl Location {
    pub fn new(
        traveler_id: String,
        trip_id: Option<String>,
        latitude: f64,
        longitude: f64,
        altitude: Option<f64>,
        speed: Option<f64>,
        heading: Option<f64>,
        source: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            trip_id,
            traveler_id,
            latitude,
            longitude,
            altitude,
            speed,
            heading,
            accuracy: None,
            timestamp: None,
            source,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct LocationSubmit {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: Option<f64>,
    pub speed: Option<f64>,
    pub heading: Option<f64>,
    pub trip_id: Option<String>,
}
