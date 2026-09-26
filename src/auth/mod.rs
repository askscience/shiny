use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use axum::body::{Body, HttpBody};
use axum::extract::{State, Request};
use axum::middleware::Next;
use axum::response::Response;
use axum::http::{HeaderValue, StatusCode};
use bytes::Bytes;
use http::header::AUTHORIZATION;
use sqlx::SqlitePool;

use crate::api::AppState;
use crate::config::Config;
use crate::models::Traveler;

/// A per-session shared secret that lets the **local** kiosk (and the
/// server-mode window) authenticate without a password. It is written to a 0600
/// file in the user's runtime dir at startup and accepted **only from
/// loopback**, so it can never be used over Iroh or the LAN. The web login is
/// untouched — remote clients still present a real password.
#[derive(Clone, Default)]
pub struct SessionAuth {
    pub token: String,
    pub user_id: Option<String>,
}

/// Build the session token and bind it to the account of the OS user running
/// the server.
pub async fn init_session(pool: &SqlitePool, config: &Config) -> SessionAuth {
    let token = uuid::Uuid::new_v4().to_string();
    let user_id = session_user_id(pool).await;
    if user_id.is_some() {
        if let Some(path) = session_token_path(config) {
            match write_secret(&path, &token) {
                Ok(()) => tracing::info!("session token written to {}", path.display()),
                Err(e) => tracing::warn!("could not write session token: {e}"),
            }
        }
    }
    SessionAuth { token, user_id }
}

fn session_token_path(config: &Config) -> Option<PathBuf> {
    if let Some(p) = config
        .session_token_file
        .as_ref()
        .filter(|p| !p.trim().is_empty())
    {
        return Some(PathBuf::from(p));
    }
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|d| !d.is_empty())
        .map(|d| PathBuf::from(d).join("shiny-session-token"))
}

fn write_secret(path: &Path, value: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(value.as_bytes())
}

/// The Shiny account bound to the OS user running this server, if any.
#[cfg(target_os = "linux")]
async fn session_user_id(pool: &SqlitePool) -> Option<String> {
    let uid = unsafe { libc::getuid() };
    let user = crate::services::unix_user::lookup_uid(uid)?;
    if !user.is_human() {
        return None;
    }
    let normalized = user.name.to_lowercase();
    sqlx::query_scalar::<_, String>(
        "SELECT id FROM travelers WHERE unix_user = ?1 OR username = ?2 LIMIT 1",
    )
    .bind(&user.name)
    .bind(&normalized)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

#[cfg(not(target_os = "linux"))]
async fn session_user_id(_pool: &SqlitePool) -> Option<String> {
    None
}

fn is_loopback<B>(req: &Request<B>) -> bool {
    req.extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip().is_loopback())
        .unwrap_or(false)
}

pub async fn auth_middleware<B>(
    State(state): State<AppState>,
    req: Request<B>,
    next: Next,
) -> Result<Response, StatusCode>
where
    B: HttpBody<Data = Bytes> + Send + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let auth_header = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|v| v.to_string());

    let cookie_token = req
        .headers()
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(extract_session_cookie);

    // Try the Bearer token first, then the `shiny_token` session cookie. A
    // valid cookie rescues a request whose localStorage token has gone stale,
    // and keeps a user signed in across page reloads.
    let mut traveler = None;
    for candidate in [auth_header.as_ref(), cookie_token.as_ref()]
        .into_iter()
        .flatten()
    {
        match sqlx::query_as::<_, Traveler>("SELECT * FROM travelers WHERE auth_token = ?1")
            .bind(candidate)
            .fetch_optional(&state.pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        {
            Some(t) => {
                traveler = Some(t);
                break;
            }
            None => continue,
        }
    }

    // The loopback-only session token: the local kiosk's auto-trust. Never valid
    // from elsewhere.
    if traveler.is_none() {
        if let Some(user_id) = state.session.user_id.as_deref() {
            let presented = auth_header.as_deref().or(cookie_token.as_deref());
            if presented == Some(state.session.token.as_str()) && is_loopback(&req) {
                traveler = sqlx::query_as::<_, Traveler>("SELECT * FROM travelers WHERE id = ?1")
                    .bind(user_id)
                    .fetch_optional(&state.pool)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            }
        }
    }

    let traveler = traveler.ok_or(StatusCode::UNAUTHORIZED)?;

    let user_id = traveler.id.clone();
    // Resolve the bound Linux account once, before the identity is moved into
    // request extensions, so plugins get the real home as headers.
    let os_identity = state.os_identity_for(&traveler);

    let mut req = req.map(Body::new);
    {
        // Portable identity for plugin route handlers (they can't reference
        // the core `Traveler` type). Hand the id over as request headers:
        // `http::Extensions` keys by `TypeId`, and each plugin statically
        // links its own copy of the SDK, so a `UserId` inserted here has a
        // different `TypeId` than the one the plugin looks up. Header names
        // are matched by string comparison and cross the dlopen boundary.
        if let Ok(value) = HeaderValue::from_str(&user_id) {
            let headers = req.headers_mut();
            headers.insert(shiny_plugin_sdk::routes::USER_ID_HEADER, value.clone());
            headers.insert(shiny_plugin_sdk::routes::TRAVELER_ID_HEADER, value);
        }

        if let Some(os) = &os_identity {
            let headers = req.headers_mut();
            if let Ok(v) = HeaderValue::from_str(&os.name) {
                headers.insert(shiny_plugin_sdk::routes::OS_USER_HEADER, v);
            }
            if let Ok(v) = HeaderValue::from_str(&os.home) {
                headers.insert(shiny_plugin_sdk::routes::OS_HOME_HEADER, v);
            }
            if let Ok(v) = HeaderValue::from_str(&os.uid.to_string()) {
                headers.insert(shiny_plugin_sdk::routes::OS_UID_HEADER, v);
            }
        }

        let extensions = req.extensions_mut();
        extensions.insert(shiny_plugin_sdk::routes::UserId(user_id.clone()));
        extensions.insert(shiny_plugin_sdk::routes::TravelerId(user_id));
        extensions.insert(traveler);
    }

    Ok(next.run(req).await)
}

/// Extract the `shiny_token` value from a raw `Cookie` header, if present.
fn extract_session_cookie(cookie_header: &str) -> Option<String> {
    cookie_header.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == "shiny_token" && !value.is_empty()).then(|| value.to_string())
    })
}
