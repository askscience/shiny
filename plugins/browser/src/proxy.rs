//! The proxy the browser window is rendered through.
//!
//! Started once, on the plugin's own runtime (the ABI rule from `PLUGINS.md`
//! §15 is explicit that nothing bound to a runtime may cross the `dlopen`
//! boundary, so the plugin owns both the runtime and the listener).
//!
//! The in-app window does not merely *use* this proxy — the proxy **is** the
//! window's origin. That is what makes filtering complete here: the browser
//! asks `http://127.0.0.1:<port>/` for a page, and every subresource is
//! resolved against that origin, so there is no request the filter engine
//! does not see.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use shiny_filter::engine::{AdFilter, FilterConfig};
use shiny_filter::metrics::MetricsSnapshot;
use shiny_filter::proxy::{run_proxy, ProxyConfig, ProxyHandle};
use tokio::sync::RwLock;

/// The one running proxy, shared by every route and tool.
static PROXY: OnceLock<ProxyHandle> = OnceLock::new();

/// Sessions the UI holds open. Kept in memory: they are a view concern, not
/// durable state.
static SESSIONS: OnceLock<Arc<RwLock<Vec<Session>>>> = OnceLock::new();

#[derive(Clone, Debug, serde::Serialize)]
pub struct Session {
    pub id: String,
    /// The URL the session is showing, as the user would name it.
    pub url: String,
    /// Title reported by the injected probe, when known.
    pub title: Option<String>,
    pub blocked_seen: u64,
}

fn sessions() -> &'static Arc<RwLock<Vec<Session>>> {
    SESSIONS.get_or_init(|| Arc::new(RwLock::new(Vec::new())))
}

/// Start the proxy if it is not already running.
///
/// `cache_dir` is the shared filter cache, so the plugin and the native shell
/// reuse one compiled engine instead of each parsing EasyList separately.
pub async fn ensure_started(cache_dir: PathBuf) -> Result<&'static ProxyHandle, String> {
    if let Some(handle) = PROXY.get() {
        return Ok(handle);
    }

    let mut config = FilterConfig::new(cache_dir);
    // A regional list would be appended here; EasyList + EasyPrivacy are the
    // defaults (see `shiny_filter::engine::default_lists`).
    config.extra_filters.push(
        "! pinned: the browser window is never allowed to filter its own UI\n".to_string(),
    );

    let filter = AdFilter::load(config).await;
    let handle = run_proxy(ProxyConfig::default(), filter)
        .await
        .map_err(|e| format!("browser proxy failed to start: {e}"))?;

    // Racing callers are impossible in practice (one plugin, one on_load), but
    // `set` returning Err keeps the invariant explicit rather than silent.
    match PROXY.set(handle) {
        Ok(()) => Ok(PROXY.get().expect("just set")),
        Err(_) => Ok(PROXY.get().expect("set by a racing caller")),
    }
}

/// The running proxy, if it started.
pub fn handle() -> Option<&'static ProxyHandle> {
    PROXY.get()
}

/// Absolute URL to load in order to show `target` inside the window.
pub fn view_url(target: &str) -> Option<String> {
    let handle = PROXY.get()?;
    let proxied = shiny_filter::urls::encode_target(target)?;
    Some(format!("{}{}", handle.base(), proxied))
}

/// Scheme+authority of the proxy.
pub fn base() -> Option<String> {
    PROXY.get().map(|h| h.base().to_string())
}

pub fn metrics() -> Option<MetricsSnapshot> {
    PROXY.get().map(|h| h.metrics().snapshot())
}

/// Whether ad filtering is currently paused.
pub fn is_paused() -> bool {
    PROXY.get().map(|h| h.is_paused()).unwrap_or(false)
}

pub fn addr() -> Option<SocketAddr> {
    PROXY.get().map(|h| h.addr())
}

// ── Sessions ────────────────────────────────────────────────────────────

/// How many sessions are kept.
///
/// Sessions are a *view* concern: the window uses the id it was handed, and
/// never asks for an old one. Nothing ever deleted them, so a long-lived
/// server accumulated one entry per navigation, forever. Keep a generous
/// recent window and let the tail go.
const MAX_SESSIONS: usize = 64;

pub async fn create_session(url: String) -> Session {
    let session = Session {
        id: uuid::Uuid::new_v4().to_string(),
        url,
        title: None,
        blocked_seen: 0,
    };
    let mut all = sessions().write().await;
    all.push(session.clone());
    if all.len() > MAX_SESSIONS {
        let excess = all.len() - MAX_SESSIONS;
        all.drain(0..excess);
    }
    drop(all);
    session
}

pub async fn get_session(id: &str) -> Option<Session> {
    sessions().read().await.iter().find(|s| s.id == id).cloned()
}

pub async fn update_session(id: &str, url: Option<String>, title: Option<String>) -> Option<Session> {
    let mut all = sessions().write().await;
    let session = all.iter_mut().find(|s| s.id == id)?;
    if let Some(url) = url {
        if !url.trim().is_empty() {
            session.url = url;
        }
    }
    if let Some(title) = title {
        if !title.trim().is_empty() {
            session.title = Some(title);
        }
    }
    if let Some(snapshot) = metrics() {
        session.blocked_seen = snapshot.blocked;
    }
    Some(session.clone())
}

pub async fn close_session(id: &str) -> bool {
    let mut all = sessions().write().await;
    let before = all.len();
    all.retain(|s| s.id != id);
    all.len() != before
}

pub async fn list_sessions() -> Vec<Session> {
    sessions().read().await.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_view_url_shape() {
        // The shape the window relies on, independent of the running proxy.
        let encoded = shiny_filter::urls::encode_target("https://example.com/a?b=1").unwrap();
        assert_eq!(encoded, "/p/https/example.com/a?b=1");
    }

    #[tokio::test]
    async fn session_lifecycle() {
        let s = create_session("https://example.com/".into()).await;
        assert!(get_session(&s.id).await.is_some());

        let updated = update_session(&s.id, Some("https://example.com/next".into()), Some("Next".into()))
            .await
            .unwrap();
        assert_eq!(updated.url, "https://example.com/next");
        assert_eq!(updated.title.as_deref(), Some("Next"));

        // An empty update must not wipe existing state.
        let kept = update_session(&s.id, Some(String::new()), None).await.unwrap();
        assert_eq!(kept.url, "https://example.com/next");

        assert!(close_session(&s.id).await);
        assert!(get_session(&s.id).await.is_none());
    }

    #[tokio::test]
    async fn sessions_do_not_grow_without_bound() {
        // One session is created per navigation, so an unbounded list is a
        // slow leak on a long-lived server.
        let first = create_session("https://example.com/first".into()).await;
        for i in 0..MAX_SESSIONS + 10 {
            create_session(format!("https://example.com/{i}")).await;
        }
        let all = list_sessions().await;
        assert!(all.len() <= MAX_SESSIONS, "kept {} sessions", all.len());
        // The oldest are the ones dropped.
        assert!(get_session(&first.id).await.is_none());
        assert!(get_session(&all.last().unwrap().id).await.is_some());
    }
}
