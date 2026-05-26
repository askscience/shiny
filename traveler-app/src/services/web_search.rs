use serde::{Deserialize, Serialize};
use crate::errors::AppError;

#[derive(Debug, Clone)]
pub struct SearchService {
    client: reqwest::Client,
}

impl SearchService {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("TravelerApp/0.1 (traveler-app)")
                .build()
                .unwrap(),
        }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<SearchResult>, AppError> {
        let url = format!(
            "https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1",
            urlencoding(query)
        );

        let resp = self.client.get(&url).send().await?;

        if !resp.status().is_success() {
            return Err(AppError::Internal(format!(
                "Search API error: {}",
                resp.status()
            )));
        }

        let data: DuckDuckGoResponse = resp.json().await.map_err(|e| {
            AppError::Internal(format!("Failed to parse search response: {}", e))
        })?;

        let mut results = Vec::new();

        if !data.AbstractText.is_empty() {
            results.push(SearchResult {
                title: data.AbstractSource,
                snippet: data.AbstractText,
            });
        }

        for topic in &data.RelatedTopics {
            if let Some(text) = &topic.Text {
                results.push(SearchResult {
                    title: text.split(" - ").next().unwrap_or("").to_string(),
                    snippet: text.clone(),
                });
            }
            if let Some(subtopics) = &topic.Topics {
                for sub in subtopics {
                    if let Some(text) = &sub.Text {
                        results.push(SearchResult {
                            title: text.split(" - ").next().unwrap_or("").to_string(),
                            snippet: text.clone(),
                        });
                    }
                }
            }
        }

        if results.is_empty() {
            results.push(SearchResult {
                title: "No results".into(),
                snippet: format!("No information found for: {}", query),
            });
        }

        Ok(results)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub title: String,
    pub snippet: String,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct DuckDuckGoResponse {
    AbstractText: String,
    AbstractSource: String,
    RelatedTopics: Vec<RelatedTopic>,
}

#[derive(Debug, Deserialize)]
struct RelatedTopic {
    Text: Option<String>,
    Topics: Option<Vec<RelatedTopic>>,
}

fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ' ' => "+".to_string(),
            _ => format!("%{:02X}", c as u8),
        })
        .collect()
}
