use axum::extract::State;
use axum::Json;
use sha2::{Sha256, Digest};
use uuid::Uuid;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::{AuthResponse, LoginRequest, RegisterRequest, Traveler};

fn hash_password(password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    hex::encode(hasher.finalize())
}

pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM travelers WHERE email = ?1",
    )
    .bind(&req.email)
    .fetch_one(&state.pool)
    .await
    .map_err(AppError::Database)?;

    if existing > 0 {
        return Err(AppError::BadRequest("Email already registered".into()));
    }

    let token = Uuid::new_v4().to_string();
    let traveler = Traveler::new(
        req.name,
        req.email,
        hash_password(&req.password),
    );

    sqlx::query(
        "INSERT INTO travelers (id, name, email, password_hash, auth_token, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'), datetime('now'))",
    )
    .bind(&traveler.id)
    .bind(&traveler.name)
    .bind(&traveler.email)
    .bind(&traveler.password_hash)
    .bind(&token)
    .execute(&state.pool)
    .await
    .map_err(AppError::Database)?;

    Ok(Json(AuthResponse {
        token,
        traveler: traveler.to_public(),
    }))
}

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    let traveler = sqlx::query_as::<_, Traveler>(
        "SELECT * FROM travelers WHERE email = ?1",
    )
    .bind(&req.email)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::Database)?
    .ok_or_else(|| AppError::Unauthorized("Invalid email or password".into()))?;

    if traveler.password_hash != hash_password(&req.password) {
        return Err(AppError::Unauthorized("Invalid email or password".into()));
    }

    let token = Uuid::new_v4().to_string();

    sqlx::query("UPDATE travelers SET auth_token = ?1, updated_at = datetime('now') WHERE id = ?2")
        .bind(&token)
        .bind(&traveler.id)
        .execute(&state.pool)
        .await
        .map_err(AppError::Database)?;

    Ok(Json(AuthResponse {
        token,
        traveler: traveler.to_public(),
    }))
}
