//! Radio Browser API client for the radio plugin.
//!
//! Radio Browser (https://www.radio-browser.info) is a community-driven
//! directory of internet radio stations. The JSON API is public; the docs ask
//! clients to send a descriptive User-Agent and to register station clicks via
//! `/json/url/{uuid}` so station popularity stats stay accurate.

use serde::{Deserialize, Serialize};
use shiny_plugin_sdk::errors::AppError;

/// Mirrors the public API host list (DNS round-robin for api.radio-browser.info).
/// Tried in order — the first host that answers is used for the request.
const API_HOSTS: &[&str] = &[
    "https://de1.api.radio-browser.info",
    "https://de2.api.radio-browser.info",
    "https://fi1.api.radio-browser.info",
];

#[derive(Clone)]
pub struct RadioBrowserClient {
    client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Station {
    pub stationuuid: String,
    pub name: String,
    /// Resolved stream URL the player should open.
    pub url_resolved: String,
    #[serde(default)]
    pub favicon: String,
    #[serde(default)]
    pub tags: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub codec: String,
    #[serde(default)]
    pub bitrate: u32,
    #[serde(default)]
    pub votes: u32,
    #[serde(default)]
    pub clickcount: u32,
}

#[derive(Debug, Deserialize)]
struct ClickResponse {
    url: String,
}

#[derive(Debug, Default)]
pub struct StationQuery<'a> {
    pub name: Option<&'a str>,
    pub tag: Option<&'a str>,
    pub country: Option<&'a str>,
    pub language: Option<&'a str>,
    pub limit: u32,
}

impl Default for RadioBrowserClient {
    fn default() -> Self {
        Self::new()
    }
}

impl RadioBrowserClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("Shiny/0.1 (shiny-radio-plugin)")
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
        }
    }

    /// GET `path` against the API hosts in order; first success wins.
    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, AppError> {
        let mut last_err: Option<AppError> = None;
        for host in API_HOSTS {
            let url = format!("{}/json/{}", host, path);
            match self.client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    return resp.json::<T>().await.map_err(|e| {
                        AppError::Internal(format!("Failed to parse Radio Browser response: {}", e))
                    });
                }
                Ok(resp) => {
                    last_err = Some(AppError::Internal(format!(
                        "Radio Browser error: {}",
                        resp.status()
                    )));
                }
                Err(e) => {
                    last_err = Some(AppError::Http(e));
                }
            }
        }
        Err(last_err.unwrap_or_else(|| AppError::Internal("Radio Browser unreachable".into())))
    }

    /// Search stations by name/tag/country/language, ranked by votes.
    pub async fn search(&self, q: &StationQuery<'_>) -> Result<Vec<Station>, AppError> {
        let limit = q.limit.clamp(1, 100);
        let mut path = format!(
            "stations/search?hidebroken=true&order=votes&reverse=true&limit={}",
            limit
        );
        if let Some(name) = q.name.filter(|s| !s.trim().is_empty()) {
            path.push_str(&format!("&name={}", urlencoding(name.trim())));
        }
        if let Some(tag) = q.tag.filter(|s| !s.trim().is_empty()) {
            path.push_str(&format!("&tag={}", urlencoding(tag.trim())));
        }
        if let Some(country) = q.country.filter(|s| !s.trim().is_empty()) {
            path.push_str(&format!("&country={}", urlencoding(country.trim())));
        }
        if let Some(language) = q.language.filter(|s| !s.trim().is_empty()) {
            path.push_str(&format!("&language={}", urlencoding(language.trim())));
        }
        self.get(&path).await
    }

    /// Fetch a single station by its UUID.
    pub async fn by_uuid(&self, uuid: &str) -> Result<Option<Station>, AppError> {
        let stations: Vec<Station> = self
            .get(&format!("stations/byuuid/{}", urlencoding(uuid)))
            .await?;
        Ok(stations.into_iter().next())
    }

    /// Register a click for the station and get the playable stream URL.
    /// (This is the endpoint Radio Browser asks players to hit on play.)
    pub async fn register_click(&self, uuid: &str) -> Result<String, AppError> {
        let resp: ClickResponse = self
            .get(&format!("url/{}", urlencoding(uuid)))
            .await?;
        Ok(resp.url)
    }
}

fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u8),
        })
        .collect()
}
