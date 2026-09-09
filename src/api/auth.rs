use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng};
use argon2::Argon2;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::{AuthResponse, LoginRequest, RegisterRequest, Traveler, TravelerPublic};

/// Hash a password with Argon2id, returned as a PHC string
/// (`$argon2id$v=19$…`). Verification accepts both this format and the
/// legacy unsalted SHA-256 hashes of pre-upgrade accounts.
fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::Internal(format!("failed to hash password: {e}")))
}

/// The pre-upgrade scheme: unsalted SHA-256, hex encoded. Kept only so
/// existing accounts can log in and be rehashed on success.
fn legacy_hash_password(password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    hex::encode(hasher.finalize())
}

/// Outcome of checking a password against a stored hash.
enum PasswordCheck {
    Match,
    /// Correct password stored under the legacy scheme — upgrade it.
    MatchNeedsRehash,
    NoMatch,
}

fn verify_password(stored: &str, password: &str) -> PasswordCheck {
    if stored.starts_with("$argon2") {
        let parsed = match PasswordHash::new(stored) {
            Ok(p) => p,
            Err(_) => return PasswordCheck::NoMatch,
        };
        if Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
        {
            PasswordCheck::Match
        } else {
            PasswordCheck::NoMatch
        }
    } else if stored == legacy_hash_password(password) {
        PasswordCheck::MatchNeedsRehash
    } else {
        PasswordCheck::NoMatch
    }
}

/// Build the auth response and attach the `shiny_token` session cookie. The
/// cookie is what lets the browser re-authenticate automatically on the next
/// page load — no reliance on localStorage surviving a reload.
fn session_response(
    token: String,
    traveler: TravelerPublic,
) -> Result<Response, AppError> {
    let cookie = format!("shiny_token={token}; Path=/; SameSite=Lax; Max-Age=31536000; HttpOnly");
    let mut response = Json(AuthResponse { token, traveler }).into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie)
            .map_err(|_| AppError::Internal("failed to build session cookie".into()))?,
    );
    Ok(response)
}

/// Invalidate the caller's session server-side and clear the `shiny_token`
/// cookie. The cookie is `HttpOnly`, so JavaScript cannot remove it — the
/// browser relies on this `Set-Cookie` (Max-Age=0) to drop it. Without this,
/// "log out" would only wipe localStorage and the next page load would
/// auto-restore the session from the cookie.
pub async fn logout(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
) -> Result<Response, AppError> {
    sqlx::query("UPDATE travelers SET auth_token = NULL, updated_at = datetime('now') WHERE id = ?1")
        .bind(&traveler.id)
        .execute(&state.pool)
        .await
        .map_err(AppError::Database)?;

    let mut response = Json(serde_json::json!({ "success": true })).into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_static("shiny_token=; Path=/; SameSite=Lax; Max-Age=0; HttpOnly"),
    );
    Ok(response)
}

fn normalize_username(username: &str) -> String {
    username.trim().to_lowercase()
}

pub(crate) fn validate_username(username: &str) -> Result<(), AppError> {
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

pub(crate) fn validate_avatar(avatar: &Option<String>, allow_empty: bool) -> Result<(), AppError> {
    if let Some(data) = avatar {
        if data.len() > 512_000 {
            return Err(AppError::BadRequest("Profile picture is too large".into()));
        }
        // Travelers may clear their avatar with an empty string; auth profile
        // updates require a real `data:image/...` value.
        if data.is_empty() && allow_empty {
            return Ok(());
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
) -> Result<Response, AppError> {
    let username = normalize_username(&req.username);
    validate_username(&username)?;
    validate_avatar(&req.avatar, false)?;

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
    let traveler = Traveler::new(username.clone(), username.clone(), hash_password(&req.password)?);

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
    }

    let mut public = traveler.to_public();
    public.avatar = req.avatar;
    public.is_admin = is_first_user;

    session_response(token, public)
}

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Response, AppError> {
    let username = normalize_username(&req.username);

    let traveler = sqlx::query_as::<_, Traveler>(
        "SELECT * FROM travelers WHERE username = ?1",
    )
    .bind(&username)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::Database)?
    .ok_or_else(|| AppError::Unauthorized("Invalid username or password".into()))?;

    match verify_password(&traveler.password_hash, &req.password) {
        PasswordCheck::NoMatch => {
            return Err(AppError::Unauthorized("Invalid username or password".into()));
        }
        // Correct password against a legacy unsalted-SHA-256 hash: upgrade
        // the stored hash to Argon2id transparently.
        PasswordCheck::MatchNeedsRehash => {
            match hash_password(&req.password) {
                Ok(argon_hash) => {
                    let res = sqlx::query(
                        "UPDATE travelers SET password_hash = ?1, updated_at = datetime('now') WHERE id = ?2",
                    )
                    .bind(&argon_hash)
                    .bind(&traveler.id)
                    .execute(&state.pool)
                    .await;
                    if let Err(e) = res {
                        tracing::warn!(
                            "Failed to upgrade password hash for user {}: {}",
                            traveler.id,
                            e
                        );
                    }
                }
                Err(e) => tracing::warn!("Failed to rehash legacy password: {e}"),
            }
        }
        PasswordCheck::Match => {}
    }

    // Reuse the account's existing token when one is present so that logging
    // in from a second tab/device does NOT invalidate already-active sessions.
    // Previously every login minted a fresh token and overwrote this single
    // column, which silently kicked every other session back to the login
    // screen. The token is only minted on the very first login.
    let token = traveler
        .auth_token
        .clone()
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    sqlx::query("UPDATE travelers SET auth_token = ?1, updated_at = datetime('now') WHERE id = ?2")
        .bind(&token)
        .bind(&traveler.id)
        .execute(&state.pool)
        .await
        .map_err(AppError::Database)?;

    session_response(token, traveler.to_public())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argon2_hash_round_trips() {
        let stored = hash_password("correct horse battery staple").expect("hash");
        assert!(stored.starts_with("$argon2"), "PHC string expected: {stored}");
        assert!(matches!(
            verify_password(&stored, "correct horse battery staple"),
            PasswordCheck::Match
        ));
        assert!(matches!(
            verify_password(&stored, "wrong password"),
            PasswordCheck::NoMatch
        ));
    }

    #[test]
    fn argon2_hashes_are_salted() {
        let a = hash_password("same password").expect("hash a");
        let b = hash_password("same password").expect("hash b");
        assert_ne!(a, b, "each hash must use a fresh salt");
    }

    #[test]
    fn legacy_sha256_hash_is_detected_for_rehash() {
        let legacy = legacy_hash_password("old-secret");
        assert_eq!(legacy.len(), 64, "hex sha-256");
        assert!(matches!(
            verify_password(&legacy, "old-secret"),
            PasswordCheck::MatchNeedsRehash
        ));
        assert!(matches!(
            verify_password(&legacy, "not-the-password"),
            PasswordCheck::NoMatch
        ));
    }

    #[test]
    fn malformed_argon2_string_is_rejected() {
        assert!(matches!(
            verify_password("$argon2id$not-a-real-hash", "x"),
            PasswordCheck::NoMatch
        ));
    }
}
