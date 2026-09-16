//! Agent tools for the browser plugin.
//!
//! These let the AI *drive* the browser window: open a page, search the web,
//! or read a page's text back into the conversation. All three go through the
//! same filter proxy the window renders through, so what the model reads is
//! what the user would see with the ads already gone.

use async_trait::async_trait;
use serde_json::{json, Value};

use shiny_plugin_sdk::artifacts::Artifact;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::proxy;
use crate::routes::{normalize_input, Target};


/// A card the window reacts to: it carries the URL to load and shows up in the
/// artifact dock as a record of the visit.
///
/// Returning an artifact also makes core focus this window after an AI-driven
/// navigation (`agent_runner` surfaces the window of the plugin that produced
/// the turn's only artifact).
///
/// The URL travels in **`narrative`**, which looks odd until you see why: the
/// window used to read it back from `payload.url`, a key that does not exist in
/// the saved payload, so the AI could open a site and the window would silently
/// stay on its home page. A browser card's `title` *is* the URL too, but parsing
/// a display title to recover a machine value is how that kind of bug survives
/// its next refactor. `narrative` is the only free-form string field on the
/// artifact, and it is carried through storage untouched.
fn page_artifact(title: &str, url: &str, note: Option<&str>) -> Artifact {
    Artifact {
        id: uuid::Uuid::new_v4().to_string(),
        artifact_type: "browser_page".into(),
        title: title.to_string(),
        subtitle: note.map(str::to_string),
        coordinates: None,
        sections: vec![],
        actions: vec![],
        days: vec![],
        route: None,
        geometry: vec![],
        narrative: Some(url.to_string()),
        theme: None,
        destination: None,
    }
}

/// Resolve raw model input to an absolute URL, searching when it is a phrase.
fn resolve_target(input: &str) -> String {
    let searxng = std::env::var("SEARXNG_URL").ok();
    normalize_input(input).into_url(searxng.as_deref())
}

/// The view URL inside the window, or a clear error when the proxy is down.
fn view_url_for(url: &str) -> Result<String, AppError> {
    proxy::view_url(url).ok_or_else(|| {
        AppError::Internal("the browser proxy is still starting — try again in a moment".into())
    })
}

/* ── browser_open ───────────────────────────────────────────── */

pub struct BrowserOpen;

#[async_trait]
impl Tool for BrowserOpen {
    fn name(&self) -> &str {
        "browser_open"
    }

    fn aliases(&self) -> &[&str] {
        &["open_browser", "browse", "browser_goto"]
    }

    fn step_label(&self) -> &str {
        "Opening in the browser…"
    }

    fn doc_fragment(&self) -> Option<&str> {
        Some("- `browser_open` — Open a page in the Shiny browser window. params: `{ url: string }` — accepts a full URL or a search phrase (`\"rust webview\"` searches instead). The page is fetched through the built-in adblock engine.")
    }

    fn humanize(&self, _r: &str, data: &Value) -> String {
        let url = data.get("url").and_then(|v| v.as_str()).unwrap_or("the page");
        format!("Opened {url} in the browser")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let raw = req
            .params
            .param_str("url")
            .or_else(|| req.params.param_str("query"))
            .or_else(|| req.params.param_str("q"))
            .ok_or_else(|| AppError::BadRequest("url required".into()))?;

        let url = resolve_target(&raw);
        let view_url = view_url_for(&url)?;

        let target = normalize_input(&raw);
        let title = match &target {
            Target::Search(q) => format!("Search: {q}"),
            Target::Url(u) => u.clone(),
        };

        // An AI-driven navigation is a navigation: record it for history and
        // for the home surface's interest profile, exactly like the address
        // bar's route does. Best-effort — a broken DB must not fail the tool.
        let query = match &target {
            Target::Search(q) if !q.trim().is_empty() => Some(q.trim()),
            _ => None,
        };
        let _ = crate::history::record(ctx, req.user_id, &url, Some("page"), query).await;

        Ok(ActionOutcome::ok(
            "browser_open",
            json!({
                "url": url,
                "view_url": view_url,
                "blocked_rules": proxy::handle().map(|h| h.filter.rule_count()).unwrap_or(0),
            }),
        )
        .with_artifact(page_artifact(&title, &url, Some("Filtered by shiny-filter"))))
    }
}

/* ── browser_search ─────────────────────────────────────────── */

pub struct BrowserSearch;

#[async_trait]
impl Tool for BrowserSearch {
    fn name(&self) -> &str {
        "browser_search"
    }

    fn aliases(&self) -> &[&str] {
        &["search_in_browser", "browse_search"]
    }

    fn step_label(&self) -> &str {
        "Searching the web…"
    }

    fn doc_fragment(&self) -> Option<&str> {
        Some("- `browser_search` — Search the web in the browser window. params: `{ query: string }` — uses the configured SearXNG instance when `SEARXNG_URL` is set, otherwise Brave Search. Prefer this over `browser_open` when the user asks to look something up.")
    }

    fn humanize(&self, _r: &str, data: &Value) -> String {
        let q = data.get("query").and_then(|v| v.as_str()).unwrap_or("the web");
        format!("Searched the browser for “{q}”")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let query = req
            .params
            .param_str("query")
            .or_else(|| req.params.param_str("q"))
            .ok_or_else(|| AppError::BadRequest("query required".into()))?;

        let url = resolve_target(&query);
        let view_url = view_url_for(&url)?;

        // A search is the strongest interest signal there is: record the words
        // the user asked for, not just the engine URL they became.
        let _ = crate::history::record(ctx, req.user_id, &url, Some("page"), Some(&query)).await;

        Ok(ActionOutcome::ok(
            "browser_search",
            json!({ "query": query, "url": url, "view_url": view_url }),
        )
        .with_artifact(page_artifact(
            &format!("Search: {query}"),
            &url,
            Some("Results in the browser window"),
        )))
    }
}

/* ── browser_read ───────────────────────────────────────────── */

pub struct BrowserRead;

#[async_trait]
impl Tool for BrowserRead {
    fn name(&self) -> &str {
        "browser_read"
    }

    fn aliases(&self) -> &[&str] {
        &["read_page", "browser_extract"]
    }

    fn step_label(&self) -> &str {
        "Reading the page…"
    }

    fn doc_fragment(&self) -> Option<&str> {
        Some("- `browser_read` — Fetch a page through the adblock engine and return its readable text, without showing it. params: `{ url: string, max_chars?: number }` — use this when you need the page's contents to answer a question.")
    }

    fn humanize(&self, _r: &str, data: &Value) -> String {
        let chars = data.get("chars").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Read {chars} characters from the page")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let raw = req
            .params
            .param_str("url")
            .or_else(|| req.params.param_str("query"))
            .ok_or_else(|| AppError::BadRequest("url required".into()))?;

        let url = resolve_target(&raw);
        let view_url = view_url_for(&url)?;
        let max_chars = req.params.param_u32("max_chars").unwrap_or(20_000) as usize;

        let text = crate::fetch::text(&view_url).await?;
        let truncated = text.chars().count() > max_chars;
        let text: String = if truncated {
            text.chars().take(max_chars).collect()
        } else {
            text
        };

        Ok(ActionOutcome::ok(
            "browser_read",
            json!({
                "url": url,
                "text": text,
                "chars": text.chars().count(),
                "truncated": truncated,
            }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phrase_resolves_to_a_search() {
        let url = resolve_target("how tall is mont blanc");
        assert!(url.contains("q=how+tall+is+mont+blanc") || url.contains("q=how%20tall"), "got {url}");
    }

    #[test]
    fn url_resolves_to_itself() {
        assert_eq!(resolve_target("https://example.com/a"), "https://example.com/a");
    }

    #[test]
    fn artifact_is_tagged_for_the_window() {
        let a = page_artifact("Example", "https://example.com/", Some("note"));
        assert_eq!(a.artifact_type, "browser_page");
        assert_eq!(a.title, "Example");
    }

    #[test]
    fn doc_fragments_are_present() {
        // Without a doc fragment the model never learns the tool exists.
        assert!(BrowserOpen.doc_fragment().is_some());
        assert!(BrowserSearch.doc_fragment().is_some());
        assert!(BrowserRead.doc_fragment().is_some());
    }
}
