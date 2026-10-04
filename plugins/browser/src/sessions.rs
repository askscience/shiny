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
    /// Owner. Sessions are process-global, so every lookup must be scoped to
    /// the caller; without this any authenticated user could list, retarget or
    /// close another user's tabs.
    #[serde(skip_serializing)]
    pub user_id: String,
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

pub async fn create_session(user_id: &str, url: String) -> Session {
    let session = Session {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
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

pub async fn get_session(user_id: &str, id: &str) -> Option<Session> {
    sessions()
        .read()
        .await
        .iter()
        .find(|s| s.id == id && s.user_id == user_id)
        .cloned()
}

pub async fn update_session(
    user_id: &str,
    id: &str,
    url: Option<String>,
    title: Option<String>,
) -> Option<Session> {
    let mut all = sessions().write().await;
    let session = all.iter_mut().find(|s| s.id == id && s.user_id == user_id)?;
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

pub async fn close_session(user_id: &str, id: &str) -> bool {
    let mut all = sessions().write().await;
    let before = all.len();
    all.retain(|s| !(s.id == id && s.user_id == user_id));
    all.len() != before
}

pub async fn list_sessions(user_id: &str) -> Vec<Session> {
    sessions()
        .read()
        .await
        .iter()
        .filter(|s| s.user_id == user_id)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn session_lifecycle() {
        let s = create_session("u1", "https://example.com/".into()).await;
        assert!(get_session("u1", &s.id).await.is_some());

        let updated = update_session(
            "u1",
            &s.id,
            Some("https://example.com/next".into()),
            Some("Next".into()),
        )
        .await
        .unwrap();
        assert_eq!(updated.url, "https://example.com/next");
        assert_eq!(updated.title.as_deref(), Some("Next"));

        // An empty update must not wipe existing state.
        let kept = update_session("u1", &s.id, Some(String::new()), None).await.unwrap();
        assert_eq!(kept.url, "https://example.com/next");

        assert!(close_session("u1", &s.id).await);
        assert!(get_session("u1", &s.id).await.is_none());
    }

    /// Regression: sessions are process-global, and ownership must be checked.
    #[tokio::test]
    async fn sessions_are_per_user() {
        let a = create_session("alice", "https://example.com/a".into()).await;
        let b = create_session("bob", "https://example.com/b".into()).await;

        let alice = list_sessions("alice").await;
        assert_eq!(alice.len(), 1);
        assert_eq!(alice[0].url, "https://example.com/a");
        assert!(get_session("alice", &b.id).await.is_none(), "cross-user read");
        assert!(update_session("alice", &b.id, Some("https://evil/".into()), None)
            .await
            .is_none(), "cross-user write");
        assert!(!close_session("alice", &b.id).await, "cross-user close");
        assert!(get_session("bob", &b.id).await.is_some());
        assert!(close_session("bob", &b.id).await);
        assert!(!close_session("bob", &a.id).await);
    }

    #[tokio::test]
    async fn sessions_do_not_grow_without_bound() {
        // One session is created per navigation, so an unbounded list is a
        // slow leak on a long-lived server.
        let first = create_session("u1", "https://example.com/first".into()).await;
        for i in 0..MAX_SESSIONS + 10 {
            create_session("u1", format!("https://example.com/{i}")).await;
        }
        let all = list_sessions("u1").await;
        assert!(all.len() <= MAX_SESSIONS, "kept {} sessions", all.len());
        // The oldest are the ones dropped.
        assert!(get_session("u1", &first.id).await.is_none());
        assert!(get_session("u1", &all.last().unwrap().id).await.is_some());
    }
}
