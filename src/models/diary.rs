use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
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
            auto_generated: Some(if auto_generated { 1 } else { 0 }),
            created_at: None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct DiaryGenerateRequest {
    pub date: Option<String>,
    pub trip_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub message: String,
}
