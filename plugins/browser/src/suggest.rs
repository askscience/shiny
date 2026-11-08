//! Address-bar suggestions: what the user has been to, saved, or searched for,
//! matching what they are typing.
//!
//! Two results for one keystroke:
//! * `completion` — the address the bar fills in inline, the typed text plus a
//!   selected tail (typing `you` offers `youtube.com`). It is only ever a
//!   prefix match on the address without its scheme and `www.`, so accepting
//!   it never changes what the user already typed. Below the first `/` it
//!   completes to the site (`youtube.com`); past it, to the full address the
//!   user has visited.
//! * `items` — a short list under the bar: bookmarks, visited pages and past
//!   searches whose address, title or phrase contains every typed word.
//!
//! Ranking, highest first: a bookmark, how often the site or page was opened,
//! then how recently. Best-effort like the rest of the plugin: an unreadable
//! database gives no suggestions, never an error.

use std::collections::HashMap;
use std::sync::Arc;

use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::services::PluginCtx;

/// History rows considered per keystroke (distinct URLs, newest first).
const HISTORY_SCAN: i64 = 3000;
/// Rows shown under the bar.
pub const DEFAULT_LIMIT: usize = 6;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Bookmark,
    History,
    Search,
}

/// One row under the address bar.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Item {
    pub kind: Kind,
    /// What Enter navigates to: an address, or the phrase of a past search.
    pub value: String,
    /// The second line: the address without scheme, or nothing for a search.
    pub display: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Suggestions {
    pub completion: Option<String>,
    pub items: Vec<Item>,
}

/// One visited URL, aggregated over its history rows.
#[derive(Debug, Clone)]
pub struct Visit {
    pub url: String,
    pub count: i64,
    /// `YYYY-MM-DD HH:MM:SS`, so it sorts as text.
    pub last: String,
    /// Set when the visit was a search (the phrase the user typed).
    pub query: Option<String>,
}

/// The address as people type it: no scheme, no `www.`, no trailing `/`.
pub fn display(url: &str) -> String {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    rest.strip_suffix('/').unwrap_or(rest).to_string()
}

fn host_of(display: &str) -> &str {
    display.split(['/', '?', '#']).next().unwrap_or(display)
}

/// The user's visits, one per URL, newest first.
pub fn visits(ctx: &Arc<PluginCtx>, user_id: &str) -> Vec<Visit> {
    let sql = format!(
        "SELECT url, COUNT(*), MAX(created_at), MAX(query) FROM {}
         WHERE traveler_id = ?1
         GROUP BY url
         ORDER BY MAX(created_at) DESC
         LIMIT ?2",
        crate::history::TABLE
    );
    ctx.db()
        .query(&sql, &[Value::text(user_id.to_string()), Value::Int(HISTORY_SCAN)])
        .unwrap_or_default()
        .into_iter()
        .filter_map(|row| {
            let mut it = row.into_iter();
            let url = text(it.next())?;
            let count = match it.next() {
                Some(Value::Int(n)) => n,
                _ => 1,
            };
            let last = text(it.next()).unwrap_or_default();
            let query = text(it.next()).filter(|q| !q.trim().is_empty());
            Some(Visit { url, count, last, query })
        })
        .collect()
}

fn text(value: Option<Value>) -> Option<String> {
    match value {
        Some(Value::Text(text)) => Some(text),
        _ => None,
    }
}

/// Suggestions for `typed`, from the user's bookmarks and history.
pub fn suggest(ctx: &Arc<PluginCtx>, user_id: &str, typed: &str, limit: usize) -> Suggestions {
    if typed.trim().is_empty() {
        return Suggestions { completion: None, items: Vec::new() };
    }
    let bookmarks = crate::bookmarks::list(ctx, user_id, 200);
    let visits = visits(ctx, user_id);
    rank(typed, &bookmarks, &visits, limit)
}

/// The pure half of [`suggest`], over rows already read.
pub fn rank(
    typed: &str,
    bookmarks: &[crate::bookmarks::Bookmark],
    visits: &[Visit],
    limit: usize,
) -> Suggestions {
    let needle = typed.trim().to_lowercase();
    let words: Vec<&str> = needle.split_whitespace().collect();
    if words.is_empty() {
        return Suggestions { completion: None, items: Vec::new() };
    }
    let completion = completion(typed, &needle, bookmarks, visits);

    // (score, recency, item); higher first.
    let mut scored: Vec<(i64, String, Item)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let contains_all = |haystack: &str| words.iter().all(|w| haystack.contains(w));
    let starts = |d: &str| d.to_lowercase().starts_with(&needle);

    for b in bookmarks {
        let shown = display(&b.url);
        let title = b.title.clone().filter(|t| !t.trim().is_empty());
        let hay = format!("{} {}", shown.to_lowercase(), title.as_deref().unwrap_or("").to_lowercase());
        if !contains_all(&hay) || !seen.insert(shown.to_lowercase()) {
            continue;
        }
        let visits_of = visits.iter().find(|v| display(&v.url).eq_ignore_ascii_case(&shown));
        let score = 1000 + if starts(&shown) { 500 } else { 0 } + visits_of.map_or(0, |v| v.count.min(200));
        scored.push((
            score,
            visits_of.map(|v| v.last.clone()).unwrap_or_else(|| b.created_at.clone()),
            Item { kind: Kind::Bookmark, value: b.url.clone(), display: shown, title },
        ));
    }

    for v in visits {
        if let Some(query) = &v.query {
            let key = format!("?search {}", query.to_lowercase());
            if contains_all(&query.to_lowercase()) && seen.insert(key) {
                let score = v.count.min(200) + if query.to_lowercase().starts_with(&needle) { 300 } else { 0 };
                scored.push((
                    score,
                    v.last.clone(),
                    Item { kind: Kind::Search, value: query.clone(), display: String::new(), title: None },
                ));
            }
            continue;
        }
        let shown = display(&v.url);
        if !contains_all(&shown.to_lowercase()) || !seen.insert(shown.to_lowercase()) {
            continue;
        }
        let score = v.count.min(200) + if starts(&shown) { 500 } else { 0 };
        scored.push((
            score,
            v.last.clone(),
            Item { kind: Kind::History, value: v.url.clone(), display: shown, title: None },
        ));
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    let mut items: Vec<Item> = scored.into_iter().map(|(_, _, item)| item).collect();
    // The inline completion is the first row, so the list and the bar agree.
    if let Some(done) = &completion {
        if let Some(at) = items.iter().position(|i| i.display.eq_ignore_ascii_case(done)) {
            let item = items.remove(at);
            items.insert(0, item);
        }
    }
    items.truncate(limit.clamp(1, 12));
    Suggestions { completion, items }
}

/// The inline completion: the most-used address that starts with what was
/// typed. Spaces mean a search, never an address.
fn completion(
    typed: &str,
    needle: &str,
    bookmarks: &[crate::bookmarks::Bookmark],
    visits: &[Visit],
) -> Option<String> {
    if typed.trim() != typed.trim_start() || needle.contains(char::is_whitespace) {
        return None;
    }
    let typed_body = display(needle);
    let typed_body = if needle.ends_with('/') { format!("{typed_body}/") } else { typed_body };
    let past_host = typed_body.contains('/');

    // Score per candidate address: bookmarks count as many visits.
    let mut weight: HashMap<String, (i64, String)> = HashMap::new();
    let mut add = |shown: String, w: i64, last: &str| {
        let key = if past_host { shown } else { host_of(&shown).to_string() };
        if !key.to_lowercase().starts_with(&typed_body) || key.len() <= typed_body.len() {
            return;
        }
        let entry = weight.entry(key).or_insert((0, String::new()));
        entry.0 += w;
        if last > entry.1.as_str() {
            entry.1 = last.to_string();
        }
    };
    for b in bookmarks {
        add(display(&b.url), 50, &b.created_at);
    }
    for v in visits.iter().filter(|v| v.query.is_none()) {
        add(display(&v.url), v.count, &v.last);
    }
    let (best, _) = weight
        .into_iter()
        .max_by(|a, b| a.1 .0.cmp(&b.1 .0).then_with(|| a.1 .1.cmp(&b.1 .1)).then_with(|| b.0.len().cmp(&a.0.len())))?;
    // Returned without scheme or `www.`: the window appends only the tail past
    // what the user typed, so their own letters are never rewritten.
    Some(best.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bookmarks::Bookmark;

    fn visit(url: &str, count: i64, last: &str) -> Visit {
        Visit { url: url.into(), count, last: last.into(), query: None }
    }

    fn search(url: &str, query: &str) -> Visit {
        Visit { url: url.into(), count: 1, last: "2026-11-01 10:00:00".into(), query: Some(query.into()) }
    }

    fn bookmark(url: &str, title: &str) -> Bookmark {
        Bookmark { id: url.into(), url: url.into(), title: Some(title.into()), created_at: "2026-10-01 10:00:00".into() }
    }

    #[test]
    fn completes_to_the_most_visited_site() {
        let visits = [
            visit("https://www.youtube.com/watch?v=1", 30, "2026-11-01 10:00:00"),
            visit("https://www.youtube.com/", 5, "2026-11-02 10:00:00"),
            visit("https://yourls.org/", 2, "2026-11-07 10:00:00"),
        ];
        let s = rank("you", &[], &visits, 6);
        assert_eq!(s.completion.as_deref(), Some("youtube.com"));
        assert_eq!(s.items[0].display, "youtube.com");
    }

    #[test]
    fn past_the_host_it_completes_the_full_address() {
        let visits = [visit("https://github.com/askscience/shiny", 4, "2026-11-01 10:00:00")];
        let s = rank("github.com/ask", &[], &visits, 6);
        assert_eq!(s.completion.as_deref(), Some("github.com/askscience/shiny"));
    }

    #[test]
    fn no_completion_for_a_search_or_a_finished_address() {
        let visits = [visit("https://youtube.com/", 4, "2026-11-01 10:00:00")];
        assert_eq!(rank("you tube", &[], &visits, 6).completion, None);
        assert_eq!(rank("youtube.com", &[], &visits, 6).completion, None);
        assert_eq!(rank("zzz", &[], &visits, 6).completion, None);
    }

    #[test]
    fn bookmarks_lead_and_match_their_title() {
        let visits = [visit("https://news.example/", 50, "2026-11-01 10:00:00")];
        let marks = [bookmark("https://chat.deepseek.com/", "DeepSeek - Into the Unknown")];
        let s = rank("unknown", &marks, &visits, 6);
        assert_eq!(s.items.len(), 1);
        assert_eq!(s.items[0].kind, Kind::Bookmark);
        assert_eq!(s.items[0].title.as_deref(), Some("DeepSeek - Into the Unknown"));
        // A title match is no address prefix: nothing to fill in.
        assert_eq!(s.completion, None);
    }

    #[test]
    fn past_searches_come_back_as_phrases() {
        let visits = [search("https://duckduckgo.com/?q=rust+axum", "rust axum")];
        let s = rank("rust", &[], &visits, 6);
        assert_eq!(s.items[0].kind, Kind::Search);
        assert_eq!(s.items[0].value, "rust axum");
        assert_eq!(s.completion, None, "a search result page is not a completion");
    }

    #[test]
    fn display_strips_scheme_www_and_slash() {
        assert_eq!(display("https://www.youtube.com/"), "youtube.com");
        assert_eq!(display("http://a.b/c?d"), "a.b/c?d");
    }
}
