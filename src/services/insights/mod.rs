//! Context insight cards for the active destination (weather + events/places).
//!
//! Data sources (all free, no API keys):
//! - Weather: [Open-Meteo](https://open-meteo.com)
//! - Event headlines: DuckDuckGo Lite (via existing `SearchService`)
//! - Places: OpenStreetMap Overpass public instance
//!
//! There is no reliable global “events calendar” API without registration;
//! search + OSM venues is the practical free combination.

mod events;
mod types;
mod weather;

pub use types::InsightCard;

use crate::errors::AppError;
use crate::services::web_search::SearchService;

const MAX_CARDS: usize = 5;

/// Build up to five insight cards for a destination coordinate.
pub async fn build_context_cards(
    http: &reqwest::Client,
    search: &SearchService,
    destination: &str,
    lat: f64,
    lon: f64,
) -> Result<Vec<InsightCard>, AppError> {
    let mut cards: Vec<InsightCard> = Vec::new();

    // 1) Weather (one card)
    if let Ok(Some(w)) = weather::fetch_forecast(http, destination, lat, lon).await {
        cards.push(w);
    }

    // 2) Event-style headlines from web search
    cards.extend(events::cards_from_search(search, destination).await);

    // 3) Cultural places from Overpass (fills remaining slots, up to MAX_CARDS total)
    if cards.len() < MAX_CARDS {
        let room = MAX_CARDS - cards.len();
        if let Ok(mut places) = events::cards_from_overpass(http, lat, lon, destination, room).await {
            cards.append(&mut places);
        }
    }

    cards.truncate(MAX_CARDS);
    Ok(cards)
}
