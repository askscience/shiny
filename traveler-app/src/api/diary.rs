use axum::extract::{Path, State, Extension, Query};
use axum::Json;
use serde::{Serialize, Deserialize};

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::{DiaryEntry, Traveler};

#[derive(Serialize)]
pub struct DiaryListResponse {
    pub success: bool,
    pub data: Vec<DiaryEntry>,
}

#[derive(Serialize)]
pub struct DiaryResponse {
    pub success: bool,
    pub data: DiaryEntry,
}

#[derive(Serialize)]
pub struct DiaryGenerateResponse {
    pub success: bool,
    pub message: String,
    pub data: Option<DiaryEntry>,
}

#[derive(Deserialize)]
pub struct DiaryQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct DiarySearchQuery {
    pub q: String,
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct DiaryGenerateBody {
    pub date: Option<String>,
    pub trip_id: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Query(params): Query<DiaryQuery>,
) -> Result<Json<DiaryListResponse>, AppError> {
    let limit = params.limit.unwrap_or(50);

    let entries = if let (Some(from), Some(to)) = (&params.from, &params.to) {
        sqlx::query_as::<_, DiaryEntry>(
            "SELECT * FROM diary_entries WHERE traveler_id = ?1 AND date >= ?2 AND date <= ?3 \
             ORDER BY date DESC LIMIT ?4",
        )
        .bind(&traveler.id)
        .bind(from)
        .bind(to)
        .bind(limit)
        .fetch_all(&state.pool)
        .await
        .map_err(AppError::Database)?
    } else {
        sqlx::query_as::<_, DiaryEntry>(
            "SELECT * FROM diary_entries WHERE traveler_id = ?1 \
             ORDER BY date DESC LIMIT ?2",
        )
        .bind(&traveler.id)
        .bind(limit)
        .fetch_all(&state.pool)
        .await
        .map_err(AppError::Database)?
    };

    Ok(Json(DiaryListResponse {
        success: true,
        data: entries,
    }))
}

pub async fn get_by_date(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Path(date): Path<String>,
) -> Result<Json<DiaryResponse>, AppError> {
    let entry = sqlx::query_as::<_, DiaryEntry>(
        "SELECT * FROM diary_entries WHERE traveler_id = ?1 AND date = ?2",
    )
    .bind(&traveler.id)
    .bind(&date)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::Database)?
    .ok_or_else(|| AppError::NotFound("No diary entry for this date".into()))?;

    Ok(Json(DiaryResponse {
        success: true,
        data: entry,
    }))
}

pub async fn search(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Query(params): Query<DiarySearchQuery>,
) -> Result<Json<DiaryListResponse>, AppError> {
    let limit = params.limit.unwrap_or(20);

    let entries = sqlx::query_as::<_, DiaryEntry>(
        "SELECT * FROM diary_entries WHERE traveler_id = ?1 AND \
         (content_markdown LIKE ?2 OR title LIKE ?2 OR summary LIKE ?2 OR tags LIKE ?2) \
         ORDER BY date DESC LIMIT ?3",
    )
    .bind(&traveler.id)
    .bind(format!("%{}%", params.q))
    .bind(limit)
    .fetch_all(&state.pool)
    .await
    .map_err(AppError::Database)?;

    Ok(Json(DiaryListResponse {
        success: true,
        data: entries,
    }))
}

pub async fn generate(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(body): Json<DiaryGenerateBody>,
) -> Result<Json<DiaryGenerateResponse>, AppError> {
    let date = body.date.unwrap_or_else(|| {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    });

    match state
        .diary_gen
        .generate_for_date(&traveler.id, &date)
        .await
    {
        Ok(entry) => Ok(Json(DiaryGenerateResponse {
            success: true,
            message: "Diary entry generated".into(),
            data: Some(entry),
        })),
        Err(e) => Ok(Json(DiaryGenerateResponse {
            success: false,
            message: format!("Failed to generate diary: {}", e),
            data: None,
        })),
    }
}
