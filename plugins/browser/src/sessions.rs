//! Browser-window sessions.
//!
//! A session is a *view* concern, not durable state: the window creates one per
//! tab and reports its URL so the tab strip can be listed. Nothing here touches
//! the network or the filter engine; the page is rendered by a native child
//! webview owned by the shell (`crates/peakd`'s `browse` module).

use std::sync::{Arc, OnceLock};

use tokio::sync::RwLock;

#[derive(Clone, Debug, serde::Serialize)]
pub struct Session {
    pub id: String,
    /// The URL the session is showing, as the user would name it.
    pub url: String,
    /// Title reported by the page, when known.
    pub title: Option<String>,
    /// Retained for the window's status line; the shell no longer reports a
    /// blocked count, so this stays at whatever the last update set.
    pub blocked_seen: u64,
}

static SESSIONS: OnceLock<Arc<RwLock<Vec<Session>>>> = OnceLock::new();

fn sessions() -> &'static Arc<RwLock<Vec<Session>>> {
    SESSIONS.get_or_init(|| Arc::new(RwLock::new(Vec::new())))
}

/// How many sessions are kept.
///
/// Sessions are a *view* concern: the window uses the id it was handed and
/// never asks for an old one. A long-lived server used to accumulate one entry
/// per navigation forever, so keep a generous recent window and drop the tail.
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

pub async fn update_session(
    id: &str,
    url: Option<String>,
    title: Option<String>,
) -> Option<Session> {
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

    #[tokio::test]
    async fn session_lifecycle() {
        let s = create_session("https://example.com/".into()).await;
        assert!(get_session(&s.id).await.is_some());

        let updated = update_session(
            &s.id,
            Some("https://example.com/next".into()),
            Some("Next".into()),
        )
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
