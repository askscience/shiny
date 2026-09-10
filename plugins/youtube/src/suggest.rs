//! "Up next" recommendations — a deliberately small content-based ranker.
//!
//! There is no ML and no YouTube API here. The idea is:
//!
//! 1. Turn the video you are watching (and what you watched before) into a bag
//!    of content words — [`keywords`].
//! 2. Ask the ordinary search scraper for a handful of candidates, using
//!    queries derived from the seed ([`queries_for`]).
//! 3. Score every candidate on keyword overlap, same-channel affinity, watch
//!    history affinity and how early YouTube ranked it — [`rank`].
//!
//! It is cheap, explainable and good enough to feel like a "related videos"
//! rail. The scoring and tokenising halves are pure functions so they can be
//! unit-tested without the network.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use shiny_plugin_sdk::errors::AppError;

use crate::youtube_client::{self, VideoResult};

/// A video used as a recommendation seed, or a remembered watch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Seed {
    #[serde(default)]
    pub video_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub channel: String,
}

impl Seed {
    /// True when there is enough here to key a recommendation off.
    pub fn is_usable(&self) -> bool {
        !self.video_id.is_empty() || !self.title.is_empty()
    }
}

/* ── scoring weights ────────────────────────────────────────── */

const W_TITLE: f64 = 3.0; // keyword overlap with the seed title
const W_CHANNEL: f64 = 2.5; // same channel as the seed
const W_HISTORY: f64 = 1.5; // overlap with what you watched before
const W_RANK: f64 = 1.0; // YouTube's own ordering, as a weak popularity prior
const W_SHORT: f64 = 0.75; // penalty for sub-minute (Shorts-ish) results
const HISTORY_CAP: usize = 60;
const HISTORY_AFFINITY: usize = 12;

/// Words that carry no signal about *what* a video is.
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "of", "to", "in", "on", "for", "with", "is", "it", "this",
    "that", "you", "your", "my", "me", "we", "he", "she", "they", "i", "at", "by", "from", "as",
    "be", "are", "was", "were", "will", "can", "just", "official", "video", "full", "hd", "4k",
    "lyrics", "lyric", "audio", "live", "feat", "ft", "new", "song", "songs", "music", "remix",
    "version", "edit", "mix", "vs", "ep", "album", "track", "best", "top", "how",
    // Generic video furniture that otherwise shows up as category chips.
    "episode", "episodes", "season", "seasons", "trailer", "trailers", "preview", "previews",
    "clip", "clips", "short", "shorts", "part", "parts", "scene", "scenes", "compilation",
    "reaction", "reactions", "moment", "moments", "movie", "movies", "series", "show", "shows",
    "watch", "entire", "every", "all", "more", "most", "gets", "get", "got", "make", "makes",
    "made", "one", "two", "three", "first", "last", "back", "out", "up", "down", "over",
    // Short function words from other languages, so 2-letter terms like "ai",
    // "3d" or "vr" can survive the length filter.
    "ed", "el", "la", "le", "de", "da", "di", "al", "un", "il", "lo", "en", "et", "du", "au",
    "und", "der", "die", "das", "les", "des",
];

/* ── per-user model: watches + category signals ─────────────── */

/// A topic signal: the terms a watch or a search contributed.
struct Signal {
    terms: Vec<(String, bool)>,
}

/// Everything remembered about one user.
#[derive(Default)]
struct UserModel {
    watches: VecDeque<Seed>,
    signals: VecDeque<Signal>,
}

type Store = Mutex<HashMap<String, UserModel>>;

fn store() -> &'static Store {
    static S: OnceLock<Store> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

const SIGNAL_CAP: usize = 200;

/// Add a term, keeping the strongest trust we've seen for it. `trusted` marks
/// terms that are meaningful on their own (a query phrase, a channel name);
/// title keywords are untrusted and must recur before they become a chip.
fn push_term(out: &mut Vec<(String, bool)>, term: String, trusted: bool) {
    if term.is_empty() {
        return;
    }
    if let Some(slot) = out.iter_mut().find(|(t, _)| *t == term) {
        slot.1 |= trusted;
    } else {
        out.push((term, trusted));
    }
}

/// Terms one event contributes. A whole short query is kept as a phrase so
/// "harry potter" stays one category instead of splitting into two words; a
/// channel is its own topic; a watch (or a long query) contributes the
/// strongest title keywords instead.
fn signal_terms(text: &str, channel: &str, keep_phrase: bool) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    let phrase = text.trim().to_lowercase();
    let short_phrase = keep_phrase && !phrase.is_empty() && phrase.split_whitespace().count() <= 5;
    if short_phrase {
        push_term(&mut out, phrase, true);
    }
    let chan = channel.trim().to_lowercase();
    if !chan.is_empty() {
        push_term(&mut out, chan, true);
    }
    if !short_phrase {
        // ≥4 chars keeps filler like "hip"/"hop" out of the chips.
        for term in keywords(text).into_iter().filter(|t| t.len() >= 4).take(4) {
            push_term(&mut out, term, false);
        }
    }
    out
}

fn push_signal(model: &mut UserModel, terms: Vec<(String, bool)>) {
    if terms.is_empty() {
        return;
    }
    model.signals.push_front(Signal { terms });
    model.signals.truncate(SIGNAL_CAP);
}

/// Remember a watch: most-recent-first, deduped by `video_id`, capped. The
/// watch also feeds the category model (its channel + title keywords).
pub fn remember(user: &str, item: &Seed) {
    if item.video_id.is_empty() && item.title.is_empty() {
        return;
    }
    let terms = signal_terms(&item.title, &item.channel, false);
    let Ok(mut all) = store().lock() else { return };
    let model = all.entry(user.to_string()).or_default();
    if !item.video_id.is_empty() {
        model.watches.retain(|s| s.video_id != item.video_id);
    }
    model.watches.push_front(item.clone());
    model.watches.truncate(HISTORY_CAP);
    push_signal(model, terms);
}

/// Record a search as a category signal — both the user's in-tile searches and
/// the AI's `youtube_search` calls land here.
pub fn remember_query(user: &str, query: &str) {
    let terms = signal_terms(query, "", true);
    if terms.is_empty() {
        return;
    }
    let Ok(mut all) = store().lock() else { return };
    push_signal(all.entry(user.to_string()).or_default(), terms);
}

/// The user's most recent watches, newest first.
pub fn recent(user: &str, limit: usize) -> Vec<Seed> {
    let Ok(all) = store().lock() else { return Vec::new() };
    all.get(user)
        .map(|m| m.watches.iter().take(limit).cloned().collect())
        .unwrap_or_default()
}

/* ── categories ─────────────────────────────────────────────── */

/// A topic chip: the name plus how strongly this user leans toward it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Category {
    pub name: String,
    pub score: f64,
    pub count: u32,
}

/// Category names ranked by usage: recent signals weigh more (so a topic the
/// user keeps coming back to accumulates), most-watched first. At most `limit`,
/// hard-capped at 20.
pub fn categories(user: &str, limit: usize) -> Vec<Category> {
    let Ok(all) = store().lock() else { return Vec::new() };
    let Some(model) = all.get(user) else { return Vec::new() };

    #[derive(Clone, Copy, Default)]
    struct Agg {
        score: f64,
        count: u32,
        trusted: bool,
    }

    let mut agg: HashMap<&str, Agg> = HashMap::new();
    for (age, signal) in model.signals.iter().enumerate() {
        let weight = 1.0 / (1.0 + age as f64 * 0.10); // recency decay
        for (term, trusted) in &signal.terms {
            let entry = agg.entry(term.as_str()).or_default();
            entry.score += weight;
            entry.count += 1;
            entry.trusted |= *trusted;
        }
    }

    // Candidates: a title keyword has to recur before it becomes a chip —
    // one-off odd words are what made the list look random.
    let mut candidates: Vec<(String, f64, u32)> = agg
        .into_iter()
        .filter(|(name, a)| name.chars().count() >= 3 && (a.trusted || a.count >= 2))
        .map(|(name, a)| (name.to_string(), a.score, a.count))
        .collect();
    candidates.sort_by(|a, b| {
        // Multi-word topics first, so fragments have a parent to fold into.
        b.0.split_whitespace()
            .count()
            .cmp(&a.0.split_whitespace().count())
            .then(b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
            .then(b.0.len().cmp(&a.0.len()))
            .then(a.0.cmp(&b.0))
    });

    // Fold fragments into the topic they belong to: "rick" and "mort" land in
    // "rick and morty", "salvatore san" in "salvatore sanfilippo". Topics that
    // merely overlap (adult swim vs rick and morty) stay separate.
    let mut clusters: Vec<(BTreeSet<String>, Vec<(String, f64, u32)>)> = Vec::new();
    for (name, score, count) in candidates {
        let tokens = name_tokens(&name);
        let mut best: Option<(usize, f64)> = None;
        for (i, (cluster_tokens, _)) in clusters.iter().enumerate() {
            let j = cluster_similarity(cluster_tokens, &tokens);
            let score = if j >= 0.6 {
                j + 1.0
            } else if (cluster_tokens.len() == 1 || tokens.len() == 1) && j > 0.0 {
                j // fragment rule
            } else {
                continue;
            };
            if best.map_or(true, |(_, bs)| score > bs) {
                best = Some((i, score));
            }
        }
        match best {
            Some((i, _)) => {
                let (cluster_tokens, members) = &mut clusters[i];
                for t in tokens {
                    cluster_tokens.insert(t);
                }
                members.push((name, score, count));
            }
            None => clusters.push((tokens, vec![(name, score, count)])),
        }
    }

    let mut out: Vec<Category> = clusters
        .into_iter()
        .map(|(_, members)| {
            let score: f64 = members.iter().map(|m| m.1).sum();
            let count: u32 = members.iter().map(|m| m.2).sum();
            // The representative is the most descriptive member — most words,
            // then best score, then longest name — so never a bare fragment.
            let name = members
                .into_iter()
                .max_by(|a, b| {
                    a.0.split_whitespace()
                        .count()
                        .cmp(&b.0.split_whitespace().count())
                        .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                        .then(a.0.len().cmp(&b.0.len()))
                })
                .map(|m| m.0)
                .unwrap_or_default();
            Category {
                name,
                score: (score * 100.0).round() / 100.0,
                count,
            }
        })
        .filter(|c| !c.name.is_empty())
        .collect();

    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.count.cmp(&a.count))
            .then(a.name.cmp(&b.name))
    });
    out.truncate(limit.clamp(1, 20));
    out
}

/// A category name's content tokens (stopwords dropped).
fn name_tokens(name: &str) -> BTreeSet<String> {
    keywords(name).into_iter().collect()
}

/// Do two words refer to the same thing? Either the same word, or a short
/// fragment of a longer one ("mort" ~ "morty", "san" ~ "sanfilippo").
fn tokens_match(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    short.len() >= 3 && long.len() >= 5 && long.starts_with(short)
}

/// How much two category token sets overlap (0..1), counting words that match
/// exactly or as a short fragment of a longer one ("mort" ~ "morty").
fn cluster_similarity(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let bv: Vec<&String> = b.iter().collect();
    let mut used = vec![false; bv.len()];
    let mut matched = 0usize;
    for x in a {
        for (i, y) in bv.iter().enumerate() {
            if !used[i] && tokens_match(x, y) {
                used[i] = true;
                matched += 1;
                break;
            }
        }
    }
    let union = a.len() + b.len() - matched;
    if union == 0 {
        0.0
    } else {
        matched as f64 / union as f64
    }
}

/* ── pure helpers (unit-tested) ─────────────────────────────── */

/// Content words from a title/channel: lowercased, stopworded, ≥2 chars,
/// deduped in first-seen order. Two-char terms matter ("ai", "3d", "vr"); the
/// category chips apply their own ≥4-char rule for title keywords.
pub fn keywords(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        let token = raw.to_lowercase();
        if token.chars().count() < 2 || STOPWORDS.contains(&token.as_str()) {
            continue;
        }
        if seen.insert(token.clone()) {
            out.push(token);
        }
    }
    out
}

/// `"3:21"` → 201, `"1:02:03"` → 3723, unknown → 0.
pub fn duration_secs(text: &str) -> u32 {
    if text.trim().is_empty() {
        return 0;
    }
    let mut secs: u32 = 0;
    for part in text.split(':') {
        match part.trim().parse::<u32>() {
            Ok(n) => secs = secs.saturating_mul(60).saturating_add(n),
            Err(_) => return 0,
        }
    }
    secs
}

/// Ratio of `candidate` tokens present in `reference` (0..1).
fn overlap(candidate: &[String], reference: &HashSet<String>) -> f64 {
    if candidate.is_empty() || reference.is_empty() {
        return 0.0;
    }
    let hits = candidate.iter().filter(|t| reference.contains(*t)).count();
    hits as f64 / candidate.len() as f64
}

/// Score one candidate. `rank_pos`/`total` are its position in the merged
/// search results, used as a weak popularity prior.
pub fn score_candidate(
    candidate: &VideoResult,
    seed: &Seed,
    seed_terms: &HashSet<String>,
    history_terms: &HashSet<String>,
    rank_pos: usize,
    total: usize,
) -> f64 {
    let terms = keywords(&candidate.title);
    let mut score = W_TITLE * overlap(&terms, seed_terms);

    if !seed.channel.trim().is_empty() && candidate.channel.eq_ignore_ascii_case(seed.channel.trim())
    {
        score += W_CHANNEL;
    }
    score += W_HISTORY * overlap(&terms, history_terms);

    if total > 0 {
        score += W_RANK * (1.0 - rank_pos as f64 / total as f64);
    }

    let secs = duration_secs(&candidate.duration);
    if secs > 0 && secs < 60 {
        score -= W_SHORT;
    }
    score
}

/// Rank candidates for `seed`, dropping the seed itself and anything the user
/// has already watched. Deterministic: ties keep the original search order.
pub fn rank(
    seed: &Seed,
    history: &[Seed],
    candidates: Vec<VideoResult>,
    limit: usize,
) -> Vec<VideoResult> {
    let seed_terms: HashSet<String> =
        keywords(&format!("{} {}", seed.title, seed.channel)).into_iter().collect();
    let history_terms: HashSet<String> =
        history.iter().flat_map(|h| keywords(&h.title)).collect();
    let watched: HashSet<&str> = history.iter().map(|h| h.video_id.as_str()).collect();

    let total = candidates.len();
    let mut scored: Vec<(f64, usize, VideoResult)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for (pos, candidate) in candidates.into_iter().enumerate() {
        if candidate.video_id == seed.video_id
            || watched.contains(candidate.video_id.as_str())
            || !seen.insert(candidate.video_id.clone())
        {
            continue;
        }
        let score = score_candidate(&candidate, seed, &seed_terms, &history_terms, pos, total);
        scored.push((score, pos, candidate));
    }

    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });
    scored.into_iter().take(limit).map(|(_, _, c)| c).collect()
}

/// Seed → one or two search queries that widen the net without drifting too
/// far: channel + strongest keywords, then the keywords alone.
pub fn queries_for(seed: &Seed) -> Vec<String> {
    let terms = keywords(&seed.title);
    let channel = seed.channel.trim();
    let mut out: Vec<String> = Vec::new();

    if !channel.is_empty() && !terms.is_empty() {
        out.push(format!("{channel} {}", terms.iter().take(4).cloned().collect::<Vec<_>>().join(" ")));
    }
    if !terms.is_empty() {
        out.push(terms.iter().take(5).cloned().collect::<Vec<_>>().join(" "));
    }
    if !channel.is_empty() {
        out.push(channel.to_string());
    }
    out.dedup();
    out.truncate(2);
    out
}

/* ── network-backed entry point ─────────────────────────────── */

/// Videos for one topic (a category chip): search the topic, then rank what
/// comes back. Unlike [`suggest`] this records nothing — browsing a category
/// must not look like watching it.
pub async fn videos_for(user: &str, topic: &str, limit: usize) -> Result<Vec<VideoResult>, AppError> {
    let seed = Seed {
        video_id: String::new(),
        title: topic.to_string(),
        channel: String::new(),
    };
    if !seed.is_usable() {
        return Err(AppError::BadRequest("topic required".into()));
    }

    let history = recent(user, HISTORY_AFFINITY);
    let mut candidates: Vec<VideoResult> = Vec::new();
    for query in queries_for(&seed) {
        if let Ok(mut hits) = youtube_client::search(&query, 12).await {
            candidates.append(&mut hits);
        }
    }
    if candidates.is_empty() {
        return Err(AppError::NotFound(format!("Nothing found for “{topic}”")));
    }
    Ok(rank(&seed, &history, candidates, limit.clamp(1, 24)))
}

/// Recommend videos for `user`. With no usable seed this falls back to the
/// most recent watch. The seed is remembered as a watch on the way through,
/// so simply asking for suggestions builds the history.
pub async fn suggest(
    user: &str,
    seed: Option<Seed>,
    limit: usize,
) -> Result<(Vec<VideoResult>, Seed), AppError> {
    let seed = match seed.filter(Seed::is_usable) {
        Some(s) => s,
        // No seed: prefer the last watch, then the category this user leans on
        // most (built from their searches), and only then trending.
        None => {
            if let Some(last) = recent(user, 1).into_iter().next() {
                last
            } else if let Some(top) = categories(user, 1).into_iter().next() {
                Seed {
                    video_id: String::new(),
                    title: top.name,
                    channel: String::new(),
                }
            } else {
                let hits = youtube_client::search("trending", limit.clamp(1, 24)).await?;
                if hits.is_empty() {
                    return Err(AppError::NotFound("No suggestions found right now".into()));
                }
                return Ok((
                    hits,
                    Seed {
                        video_id: String::new(),
                        title: "Trending on YouTube".into(),
                        channel: String::new(),
                    },
                ));
            }
        }
    };

    remember(user, &seed);

    let history = recent(user, HISTORY_AFFINITY);
    let mut candidates: Vec<VideoResult> = Vec::new();
    for query in queries_for(&seed) {
        if let Ok(mut hits) = youtube_client::search(&query, 12).await {
            candidates.append(&mut hits);
        }
    }
    if candidates.is_empty() {
        return Err(AppError::NotFound(
            "No suggestions found — try again in a moment".into(),
        ));
    }

    let ranked = rank(&seed, &history, candidates, limit.clamp(1, 24));
    if ranked.is_empty() {
        return Err(AppError::NotFound("No new suggestions right now".into()));
    }
    Ok((ranked, seed))
}

/* ── tests ──────────────────────────────────────────────────── */

#[cfg(test)]
mod tests {
    use super::*;

    fn vid(id: &str, title: &str, channel: &str) -> VideoResult {
        VideoResult {
            video_id: id.into(),
            title: title.into(),
            channel: channel.into(),
            duration: "3:00".into(),
            thumbnail: None,
        }
    }

    #[test]
    fn keywords_strip_stopwords_and_short_tokens() {
        let k = keywords("The Official Video of a Great Live Song");
        assert_eq!(k, vec!["great"]);
    }

    #[test]
    fn keywords_dedupe_in_order() {
        assert_eq!(keywords("lofi beats lofi mix"), vec!["lofi", "beats"]);
    }

    #[test]
    fn duration_parsing() {
        assert_eq!(duration_secs("3:21"), 201);
        assert_eq!(duration_secs("1:02:03"), 3723);
        assert_eq!(duration_secs(""), 0);
        assert_eq!(duration_secs("LIVE"), 0);
    }

    #[test]
    fn queries_prefer_channel_then_keywords() {
        let seed = Seed {
            video_id: "a".into(),
            title: "Deep Focus Piano Study".into(),
            channel: "AmbientLab".into(),
        };
        let q = queries_for(&seed);
        assert_eq!(q[0], "AmbientLab deep focus piano study");
        assert!(q[1].contains("piano"));
        assert!(q.len() <= 2);
    }

    #[test]
    fn rank_excludes_seed_and_watched_and_prefers_same_channel() {
        let seed = Seed {
            video_id: "seed".into(),
            title: "Deep Focus Piano Study".into(),
            channel: "AmbientLab".into(),
        };
        let history = vec![Seed {
            video_id: "old".into(),
            title: "Rain Sounds".into(),
            channel: "Other".into(),
        }];
        let candidates = vec![
            vid("seed", "Deep Focus Piano Study", "AmbientLab"), // the seed itself
            vid("old", "Rain Sounds", "Other"),                  // already watched
            vid("other", "Cooking Pasta Tonight", "Chef"),       // unrelated, ranked first
            vid("same", "Piano Study Session", "AmbientLab"),    // strong match, ranked last
        ];
        let ranked = rank(&seed, &history, candidates, 10);
        let ids: Vec<&str> = ranked.iter().map(|v| v.video_id.as_str()).collect();
        assert!(!ids.contains(&"seed"));
        assert!(!ids.contains(&"old"));
        assert_eq!(ids.first(), Some(&"same"), "same-channel keyword match should win: {ids:?}");
    }

    #[test]
    fn short_videos_are_penalised() {
        let seed = Seed { video_id: "s".into(), title: "Piano Study".into(), channel: "Lab".into() };
        let mut short = vid("short", "Piano Study Loop", "Someone");
        short.duration = "0:30".into();
        let full = vid("full", "Piano Study Loop", "Someone");
        let ranked = rank(&seed, &[], vec![short.clone(), full.clone()], 10);
        assert_eq!(ranked[0].video_id, "full");
    }

    #[test]
    fn categories_are_ranked_by_usage_and_keep_phrases() {
        let user = "test-categories-ranking";
        remember_query(user, "harry potter");
        remember_query(user, "harry potter");
        remember_query(user, "harry potter"); // watched/searched most
        remember_query(user, "jazz piano"); // only once
        let cats = categories(user, 20);
        assert_eq!(cats[0].name, "harry potter", "most-used category goes first: {cats:?}");
        assert!(cats.iter().any(|c| c.name == "jazz piano"));
        assert!(cats.iter().all(|c| c.name != "harry"), "phrase should not be split");
    }

    #[test]
    fn categories_never_exceed_twenty() {
        let user = "test-categories-cap";
        for i in 0..30 {
            remember_query(user, &format!("topic{i:02} unique"));
        }
        assert_eq!(categories(user, 50).len(), 20);
        assert!(categories(user, 5).len() <= 5);
    }

    #[test]
    fn watches_feed_categories_through_channel_and_keywords() {
        let user = "test-categories-watch";
        for _ in 0..3 {
            remember(
                user,
                &Seed {
                    video_id: "v1".into(),
                    title: "Deep Focus Piano Study".into(),
                    channel: "AmbientLab".into(),
                },
            );
        }
        let cats = categories(user, 20);
        let names: Vec<&str> = cats.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"ambientlab"), "{names:?}");
        assert!(names.contains(&"piano") || names.contains(&"focus"), "{names:?}");
    }

    #[test]
    fn fragments_fold_into_their_topic() {
        let user = "test-category-fragments";
        remember_query(user, "rick and morty");
        for _ in 0..3 {
            // Title keywords (untrusted) that recur, plus the channel.
            remember(
                user,
                &Seed {
                    video_id: "v1".into(),
                    title: "Rick Morty Compilation".into(),
                    channel: String::new(),
                },
            );
        }
        let names: Vec<String> = categories(user, 20).into_iter().map(|c| c.name).collect();
        assert!(names.contains(&"rick and morty".to_string()), "{names:?}");
        assert!(!names.contains(&"rick".to_string()), "fragment should fold in: {names:?}");
        assert!(!names.contains(&"morty".to_string()), "fragment should fold in: {names:?}");
    }

    #[test]
    fn one_off_title_words_are_dropped() {
        let user = "test-category-oneoff";
        // A trusted query survives on its own...
        remember_query(user, "harry potter");
        // ...but a single odd title word does not.
        remember(
            user,
            &Seed {
                video_id: "v1".into(),
                title: "Earty Jewels Moma".into(),
                channel: String::new(),
            },
        );
        let names: Vec<String> = categories(user, 20).into_iter().map(|c| c.name).collect();
        assert_eq!(names, vec!["harry potter".to_string()], "{names:?}");
    }

    #[test]
    fn distinct_topics_that_merely_overlap_stay_separate() {
        let user = "test-category-distinct";
        remember_query(user, "adult swim");
        remember_query(user, "adult swim rick morty");
        let names: Vec<String> = categories(user, 20).into_iter().map(|c| c.name).collect();
        // Only single-word fragments fold; two real phrases that merely share
        // a word stay as separate chips.
        assert!(names.contains(&"adult swim".to_string()), "{names:?}");
        assert!(names.contains(&"adult swim rick morty".to_string()), "{names:?}");
    }
}
