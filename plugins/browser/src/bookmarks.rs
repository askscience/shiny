//! Per-user bookmarks for the Browser window.
//!
//! Best-effort by design, like [`crate::history`] and [`crate::downloads`]
//! (`PLUGINS.md` §15): a bookmark is a convenience, and a plugin-owned SQLite
//! connection that is unavailable in the running server must never make the
//! window fail to save or open one. Every call here swallows DB errors and
//! returns an empty/neutral result.
//!
//! Rows live in `browser_bookmarks` (migration `006`). Duplicate URLs within
//! one user are collapsed *here* rather than by a UNIQUE constraint so that
//! re-bookmarking a page updates its title instead of raising and losing the
//! edit.

use std::sync::Arc;

use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::services::PluginCtx;

/// The table this module reads and writes.
pub const TABLE: &str = "browser_bookmarks";

/// Ensure the table exists on an install whose migration has not run.
///
/// Idempotent; mirrors [`crate::history::ensure_schema`]. Cheap (one no-op
/// statement).
pub fn ensure_schema(ctx: &PluginCtx) {
    let _ = ctx.db().execute(
        "CREATE TABLE IF NOT EXISTS browser_bookmarks (\
             id TEXT PRIMARY KEY, \
             traveler_id TEXT NOT NULL, \
             url TEXT NOT NULL, \
             title TEXT, \
             created_at TEXT NOT NULL DEFAULT (datetime('now')))",
        &[],
    );
}

/// One saved page.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Bookmark {
    /// Server-generated; the window uses it to remove the row.
    pub id: String,
    pub url: String,
    /// The page title at save time, when the window knew one.
    pub title: Option<String>,
    /// `YYYY-MM-DD HH:MM:SS` (UTC), as SQLite's `datetime('now')` writes it.
    pub created_at: String,
}

/// A user's bookmarks, newest first.
///
/// Returns an empty list on failure, so callers never handle the "database is
/// broken" case separately.
pub fn list(ctx: &Arc<PluginCtx>, user_id: &str, limit: usize) -> Vec<Bookmark> {
    let limit = limit.clamp(1, 200) as i64;
    let sql = format!(
        "SELECT id, url, title, created_at FROM {TABLE}
         WHERE traveler_id = ?1
         ORDER BY created_at DESC, rowid DESC
         LIMIT ?2"
    );
    ctx.db()
        .query(&sql, &[Value::text(user_id.to_string()), Value::Int(limit)])
        .unwrap_or_default()
        .into_iter()
        .filter_map(bookmark_row)
        .collect()
}

/// Save `url` for a user, returning the resulting row.
///
/// Adding a URL that is already saved is not an error: the existing row is
/// kept and its title refreshed when one is supplied, so the toolbar's heart is
/// idempotent. Returns `None` only when the row cannot be written or read back
/// (a broken database, or an empty URL).
pub fn add(
    ctx: &Arc<PluginCtx>,
    user_id: &str,
    url: &str,
    title: Option<&str>,
) -> Option<Bookmark> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let title = title.map(str::trim).filter(|t| !t.is_empty());

    // Dedupe by URL: keep the original id and created_at, refresh the title.
    if let Some(existing) = find_by_url(ctx, user_id, url) {
        let Some(title) = title else {
            return Some(existing);
        };
        let _ = ctx.db().execute(
            &format!("UPDATE {TABLE} SET title = ?1 WHERE id = ?2 AND traveler_id = ?3"),
            &[
                Value::text(title.to_string()),
                Value::text(existing.id.clone()),
                Value::text(user_id.to_string()),
            ],
        );
        return Some(Bookmark {
            title: Some(title.to_string()),
            ..existing
        });
    }

    let id = uuid::Uuid::new_v4().to_string();
    let sql = format!(
        "INSERT INTO {TABLE} (id, traveler_id, url, title, created_at)
         VALUES (?1, ?2, ?3, ?4, datetime('now'))"
    );
    match ctx.db().execute(
        &sql,
        &[
            Value::text(id.clone()),
            Value::text(user_id.to_string()),
            Value::text(url.to_string()),
            match title {
                Some(title) => Value::text(title.to_string()),
                None => Value::Null,
            },
        ],
    ) {
        // Read the row back rather than reconstruct it, so `created_at` is
        // whatever SQLite actually wrote.
        Ok(_) => find_by_id(ctx, user_id, &id),
        Err(err) => {
            tracing::debug!("browser: bookmark not saved: {err}");
            None
        }
    }
}

/// Drop one bookmark by id. `false` means nothing matched (or the DB is
/// unreadable) — never fatal.
pub fn remove(ctx: &Arc<PluginCtx>, user_id: &str, id: &str) -> bool {
    let sql = format!("DELETE FROM {TABLE} WHERE traveler_id = ?1 AND id = ?2");
    matches!(
        ctx.db().execute(
            &sql,
            &[Value::text(user_id.to_string()), Value::text(id.to_string())],
        ),
        Ok(changed) if changed > 0
    )
}

fn find_by_url(ctx: &Arc<PluginCtx>, user_id: &str, url: &str) -> Option<Bookmark> {
    let sql = format!(
        "SELECT id, url, title, created_at FROM {TABLE}
         WHERE traveler_id = ?1 AND url = ?2
         ORDER BY rowid DESC LIMIT 1"
    );
    ctx.db()
        .query(
            &sql,
            &[Value::text(user_id.to_string()), Value::text(url.to_string())],
        )
        .ok()
        .and_then(|rows| rows.into_iter().next())
        .and_then(bookmark_row)
}

fn find_by_id(ctx: &Arc<PluginCtx>, user_id: &str, id: &str) -> Option<Bookmark> {
    let sql = format!(
        "SELECT id, url, title, created_at FROM {TABLE}
         WHERE traveler_id = ?1 AND id = ?2 LIMIT 1"
    );
    ctx.db()
        .query(
            &sql,
            &[Value::text(user_id.to_string()), Value::text(id.to_string())],
        )
        .ok()
        .and_then(|rows| rows.into_iter().next())
        .and_then(bookmark_row)
}

fn bookmark_row(row: Vec<Value>) -> Option<Bookmark> {
    let mut it = row.into_iter();
    let id = value_text(it.next())?;
    let url = value_text(it.next())?;
    let title = value_text(it.next());
    let created_at = value_text(it.next()).unwrap_or_default();
    Some(Bookmark {
        id,
        url,
        title,
        created_at,
    })
}

fn value_text(value: Option<Value>) -> Option<String> {
    match value {
        Some(Value::Text(text)) => Some(text),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(path: &str) -> Arc<PluginCtx> {
        let _ = std::fs::remove_file(path);
        crate::history::tests_support::ctx_for(path)
    }

    #[test]
    fn add_dedupes_by_url_and_refreshes_the_title() {
        let ctx = ctx("/tmp/browser-bookmarks-dedupe.db");
        ensure_schema(&ctx);

        let first = add(&ctx, "u1", "https://example.com/a", Some("Example")).expect("add");
        assert_eq!(first.url, "https://example.com/a");
        assert_eq!(first.title.as_deref(), Some("Example"));

        // The same URL again is not a duplicate: the id is kept and the title
        // is refreshed.
        let again =
            add(&ctx, "u1", "https://example.com/a", Some("Example — renamed")).expect("dedupe");
        assert_eq!(again.id, first.id);
        assert_eq!(again.title.as_deref(), Some("Example — renamed"));
        assert_eq!(list(&ctx, "u1", 10).len(), 1);

        // Re-adding without a title keeps the stored one.
        let untitled = add(&ctx, "u1", "https://example.com/a", None).expect("untitled");
        assert_eq!(untitled.id, first.id);
        assert_eq!(untitled.title.as_deref(), Some("Example — renamed"));
    }

    #[test]
    fn list_remove_and_user_isolation() {
        let ctx = ctx("/tmp/browser-bookmarks-list.db");
        ensure_schema(&ctx);
        let a = add(&ctx, "u1", "https://example.com/a", None).expect("a");
        let b = add(&ctx, "u1", "https://example.com/b", Some("B")).expect("b");
        assert_ne!(a.id, b.id);

        let rows = list(&ctx, "u1", 10);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows.iter().any(|r| r.id == a.id));
        // A different user sees none of them.
        assert!(list(&ctx, "u2", 10).is_empty());

        assert!(remove(&ctx, "u1", &a.id));
        // Removing the same id again matches nothing.
        assert!(!remove(&ctx, "u1", &a.id));
        let rows = list(&ctx, "u1", 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, b.id);
    }

    #[test]
    fn an_empty_url_is_ignored() {
        let ctx = ctx("/tmp/browser-bookmarks-empty.db");
        ensure_schema(&ctx);
        assert!(add(&ctx, "u1", "   ", Some("nope")).is_none());
        assert!(list(&ctx, "u1", 10).is_empty());
    }

    #[test]
    fn a_missing_table_is_not_fatal() {
        // No `ensure_schema`: reads are empty, writes/removes are swallowed.
        let ctx = ctx("/tmp/browser-bookmarks-missing.db");
        assert!(list(&ctx, "u1", 10).is_empty());
        assert!(add(&ctx, "u1", "https://example.com/a", None).is_none());
        assert!(!remove(&ctx, "u1", "nope"));
    }
}
