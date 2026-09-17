//! Related-news cards for the browser window's home surface.
//!
//! The window's `about:home` is otherwise an empty "type an address" panel.
//! This module turns it into something useful: a small shelf of news cards
//! chosen from **what the user actually searches for**, so opening the browser
//! starts from their interests instead of a blank page.
//!
//! # The algorithm
//!
//! 1. **Interest profile.** Every navigation is recorded (see
//!    [`crate::history`]) together with the raw input the user typed —
//!    a search phrase is a much stronger statement of interest than a URL
//!    someone landed on. The profile is a decayed token frequency table:
//!
//!    ```text
//!    weight(token) = Σ over rows  base(row) · 0.5^(age_days / HALF_LIFE_DAYS)
//!    ```
//!
//!    where `base` is higher for a typed search than for a visited URL, and
//!    tokens from the most recent rows count more than older ones. That is the
//!    whole personalisation signal: recent behaviour, deliberately not a
//!    long-term profile, so interests decay instead of calcifying.
//!
//! 2. **Topics.** A topic is the phrase the user actually typed, not a single
//!    word out of it. The profile keeps the word pairs that came out of real
//!    queries, and a topic is the richest phrase containing a strong word
//!    (`"electric vehicle battery"`, not `"battery"`). Words paired with many
//!    different things are treated as filler and skipped outright. This is not
//!    cosmetic: measured live, `"quantum computing breakthrough"` reduced to
//!    `"breakthrough"` matched a tennis final, and `"computing"` alone matched
//!    a docking-station review. Topics are then filtered so that several words
//!    of one search do not each become an interest.
//!
//! 3. **Fetching.** Each topic is looked up through a search engine — the same
//!    engine the window's address bar uses, so this adds no new dependency —
//!    with a news-biased query. Results are parsed from the engine's HTML.
//!
//! 4. **Ranking.** A card's score combines the weight of the topic that
//!    produced it, how recently it was published (with a hard tier, so a
//!    month-old story cannot lead a shelf labelled news), whether the source is
//!    a known news publisher, and how much of the user's other interests the
//!    headline touches. Shopping, social, content-farm and non-journalism
//!    results are dropped, one publisher is capped to a share of the shelf, and
//!    the shelf is interleaved across topics so one interest cannot fill it.
//!
//! Everything here is best-effort: no database, no network, or a changed
//! search-engine layout degrade to "no cards" rather than an error page. The
//! home surface must never be the thing that breaks the browser.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use shiny_plugin_sdk::errors::AppError;

use crate::history::{self, HistoryRow};

/// How long a cached set of cards stays fresh.
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);

/// Oldest age, in days, a card may carry while fresher options exist.
///
/// A "news" shelf whose first card is from last month reads as broken, so stale
/// items are held back unless the shelf would otherwise be empty.
const STALE_AFTER_DAYS: i64 = 30;

/// Age, in days, below which a story counts as current coverage.
///
/// Cards older than this are placed on a lower score tier, so "news" leads
/// with this week even when an older item matches the topic more strongly.
const FRESH_DAYS: i64 = 7;

/// At most this many cards from one publisher.
///
/// A single outlet with a large archive can otherwise fill the shelf: several
/// cards from one domain is what the live engine returned for a broad topic
/// before this cap existed.
const MAX_PER_HOST: usize = 2;

/// Path fragments that mark a result as something other than news.
///
/// The engine's news tab still returns gaming builds, product pages and store
/// listings for a bare topic word. These are matched against the URL (slugs
/// included), because that is where the giveaway reliably is: a headline can
/// look innocent while its slug says `/aram-build/`.
const JUNK_MARKERS: &[&str] = &[
    "/wiki/",
    "/build/",
    "/builds/",
    "-build",
    "-builds",
    "/items/",
    "/guide/",
    "/guides/",
    "/walkthrough",
    "/patch-notes",
    "/gameplay",
    "/shop/",
    "/product/",
    "/products/",
    "/dp/",
    "/deal",
    "/coupon",
    "/promo",
    "/signup",
    "/subscribe",
    "/login",
    "/cart/",
    "/download",
    "/apk",
    "/torrent",
];

/// Hosts whose whole purpose is not journalism, whatever the article says.
const JUNK_HOSTS: &[&str] = &[
    "fandom.com", "wikia.com", "game8.co", "gamewith.net", "op.gg", "u.gg", "mobalytics.gg",
    "leagueofgraphs.com", "probuilds.net",
];

/// How many topics one refresh is allowed to hit the network for.
///
/// A ceiling rather than a target: the selector stops as soon as it has enough
/// *distinct* topics, and the cache makes repeat loads free. Four bounds a cold
/// home-page load to four engine requests made concurrently.
const MAX_TOPIC_FETCHES: usize = 4;

/// Per-request network timeout for one engine fetch.
const FETCH_TIMEOUT: Duration = Duration::from_secs(12);

/// Half-life of an interest, in days.
///
/// A week: something you looked at today outweighs the same term from a
/// fortnight ago by ~4×.
const HALF_LIFE_DAYS: f64 = 7.0;

/* ── Public types ─────────────────────────────────────────────── */

/// One card the home surface renders.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct NewsCard {
    pub title: String,
    pub url: String,
    /// Registrable-ish host, for the card's source line.
    pub source: String,
    pub snippet: String,
    /// Thumbnail URL the engine supplied for the story, when it has one.
    ///
    /// The home surface loads it directly (see `cardHtml` in `web/plugin.js`);
    /// it is a presentation asset, not part of the ranking.
    pub image: Option<String>,
    /// The interest this card was fetched for.
    pub topic: String,
    /// Publication age as the engine reported it ("2 hours ago"); free text.
    pub age: Option<String>,
    pub score: f64,
}

/// The full home payload: what to show, and why.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HomeNews {
    pub cards: Vec<NewsCard>,
    /// Top interests, strongest first — the UI shows these as chips and it is
    /// the most direct way for a user to see the algorithm is not a black box.
    pub topics: Vec<String>,
    /// True when the profile had enough signal to personalise anything.
    pub personalized: bool,
    /// Set when the fetch failed, so the UI can say so instead of showing an
    /// empty shelf that looks like "no news exists".
    pub error: Option<String>,
}

/// A remembered token and its current weight (used by the API and tests).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Interest {
    pub term: String,
    pub weight: f64,
}

/// What the history says about a user's interests.
#[derive(Debug, Clone, Default)]
pub struct Profile {
    /// Ranked terms: single tokens and repeated phrases.
    pub interests: Vec<Interest>,
    /// Word pairs that keep appearing together, counted per history row.
    ///
    /// This is how the selector knows that "aurora" and "storm" are one
    /// interest and not two: they are not synonyms to a tokeniser, but the
    /// user's own searches put them side by side. Derived from the same rows as
    /// `interests`, so it costs one extra map and no new data.
    pub pairs: HashMap<String, f64>,
    /// Which word followed which, for rebuilding the user's own word order.
    ///
    /// A bag of unordered pairs cannot tell "aurora borealis" from "storm
    /// aurora": only the order a query was typed in can. Values are `(before,
    /// after)` lists, deduplicated.
    pub adjacency: HashMap<String, Vec<(String, String)>>,
}

impl Profile {
    pub fn is_empty(&self) -> bool {
        self.interests.is_empty()
    }

    /// Every ordered neighbour pair, for phrase building.
    ///
    /// Falls back to the unordered keys when no adjacency was recorded (a
    /// hand-built `Profile` in a test, for instance).
    fn ordered_pairs(&self) -> Vec<(String, String, f64)> {
        if self.adjacency.is_empty() {
            return self
                .pairs
                .iter()
                .filter_map(|(phrase, count)| {
                    let (a, b) = phrase.split_once(' ')?;
                    Some((a.to_string(), b.to_string(), *count))
                })
                .collect();
        }
        let mut out: Vec<(String, String, f64)> = Vec::new();
        for (word, neighbours) in &self.adjacency {
            for (before, after) in neighbours {
                let count = self
                    .pairs
                    .get(&format!("{before} {after}"))
                    .copied()
                    .unwrap_or(1.0);
                out.push((before.clone(), word.clone(), count));
                out.push((word.clone(), after.clone(), count));
            }
        }
        out
    }

    /// How many *different* words this one keeps company with.
    ///
    /// A specific term carries its subject with it ("quantum computing",
    /// "wireless keyboard"); a generic one turns up next to everything
    /// ("breakthrough", "computing" — measured live, those two filled half the
    /// shelf on their own). This count is what tells them apart.
    pub fn connectivity(&self, word: &str) -> usize {
        let mut partners: HashSet<String> = HashSet::new();
        for (left, right, count) in self.ordered_pairs() {
            // Even a single sighting is evidence of a relation: queries are
            // usually typed once, so requiring two would mean the phrase path
            // never fires on real history.
            if count < 1.0 {
                continue;
            }
            if left == word {
                partners.insert(right);
            } else if right == word {
                partners.insert(left);
            }
        }
        partners.len()
    }

    /// The best term that represents `word`: its own word, or a two-word phrase
    /// it appears in when that phrase is more specific than the word alone.
    pub fn representative(&self, word: &str) -> String {
        let mut best: Option<(String, f64)> = None;
        for (left, right, count) in self.ordered_pairs() {
            if count < 1.0 {
                continue;
            }
            if left != word && right != word {
                continue;
            }
            let phrase = format!("{left} {right}");
            match &best {
                Some((_, best_count)) if count <= *best_count => {}
                _ => best = Some((phrase, count)),
            }
        }
        match best {
            Some((phrase, _)) => phrase,
            None => word.to_string(),
        }
    }

    /// The scored word pairs the user's own queries produced, richest first.
    fn phrase_weights(&self) -> Vec<(String, f64)> {
        let mut out: Vec<(String, f64)> = self
            .pairs
            .iter()
            .map(|(phrase, weight)| (phrase.clone(), *weight))
            .collect();
        out.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        out
    }

    /// Whether two terms keep appearing together for this user.
    ///
    /// Stricter than [`Profile::connectivity`]: a one-off co-occurrence is
    /// enough to *build* a phrase, but treating two unrelated words as the same
    /// subject needs repetition.
    pub fn related(&self, a: &str, b: &str) -> bool {
        if a == b {
            return true;
        }
        for (left, right, count) in self.ordered_pairs() {
            if count < 2.0 {
                continue;
            }
            if (left == a && right == b) || (left == b && right == a) {
                return true;
            }
        }
        false
    }
}

/* ── The interest profile ─────────────────────────────────────── */

/// Words that carry no topical signal.
///
/// English plus the handful of function words that show up most in the
/// languages this app is likely to be used from. Deliberately a fixed list
/// rather than a stemmer: an honest over-approximation makes the profile
/// worse, an under-approximation only makes it quieter.
const STOPWORDS: &[&str] = &[
    "a", "ab", "about", "after", "all", "also", "an", "and", "any", "are", "as", "at", "aus",
    "aux", "be", "been", "bei", "but", "by", "can", "come", "con", "da", "de", "del", "der", "des",
    "did", "die", "do", "does", "durch", "ein", "eine", "el", "en", "es", "et", "for", "from",
    "get", "gli", "how", "il", "in", "into", "is", "it", "its", "i", "la", "le", "les", "lo",
    "los", "mit", "my", "nach", "new", "news", "not", "of", "on", "or", "para", "per", "por",
    "que", "s", "search", "sur", "that", "the", "their", "them", "this", "to", "un", "una", "und",
    "uno", "von", "was", "what", "when", "where", "which", "who", "why", "with", "you", "your",
    "www", "com", "http", "https", "html", "index", "page", "best", "top", "vs", "review",
    "reviews", "buy", "price", "cheap", "deal", "deals", "coupon", "download", "free", "online",
];

/// Hosts that name a search engine rather than a topic.
///
/// A URL on one of these is a *query*, already captured by the query column —
/// extracting tokens from `/search?q=…` would just double-count the phrase and
/// add "search"/"q".
const SEARCH_HOSTS: &[&str] = &[
    "search.brave.com",
    "brave.com",
    "google.com",
    "google.de",
    "google.it",
    "bing.com",
    "duckduckgo.com",
    "searx.be",
    "searxng.site",
    "startpage.com",
    "ecosia.org",
    "mojeek.com",
    "qwant.com",
    "yandex.com",
    "baidu.com",
];

/// Domains that are news publishers or otherwise worth surfacing.
///
/// Used only as a *boost*: an unknown-but-real domain can still be shown, it
/// just has to out-score the known-publisher bonus on recency and relevance.
const NEWS_DOMAINS: &[&str] = &[
    "reuters.com", "apnews.com", "bbc.com", "bbc.co.uk", "nytimes.com", "washingtonpost.com",
    "theguardian.com", "guardian.co.uk", "aljazeera.com", "npr.org", "cnn.com", "cnbc.com",
    "bloomberg.com", "ft.com", "wsj.com", "economist.com", "forbes.com", "time.com",
    "theatlantic.com", "politico.com", "axios.com", "thehill.com", "foreignpolicy.com",
    "dw.com", "france24.com", "lemonde.fr", "lefigaro.fr", "spiegel.de", "zeit.de", "faz.net",
    "corriere.it", "repubblica.it", "ansa.it", "ilsole24ore.com", "elpais.com", "elmundo.es",
    "folha.uol.com.br", "globo.com", "clarin.com", "nacion.com", "timesofindia.com",
    "thehindu.com", "scmp.com", "japantimes.co.jp", "nhk.or.jp", "asahi.com", "straitstimes.com",
    "abc.net.au", "cbc.ca", "theglobeandmail.com", "nature.com", "science.org", "scientificamerican.com",
    "newscientist.com", "arstechnica.com", "theverge.com", "wired.com", "techcrunch.com",
    "engadget.com", "zdnet.com", "cnet.com", "tomshardware.com", "anandtech.com", "pcgamer.com",
    "polygon.com", "variety.com", "hollywoodreporter.com", "billboard.com", "rollingstone.com",
    "pitchfork.com", "espn.com", "skysports.com", "marca.com",
];

/// Hosts that are never a news card: shops, content farms, social feeds.
///
/// Cheaper and more predictable than trying to classify them positively.
const BLOCKED_HOSTS: &[&str] = &[
    "pinterest.", "quora.com", "facebook.com", "instagram.com", "tiktok.com", "x.com",
    "twitter.com", "reddit.com", "youtube.com", "amazon.", "ebay.", "aliexpress.",
    "walmart.com", "etsy.com", "temu.com", "booking.com", "tripadvisor.", "yelp.",
    "glassdoor.", "indeed.com", "linkedin.com", "medium.com", "substack.com", "snvhost.com",
    "jjgirls.com", "sexcams.plus", "jdrucker.com", "doubleclick.net", "googleadservices.com",
];

/// Split text into comparable lowercase tokens.
///
/// Keeps letters and digits, drops everything else, and drops tokens that are
/// too short or too long to be a real term (so `a`, `…`, `3` and a base64 blob
/// never become an "interest").
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                current.push(lower);
            }
        } else if !current.is_empty() {
            push_token(&mut out, &current);
            current.clear();
        }
    }
    if !current.is_empty() {
        push_token(&mut out, &current);
    }
    out
}

fn push_token(out: &mut Vec<String>, token: &str) {
    if token.len() < 3 || token.len() > 24 {
        return;
    }
    if token.chars().all(|c| c.is_ascii_digit()) {
        return;
    }
    if STOPWORDS.contains(&token) {
        return;
    }
    out.push(token.to_string());
}

/// The registrable part of a host, near enough: `www.bbc.co.uk` → `bbc.co.uk`.
///
/// Not a public-suffix implementation on purpose — the only consumer is a
/// source label and a domain lookup, and both want the "bbc.co.uk" answer.
pub fn host_of(url: &str) -> String {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default();
    let host = host.trim_start_matches("www.").to_ascii_lowercase();
    host
}

/// Whether `host` belongs to a known search engine.
fn is_search_host(host: &str) -> bool {
    SEARCH_HOSTS
        .iter()
        .any(|s| host == *s || host.ends_with(&format!(".{s}")))
}

/// Whether a host matches any entry in a pattern list.
///
/// Entries ending in `.` are prefix matches (`amazon.` catches every regional
/// Amazon); everything else matches the host or a subdomain of it.
fn host_matches(host: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|p| {
        if let Some(prefix) = p.strip_suffix('.') {
            host == prefix || host.starts_with(&format!("{prefix}.")) || host.contains(&format!(".{prefix}."))
        } else {
            host == *p || host.ends_with(&format!(".{p}"))
        }
    })
}

/// Weight one history row contributes to the profile.
///
/// A typed search is a statement of interest; a visited page is weaker
/// evidence (it may have been a link from somewhere else). `news_click` is a
/// click on a recommended card: the user voted for that topic.
fn row_base_weight(row: &HistoryRow) -> f64 {
    match row.mode.as_str() {
        "news_click" => 3.0,
        "text" => 2.0,
        _ => if row.query.is_some() { 2.0 } else { 1.0 },
    }
}

/// Raw material for one row's tokens: the typed query, plus the URL's slug.
fn row_text(row: &HistoryRow) -> String {
    let mut text = String::new();
    if let Some(q) = &row.query {
        text.push_str(q);
        text.push(' ');
    }
    let host = host_of(&row.url);
    if !is_search_host(&host) {
        if let Ok(parsed) = url::Url::parse(&row.url) {
            text.push_str(parsed.path());
            text.push(' ');
        }
    }
    text
}

/// Build the decayed interest profile from history, newest row first.
///
/// Returns single tokens and the adjacent two-word phrases among them: a
/// phrase inherits the average of its parts, which is enough to make
/// "wireless keyboard" outrank a bare "keyboard" when both words keep
/// appearing together.
pub fn profile_from_history(rows: &[HistoryRow], now: chrono::DateTime<chrono::Utc>) -> Profile {
    let mut tokens: HashMap<String, f64> = HashMap::new();
    // Adjacent token pairs, counted per row so a phrase is not double-counted
    // by repetition inside one row.
    let mut pairs: HashMap<String, f64> = HashMap::new();
    let mut pair_counts: HashMap<String, f64> = HashMap::new();
    let mut adjacency: HashMap<String, Vec<(String, String)>> = HashMap::new();

    for (index, row) in rows.iter().enumerate() {
        let age_days = elapsed_days(&row.created_at, now);
        // Rows arrive newest-first, but never trust the sort: use the age when
        // it parses, and fall back to position when it does not.
        let decay = if age_days.is_finite() {
            0.5f64.powf(age_days / HALF_LIFE_DAYS)
        } else {
            0.5f64.powf(index as f64 / 10.0)
        };
        let weight = row_base_weight(row) * decay;
        if weight <= 0.0 {
            continue;
        }

        let terms = tokenize(&row_text(row));
        for term in &terms {
            *tokens.entry(term.clone()).or_insert(0.0) += weight;
        }
        let mut seen_pairs: HashSet<String> = HashSet::new();
        for pair in terms.windows(2) {
            let phrase = format!("{} {}", pair[0], pair[1]);
            if seen_pairs.insert(phrase.clone()) {
                *pairs.entry(phrase.clone()).or_insert(0.0) += weight * 1.25;
                *pair_counts.entry(phrase).or_insert(0.0) += 1.0;
                // Remember which word came first: a bag of unordered pairs
                // cannot tell "aurora borealis" from "storm aurora", and the
                // phrase the user actually typed is the better search.
                adjacency
                    .entry(pair[0].clone())
                    .or_default()
                    .push((pair[0].clone(), pair[1].clone()));
            }
        }
    }

    let mut interests: Vec<Interest> = tokens
        .into_iter()
        .map(|(term, weight)| Interest { term, weight })
        .collect();

    // A phrase is only interesting if it *repeats* or is genuinely strong on
    // its own; otherwise the profile fills up with one-off bigrams.
    for (phrase, weight) in &pairs {
        let count = pair_counts.get(phrase).copied().unwrap_or(0.0);
        if count >= 2.0 || *weight >= 4.0 {
            interests.push(Interest {
                term: phrase.clone(),
                weight: *weight,
            });
        }
    }

    interests.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.term.cmp(&b.term))
    });
    interests.truncate(64);

    Profile {
        interests,
        pairs: pair_counts,
        adjacency,
    }
}

/// Age of a history row in days; `INFINITY` when the timestamp is unusable.
fn elapsed_days(created_at: &str, now: chrono::DateTime<chrono::Utc>) -> f64 {
    let parsed = chrono::NaiveDateTime::parse_from_str(created_at, "%Y-%m-%d %H:%M:%S")
        .map(|naive| naive.and_utc());
    match parsed {
        Ok(at) => (now - at).num_seconds().max(0) as f64 / 86_400.0,
        Err(_) => f64::INFINITY,
    }
}

/// Pick the topics to fetch, preferring *distinct* interests.
///
/// The naive choice — the top N tokens — selects the three or four words of a
/// single search ("aurora", "solar", "storm", "borealis") and then shows the
/// user one interest forever, however much else they have searched for since.
/// Measured live: after adding quantum-computing and electric-vehicle searches,
/// the top two tokens were still two words of the first query.
///
/// Two candidates are treated as one interest when either they share a word, or
/// the user's own history keeps putting them in the same phrase
/// ([`Profile::related`]). That is the whole point of carrying the co-occurrence
/// counts around: "storm" and "aurora" are not synonyms to a tokeniser, but a
/// user who searches both means one subject.
pub fn select_topics(profile: &Profile, limit: usize) -> Vec<String> {
    let mut chosen: Vec<String> = Vec::new();
    let mut covered: HashSet<String> = HashSet::new();

    for interest in &profile.interests {
        if chosen.len() >= limit {
            break;
        }
        let words: Vec<String> = interest
            .term
            .split_whitespace()
            .map(str::to_string)
            .collect();

        // Already represented by something chosen — this is also what keeps the
        // rest of a query's words from returning as separate interests once the
        // phrase built from them has been picked.
        if words.iter().all(|w| covered.contains(w)) {
            continue;
        }

        let search = if words.len() > 1 {
            interest.term.clone()
        } else {
            let word = &words[0];
            if profile.connectivity(word) > 3 {
                // Paired with everything: the word is not a subject, and neither
                // is any single phrase built from it ("medical breakthrough" is
                // as meaningless as "breakthrough"). Skip it outright.
                continue;
            }
            // Prefer the most specific thing the user actually typed. Without
            // this a query like "quantum computing breakthrough" is searched as
            // "breakthrough" — which matched a tennis final in a live run.
            let phrase = best_phrase(profile, word).map(|(phrase, _)| phrase);
            // A one-word interest related to something already chosen is that
            // same subject: skip it.
            match phrase {
                Some(phrase) => phrase,
                None => {
                    // Nothing specific to search: a bare word already chosen as
                    // part of another interest is not a new one.
                    if chosen.iter().any(|already| profile.related(word, already)) {
                        continue;
                    }
                    word.clone()
                }
            }
        };

        for word in search.split_whitespace() {
            covered.insert(word.to_string());
        }
        if !chosen.contains(&search) {
            chosen.push(search);
        }
    }
    chosen
}

/// The most specific phrase the user actually typed that contains `seed`.
///
/// Built from the profile's own bigrams — the word pairs that came out of real
/// queries, richest first — rather than by growing an n-gram greedily, which
/// walks into a different query's words (`solar storm aurora` produced "solar
/// forecast"). A candidate is accepted only if the words sit next to each other
/// in a typed query, which is exactly what the adjacency map records.
fn best_phrase(profile: &Profile, seed: &str) -> Option<(String, f64)> {
    let mut candidates: Vec<(Vec<String>, f64)> = Vec::new();

    // Every typed bigram containing the seed, with its scored weight.
    for (phrase, weight) in profile.phrase_weights() {
        let words: Vec<String> = phrase.split_whitespace().map(str::to_string).collect();
        if words.len() != 2 || !words.iter().any(|w| w == seed) {
            continue;
        }
        candidates.push((words, weight));
    }

    // Extend a candidate only through a bigram that overlaps it.
    let base = candidates.clone();
    for (words, weight) in base {
        for (phrase, phrase_weight) in profile.phrase_weights() {
            let next: Vec<String> = phrase.split_whitespace().map(str::to_string).collect();
            if next.len() != 2 {
                continue;
            }
            if next[0] == words[words.len() - 1] && !words.contains(&next[1]) {
                let mut extended = words.clone();
                extended.push(next[1].clone());
                candidates.push((extended, weight + phrase_weight));
            }
            if next[1] == words[0] && !words.contains(&next[0]) {
                let mut extended = vec![next[0].clone()];
                extended.extend(words.clone());
                candidates.push((extended, weight + phrase_weight));
            }
        }
    }

    candidates
        .into_iter()
        .max_by(|(a_words, a_weight), (b_words, b_weight)| {
            // More words first (more specific), then weight, then alphabetical
            // so the same history always yields the same shelf.
            a_words
                .len()
                .cmp(&b_words.len())
                .then(
                    a_weight
                        .partial_cmp(b_weight)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                // `max_by` keeps the *last* maximum: reverse the tie-break so
                // the alphabetically first candidate wins.
                .then(b_words.join(" ").cmp(&a_words.join(" ")))
        })
        .map(|(words, weight)| (words.join(" "), weight))
}

/* ── Engine HTML parsing ──────────────────────────────────────── */

/// One parsed search-engine result, before scoring.
#[derive(Debug, Clone, PartialEq)]
pub struct RawResult {
    pub title: String,
    pub url: String,
    pub source: String,
    pub age: Option<String>,
    pub snippet: String,
    pub image: Option<String>,
}

/// Parse result snippets out of a search-engine results page.
///
/// The engine's markup is treated as hostile input: this walks `class="snippet"`
/// blocks (with or without the Svelte hash that follows the class name) and
/// pulls the pieces it recognises. Anything unrecognised is skipped, so a
/// redesign yields fewer cards rather than garbage ones.
pub fn parse_results(html: &str) -> Vec<RawResult> {
    let mut out = Vec::new();
    for block in snippet_blocks(html) {
        let Some(url) = first_external_href(&block) else {
            continue;
        };
        let title = class_text(&block, "title").unwrap_or_default();
        let title = if title.is_empty() {
            anchor_text(&block)
        } else {
            title
        };
        let title = clean_text(&title);
        if title.is_empty() || title.len() < 8 {
            continue;
        }
        let source = class_text(&block, "desktop-small-semibold")
            .map(|s| clean_text(&s))
            .unwrap_or_else(|| host_of(&url));
        let age = class_text(&block, "desktop-small-regular").map(|s| clean_text(&s));
        let snippet = class_text(&block, "description")
            .map(|s| clean_text(&s))
            .unwrap_or_default();
        out.push(RawResult {
            title,
            url,
            source,
            age: age.filter(|a| !a.is_empty()),
            snippet,
            image: first_content_image(&block),
        });
    }
    out
}

/// Slice the document into `class="snippet …"` blocks by brace-free scanning.
///
/// A block runs to the next result marker: the engine emits results as flat
/// siblings, so "up to the next result" is the correct extent without needing a
/// real HTML parser (and without one more dependency in a plugin).
///
/// Only a *result* starts a block. The engine also emits
/// `class="snippet-thumbnail-wrapper"` inside a result, and treating that as a
/// boundary ended each block at its own thumbnail — which is precisely why the
/// card images were missing.
fn snippet_blocks(html: &str) -> Vec<String> {
    const MARKER: &str = "class=\"snippet";
    let mut starts: Vec<usize> = Vec::new();
    let mut search_from = 0usize;
    while let Some(rel) = html[search_from..].find(MARKER) {
        let at = search_from + rel;
        // A real result is `class="snippet"` or `class="snippet …`; anything
        // else (`snippet-thumbnail-wrapper`, `snippet-…`) belongs to the
        // result already in progress.
        let after = html[at + MARKER.len()..].chars().next();
        let is_result = match after {
            Some('"') => true,
            Some(c) if c.is_whitespace() => true,
            _ => false,
        };
        if is_result {
            starts.push(at);
        }
        search_from = at + 1;
    }
    let mut out = Vec::with_capacity(starts.len());
    for (i, start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(html.len());
        out.push(html[*start..end].to_string());
    }
    out
}

/// The first absolute http(s) href in a block that is not the engine itself.
fn first_external_href(block: &str) -> Option<String> {
    let mut from = 0usize;
    while let Some(rel) = block[from..].find("href=\"") {
        let start = from + rel + "href=\"".len();
        let Some(close_rel) = block[start..].find('"') else {
            break;
        };
        let raw = &block[start..start + close_rel];
        let url = html_unescape(&decode_proxy_url(raw));
        from = start + close_rel;
        if !url.starts_with("http://") && !url.starts_with("https://") {
            continue;
        }
        let host = host_of(&url);
        if host.is_empty() || is_search_host(&host) || host.starts_with("imgs.search") {
            continue;
        }
        return Some(url);
    }
    None
}

/// The first real thumbnail image in a result block, if it has one.
///
/// The engine renders the publisher's favicon (`class="favicon …"` or
/// `class="favicon-background …"`) before the story thumbnail, and both live on
/// the same image CDN, so the class is the only reliable way to tell them
/// apart. Favicons are skipped; the first remaining absolute http(s) `src` is
/// the story image.
fn first_content_image(block: &str) -> Option<String> {
    let mut from = 0usize;
    while let Some(rel) = block[from..].find("<img") {
        let at = from + rel;
        let tag_end = block[at..].find('>').map(|i| at + i)?;
        let tag = &block[at..=tag_end];
        from = tag_end + 1;

        let is_favicon = attribute_value(tag, "class")
            .map(|class| class.to_ascii_lowercase().contains("favicon"))
            .unwrap_or(false);
        if is_favicon {
            continue;
        }
        let Some(raw) = attribute_value(tag, "src") else {
            continue;
        };
        let url = html_unescape(&decode_proxy_url(&raw));
        if url.starts_with("http://") || url.starts_with("https://") {
            return Some(url);
        }
    }
    None
}

/// Read one attribute's value out of a single tag, quote-aware enough for the
/// engine's markup.
///
/// The name is matched on a word boundary so `data-src=` is never read as
/// `src=`.
fn attribute_value(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=");
    let mut from = 0usize;
    while let Some(rel) = tag[from..].find(&needle) {
        let at = from + rel + needle.len();
        let name_start = at - needle.len();
        if name_start > 0 {
            let before = tag.as_bytes()[name_start - 1];
            if before.is_ascii_alphanumeric() || before == b'-' || before == b'_' {
                from = at;
                continue;
            }
        }
        let bytes = tag.as_bytes();
        let value_start;
        let value_end;
        let quote = bytes.get(at).copied();
        if quote == Some(b'"') || quote == Some(b'\'') {
            let q = quote.expect("matched a quote") as char;
            value_start = at + 1;
            value_end = tag[value_start..]
                .find(q)
                .map(|i| value_start + i)?;
        } else {
            value_start = at;
            value_end = tag[value_start..]
                .find(|c: char| c.is_ascii_whitespace() || c == '>')
                .map(|i| value_start + i)
                .unwrap_or(tag.len());
        }
        return Some(tag[value_start..value_end].to_string());
    }
    None
}

/// Undo the proxy's path form when a page was parsed while proxied.
///
/// The engine is usually fetched directly, but a proxied page is a valid input
/// too (`/p/https/example.com/a` → `https://example.com/a`), and parsing it
/// wrong would show the user a `127.0.0.1` URL on a card.
fn decode_proxy_url(url: &str) -> String {
    let Some(rest) = url.split_once("/p/").map(|(_, r)| r) else {
        return url.to_string();
    };
    let Some((scheme, remainder)) = rest.split_once('/') else {
        return url.to_string();
    };
    if scheme == "http" || scheme == "https" {
        format!("{scheme}://{remainder}")
    } else {
        url.to_string()
    }
}

/// Text of the first element carrying a class token, tags stripped.
///
/// Handles the engine's two shapes: `<div class="title …">text</div>` and the
/// same tag with a `title="…"` attribute instead of a body.
fn class_text(block: &str, class: &str) -> Option<String> {
    let marker = format!("class=\"{class}");
    let at = block.find(&marker)?;
    let tag_end = block[at..].find('>').map(|i| at + i + 1)?;
    let close = block[tag_end..]
        .find("</")
        .map(|i| tag_end + i)
        .unwrap_or(block.len());
    let inner = strip_tags(&block[tag_end..close]);
    if !inner.trim().is_empty() {
        return Some(inner);
    }
    // Fall back to a `title="…"` attribute on the opening tag.
    let open_end = at + block[at..].find('>')?;
    let open_tag = &block[at..open_end];
    let attr = open_tag.find("title=\"")?;
    let start = attr + "title=\"".len();
    let end_rel = open_tag[start..].find('"')?;
    Some(html_unescape(&open_tag[start..start + end_rel]))
}

/// The first anchor's text in a block (used when there is no title element).
fn anchor_text(block: &str) -> String {
    let Some(at) = block.find("<a ") else {
        return String::new();
    };
    let Some(open_end) = block[at..].find('>') else {
        return String::new();
    };
    let body_start = at + open_end + 1;
    let Some(close_rel) = block[body_start..].find("</a>") else {
        return String::new();
    };
    strip_tags(&block[body_start..body_start + close_rel])
}

/// Remove tags and unescape entities.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    html_unescape(&out)
}

/// Collapse whitespace and drop the engine's comment markers.
fn clean_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for ch in text.chars() {
        let is_space = ch.is_whitespace();
        if is_space {
            if !last_space {
                out.push(' ');
            }
        } else {
            out.push(ch);
        }
        last_space = is_space;
    }
    out.trim().to_string()
}

/// Decode the entities the engine actually emits in titles and snippets.
fn html_unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            let window = &text[i..];
            let (replacement, len) = if window.starts_with("&amp;") {
                ("&", "&amp;".len())
            } else if window.starts_with("&quot;") {
                ("\"", "&quot;".len())
            } else if window.starts_with("&#39;") || window.starts_with("&apos;") {
                ("'", 5)
            } else if window.starts_with("&lt;") {
                ("<", 4)
            } else if window.starts_with("&gt;") {
                (">", 4)
            } else if window.starts_with("&nbsp;") {
                (" ", 6)
            } else if window.starts_with("&#x27;") {
                ("'", 6)
            } else {
                ("&", 1)
            };
            out.push_str(replacement);
            i += len;
        } else {
            let ch = text[i..].chars().next().unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/* ── Scoring ──────────────────────────────────────────────────── */

/// Score and rank raw results into cards.
///
/// `topic_weights` maps the profile terms (single tokens *and* phrases) to the
/// user's weight for them; a card may match several. `now` is used to turn
/// relative ages into a recency factor.
pub fn rank_results(
    results: &[RawResult],
    topic: &str,
    topic_weights: &HashMap<String, f64>,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<NewsCard> {
    let max_weight = topic_weights
        .values()
        .cloned()
        .fold(1.0f64, f64::max);
    let mut seen_urls: HashSet<String> = HashSet::new();

    let mut cards: Vec<(Option<i64>, NewsCard)> = results
        .iter()
        .filter_map(|raw| {
            let host = host_of(&raw.url);
            if host.is_empty()
                || host_matches(&host, BLOCKED_HOSTS)
                || host_matches(&host, JUNK_HOSTS)
            {
                return None;
            }
            if is_junk_url(&raw.url) {
                return None;
            }
            // One card per URL: the news tab emits the same story with several
            // tracking parameters.
            let key = canonical_url(&raw.url);
            if !seen_urls.insert(key) {
                return None;
            }

            let topic_weight = topic_weights.get(topic).copied().unwrap_or(0.0) / max_weight;
            let recency = recency_factor(raw.age.as_deref(), now);
            let publisher = if host_matches(&host, NEWS_DOMAINS) { 1.0 } else { 0.0 };
            // How much of the user's *other* interests the headline touches.
            let overlap = headline_overlap(&raw.title, &raw.snippet, topic_weights) - topic_weight;

            // Relevance leads, but time decides close calls, and beyond the
            // fresh window it decides outright: measured against the live
            // engine, a topic-heavy coefficient let two- and three-week-old
            // stories hold the top slots over same-topic coverage from days
            // ago. The tier multiplier is what makes "this week" actually mean
            // this week on a shelf labelled news.
            let age_days = age_in_days(raw.age.as_deref(), now);
            let freshness_tier = match age_days {
                Some(days) if days <= FRESH_DAYS => 1.0,
                Some(days) if days <= 14 => 0.5,
                _ => 0.25,
            };
            let score = (topic_weight * 2.0 + recency * 3.0 + publisher * 1.0 + overlap.max(0.0) * 2.0)
                * freshness_tier;

            Some((
                age_in_days(raw.age.as_deref(), now),
                NewsCard {
                    title: raw.title.clone(),
                    url: raw.url.clone(),
                    source: if raw.source.is_empty() { host.clone() } else { raw.source.clone() },
                    snippet: truncate_chars(&raw.snippet, 220),
                    image: raw.image.clone(),
                    topic: topic.to_string(),
                    age: raw.age.clone(),
                    score: round2(score),
                },
            ))
        })
        .collect();

    let sort = |a: &(Option<i64>, NewsCard), b: &(Option<i64>, NewsCard)| {
        b.1.score
            .partial_cmp(&a.1.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    };

    // Fresh items first; stale ones are appended only as backfill, so a topic
    // with nothing recent still produces a shelf rather than an empty page. The
    // engine ignores recency operators (`when:7d` was measured to change
    // nothing), so this cut has to happen here.
    let (mut fresh, stale): (Vec<_>, Vec<_>) = cards
        .drain(..)
        .partition(|(age_days, _)| match age_days {
            Some(days) => *days <= STALE_AFTER_DAYS,
            None => true, // unknown age is not evidence of staleness
        });
    fresh.sort_by(sort);
    let mut stale = stale;
    stale.sort_by(sort);
    fresh.extend(stale);

    // Cap how much of the shelf one publisher can take. Applied after the
    // freshness partition so a prolific outlet cannot push other sources out,
    // and *before* the stale backfill so held-back diversity is not silently
    // re-filled by the same domain.
    let mut per_host: HashMap<String, usize> = HashMap::new();
    let mut picked: Vec<NewsCard> = Vec::new();
    let mut overflow: Vec<NewsCard> = Vec::new();
    for (_, card) in fresh {
        let host = host_of(&card.url);
        let seen = per_host.entry(host).or_insert(0);
        if *seen >= MAX_PER_HOST {
            overflow.push(card);
            continue;
        }
        *seen += 1;
        picked.push(card);
    }
    picked.extend(overflow);
    picked
}

/// Whether a result's URL says it is not a news article.
fn is_junk_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    JUNK_MARKERS.iter().any(|marker| lower.contains(marker))
}

/// A card's age in days when the engine's free text can be resolved.
///
/// `None` means "the engine did not say", which is treated as fresh-by-default
/// (see the partition above) rather than guessed at.
fn age_in_days(age: Option<&str>, now: chrono::DateTime<chrono::Utc>) -> Option<i64> {
    let age = age?;
    let lower = age.to_ascii_lowercase();
    let number: Option<i64> = lower
        .split_whitespace()
        .next()
        .and_then(|first| first.parse().ok());
    let n = number.unwrap_or(1).max(1);
    if lower.contains("minute") || lower.contains("just now") || lower.contains("moment")
        || lower.contains("hour")
    {
        return Some(0);
    }
    if lower.contains("yesterday") {
        return Some(1);
    }
    if lower.contains("day") {
        return Some(n);
    }
    if lower.contains("week") {
        return Some(n * 7);
    }
    if lower.contains("month") {
        return Some(n * 30);
    }
    if lower.contains("year") {
        return Some(n * 365);
    }
    parse_loose_date(&lower, now).map(|date| (now.date_naive() - date).num_days())
}

/// Strip tracking parameters so the same story is not shown twice.
fn canonical_url(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut parsed) => {
            let keep: Vec<(String, String)> = parsed
                .query_pairs()
                .filter(|(k, _)| !k.starts_with("utm_") && k != "fbclid" && k != "gclid" && k != "ref")
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            if keep.is_empty() {
                parsed.set_query(None);
            } else {
                let mut pairs = parsed.query_pairs_mut();
                pairs.clear();
                for (k, v) in keep {
                    pairs.append_pair(&k, &v);
                }
            }
            parsed.set_fragment(None);
            parsed.to_string()
        }
        Err(_) => url.to_string(),
    }
}

/// Recency from the engine's free-text age ("2 hours ago", "February 28, 2025").
///
/// Unknown ages score in the middle: not penalised as stale, not rewarded as
/// fresh, because the engine omits the age for plenty of real news.
///
/// The spread is deliberately wide. A "news" shelf that leads with a
/// three-week-old story because it matched the user's strongest interest
/// perfectly is not doing its job — verified against the live engine, where a
/// flat scale let two- and three-week-old items hold the top three slots.
fn recency_factor(age: Option<&str>, now: chrono::DateTime<chrono::Utc>) -> f64 {
    let Some(age) = age else { return 0.5 };
    let lower = age.to_ascii_lowercase();

    if lower.contains("minute") || lower.contains("just now") || lower.contains("moment") {
        return 1.0;
    }
    if lower.contains("hour") || lower.contains("heute") || lower.contains("oggi") {
        return 1.0;
    }
    if lower.contains("yesterday") || lower.contains("gisteren") {
        return 0.85;
    }
    if lower.contains("day") {
        return 0.7;
    }
    if lower.contains("week") {
        return 0.25;
    }
    if lower.contains("month") || lower.contains("year") {
        return 0.05;
    }

    // A calendar date: fresher than a month, older than a day.
    if let Some(date) = parse_loose_date(&lower, now) {
        let days = (now.date_naive() - date).num_days().max(0) as f64;
        return (1.0 - days / 60.0).clamp(0.05, 0.85);
    }
    0.5
}

/// Parse the date shapes the engine emits (`%B %d, %Y`, `%b %d, %Y`, ISO).
fn parse_loose_date(text: &str, now: chrono::DateTime<chrono::Utc>) -> Option<chrono::NaiveDate> {
    let cleaned = text.replace(',', " ");
    let mut parts = cleaned.split_whitespace();
    let month = parts.next()?;
    let day: u32 = parts.next()?.parse().ok()?;
    let year: i32 = parts.next()?.parse().ok()?;
    let month_num = month_number(month)?;
    chrono::NaiveDate::from_ymd_opt(year, month_num, day).filter(|d| *d <= now.date_naive())
}

fn month_number(name: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "january", "february", "march", "april", "may", "june", "july", "august", "september",
        "october", "november", "december",
    ];
    let lower = name.to_ascii_lowercase();
    MONTHS
        .iter()
        .position(|m| lower.starts_with(&m[..3]))
        .map(|i| i as u32 + 1)
}

/// Sum of the user's weights for the terms a headline mentions.
fn headline_overlap(title: &str, snippet: &str, topic_weights: &HashMap<String, f64>) -> f64 {
    let mut total = 0.0;
    let text = format!("{title} {snippet}").to_ascii_lowercase();
    for (term, weight) in topic_weights {
        if term.contains(' ') {
            if text.contains(term.as_str()) {
                total += *weight;
            }
        } else if text
            .split(|c: char| !c.is_alphanumeric())
            .any(|word| word == term)
        {
            total += *weight;
        }
    }
    let max = topic_weights.values().cloned().fold(1.0f64, f64::max);
    total / max
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/* ── Fetching (with a small cache) ────────────────────────────── */

struct CacheEntry {
    at: Instant,
    results: Vec<RawResult>,
}

fn cache() -> &'static parking_lot::Mutex<HashMap<String, CacheEntry>> {
    static CACHE: OnceLock<parking_lot::Mutex<HashMap<String, CacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
}

fn cached(engine_url: &str) -> Option<Vec<RawResult>> {
    let mut guard = cache().lock();
    let entry = guard.get(engine_url)?;
    if entry.at.elapsed() > CACHE_TTL {
        guard.remove(engine_url);
        return None;
    }
    Some(entry.results.clone())
}

fn store(engine_url: &str, results: Vec<RawResult>) {
    let mut guard = cache().lock();
    // A hard cap: the key is derived from user input, so an unbounded map would
    // be a slow leak on a long-lived server.
    if guard.len() > 48 {
        guard.clear();
    }
    guard.insert(
        engine_url.to_string(),
        CacheEntry {
            at: Instant::now(),
            results,
        },
    );
}

/// The news search URL for a topic, on whichever engine the window searches with.
fn engine_url(topic: &str, searxng: Option<&str>) -> String {
    let encoded = url::form_urlencoded::byte_serialize(topic.as_bytes()).collect::<String>();
    match searxng {
        Some(base) if !base.trim().is_empty() => {
            let base = base.trim_end_matches('/');
            // SearXNG's news category is the closest equivalent of the engine's
            // news tab; it also degrades to web results when unavailable.
            format!("{base}/search?q={encoded}&categories=news")
        }
        // Brave's news tab: same host the address bar already searches with, so
        // no new key, quota or dependency is introduced. The bare topic is
        // deliberately not qualified with "news": the news tab itself is the
        // recency signal, and counting the literal word in the profile would be
        // wrong. (Adding it does widen the result set — 23 → 45 in a live spot
        // check — but the ranking, not the query, is what decides freshness.)
        _ => format!("https://search.brave.com/news?q={encoded}&source=web"),
    }
}

/// Fetch and parse one topic's news results. Cached; never fatal.
async fn fetch_topic(topic: &str, searxng: Option<&str>) -> Result<Vec<RawResult>, String> {
    let url = engine_url(topic, searxng);
    if let Some(hit) = cached(&url) {
        return Ok(hit);
    }

    let client = shiny_filter::proxy::impersonated_client_builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|e| format!("http client: {e}"))?;

    let response = client
        .get(&url)
        // The engine is asked for a document, exactly like the page fetch: the
        // classification and the mislabelled-HTML fallback both key off this.
        .header("accept", crate::fetch::ACCEPT_HTML)
        .send()
        .await
        .map_err(|e| format!("search failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("search returned HTTP {}", response.status()));
    }
    let body = response
        .text()
        .await
        .map_err(|e| format!("could not read results: {e}"))?;
    let results = parse_results(&body);
    store(&url, results.clone());
    Ok(results)
}

/// The interest profile for a user, or an empty one when history is unusable.
pub fn interests_for(
    ctx: &std::sync::Arc<shiny_plugin_sdk::services::PluginCtx>,
    user_id: &str,
) -> Profile {
    let rows = history::recent_rows(ctx, user_id, 200).unwrap_or_default();
    profile_from_history(&rows, chrono::Utc::now())
}

/// Build the home shelf: profile → topics → fetch → rank → best N.
///
/// `refresh` skips the cache when the user asked for new cards.
pub async fn home_news(
    ctx: &std::sync::Arc<shiny_plugin_sdk::services::PluginCtx>,
    user_id: &str,
    limit: usize,
    refresh: bool,
) -> Result<HomeNews, AppError> {
    let rows = history::recent_rows(ctx, user_id, 200).unwrap_or_default();
    let profile = profile_from_history(&rows, chrono::Utc::now());
    let personalized = !profile.is_empty();

    let weights: HashMap<String, f64> = profile
        .interests
        .iter()
        .map(|i| (i.term.clone(), i.weight))
        .collect();
    let topics = select_topics(&profile, MAX_TOPIC_FETCHES);

    if topics.is_empty() {
        return Ok(HomeNews {
            cards: Vec::new(),
            topics: Vec::new(),
            personalized: false,
            error: None,
        });
    }

    if refresh {
        // Drop this user's topics from the cache; other users are unaffected.
        let searxng = std::env::var("SEARXNG_URL").ok();
        let mut guard = cache().lock();
        for topic in topics.iter().take(MAX_TOPIC_FETCHES) {
            guard.remove(&engine_url(topic, searxng.as_deref()));
        }
    }

    let searxng = std::env::var("SEARXNG_URL").ok();
    let now = chrono::Utc::now();
    let mut cards: Vec<NewsCard> = Vec::new();
    let mut error: Option<String> = None;

    // Fetch the topics concurrently. Serially, a cold home load was one engine
    // round trip after another (~3.7s measured); the requests are independent,
    // so there is no reason to queue them.
    let fetches = topics
        .iter()
        .take(MAX_TOPIC_FETCHES)
        .map(|topic| fetch_topic(topic, searxng.as_deref()));
    for (topic, result) in topics.iter().take(MAX_TOPIC_FETCHES).zip(
        futures::future::join_all(fetches).await,
    ) {
        match result {
            Ok(results) => cards.extend(rank_results(&results, topic, &weights, now)),
            Err(err) => {
                tracing::debug!("browser: news fetch for '{topic}' failed: {err}");
                error.get_or_insert(err);
            }
        }
    }

    // Interleave by topic before truncating, so one strong interest cannot fill
    // the whole shelf: sort by score, then greedily cap each topic's share.
    cards.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let per_topic_cap = (limit / topics.len().max(1)).max(3);
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut picked: Vec<NewsCard> = Vec::new();
    for card in cards.iter() {
        let seen = counts.entry(card.topic.clone()).or_insert(0);
        if *seen >= per_topic_cap {
            continue;
        }
        *seen += 1;
        picked.push(card.clone());
    }
    // Backfill if the cap left the shelf short.
    if picked.len() < limit {
        for card in cards.iter() {
            if picked.len() >= limit {
                break;
            }
            if !picked.iter().any(|c| c.url == card.url) {
                picked.push(card.clone());
            }
        }
    }
    picked.truncate(limit);

    if picked.is_empty() && error.is_none() {
        error = Some("no results for your recent searches".into());
    }

    Ok(HomeNews {
        cards: picked,
        // What the shelf was actually built from, then the profile's strongest
        // remaining labels — the chips are the user's window into the ranking.
        topics: {
            let mut shown = topics.clone();
            for interest in profile.interests.iter() {
                if shown.len() >= 6 {
                    break;
                }
                if !shown.contains(&interest.term) {
                    shown.push(interest.term.clone());
                }
            }
            shown.truncate(6);
            shown
        },
        personalized,
        error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn row(url: &str, query: Option<&str>, mode: &str, at: &str) -> HistoryRow {
        HistoryRow {
            url: url.into(),
            query: query.map(str::to_string),
            mode: mode.into(),
            created_at: at.into(),
        }
    }

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(2026, 9, 13, 12, 0, 0).unwrap()
    }

    #[test]
    fn tokenizer_drops_noise_and_keeps_terms() {
        let tokens = tokenize("Best Wireless-Keyboard review 2026: the 3 top! https://x.com");
        assert!(tokens.contains(&"wireless".to_string()), "{tokens:?}");
        assert!(tokens.contains(&"keyboard".to_string()), "{tokens:?}");
        assert!(!tokens.contains(&"2026".to_string()), "a year is not an interest: {tokens:?}");
        // Filler, bare numbers and the shopping vocabulary every product URL
        // carries never become interests, or the profile fills with noise.
        assert!(!tokens.contains(&"the".to_string()), "{tokens:?}");
        assert!(!tokens.contains(&"3".to_string()), "{tokens:?}");
        assert!(!tokens.contains(&"best".to_string()), "{tokens:?}");
        assert!(!tokens.contains(&"review".to_string()), "{tokens:?}");
        assert!(!tokens.contains(&"com".to_string()), "{tokens:?}");
    }

    #[test]
    fn profile_prefers_recent_searches_over_old_urls() {
        let rows = vec![
            row("https://example.com/a", Some("mechanical keyboard"), "page", "2026-09-13 11:00:00"),
            row("https://example.com/old", None, "page", "2026-06-01 11:00:00"),
        ];
        let profile = profile_from_history(&rows, now());
        let keyboard = profile
            .interests
            .iter()
            .find(|i| i.term == "keyboard")
            .unwrap();
        // The old visit should have decayed to essentially nothing.
        assert!(keyboard.weight > 1.5, "got {keyboard:?}");
        assert!(profile.interests.iter().all(|i| i.weight > 0.0));
        assert!(profile.is_empty() == false);
    }

    #[test]
    fn search_host_urls_do_not_leak_query_tokens() {
        let rows = vec![row(
            "https://search.brave.com/search?q=aurora+borealis+forecast",
            None,
            "page",
            "2026-09-13 11:00:00",
        )];
        let profile = profile_from_history(&rows, now());
        // The URL path is skipped for search hosts, so its slug words are absent.
        assert!(
            !profile
                .interests
                .iter()
                .any(|i| i.term == "search" || i.term == "forecast"),
            "search-host slug leaked into the profile: {profile:?}"
        );
    }

    #[test]
    fn topic_selection_does_not_spend_every_slot_on_one_interest() {
        // Measured live: after the user searched for quantum computing and
        // electric-vehicle batteries, the top tokens were still three words of
        // the *first* search, so the shelf never moved on.
        // The co-occurrence counts are what a real profile carries: this user
        // searched "aurora borealis", "solar storm" and "aurora storm", so all
        // four of those words are one subject, while battery/quantum are not.
        let profile = Profile {
            interests: vec![
                Interest { term: "aurora".into(), weight: 9.0 },
                Interest { term: "storm".into(), weight: 7.0 },
                Interest { term: "solar".into(), weight: 6.0 },
                Interest { term: "borealis".into(), weight: 5.0 },
                Interest { term: "battery".into(), weight: 4.0 },
                Interest { term: "quantum".into(), weight: 3.0 },
            ],
            pairs: [
                ("aurora borealis".to_string(), 3.0),
                ("solar storm".to_string(), 3.0),
                ("aurora storm".to_string(), 2.0),
                ("solar flare".to_string(), 1.0), // seen once: not a relation
            ]
            .into_iter()
            .collect(),
            adjacency: HashMap::new(),
        };
        let topics = select_topics(&profile, 4);
        // The strongest interest leads, but as the specific phrase rather than
        // the bare word it shares with three others.
        assert_eq!(topics[0], "aurora borealis", "{topics:?}");
        assert!(
            topics.contains(&"battery".to_string()) && topics.contains(&"quantum".to_string()),
            "the shelf never leaves the first search: {topics:?}"
        );
        // "storm", "solar" and "borealis" are that same subject.
        for duplicate in ["storm", "solar", "borealis"] {
            assert!(
                !topics.contains(&duplicate.to_string()),
                "{duplicate} was searched as its own interest: {topics:?}"
            );
        }
        assert_eq!(topics.len(), 4);
    }

    #[test]
    fn broad_words_are_skipped_in_favour_of_phrases() {
        // Measured live: "breakthrough", "computing" and "electric" are so
        // generic that they matched docking-station reviews and a tennis final.
        // A word that pairs with many different words is filler, and the
        // specific phrase the user actually typed is the better query.
        let profile = Profile {
            interests: vec![
                Interest { term: "breakthrough".into(), weight: 9.0 },
                Interest { term: "computing".into(), weight: 8.0 },
                Interest { term: "quantum".into(), weight: 7.0 },
                Interest { term: "electric".into(), weight: 6.0 },
                Interest { term: "vehicle".into(), weight: 5.0 },
            ],
            pairs: [
                ("quantum computing".to_string(), 2.0),
                ("electric vehicle".to_string(), 2.0),
                // "breakthrough" pairs with everything, "computing" with three
                // other things: both are too broad to search on their own.
                ("breakthrough research".to_string(), 2.0),
                ("ai breakthrough".to_string(), 2.0),
                ("medical breakthrough".to_string(), 2.0),
                ("scientific breakthrough".to_string(), 2.0),
                ("cloud computing".to_string(), 2.0),
                ("edge computing".to_string(), 2.0),
                ("grid computing".to_string(), 2.0),
            ]
            .into_iter()
            .collect(),
            adjacency: HashMap::new(),
        };
        let topics = select_topics(&profile, 4);
        assert_eq!(topics[0], "quantum computing", "{topics:?}");
        assert!(topics.contains(&"electric vehicle".to_string()), "{topics:?}");
        assert!(
            !topics.iter().any(|t| t == "breakthrough"),
            "a word paired with everything was searched: {topics:?}"
        );
    }

    #[test]
    fn the_real_history_produces_specific_topics() {
        // Replays the exact rows a live session accumulated (7 navigations, 4
        // typed searches) — the case that first exposed the generic-word
        // problem, where "breakthrough" and "computing" from one sentence
        // turned into two separate interests and filled half the shelf with
        // docking-station reviews and a tennis final.
        let stamp = "2026-09-13 21:53:00";
        let rows = vec![
            row("https://search.brave.com/search?q=quantum+computing+breakthrough",
                Some("quantum computing breakthrough"), "page", stamp),
            row("https://search.brave.com/search?q=electric+vehicle+battery",
                Some("electric vehicle battery"), "page", stamp),
            row("https://example.com/a", None, "page", stamp),
            row("https://example.com/b", None, "page", stamp),
            row("https://example.com/c", None, "page", stamp),
            row("https://search.brave.com/search?q=solar+storm+aurora",
                Some("solar storm aurora"), "page", stamp),
            row("https://search.brave.com/search?q=aurora+borealis+forecast",
                Some("aurora borealis forecast"), "page", stamp),
        ];
        let profile = profile_from_history(&rows, now());
        let topics = select_topics(&profile, MAX_TOPIC_FETCHES);

        // Each interest is searched as the phrase the user typed, not as one
        // generic word out of it.
        assert_eq!(
            topics,
            vec![
                "aurora borealis forecast",
                "electric vehicle battery",
                "quantum computing breakthrough",
                "solar storm aurora",
            ],
            "the shelf would be built from the wrong queries"
        );
        for generic in ["breakthrough", "computing", "electric", "battery", "storm"] {
            assert!(
                !topics.iter().any(|t| t == generic),
                "a generic single word was searched: {topics:?}"
            );
        }
    }

    #[test]
    fn a_phrase_topic_is_selected_when_its_words_are_new() {
        let profile = Profile {
            interests: vec![
                Interest { term: "wireless keyboard".into(), weight: 9.0 },
                Interest { term: "keyboard".into(), weight: 6.0 },
                Interest { term: "aurora".into(), weight: 3.0 },
            ],
            pairs: HashMap::new(),
            adjacency: HashMap::new(),
        };
        let topics = select_topics(&profile, 3);
        // The phrase goes first — it is a more specific query than either word —
        // and the now-covered "keyboard" is skipped rather than repeated.
        assert_eq!(topics, vec!["wireless keyboard", "aurora"]);
    }

    #[test]
    fn parses_engine_snippets() {
        let html = include_str!("../testdata/brave_news_sample.html");
        let results = parse_results(html);
        assert!(results.len() >= 3, "parsed {} results", results.len());
        let first = &results[0];
        assert!(first.url.starts_with("https://"), "{first:?}");
        assert!(!first.title.is_empty(), "{first:?}");
        assert!(!first.source.is_empty(), "{first:?}");
        assert!(!first.url.contains("/p/https/"), "proxy URL leaked: {first:?}");
        assert!(!first.url.contains("search.brave.com"), "engine link leaked: {first:?}");
        // The story thumbnail is kept, and the publisher favicon (which shares
        // the same image CDN and is rendered first) is not mistaken for it.
        let image = first.image.as_deref().expect("first result has a thumbnail");
        assert!(image.starts_with("https://"), "bad image URL: {image}");
        assert!(!image.contains("favicon"), "favicon chosen as the thumbnail: {image}");
    }

    #[test]
    fn first_content_image_skips_favicons() {
        let block = r#"<img class="favicon-background x" src="https://imgs.example/fav-bg.png">
            <img alt="🌐" class="favicon news size-xs" src="https://imgs.example/fav.png">
            <img class="thumb x" src="https://imgs.example/story.jpg" width="112">
            <img src="https://imgs.example/second.jpg">"#;
        assert_eq!(
            first_content_image(block).as_deref(),
            Some("https://imgs.example/story.jpg")
        );
        assert_eq!(first_content_image("<p>no image</p>"), None);
        // `data-src` must not be read as `src`.
        assert_eq!(first_content_image(r#"<img data-src="https://x/a.png">"#), None);
    }

    #[test]
    fn parse_survives_garbage() {
        assert!(parse_results("").is_empty());
        assert!(parse_results("<div>not a result</div>").is_empty());
        assert!(parse_results("class=\"snippet\" href=\"not-a-url\"").is_empty());
    }

    #[test]
    fn ranking_drops_shops_and_duplicates_and_boosts_news() {
        let weights: HashMap<String, f64> =
            [("keyboard".to_string(), 4.0)].into_iter().collect();
        let results = vec![
            RawResult {
                title: "Wireless keyboard guide".into(),
                url: "https://www.amazon.com/dp/B0?utm_source=x".into(),
                source: "Amazon".into(),
                age: Some("1 hour ago".into()),
                snippet: "Buy now".into(),
                image: None,
            },
            RawResult {
                title: "The best wireless keyboards of 2026".into(),
                url: "https://www.theverge.com/keyboards?utm_source=x".into(),
                source: "The Verge".into(),
                age: Some("2 hours ago".into()),
                snippet: "Keyboard reviews".into(),
                image: None,
            },
            RawResult {
                title: "The best wireless keyboards of 2026".into(),
                url: "https://www.theverge.com/keyboards".into(),
                source: "The Verge".into(),
                age: Some("2 hours ago".into()),
                snippet: "Duplicate story".into(),
                image: None,
            },
            RawResult {
                title: "Keyboard".into(),
                url: "https://www.pcmag.com/picks/keyboards".into(),
                source: "PCMag".into(),
                age: Some("5 months ago".into()),
                snippet: "Old".into(),
                image: None,
            },
        ];
        let cards = rank_results(&results, "keyboard", &weights, now());
        assert_eq!(cards.len(), 2, "shop or duplicate survived: {cards:?}");
        assert!(cards[0].url.contains("theverge.com"));
        assert!(cards[0].score > cards[1].score, "{cards:?}");
    }

    #[test]
    fn a_week_old_story_outranks_a_stale_one_on_the_same_topic() {
        // The behaviour this encodes was verified against the live engine: a
        // flat recency scale let two- and three-week-old items hold the top
        // slots of a "news" shelf while same-topic coverage from days ago sat
        // below them.
        let weights: HashMap<String, f64> =
            [("aurora".to_string(), 5.0)].into_iter().collect();
        let results = vec![
            RawResult {
                title: "Northern lights forecast for this weekend".into(),
                url: "https://www.example-news.com/aurora-weekend".into(),
                source: "Example News".into(),
                age: Some("3 weeks ago".into()),
                snippet: "Aurora forecast".into(),
                image: None,
            },
            RawResult {
                title: "Northern lights visible tonight".into(),
                url: "https://www.example-news.com/aurora-tonight".into(),
                source: "Example News".into(),
                age: Some("4 days ago".into()),
                snippet: "Aurora alert".into(),
                image: None,
            },
        ];
        let cards = rank_results(&results, "aurora", &weights, now());
        assert_eq!(cards.len(), 2);
        assert!(
            cards[0].url.ends_with("aurora-tonight"),
            "freshness did not win: {cards:?}"
        );
        assert!(cards[0].score > cards[1].score, "{cards:?}");
    }

    #[test]
    fn stale_cards_still_backfill_an_otherwise_empty_shelf() {
        // Holding back stale items must not turn "nothing recent" into "nothing".
        let weights: HashMap<String, f64> =
            [("aurora".to_string(), 5.0)].into_iter().collect();
        let results = vec![RawResult {
            title: "A much older aurora study".into(),
            url: "https://www.example-news.com/old-aurora".into(),
            source: "Example News".into(),
            age: Some("8 months ago".into()),
            snippet: "Aurora".into(),
            image: None,
        }];
        let cards = rank_results(&results, "aurora", &weights, now());
        assert_eq!(cards.len(), 1, "the shelf was emptied instead of backfilled");
    }

    #[test]
    fn junk_paths_and_hosts_are_dropped() {
        let weights: HashMap<String, f64> =
            [("aurora".to_string(), 5.0)].into_iter().collect();
        let results = vec![
            RawResult {
                title: "Aurora Build, Augments and Items".into(),
                url: "https://www.aram-mayhem.com/en/aurora-build".into(),
                source: "ARAM Mayhem".into(),
                age: Some("4 days ago".into()),
                snippet: "Aurora build".into(),
                image: None,
            },
            RawResult {
                title: "Aurora (disambiguation)".into(),
                url: "https://example.fandom.com/wiki/Aurora".into(),
                source: "Fandom".into(),
                age: Some("2 days ago".into()),
                snippet: "Aurora".into(),
                image: None,
            },
            RawResult {
                title: "Northern lights visible tonight".into(),
                url: "https://www.example-news.com/aurora-tonight".into(),
                source: "Example News".into(),
                age: Some("1 day ago".into()),
                snippet: "Aurora".into(),
                image: None,
            },
        ];
        let cards = rank_results(&results, "aurora", &weights, now());
        assert_eq!(cards.len(), 1, "junk survived: {cards:?}");
        assert!(cards[0].url.ends_with("aurora-tonight"));
    }

    #[test]
    fn one_publisher_cannot_fill_the_shelf() {
        let weights: HashMap<String, f64> =
            [("aurora".to_string(), 5.0)].into_iter().collect();
        let mut results = Vec::new();
        for i in 0..6 {
            results.push(RawResult {
                title: format!("Aurora story {i}"),
                url: format!("https://www.prolific.com/aurora-{i}"),
                source: "Prolific".into(),
                age: Some("1 day ago".into()),
                snippet: "Aurora".into(),
                image: None,
            });
        }
        results.push(RawResult {
            title: "Aurora from another outlet".into(),
            url: "https://www.other.com/aurora".into(),
            source: "Other".into(),
            age: Some("2 days ago".into()),
            snippet: "Aurora".into(),
            image: None,
        });
        let cards = rank_results(&results, "aurora", &weights, now());
        // The cap decides *order*: other outlets come before a publisher's
        // third card, so the top of the shelf is never one masthead.
        let first_other = cards.iter().position(|c| c.url.contains("other.com"));
        let third_prolific = cards
            .iter()
            .enumerate()
            .filter(|(_, c)| c.url.contains("prolific.com"))
            .nth(MAX_PER_HOST)
            .map(|(index, _)| index);
        assert!(first_other.is_some(), "{cards:?}");
        assert!(
            first_other < third_prolific,
            "a third card from one host outranked another outlet: {cards:?}"
        );
        // Nothing is discarded outright: overflow is kept behind the rest, so
        // `home_news`'s own `limit` — not this function — decides the cutoff.
        assert_eq!(cards.len(), 7, "{cards:?}");
    }

    #[test]
    fn ages_parse_into_days() {
        assert_eq!(age_in_days(Some("30 minutes ago"), now()), Some(0));
        assert_eq!(age_in_days(Some("13 hours ago"), now()), Some(0));
        assert_eq!(age_in_days(Some("2 days ago"), now()), Some(2));
        assert_eq!(age_in_days(Some("3 weeks ago"), now()), Some(21));
        assert_eq!(age_in_days(Some("2 months ago"), now()), Some(60));
        assert_eq!(age_in_days(Some("September 11, 2026"), now()), Some(2));
        // An age the engine did not state is not evidence of staleness.
        assert_eq!(age_in_days(None, now()), None);
    }

    #[test]
    fn unknown_age_is_neutral_and_dates_decay() {
        assert_eq!(recency_factor(None, now()), 0.5);
        assert!(recency_factor(Some("10 minutes ago"), now()) > 0.9);
        // The shelf must not lead with month-old stories, so stale ages fall
        // far enough that only a much stronger topic match can outrank freshness.
        assert!(recency_factor(Some("3 weeks ago"), now()) < 0.3);
        assert!(recency_factor(Some("2 months ago"), now()) < 0.1);
        let old_date = recency_factor(Some("February 28, 2025"), now());
        assert!((0.05..=0.85).contains(&old_date), "got {old_date}");
        // A date from this week sits between "yesterday" and "a week ago".
        let recent = recency_factor(Some("September 11, 2026"), now());
        assert!(recent > 0.5, "got {recent}");
    }

    #[test]
    fn canonical_url_strips_tracking() {
        assert_eq!(
            canonical_url("https://a.com/x?utm_source=t&id=2#frag"),
            "https://a.com/x?id=2"
        );
        assert_eq!(canonical_url("https://a.com/x?utm_source=t"), "https://a.com/x");
    }

    #[test]
    fn search_engine_urls_are_recognised() {
        assert!(is_search_host("search.brave.com"));
        assert!(is_search_host("www.google.com"));
        assert!(!is_search_host("notgoogle.com"));
    }

    #[test]
    fn host_of_strips_www_and_ports() {
        assert_eq!(host_of("https://www.bbc.co.uk/news/x"), "bbc.co.uk");
        assert_eq!(host_of("http://127.0.0.1:8080/a"), "127.0.0.1");
        assert_eq!(host_of("not a url"), "");
    }

    /* ── End to end: history → profile → engine → ranked cards ─────────
     *
     * The unit tests above cover each stage; this one covers the seam between
     * them, which is where the feature actually lives: rows in the database,
     * an HTTP fetch, cards out. It runs against a loopback stub engine (through
     * the same `SEARXNG_URL` configuration the product supports for a
     * self-hosted instance), so the real fetch path is exercised without
     * depending on the public internet.
     */

    /// Serialises the tests that mutate `SEARXNG_URL`, which is process-global.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// A tiny HTTP server that answers every request with `body`.
    async fn stub_engine(body: String) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stub engine");
        let addr = listener.local_addr().expect("stub addr");
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let body = body.clone();
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = vec![0u8; 8192];
                    let _ = socket.read(&mut buf).await;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.flush().await;
                });
            }
        });
        (format!("http://{addr}"), handle)
    }

    /// A ctx over a scratch database holding the given `peakd_history` rows
    /// (`url`, optional typed query, `created_at`).
    fn ctx_with_history(
        path: &str,
        rows: &[(&str, Option<&str>, &str)],
    ) -> std::sync::Arc<shiny_plugin_sdk::services::PluginCtx> {
        use shiny_plugin_sdk::db::{Db, Value};
        let _ = std::fs::remove_file(path);
        let ctx = crate::history::tests_support::ctx_for(path);
        let db = Db::open(&format!("sqlite://{path}")).expect("open scratch db");
        db.execute(
            "CREATE TABLE peakd_history (id TEXT PRIMARY KEY, traveler_id TEXT NOT NULL,
             url TEXT NOT NULL, mode TEXT NOT NULL DEFAULT 'page', query TEXT,
             created_at TEXT NOT NULL DEFAULT (datetime('now')))",
            &[],
        )
        .expect("create peakd_history");
        db.execute(
            "CREATE TABLE travelers (id TEXT PRIMARY KEY, name TEXT NOT NULL,
             email TEXT NOT NULL UNIQUE, password_hash TEXT NOT NULL)",
            &[],
        )
        .expect("create travelers");
        db.execute(
            "INSERT INTO travelers (id, name, email, password_hash) VALUES ('u1','u','u@x','h')",
            &[],
        )
        .expect("seed traveler");
        for (index, (url, query, at)) in rows.iter().enumerate() {
            db.execute(
                "INSERT INTO peakd_history (id, traveler_id, url, mode, query, created_at)
                 VALUES (?1, 'u1', ?2, 'page', ?3, ?4)",
                &[
                    Value::text(format!("row-{index}")),
                    Value::text((*url).to_string()),
                    match query {
                        Some(q) => Value::text((*q).to_string()),
                        None => Value::Null,
                    },
                    Value::text((*at).to_string()),
                ],
            )
            .expect("seed history row");
        }
        ctx
    }

    #[test]
    fn home_news_end_to_end_ranks_from_history() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // A real capture for a keyboard query, served by the stub "engine".
        let fixture = include_str!("../testdata/brave_news_sample.html");
        // Timestamps are compared against `Utc::now()` when the profile decays,
        // so the rows must be recent or every weight decays to nothing.
        let today = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let ctx = ctx_with_history(
            "/tmp/browser-news-e2e.db",
            &[
                (
                    "https://search.brave.com/search?q=wireless+keyboard",
                    Some("wireless keyboard"),
                    &today,
                ),
                (
                    "https://search.brave.com/search?q=wireless+keyboard+review",
                    Some("wireless keyboard review"),
                    &today,
                ),
                ("https://example.com/archive", None, "2025-01-01 10:00:00"),
            ],
        );

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let (base, server) = stub_engine(fixture.to_string()).await;
            std::env::set_var("SEARXNG_URL", &base);

            let news = home_news(&ctx, "u1", 12, false).await.expect("home_news");

            std::env::remove_var("SEARXNG_URL");
            server.abort();

            assert!(news.personalized, "profile should be non-empty");
            assert!(
                news.topics.iter().any(|t| t == "keyboard" || t == "wireless"),
                "topics were {:?}",
                news.topics
            );
            assert!(!news.cards.is_empty(), "no cards: {news:?}");
            // Every card must be a real, direct URL — never the proxy path form
            // and never a search-engine link.
            for card in &news.cards {
                assert!(card.url.starts_with("http"), "{card:?}");
                assert!(!card.url.contains("/p/https/"), "{card:?}");
                assert!(!card.url.contains("search.brave.com"), "{card:?}");
                assert!(!card.source.is_empty(), "{card:?}");
                assert!(card.score > 0.0, "{card:?}");
            }
            // The shelf carries thumbnails the home surface can render.
            assert!(
                news.cards.iter().any(|c| c.image.is_some()),
                "no card carried an image: {news:?}"
            );
        });
    }

    #[test]
    fn home_news_is_empty_without_history() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let ctx = ctx_with_history("/tmp/browser-news-empty.db", &[]);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let news = home_news(&ctx, "u1", 12, false).await.expect("home_news");
            assert!(!news.personalized);
            assert!(news.cards.is_empty());
            // An empty profile is a normal state the UI explains, not an error.
            assert!(news.error.is_none(), "{news:?}");
        });
    }
}
