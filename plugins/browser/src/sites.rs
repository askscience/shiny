//! Favourite-site news: headlines from the sites the user actually visits.
//!
//! [`crate::news`] builds the shelf from what the user *searches* for. That
//! misses the other half of a reading habit: someone who opens the same paper
//! or tech site every morning wants that site's front page, whatever they
//! searched for this week. This module adds it.
//!
//! 1. **Top sites.** History rows are grouped by host with the same kind of
//!    decay the interest profile uses (`0.5^(age / SITE_HALF_LIFE_DAYS)`), so a
//!    site visited daily last week outranks one binged a month ago. Search
//!    engines, social networks, shops, local addresses and hosts seen fewer
//!    than [`MIN_VISITS`] times are skipped: one visit is not a habit.
//! 2. **Feeds.** Each site's RSS/Atom feed is discovered once from its home
//!    page (`<link rel="alternate" type="application/rss+xml">`, then the
//!    usual `/feed`, `/rss`, `/rss.xml`… paths) and remembered per host. A site
//!    with no feed simply contributes nothing.
//! 3. **Cards.** Feed items become ordinary [`NewsCard`]s whose `topic` is the
//!    site, scored on the search cards' scale by the site's weight and the
//!    item's age, so the shelf's
//!    interleaving and per-topic cap treat a favourite site like an interest.
//!
//! Best-effort throughout, like the rest of the shelf: no history, no network
//! or a malformed feed yield no cards, never an error page.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::history::HistoryRow;
use crate::news::{self, NewsCard};

/// A site counts once it was opened this many times in the history window.
const MIN_VISITS: usize = 3;
/// Decay of a site visit, in days (habits change slower than searches).
const SITE_HALF_LIFE_DAYS: f64 = 14.0;
/// At most this many sites are fetched per shelf.
pub const MAX_SITES: usize = 3;
/// Items taken from one feed before ranking.
const ITEMS_PER_FEED: usize = 8;
/// Fetched feed items stay fresh this long.
const ITEMS_TTL: Duration = Duration::from_secs(15 * 60);
/// A host's discovered feed URL (or "no feed") is remembered this long.
const FEED_URL_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Feeds are parsed from at most this many bytes.
const MAX_FEED_BYTES: usize = 2 * 1024 * 1024;
/// Common feed locations, tried when the home page does not advertise one.
const FEED_PATHS: &[&str] = &["/feed", "/rss", "/rss.xml", "/feed.xml", "/index.xml", "/atom.xml", "/feeds/all.atom.xml"];

/// A site the user keeps coming back to.
#[derive(Debug, Clone, PartialEq)]
pub struct Site {
    pub host: String,
    pub weight: f64,
    pub visits: usize,
}

/// One item of a feed.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedItem {
    pub title: String,
    pub url: String,
    pub summary: String,
    pub image: Option<String>,
    pub published: Option<chrono::DateTime<chrono::Utc>>,
}

/// Whether a host can be a "favourite site" at all.
fn eligible_host(host: &str) -> bool {
    !host.is_empty()
        && host.contains('.')
        && host != "localhost"
        && host.parse::<std::net::IpAddr>().is_err()
        && !host.ends_with(".local")
        && !news::is_search_host(host)
        && !news::host_matches(host, news::BLOCKED_HOSTS)
        && !news::host_matches(host, news::JUNK_HOSTS)
}

/// The user's most visited sites, strongest first.
pub fn top_sites(rows: &[HistoryRow], now: chrono::DateTime<chrono::Utc>, limit: usize) -> Vec<Site> {
    let mut by_host: HashMap<String, (f64, usize)> = HashMap::new();
    for row in rows {
        if !row.url.starts_with("http://") && !row.url.starts_with("https://") {
            continue;
        }
        let host = news::host_of(&row.url);
        if !eligible_host(&host) {
            continue;
        }
        let age = news::elapsed_days(&row.created_at, now);
        let decay = if age.is_finite() { 0.5f64.powf(age / SITE_HALF_LIFE_DAYS) } else { 0.25 };
        let entry = by_host.entry(host).or_insert((0.0, 0));
        entry.0 += decay;
        entry.1 += 1;
    }
    let mut sites: Vec<Site> = by_host
        .into_iter()
        .filter(|(_, (_, visits))| *visits >= MIN_VISITS)
        .map(|(host, (weight, visits))| Site { host, weight, visits })
        .collect();
    sites.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.host.cmp(&b.host))
    });
    sites.truncate(limit);
    sites
}

/* ── Feed parsing ─────────────────────────────────────────────── */

/// The text inside the first `<name …>…</name>` of `block` (CDATA unwrapped).
fn element_text(block: &str, name: &str) -> Option<String> {
    let open = find_tag(block, name)?;
    let after_open = block[open..].find('>')? + open + 1;
    if block[open..after_open].ends_with("/>") {
        return None;
    }
    let close = block[after_open..].find(&format!("</{name}"))? + after_open;
    let raw = block[after_open..close].trim();
    let raw = raw
        .strip_prefix("<![CDATA[")
        .and_then(|r| r.strip_suffix("]]>"))
        .unwrap_or(raw);
    Some(raw.to_string())
}

/// Byte offset of `<name` followed by a delimiter (so `<link` never matches `<links`).
fn find_tag(block: &str, name: &str) -> Option<usize> {
    let needle = format!("<{name}");
    let mut from = 0;
    while let Some(rel) = block[from..].find(&needle) {
        let at = from + rel;
        let next = block.as_bytes().get(at + needle.len()).copied();
        if matches!(next, Some(b' ' | b'>' | b'/' | b'\t' | b'\n' | b'\r')) {
            return Some(at);
        }
        from = at + needle.len();
    }
    None
}

/// The opening tag (`<name …>`) of the first `name` element.
fn open_tag<'a>(block: &'a str, name: &str) -> Option<&'a str> {
    let at = find_tag(block, name)?;
    let end = block[at..].find('>')? + at + 1;
    Some(&block[at..end])
}

/// Split a feed into its `<item>` (RSS) or `<entry>` (Atom) blocks.
fn blocks<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let close = format!("</{name}>");
    let mut from = 0;
    while let Some(at) = find_tag(&xml[from..], name).map(|i| i + from) {
        let Some(end) = xml[at..].find(&close).map(|i| at + i + close.len()) else { break };
        out.push(&xml[at..end]);
        from = end;
    }
    out
}

fn parse_date(text: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let text = text.trim();
    chrono::DateTime::parse_from_rfc2822(text)
        .or_else(|_| chrono::DateTime::parse_from_rfc3339(text))
        .map(|d| d.with_timezone(&chrono::Utc))
        .ok()
}

fn clean(text: &str) -> String {
    // Feeds often escape their HTML once more (`&lt;p&gt;`): unescape, strip, unescape.
    news::clean_text(&news::html_unescape(&news::strip_tags(&news::html_unescape(text))))
}

fn item_image(block: &str) -> Option<String> {
    for name in ["media:thumbnail", "media:content", "enclosure"] {
        if let Some(tag) = open_tag(block, name) {
            let is_image = name != "enclosure"
                || news::attribute_value(tag, "type").is_some_and(|t| t.starts_with("image/"));
            if is_image {
                if let Some(url) = news::attribute_value(tag, "url").filter(|u| u.starts_with("http")) {
                    return Some(news::html_unescape(&url));
                }
            }
        }
    }
    None
}

/// Parse an RSS 2.0 / RSS 1.0 / Atom document into items, newest first.
pub fn parse_feed(xml: &str) -> Vec<FeedItem> {
    let xml = &xml[..floor_char_boundary(xml, MAX_FEED_BYTES)];
    let atom = find_tag(xml, "feed").is_some() && find_tag(xml, "rss").is_none();
    let mut items: Vec<FeedItem> = Vec::new();
    for block in blocks(xml, if atom { "entry" } else { "item" }) {
        let title = element_text(block, "title").map(|t| clean(&t)).unwrap_or_default();
        let url = if atom {
            // The `alternate` link (or the first one without a rel).
            let mut chosen = None;
            let mut from = 0;
            while let Some(at) = find_tag(&block[from..], "link").map(|i| i + from) {
                let end = block[at..].find('>').map(|i| at + i + 1).unwrap_or(block.len());
                let tag = &block[at..end];
                let rel = news::attribute_value(tag, "rel");
                if rel.is_none() || rel.as_deref() == Some("alternate") {
                    chosen = news::attribute_value(tag, "href");
                    break;
                }
                from = end;
            }
            chosen.unwrap_or_default()
        } else {
            element_text(block, "link").unwrap_or_default()
        };
        let url = news::html_unescape(url.trim());
        if title.is_empty() || !(url.starts_with("http://") || url.starts_with("https://")) {
            continue;
        }
        let summary = ["description", "summary", "content"]
            .iter()
            .find_map(|n| element_text(block, n))
            .map(|t| news::truncate_chars(&clean(&t), 220))
            .unwrap_or_default();
        let published = ["pubDate", "published", "updated", "dc:date"]
            .iter()
            .find_map(|n| element_text(block, n))
            .and_then(|t| parse_date(&t));
        items.push(FeedItem {
            title: news::truncate_chars(&title, 180),
            url,
            summary,
            image: item_image(block),
            published,
        });
    }
    items.sort_by(|a, b| b.published.cmp(&a.published));
    items
}

fn floor_char_boundary(s: &str, max: usize) -> usize {
    if s.len() <= max {
        return s.len();
    }
    let mut i = max;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The feed URL a home page advertises, resolved against the page.
pub fn discover_feed_link(html: &str, base: &str) -> Option<String> {
    let base = url::Url::parse(base).ok()?;
    let mut from = 0;
    while let Some(at) = find_tag(&html[from..], "link").map(|i| i + from) {
        let end = html[at..].find('>').map(|i| at + i + 1).unwrap_or(html.len());
        let tag = &html[at..end];
        from = end;
        let rel = news::attribute_value(tag, "rel").unwrap_or_default().to_ascii_lowercase();
        let kind = news::attribute_value(tag, "type").unwrap_or_default().to_ascii_lowercase();
        if rel.split_whitespace().any(|r| r == "alternate")
            && (kind.contains("rss+xml") || kind.contains("atom+xml"))
        {
            let href = news::html_unescape(&news::attribute_value(tag, "href")?);
            // Skip per-comment feeds when the page lists the main feed too.
            if href.contains("comments") {
                continue;
            }
            return base.join(&href).ok().map(|u| u.to_string());
        }
    }
    None
}

/// Whether a body looks like a feed and not an HTML error page.
fn looks_like_feed(body: &str) -> bool {
    let head = &body[..floor_char_boundary(body, 2048)];
    head.contains("<rss") || head.contains("<feed") || head.contains("<rdf:RDF")
}

/* ── Fetching (cached) ────────────────────────────────────────── */

type FeedUrlCache = parking_lot::Mutex<HashMap<String, (Option<String>, Instant)>>;
type ItemsCache = parking_lot::Mutex<HashMap<String, (Vec<FeedItem>, Instant)>>;

fn feed_urls() -> &'static FeedUrlCache {
    static CACHE: OnceLock<FeedUrlCache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn items_cache() -> &'static ItemsCache {
    static CACHE: OnceLock<ItemsCache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// One GET. A client per request keeps this module free of the HTTP crate's
/// types; it runs a handful of times per quarter hour at most.
async fn get_text(url: &str, accept: &str) -> Result<String, String> {
    let client = shiny_filter::proxy::impersonated_client_builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let response = client
        .get(url)
        .header("accept", accept)
        .send()
        .await
        .map_err(|e| format!("{url}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("{url}: HTTP {}", response.status()));
    }
    response.text().await.map_err(|e| format!("{url}: {e}"))
}

const ACCEPT_FEED: &str = "application/rss+xml, application/atom+xml, application/xml;q=0.9, text/xml;q=0.8, */*;q=0.5";

async fn find_feed(host: &str) -> Option<String> {
    let home = format!("https://{host}/");
    if let Ok(html) = get_text(&home, crate::fetch::ACCEPT_HTML).await {
        if let Some(link) = discover_feed_link(&html, &home) {
            return Some(link);
        }
    }
    for path in FEED_PATHS {
        let url = format!("https://{host}{path}");
        if let Ok(body) = get_text(&url, ACCEPT_FEED).await {
            if looks_like_feed(&body) {
                return Some(url);
            }
        }
    }
    None
}

/// The newest items of a site's feed. `refresh` skips the item cache (the
/// discovered feed URL is kept: it rarely changes).
pub async fn site_items(host: &str, refresh: bool) -> Result<Vec<FeedItem>, String> {
    if !refresh {
        if let Some((items, at)) = items_cache().lock().get(host) {
            if at.elapsed() < ITEMS_TTL {
                return Ok(items.clone());
            }
        }
    }
    let known = feed_urls().lock().get(host).cloned();
    let feed = match known {
        Some((feed, at)) if at.elapsed() < FEED_URL_TTL => feed,
        _ => {
            let feed = find_feed(host).await;
            feed_urls().lock().insert(host.to_string(), (feed.clone(), Instant::now()));
            feed
        }
    };
    let Some(feed) = feed else {
        items_cache().lock().insert(host.to_string(), (Vec::new(), Instant::now()));
        return Ok(Vec::new());
    };
    let body = get_text(&feed, ACCEPT_FEED).await?;
    let mut items = parse_feed(&body);
    items.truncate(ITEMS_PER_FEED);
    items_cache().lock().insert(host.to_string(), (items.clone(), Instant::now()));
    Ok(items)
}

/// A relative age in the engine's own words, so the shelf's recency scoring
/// and the card's age line treat feed items like search results.
fn age_text(published: chrono::DateTime<chrono::Utc>, now: chrono::DateTime<chrono::Utc>) -> String {
    let minutes = (now - published).num_minutes().max(0);
    match minutes {
        0..=59 => format!("{} minutes ago", minutes.max(1)),
        60..=1439 => format!("{} hours ago", minutes / 60),
        1440..=2879 => "yesterday".to_string(),
        _ => format!("{} days ago", minutes / 1440),
    }
}

/// Cards for one site. `max_weight` is the strongest site's weight, so the
/// best site's freshest story lands on the same scale as a search card.
pub fn site_cards(
    items: &[FeedItem],
    site: &Site,
    max_weight: f64,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<NewsCard> {
    let rel = if max_weight > 0.0 { site.weight / max_weight } else { 1.0 };
    items
        .iter()
        .filter(|item| !news::is_junk_url(&item.url))
        // A favourite site's archive is not news: hold back anything older
        // than a week when it has a date.
        .filter(|item| item.published.is_none_or(|p| (now - p).num_days() <= 7))
        .map(|item| {
            let age = item.published.map(|p| age_text(p, now));
            let recency = news::recency_factor(age.as_deref(), now);
            // Same scale as a search card in `news::rank_results`
            // (topic·2 + recency·3 + publisher·1): the site's weight stands in
            // for the topic weight, and a site the user visits is a source
            // they trust, so it always gets the publisher point.
            let score = ((rel * 2.0 + recency * 3.0 + 1.0) * 100.0).round() / 100.0;
            NewsCard {
                title: item.title.clone(),
                url: news::canonical_url(&item.url),
                source: site.host.clone(),
                snippet: item.summary.clone(),
                image: item.image.clone(),
                topic: site.host.clone(),
                age,
                score,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap()
    }

    fn row(url: &str, at: &str) -> HistoryRow {
        HistoryRow { url: url.into(), query: None, mode: "page".into(), created_at: at.into() }
    }

    #[test]
    fn top_sites_needs_repeat_visits_and_skips_search_and_social() {
        let mut rows = Vec::new();
        for i in 0..5 {
            rows.push(row(&format!("https://www.ilpost.it/a{i}"), "2026-10-30 08:00:00"));
            rows.push(row("https://duckduckgo.com/?q=x", "2026-10-30 08:00:00"));
            rows.push(row("https://www.reddit.com/r/x", "2026-10-30 08:00:00"));
        }
        for i in 0..3 {
            rows.push(row(&format!("https://arstechnica.com/s{i}"), "2026-09-01 08:00:00"));
        }
        rows.push(row("https://once.example.org/", "2026-10-30 08:00:00"));
        rows.push(row("http://127.0.0.1:8080/", "2026-10-30 08:00:00"));
        let sites = top_sites(&rows, now(), 5);
        let hosts: Vec<&str> = sites.iter().map(|s| s.host.as_str()).collect();
        assert_eq!(hosts, vec!["ilpost.it", "arstechnica.com"], "{sites:?}");
    }

    #[test]
    fn parses_rss_items() {
        let xml = r#"<?xml version="1.0"?><rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel>
            <title>Site</title><link>https://site.example/</link>
            <item><title><![CDATA[Older &amp; wiser]]></title><link>https://site.example/old?utm_source=rss</link>
              <pubDate>Thu, 29 Oct 2026 10:00:00 +0000</pubDate><description>&lt;p&gt;Old &lt;b&gt;news&lt;/b&gt;&lt;/p&gt;</description></item>
            <item><title>Fresh story</title><link>https://site.example/fresh</link>
              <pubDate>Sat, 31 Oct 2026 09:00:00 +0000</pubDate>
              <media:thumbnail url="https://img.example/f.jpg"/></item>
        </channel></rss>"#;
        let items = parse_feed(xml);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Fresh story");
        assert_eq!(items[0].image.as_deref(), Some("https://img.example/f.jpg"));
        assert_eq!(items[1].title, "Older & wiser");
        assert_eq!(items[1].summary, "Old news");
    }

    #[test]
    fn parses_atom_entries() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>A</title><link href="https://a.example/"/>
            <entry><title type="html">Atom item</title>
              <link rel="replies" href="https://a.example/c"/><link rel="alternate" href="https://a.example/post"/>
              <updated>2026-10-31T08:00:00Z</updated><summary>Short</summary></entry></feed>"#;
        let items = parse_feed(xml);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].url, "https://a.example/post");
        assert!(items[0].published.is_some());
    }

    #[test]
    fn discovers_the_advertised_feed() {
        let html = r#"<head><link rel="alternate" type="application/rss+xml" href="/comments/feed">
            <link rel="alternate" type="application/rss+xml" title="Main" href="/feed/"></head>"#;
        assert_eq!(
            discover_feed_link(html, "https://www.ilpost.it/").as_deref(),
            Some("https://www.ilpost.it/feed/")
        );
        assert_eq!(discover_feed_link("<html></html>", "https://x.example/"), None);
    }

    #[test]
    fn site_cards_drop_old_items_and_carry_the_site_as_topic() {
        let site = Site { host: "ilpost.it".into(), weight: 2.0, visits: 5 };
        let items = vec![
            FeedItem { title: "New".into(), url: "https://ilpost.it/n".into(), summary: String::new(), image: None,
                published: Some(now() - chrono::Duration::hours(2)) },
            FeedItem { title: "Old".into(), url: "https://ilpost.it/o".into(), summary: String::new(), image: None,
                published: Some(now() - chrono::Duration::days(20)) },
        ];
        let cards = site_cards(&items, &site, 2.0, now());
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].topic, "ilpost.it");
        assert_eq!(cards[0].age.as_deref(), Some("2 hours ago"));
        assert!(cards[0].score > 5.0, "{}", cards[0].score);
    }
}
