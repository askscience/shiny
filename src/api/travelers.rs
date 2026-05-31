use axum::extract::{Extension, State};
use axum::Json;
use serde::Serialize;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::{Traveler, TravelerPublic, UpdateTravelerRequest};

#[derive(Serialize)]
pub struct TravelerResponse {
    success: bool,
    data: TravelerPublic,
}

fn normalize_username(username: &str) -> String {
    username.trim().to_lowercase()
}

fn validate_username(username: &str) -> Result<(), AppError> {
    if username.len() < 2 {
        return Err(AppError::BadRequest("Username must be at least 2 characters".into()));
    }
    if username.len() > 32 {
        return Err(AppError::BadRequest("Username must be 32 characters or fewer".into()));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AppError::BadRequest(
            "Username may only contain letters, numbers, underscores, and hyphens".into(),
        ));
    }
    Ok(())
}

fn validate_avatar(avatar: &Option<String>) -> Result<(), AppError> {
    if let Some(data) = avatar {
        if data.len() > 512_000 {
            return Err(AppError::BadRequest("Profile picture is too large".into()));
        }
        if !data.is_empty() && !data.starts_with("data:image/") {
            return Err(AppError::BadRequest("Invalid profile picture format".into()));
        }
    }
    Ok(())
}

pub async fn get_me(
    Extension(traveler): Extension<Traveler>,
) -> Json<TravelerResponse> {
    Json(TravelerResponse {
        success: true,
        data: traveler.to_public(),
    })
}

pub async fn update_me(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(req): Json<UpdateTravelerRequest>,
) -> Result<Json<TravelerResponse>, AppError> {
    validate_avatar(&req.avatar)?;

    if let Some(name) = &req.name {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::BadRequest("Name cannot be empty".into()));
        }
        sqlx::query("UPDATE travelers SET name = ?1, updated_at = datetime('now') WHERE id = ?2")
            .bind(name)
            .bind(&traveler.id)
            .execute(&state.pool)
            .await
            .map_err(AppError::Database)?;
    }

    if let Some(username) = &req.username {
        let username = normalize_username(username);
        validate_username(&username)?;

        let existing = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM travelers WHERE username = ?1 AND id != ?2",
        )
        .bind(&username)
        .bind(&traveler.id)
        .fetch_one(&state.pool)
        .await
        .map_err(AppError::Database)?;

        if existing > 0 {
            return Err(AppError::BadRequest("Username already taken".into()));
        }

        sqlx::query(
            "UPDATE travelers SET username = ?1, email = ?2, updated_at = datetime('now') WHERE id = ?3",
        )
        .bind(&username)
        .bind(format!("{username}@shiny.local"))
        .bind(&traveler.id)
        .execute(&state.pool)
        .await
        .map_err(AppError::Database)?;
    }

    if let Some(avatar) = &req.avatar {
        let value = if avatar.is_empty() { None } else { Some(avatar.as_str()) };
        sqlx::query("UPDATE travelers SET avatar = ?1, updated_at = datetime('now') WHERE id = ?2")
            .bind(value)
            .bind(&traveler.id)
            .execute(&state.pool)
            .await
            .map_err(AppError::Database)?;
    }

    let updated = sqlx::query_as::<_, Traveler>(
        "SELECT * FROM travelers WHERE id = ?1",
    )
    .bind(&traveler.id)
    .fetch_one(&state.pool)
    .await
    .map_err(AppError::Database)?;

    Ok(Json(TravelerResponse {
        success: true,
        data: updated.to_public(),
    }))
}
