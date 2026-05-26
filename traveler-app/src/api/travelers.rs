use axum::extract::{State, Extension};
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
    if let Some(name) = &req.name {
        sqlx::query("UPDATE travelers SET name = ?1, updated_at = datetime('now') WHERE id = ?2")
            .bind(name)
            .bind(&traveler.id)
            .execute(&state.pool)
            .await
            .map_err(AppError::Database)?;
    }

    if let Some(email) = &req.email {
        let existing = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM travelers WHERE email = ?1 AND id != ?2",
        )
        .bind(email)
        .bind(&traveler.id)
        .fetch_one(&state.pool)
        .await
        .map_err(AppError::Database)?;

        if existing > 0 {
            return Err(AppError::BadRequest("Email already in use".into()));
        }

        sqlx::query("UPDATE travelers SET email = ?1, updated_at = datetime('now') WHERE id = ?2")
            .bind(email)
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
