use axum::extract::{ConnectInfo, Query, State};
use axum::http::header::SET_COOKIE;
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng};
use argon2::Argon2;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use std::net::SocketAddr;

use serde::Deserialize;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::{AuthResponse, LoginRequest, RegisterRequest, Traveler, TravelerPublic};
use crate::services::auth_helper::VerifyOutcome;
use crate::services::unix_user::UnixUser;

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

/// Listing OS accounts is a host capability; the server is reachable on the
/// LAN, so only the machine itself may enumerate it.
fn require_local(remote: &SocketAddr) -> Result<(), AppError> {
    if remote.ip().is_loopback() {
        return Ok(());
    }
    Err(AppError::Unauthorized(
        "the Linux user list is only available from the local machine".into(),
    ))
}

/// `GET /api/auth/unix-users` — the real Linux accounts the login picker can
/// offer. Empty (and `enabled: false`) unless Linux-user mode is on, so the
/// frontend can fall back to its stored profiles.
pub async fn unix_users(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
) -> Result<Response, AppError> {
    require_local(&remote)?;
    let enabled = state.config.linux_users;
    let users: Vec<serde_json::Value> = if enabled {
        crate::services::unix_user::list_human_users()
            .into_iter()
            .map(|u| {
                serde_json::json!({
                    "name": u.name,
                    "display_name": u.display_name(),
                    "uid": u.uid,
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(Json(serde_json::json!({ "enabled": enabled, "users": users })).into_response())
}

/// `GET /api/auth/session?token=…` — the loopback-only kiosk auto-login.
///
/// The server's per-session token (written to `$XDG_RUNTIME_DIR`) is exchanged
/// for the account's durable `shiny_token` cookie and a redirect to the app. It
/// is rejected from any non-loopback peer, so it can never be used over Iroh or
/// the LAN — remote clients still log in with a password.
#[derive(Deserialize)]
pub struct SessionQuery {
    token: Option<String>,
}

pub async fn session_bootstrap(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Query(query): Query<SessionQuery>,
) -> Result<Response, AppError> {
    if !remote.ip().is_loopback() {
        return Err(AppError::Unauthorized("this endpoint is local-only".into()));
    }
    let Some(user_id) = state.session.user_id.clone() else {
        return Err(AppError::Unauthorized("no session account for this machine".into()));
    };
    let provided = query.token.unwrap_or_default();
    if provided.is_empty() || provided != state.session.token {
        return Err(AppError::Unauthorized("invalid session token".into()));
    }

    // Ensure the account has a durable token, then hand it to the browser.
    let durable = Uuid::new_v4().to_string();
    sqlx::query(
        "UPDATE travelers SET auth_token = COALESCE(NULLIF(auth_token, ''), ?1), \
         updated_at = datetime('now') WHERE id = ?2",
    )
    .bind(&durable)
    .bind(&user_id)
    .execute(&state.pool)
    .await
    .map_err(AppError::Database)?;

    let token =
        sqlx::query_scalar::<_, Option<String>>("SELECT auth_token FROM travelers WHERE id = ?1")
            .bind(&user_id)
            .fetch_one(&state.pool)
            .await
            .map_err(AppError::Database)?
            .unwrap_or_default();

    let cookie = format!("shiny_token={token}; Path=/; SameSite=Lax; Max-Age=31536000; HttpOnly");
    let mut response = axum::response::Redirect::to("/").into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie)
            .map_err(|_| AppError::Internal("failed to build session cookie".into()))?,
    );
    Ok(response)
}

/// Resolve the traveler's Linux account and cache `unix_user`/`unix_uid`/
/// `unix_home` on the row. No-op when Linux-user mode is off or the account
/// doesn't exist in NSS.
async fn sync_unix_identity(state: &AppState, traveler: &mut Traveler) {
    if !state.config.linux_users {
        return;
    }
    let Some(name) = traveler
        .unix_user
        .as_deref()
        .or(traveler.username.as_deref())
    else {
        return;
    };
    let Some(user) = crate::services::unix_user::lookup_name(name) else {
        return;
    };
    let changed = traveler.unix_user.as_deref() != Some(user.name.as_str())
        || traveler.unix_home.as_deref() != Some(user.home.as_str())
        || traveler.unix_uid != Some(user.uid as i64);
    if changed {
        let res = sqlx::query(
            "UPDATE travelers SET unix_user = ?1, unix_uid = ?2, unix_home = ?3, \
             updated_at = datetime('now') WHERE id = ?4",
        )
        .bind(&user.name)
        .bind(user.uid as i64)
        .bind(&user.home)
        .bind(&traveler.id)
        .execute(&state.pool)
        .await;
        if let Err(e) = res {
            tracing::warn!("Failed to cache Linux identity for {}: {}", traveler.id, e);
        }
    }
    traveler.unix_user = Some(user.name);
    traveler.unix_uid = Some(user.uid as i64);
    traveler.unix_home = Some(user.home);
}

/// Resolve the OS account named by the login form. Tries the typed value first
/// (NSS is case-sensitive) and then the normalized lowercase form.
fn resolve_os_user(input: &str, normalized: &str) -> Option<UnixUser> {
    crate::services::unix_user::lookup_name(input)
        .or_else(|| crate::services::unix_user::lookup_name(normalized))
        .filter(|u| u.is_human())
}

/// Find the Shiny account bound to a Linux user, provisioning one on first
/// PAM login. The account is keyed by `unix_user` (exact) and `username`
/// (normalized), so an account created earlier by a local registration is
/// adopted rather than duplicated.
async fn find_or_provision_linux_user(
    state: &AppState,
    user: &UnixUser,
) -> Result<Traveler, AppError> {
    let username = normalize_username(&user.name);

    if let Some(existing) = sqlx::query_as::<_, Traveler>(
        "SELECT * FROM travelers WHERE unix_user = ?1 OR username = ?2 LIMIT 1",
    )
    .bind(&user.name)
    .bind(&username)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::Database)?
    {
        return Ok(existing);
    }

    let existing_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM travelers")
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);
    let is_first_user = existing_count == 0;

    let id = Uuid::new_v4().to_string();
    let email = format!("{username}@shiny.local");
    let name = user.display_name();

    sqlx::query(
        "INSERT INTO travelers \
         (id, name, email, password_hash, auth_token, username, avatar, \
          unix_user, unix_uid, unix_home, auth_source, is_admin, created_at, updated_at) \
         VALUES (?1, ?2, ?3, '!pam', NULL, ?4, NULL, ?5, ?6, ?7, 'pam', ?8, datetime('now'), datetime('now'))",
    )
    .bind(&id)
    .bind(&name)
    .bind(&email)
    .bind(&username)
    .bind(&user.name)
    .bind(user.uid as i64)
    .bind(&user.home)
    .bind(is_first_user as i64)
    .execute(&state.pool)
    .await
    .map_err(AppError::Database)?;

    tracing::info!(
        "Provisioned Shiny account for Linux user {} (uid {})",
        user.name,
        user.uid
    );

    // Give plugins a chance to provision per-user state (e.g. the Files home).
    state.plugins.notify_user_registered(&id).await;

    sqlx::query_as::<_, Traveler>("SELECT * FROM travelers WHERE id = ?1")
        .bind(&id)
        .fetch_one(&state.pool)
        .await
        .map_err(AppError::Database)
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
    if state.config.linux_users {
        return Err(AppError::BadRequest(
            "This system signs in with Linux accounts; self-registration is disabled.".into(),
        ));
    }
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

    // Give plugins a chance to provision per-user state for the new account.
    // The Files plugin creates the user's classic home folders here.
    state.plugins.notify_user_registered(&traveler.id).await;

    session_response(token, public)
}

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Response, AppError> {
    let input = req.username.trim().to_string();
    let username = normalize_username(&input);

    // PAM path: when enabled, the real Linux password is authoritative. A
    // denial is final; only an unreachable helper falls back to the local hash.
    let mut os_user = None;
    if state.config.auth_enabled {
        if let Some(user) = resolve_os_user(&input, &username) {
            match crate::services::auth_helper::verify(
                &state.config.auth_sock,
                &user.name,
                &req.password,
            )
            .await
            {
                VerifyOutcome::Ok => os_user = Some(user),
                VerifyOutcome::Denied => {
                    return Err(AppError::Unauthorized("Invalid username or password".into()));
                }
                VerifyOutcome::Unavailable => {
                    tracing::warn!("shiny-auth helper unavailable — trying the local hash");
                }
            }
        }
    }

    let mut traveler = if let Some(user) = &os_user {
        find_or_provision_linux_user(&state, user).await?
    } else {
        sqlx::query_as::<_, Traveler>(
            "SELECT * FROM travelers WHERE username = ?1",
        )
        .bind(&username)
        .fetch_optional(&state.pool)
        .await
        .map_err(AppError::Database)?
        .ok_or_else(|| AppError::Unauthorized("Invalid username or password".into()))?
    };

    // Check the stored hash only when PAM did not already vouch for the login.
    if os_user.is_none() {
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
    }

    // Bind (or refresh) the real Linux account backing this login.
    sync_unix_identity(&state, &mut traveler).await;

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
