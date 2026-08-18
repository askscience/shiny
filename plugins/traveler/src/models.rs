//! Row types for the traveler-owned tables (trips, locations, diary_entries).
//! Mirrors the core models; the plugin owns these tables via its migrations.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
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

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
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
    #[allow(clippy::too_many_arguments)]
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

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DiaryEntry {
    pub id: String,
    pub traveler_id: String,
    pub trip_id: Option<String>,
    pub date: String,
    pub title: Option<String>,
    pub content_markdown: String,
    pub summary: Option<String>,
    pub mood: Option<String>,
    pub tags: Option<String>,
    pub auto_generated: Option<i32>,
    pub created_at: Option<String>,
}

impl DiaryEntry {
    pub fn new(
        traveler_id: String,
        trip_id: Option<String>,
        date: String,
        content_markdown: String,
        auto_generated: bool,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            traveler_id,
            trip_id,
            date,
            title: None,
            content_markdown,
            summary: None,
            mood: None,
            tags: None,
            auto_generated: Some(auto_generated as i32),
            created_at: None,
        }
    }
}
