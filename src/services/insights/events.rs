//! Events & places — no single free global events API without keys.
//! We combine:
//! 1. DuckDuckGo Lite (already used app-wide, no registration) for event headlines
//! 2. OpenStreetMap Overpass (free) for cultural venues / attractions near the destination

use crate::errors::AppError;
use crate::services::insights::types::InsightCard;
use crate::services::web_search::SearchService;
use serde::Deserialize;
use uuid::Uuid;

const MAX_EVENT_SEARCH: usize = 2;
/// Fills remaining card slots when DuckDuckGo has no event headlines.
pub(crate) const MAX_OVERPASS_PLACES: usize = 4;

#[derive(Debug, Deserialize)]
struct OverpassResponse {
    elements: Vec<OverpassElement>,
}

#[derive(Debug, Deserialize)]
struct OverpassElement {
    tags: Option<OverpassTags>,
}

#[derive(Debug, Deserialize)]
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

/// Headlines from web search (festivals, concerts, fairs).
pub async fn cards_from_search(
    search: &SearchService,
    destination: &str,
) -> Vec<InsightCard> {
    let query = format!(
        "{} events festivals concerts things to do",
        destination
    );
    let Ok(results) = search.search(&query).await else {
        return vec![];
    };

    results
        .into_iter()
        .filter(|r| !is_junk_search_row(r))
        .take(MAX_EVENT_SEARCH)
        .map(|r| {
            let title = if r.title.is_empty() {
                format!("Happening in {}", destination)
            } else {
                truncate(&r.title, 64)
            };
            let body = truncate(&r.snippet, 140);
            InsightCard {
                id: Uuid::new_v4().to_string(),
                kind: "event".into(),
                title,
                body,
                icon: "event".into(),
            }
        })
        .collect()
}

/// Museums, theatres, attractions from Overpass (stable “what to see” cards).
pub async fn cards_from_overpass(
    client: &reqwest::Client,
    lat: f64,
    lon: f64,
    destination: &str,
    max_cards: usize,
) -> Result<Vec<InsightCard>, AppError> {
    let limit = max_cards.min(MAX_OVERPASS_PLACES);
    if limit == 0 {
        return Ok(vec![]);
    }
    let query = format!(
        "[out:json][timeout:12];\
         (node[\"tourism\"~\"attraction|museum\"](around:6000,{lat},{lon});\
          way[\"tourism\"~\"attraction|museum\"](around:6000,{lat},{lon});\
          node[\"amenity\"~\"theatre|arts_centre\"](around:6000,{lat},{lon});\
         );\
         out body 10;",
        lat = lat,
        lon = lon
    );

    // Overpass expects `data=` form body; raw POST without Content-Type returns 406.
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

    let mut cards = Vec::new();
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

        let (body, icon) = place_card_copy(&tags, destination, &name);

        cards.push(InsightCard {
            id: Uuid::new_v4().to_string(),
            kind: "place".into(),
            title: truncate(&name, 56),
            body,
            icon: icon.into(),
        });

        if cards.len() >= limit {
            break;
        }
    }

    Ok(cards)
}

/// Human-readable blurb + icon stem from OSM tags (no generic GPS pin).
fn place_card_copy(tags: &OverpassTags, destination: &str, name: &str) -> (String, &'static str) {
    let tourism = tags.tourism.as_deref().unwrap_or("").to_lowercase();
    let amenity = tags.amenity.as_deref().unwrap_or("").to_lowercase();
    let icon = place_icon_stem(&tourism, &amenity);

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
        if h.eq_ignore_ascii_case("24/7") {
            " Open around the clock.".to_string()
        } else if h.len() > 6 {
            " Check opening hours before you go.".to_string()
        } else {
            String::new()
        }
    }).unwrap_or_default();

    let body = match amenity.as_str() {
        "theatre" | "arts_centre" => format!(
            "Performing arts venue in {}{} — look for opera, drama, or concerts on the bill.{}",
            destination, street, hours_hint
        ),
        _ => match tourism.as_str() {
            "museum" => format!(
                "Museum in {}{} — plan an hour or two for permanent collections and temporary shows.{}",
                destination, street, hours_hint
            ),
            "gallery" => format!(
                "Art gallery in {}{} — exhibitions change often; good for a slow afternoon.{}",
                destination, street, hours_hint
            ),
            "attraction" | "viewpoint" => format!(
                "Popular sight in {}{} — arrive earlier to skip the biggest crowds.{}",
                destination, street, hours_hint
            ),
            _ if tags.historic.is_some() => format!(
                "Historic landmark in {}{} — best explored on foot with a camera.{}",
                destination, street, hours_hint
            ),
            _ => {
                let kind = if name.to_lowercase().contains("teatro") {
                    "Theatre"
                } else if name.to_lowercase().contains("museo") {
                    "Museum"
                } else {
                    "Cultural stop"
                };
                format!(
                    "{} in {}{} — a local favourite between your main plans.{}",
                    kind, destination, street, hours_hint
                )
            }
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

/// DuckDuckGo instant API returns this placeholder when RelatedTopics is empty.
fn is_junk_search_row(r: &crate::services::web_search::SearchResult) -> bool {
    let title = r.title.trim().to_lowercase();
    let snippet = r.snippet.trim().to_lowercase();
    title.contains("no result") || snippet.contains("no information found")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::web_search::SearchResult;

    #[test]
    fn junk_placeholder_is_skipped() {
        let row = SearchResult {
            title: "No results".into(),
            snippet: "No information found for: Milan events".into(),
        };
        assert!(is_junk_search_row(&row));
    }

    #[test]
    fn theatre_gets_theatre_copy_and_icon() {
        let tags = OverpassTags {
            name: Some("Teatro Manzoni".into()),
            tourism: None,
            amenity: Some("theatre".into()),
            historic: None,
            description: None,
            addr_street: Some("Via Alessandro Manzoni".into()),
            opening_hours: None,
        };
        let (body, icon) = place_card_copy(&tags, "Milan", "Teatro Manzoni");
        assert_eq!(icon, "place-theatre");
        assert!(body.contains("Performing arts"));
        assert!(!body.contains("Worth a visit"));
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
