use axum::extract::State;
use axum::Json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::{AuthResponse, LoginRequest, RegisterRequest, Traveler};

fn hash_password(password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    hex::encode(hasher.finalize())
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
        if !data.starts_with("data:image/") {
            return Err(AppError::BadRequest("Invalid profile picture format".into()));
        }
    }
    Ok(())
}

pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    let username = normalize_username(&req.username);
    validate_username(&username)?;
    validate_avatar(&req.avatar)?;

    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM travelers WHERE username = ?1",
    )
    .bind(&username)
    .fetch_one(&state.pool)
    .await
    .map_err(AppError::Database)?;

    if existing > 0 {
        return Err(AppError::BadRequest("Username already taken".into()));
    }

    let token = Uuid::new_v4().to_string();
    let traveler = Traveler::new(username.clone(), username.clone(), hash_password(&req.password));

    sqlx::query(
        "INSERT INTO travelers (id, name, email, password_hash, auth_token, username, avatar, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'), datetime('now'))",
    )
    .bind(&traveler.id)
    .bind(&traveler.name)
    .bind(&traveler.email)
    .bind(&traveler.password_hash)
    .bind(&token)
    .bind(&username)
    .bind(&req.avatar)
    .execute(&state.pool)
    .await
    .map_err(AppError::Database)?;

    // First registered traveler becomes admin automatically. This makes the
    // Plugins UI usable out of the box without setting ADMIN_TOKEN env.
    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM travelers")
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);
    let is_first_user = user_count == 1;
    if is_first_user {
        sqlx::query("UPDATE travelers SET is_admin = 1 WHERE id = ?1")
            .bind(&traveler.id)
            .execute(&state.pool)
            .await
            .ok();
        let _ = sqlx::query("UPDATE travelers SET is_admin = 1 WHERE id = ?1")
            .bind(&traveler.id)
            .execute(&state.pool)
            .await;
    }

    let mut public = traveler.to_public();
    public.avatar = req.avatar;
    public.is_admin = is_first_user;

    Ok(Json(AuthResponse {
        token,
        traveler: public,
    }))
}

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    let username = normalize_username(&req.username);

    let traveler = sqlx::query_as::<_, Traveler>(
        "SELECT * FROM travelers WHERE username = ?1",
    )
    .bind(&username)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::Database)?
    .ok_or_else(|| AppError::Unauthorized("Invalid username or password".into()))?;

    if traveler.password_hash != hash_password(&req.password) {
        return Err(AppError::Unauthorized("Invalid username or password".into()));
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
