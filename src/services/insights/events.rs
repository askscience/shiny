//! Events & places — web research + Ollama summarization.
//!
//! Events: DuckDuckGo (instant + HTML) → Ollama extracts dated summaries.
//! Places: OpenStreetMap Overpass → Ollama writes short visitor tips (fallback: templates).

use chrono::Local;
use crate::errors::AppError;
use crate::services::insights::types::InsightCard;
use crate::services::ollama::OllamaClient;
use crate::services::web_search::{SearchResult, SearchService, is_aggregator_row, is_junk_search_row, text_chunks};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

const MAX_EVENT_SEARCH: usize = 2;
pub(crate) const MAX_OVERPASS_PLACES: usize = 4;

#[derive(Debug, Deserialize)]
struct OverpassResponse {
    elements: Vec<OverpassElement>,
}

#[derive(Debug, Deserialize)]
struct OverpassElement {
    tags: Option<OverpassTags>,
}

#[derive(Debug, Clone, Deserialize)]
struct OverpassTags {
    name: Option<String>,
    tourism: Option<String>,
    amenity: Option<String>,
    historic: Option<String>,
    #[serde(rename = "description")]
    description: Option<String>,
    #[serde(rename = "addr:street")]
    addr_street: Option<String>,
    opening_hours: Option<String>,
}

struct PlaceDraft {
    name: String,
    tags: OverpassTags,
}

/// Event headlines from web research, summarized with Ollama when available.
pub async fn cards_from_search(
    search: &SearchService,
    ollama: &OllamaClient,
    destination: &str,
    model: Option<&str>,
) -> Vec<InsightCard> {
    let sources = gather_event_sources(search, destination).await;
    if sources.is_empty() {
        return vec![];
    }

    if !ollama.is_available().await {
        return vec![];
    }

    match summarize_events(ollama, destination, &sources, model).await {
        Ok(cards) => cards
            .into_iter()
            .filter(|c| !is_junk_event_card(c))
            .take(MAX_EVENT_SEARCH)
            .collect(),
        Err(e) => {
            tracing::warn!("Event summarization failed for {}: {}", destination, e);
            vec![]
        }
    }
}

async fn gather_event_sources(search: &SearchService, destination: &str) -> Vec<SearchResult> {
    let now = Local::now();
    let month_year = now.format("%B %Y").to_string();
    let today = now.format("%Y-%m-%d").to_string();

    let queries = [
        format!("{destination} concert festival schedule {month_year}"),
        format!("{destination} exhibition opening {month_year}"),
        format!("{destination} opera theatre program {today}"),
    ];

    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    for query in &queries {
        if let Ok(rows) = search.search_html(query, 6).await {
            for row in rows {
                if is_junk_search_row(&row) || is_aggregator_row(&row) {
                    continue;
                }
                let key = row.title.to_lowercase();
                if seen.insert(key) {
                    out.push(row);
                }
            }
        }
        if out.len() >= 10 {
            break;
        }
    }

    if out.len() < 4 {
        if let Ok(rows) = search.search(&queries[0]).await {
            for row in rows {
                if is_junk_search_row(&row) || is_aggregator_row(&row) {
                    continue;
                }
                let key = row.title.to_lowercase();
                if seen.insert(key) {
                    out.push(row);
                }
            }
        }
    }

    out
}

async fn summarize_events(
    ollama: &OllamaClient,
    destination: &str,
    sources: &[SearchResult],
    model: Option<&str>,
) -> Result<Vec<InsightCard>, AppError> {
    let mut cards: Vec<InsightCard> = Vec::new();

    for source in sources {
        if cards.len() >= MAX_EVENT_SEARCH {
            break;
        }
        let Some(card) = extract_event_from_source(ollama, destination, source, model).await? else {
            continue;
        };
        if is_junk_event_card(&card) {
            continue;
        }
        if cards.iter().any(|c| c.title.eq_ignore_ascii_case(&card.title)) {
            continue;
        }
        cards.push(card);
    }

    Ok(cards)
}

async fn extract_event_from_source(
    ollama: &OllamaClient,
    destination: &str,
    source: &SearchResult,
    model: Option<&str>,
) -> Result<Option<InsightCard>, AppError> {
    let snippet = if source.snippet.trim().is_empty() {
        "(no snippet)".to_string()
    } else {
        source.snippet.trim().to_string()
    };

    let chunks = text_chunks(&snippet, 1200);
    let chunks = if chunks.is_empty() {
        vec![snippet]
    } else {
        chunks
    };

    for chunk in chunks {
        if let Some(card) =
            extract_event_from_chunk(ollama, destination, source, &chunk, model).await?
        {
            return Ok(Some(card));
        }
    }

    Ok(None)
}

async fn extract_event_from_chunk(
    ollama: &OllamaClient,
    destination: &str,
    source: &SearchResult,
    snippet: &str,
    model: Option<&str>,
) -> Result<Option<InsightCard>, AppError> {
    let today = Local::now().format("%A %d %B %Y").to_string();

    let system = "You extract one real-world event from a single search result. \
        Never output ticket sellers or booking pages. \
        Reply with ONLY JSON: {\"title\":\"...\",\"body\":\"...\"} or null.";

    let prompt = format!(
        "Today is {today}. Destination: {destination}.\n\
         Look at this ONE source only.\n\
         If it describes a specific upcoming event (concert, festival, exhibition, opera, theatre, sport), \
         reply with JSON:\n\
         {{\"title\":\"Event name\",\"body\":\"Sat 14 Jun, 20:30 — short detail\"}}\n\
         Rules for body:\n\
         - start with date or \"Date TBC —\"\n\
         - include start time when known\n\
         - max 120 chars\n\
         If it is a generic listing page, ticket shop, or travel promo, reply: null\n\n\
         Source:\n- {}: {}",
        source.title.trim(),
        snippet
    );

    let raw = ollama
        .chat(
            vec![
                ("system".to_string(), system.to_string()),
                ("user".to_string(), prompt),
            ],
            model,
        )
        .await?;

    parse_single_event_card(&raw)
}

fn parse_single_event_card(raw: &str) -> Result<Option<InsightCard>, AppError> {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("null") || trimmed.is_empty() {
        return Ok(None);
    }

    let json_str = extract_json_object_or_array(trimmed);
    if json_str == "null" {
        return Ok(None);
    }

    let value: Value = serde_json::from_str(&json_str).map_err(|e| {
        AppError::Internal(format!("Failed to parse event summary: {}", e))
    })?;

    if value.is_null() {
        return Ok(None);
    }

    let obj = if let Some(arr) = value.as_array() {
        arr.first().cloned().unwrap_or(Value::Null)
    } else {
        value
    };

    if obj.is_null() {
        return Ok(None);
    }

    let title = obj.get("title").and_then(|v| v.as_str()).unwrap_or("").trim();
    let body = obj.get("body").and_then(|v| v.as_str()).unwrap_or("").trim();
    if title.is_empty() || body.is_empty() {
        return Ok(None);
    }

    Ok(Some(InsightCard {
        id: Uuid::new_v4().to_string(),
        kind: "event".into(),
        title: truncate(title, 64),
        body: truncate(body, 140),
        icon: "event".into(),
    }))
}

fn parse_event_cards(raw: &str) -> Result<Vec<InsightCard>, AppError> {
    let json_str = extract_json_array(raw);
    let value: Value = serde_json::from_str(&json_str).map_err(|e| {
        AppError::Internal(format!("Failed to parse event summaries: {}", e))
    })?;

    let arr = value.as_array().ok_or_else(|| {
        AppError::Internal("Event summary response was not a JSON array".into())
    })?;

    let cards = arr
        .iter()
        .filter_map(|item| {
            let title = item.get("title")?.as_str()?.trim();
            let body = item.get("body")?.as_str()?.trim();
            if title.is_empty() || body.is_empty() {
                return None;
            }
            Some(InsightCard {
                id: Uuid::new_v4().to_string(),
                kind: "event".into(),
                title: truncate(title, 64),
                body: truncate(body, 140),
                icon: "event".into(),
            })
        })
        .take(MAX_EVENT_SEARCH)
        .collect();

    Ok(cards)
}

/// Museums, theatres, attractions from Overpass, summarized with Ollama when available.
pub async fn cards_from_overpass(
    client: &reqwest::Client,
    ollama: &OllamaClient,
    lat: f64,
    lon: f64,
    destination: &str,
    max_cards: usize,
    model: Option<&str>,
) -> Result<Vec<InsightCard>, AppError> {
    let limit = max_cards.min(MAX_OVERPASS_PLACES);
    if limit == 0 {
        return Ok(vec![]);
    }

    let drafts = fetch_place_drafts(client, lat, lon, limit).await?;
    if drafts.is_empty() {
        return Ok(vec![]);
    }

    if ollama.is_available().await {
        let mut cards = Vec::new();
        for draft in &drafts {
            let mut card = draft_to_card(draft, destination);
            if let Ok(Some(body)) =
                summarize_one_place(ollama, destination, draft, model).await
            {
                card.body = body;
            }
            cards.push(card);
        }
        return Ok(cards);
    }

    Ok(drafts
        .into_iter()
        .map(|d| draft_to_card(&d, destination))
        .collect())
}

async fn fetch_place_drafts(
    client: &reqwest::Client,
    lat: f64,
    lon: f64,
    limit: usize,
) -> Result<Vec<PlaceDraft>, AppError> {
    let query = format!(
        "[out:json][timeout:12];\
         (node[\"tourism\"~\"attraction|museum|gallery\"](around:6000,{lat},{lon});\
          way[\"tourism\"~\"attraction|museum|gallery\"](around:6000,{lat},{lon});\
          node[\"amenity\"~\"theatre|arts_centre\"](around:6000,{lat},{lon});\
         );\
         out body 12;",
        lat = lat,
        lon = lon
    );

    let resp = client
        .post("https://overpass-api.de/api/interpreter")
        .form(&[("data", query.as_str())])
        .send()
        .await?;

    if !resp.status().is_success() {
        return Ok(vec![]);
    }

    let data: OverpassResponse = resp.json().await.map_err(|e| {
        AppError::Internal(format!("Overpass parse error: {}", e))
    })?;

    let mut drafts = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for el in data.elements {
        let tags = match el.tags {
            Some(t) => t,
            None => continue,
        };
        let name = match tags.name.as_deref() {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => continue,
        };
        let key = name.to_lowercase();
        if !seen.insert(key) {
            continue;
        }
        drafts.push(PlaceDraft { name, tags });
        if drafts.len() >= limit {
            break;
        }
    }

    Ok(drafts)
}

async fn summarize_one_place(
    ollama: &OllamaClient,
    destination: &str,
    draft: &PlaceDraft,
    model: Option<&str>,
) -> Result<Option<String>, AppError> {
    let system = "You write one concise tourist tip. Reply with ONLY JSON: {\"body\":\"...\"} or null.";

    let prompt = format!(
        "Place in {destination}: {}\n\
         Facts: {}\n\
         Write one practical visitor sentence (max 110 chars). Include opening hours if given.\n\
         If you cannot say anything useful, reply null.\n\
         JSON: {{\"body\":\"Thu–Sun 10:00–18:00 — …\"}}",
        draft.name,
        place_source_line(draft)
    );

    let raw = ollama
        .chat(
            vec![
                ("system".to_string(), system.to_string()),
                ("user".to_string(), prompt),
            ],
            model,
        )
        .await?;

    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("null") {
        return Ok(None);
    }

    let json_str = extract_json_object_or_array(trimmed);
    let value: Value = serde_json::from_str(&json_str).map_err(|e| {
        AppError::Internal(format!("Failed to parse place summary: {}", e))
    })?;

    if value.is_null() {
        return Ok(None);
    }

    let body = value
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if body.is_empty() {
        return Ok(None);
    }

    Ok(Some(truncate(body, 140)))
}

fn place_source_line(d: &PlaceDraft) -> String {
    let tourism = d.tags.tourism.as_deref().unwrap_or("");
    let amenity = d.tags.amenity.as_deref().unwrap_or("");
    let street = d.tags.addr_street.as_deref().unwrap_or("");
    let hours = d.tags.opening_hours.as_deref().unwrap_or("");
    let desc = d.tags.description.as_deref().unwrap_or("");
    format!(
        "- {} (type: {} {}, street: {}, hours: {}, note: {})",
        d.name,
        tourism,
        amenity,
        street,
        hours,
        truncate(desc, 80)
    )
}

fn draft_to_card(d: &PlaceDraft, destination: &str) -> InsightCard {
    let tourism = d.tags.tourism.as_deref().unwrap_or("").to_lowercase();
    let amenity = d.tags.amenity.as_deref().unwrap_or("").to_lowercase();
    let (body, icon) = place_card_copy(&d.tags, destination, &d.name, &tourism, &amenity);
    InsightCard {
        id: Uuid::new_v4().to_string(),
        kind: "place".into(),
        title: truncate(&d.name, 56),
        body,
        icon: icon.into(),
    }
}

fn place_card_copy(
    tags: &OverpassTags,
    destination: &str,
    name: &str,
    tourism: &str,
    amenity: &str,
) -> (String, &'static str) {
    let icon = place_icon_stem(tourism, amenity);

    if let Some(desc) = tags
        .description
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| s.len() > 16)
    {
        return (truncate(desc, 140), icon);
    }

    let street = tags
        .addr_street
        .as_ref()
        .map(|s| format!(", {}", s.trim()))
        .unwrap_or_default();

    let hours_hint = tags.opening_hours.as_ref().map(|h| {
        let h = h.trim();
        if h.len() > 6 {
            format!(" Hours: {}.", h)
        } else {
            String::new()
        }
    }).unwrap_or_default();

    let body = match amenity {
        "theatre" | "arts_centre" => format!(
            "Performing arts in {}{}.{}",
            destination, street, hours_hint
        ),
        _ => match tourism {
            "museum" => format!("Museum in {}{}.{}", destination, street, hours_hint),
            "gallery" => format!("Gallery in {}{}.{}", destination, street, hours_hint),
            "attraction" | "viewpoint" => format!(
                "Sight in {}{}.{}",
                destination, street, hours_hint
            ),
            _ if tags.historic.is_some() => format!(
                "Historic site in {}{}.{}",
                destination, street, hours_hint
            ),
            _ => format!("{} — {}{}", name, destination, hours_hint),
        },
    };

    (truncate(&body, 140), icon)
}

fn place_icon_stem(tourism: &str, amenity: &str) -> &'static str {
    match amenity {
        "theatre" | "arts_centre" => "place-theatre",
        _ => match tourism {
            "museum" => "place-museum",
            "gallery" => "place-gallery",
            "attraction" | "viewpoint" | "theme_park" => "place-landmark",
            _ => "place-landmark",
        },
    }
}

fn extract_json_object_or_array(raw: &str) -> String {
    let trimmed = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if trimmed.eq_ignore_ascii_case("null") {
        return "null".to_string();
    }
    if trimmed.starts_with('{') {
        if let Some(end) = trimmed.rfind('}') {
            return trimmed[..=end].to_string();
        }
    }
    extract_json_array(trimmed)
}

fn extract_json_array(raw: &str) -> String {
    let trimmed = raw.trim();
    let unfenced = trimmed
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if let Some(start) = unfenced.find('[') {
        if let Some(end) = unfenced.rfind(']') {
            return unfenced[start..=end].to_string();
        }
    }
    "[]".to_string()
}

fn is_junk_event_card(card: &InsightCard) -> bool {
    if is_aggregator_row(&SearchResult {
        title: card.title.clone(),
        snippet: card.body.clone(),
    }) {
        return true;
    }
    let body = card.body.trim();
    if body.len() < 12 {
        return true;
    }
    !has_date_hint(body) && !body.to_lowercase().starts_with("date tbc")
}

fn has_date_hint(body: &str) -> bool {
    let b = body.to_lowercase();
    const MONTHS: &[&str] = &[
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    const DAYS: &[&str] = &["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    MONTHS.iter().any(|m| b.contains(m))
        || DAYS.iter().any(|d| b.contains(d))
        || b.chars().filter(|c| *c == '/').count() >= 2
        || b.contains(':')
            && b
                .split_whitespace()
                .any(|w| w.len() == 5 && w.chars().nth(2) == Some(':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junk_placeholder_is_skipped() {
        let row = SearchResult {
            title: "No results".into(),
            snippet: "No information found for: Milan events".into(),
        };
        assert!(is_junk_search_row(&row));
    }

    #[test]
    fn parses_event_json_array() {
        let raw = r#"Here you go:
        [{"title":"Jazz Night","body":"Sat 14 Jun, 20:30 — live quartet at Blue Note"}]"#;
        let cards = parse_event_cards(raw).unwrap();
        assert_eq!(cards.len(), 1);
        assert!(cards[0].body.contains("20:30"));
    }

    #[test]
    fn theatre_gets_theatre_icon() {
        let tags = OverpassTags {
            name: Some("Teatro Manzoni".into()),
            tourism: None,
            amenity: Some("theatre".into()),
            historic: None,
            description: None,
            addr_street: Some("Via Alessandro Manzoni".into()),
            opening_hours: Some("Tu-Su 10:00-19:00".into()),
        };
        let draft = PlaceDraft {
            name: "Teatro Manzoni".into(),
            tags,
        };
        let card = draft_to_card(&draft, "Milan");
        assert_eq!(card.icon, "place-theatre");
        assert!(card.body.contains("10:00"));
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}
