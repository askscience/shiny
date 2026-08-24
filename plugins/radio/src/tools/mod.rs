//! Radio plugin tools: search stations, play, stop.

use async_trait::async_trait;
use serde_json::{json, Value};

use shiny_plugin_sdk::artifacts::{Artifact, ArtifactAction, ArtifactSection};
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::radio_browser::{RadioBrowserClient, Station, StationQuery};

/// Compact per-station JSON the LLM sees (and the UI reuses).
fn station_json(s: &Station) -> Value {
    json!({
        "stationuuid": s.stationuuid,
        "name": s.name,
        "country": s.country,
        "language": s.language,
        "tags": s.tags,
        "codec": s.codec,
        "bitrate": s.bitrate,
        "votes": s.votes,
    })
}

/// The now-playing card. The frontend's Radio window starts playback from the
/// `stream_url` inside the `radio_play` action params.
fn station_artifact(s: &Station, stream_url: &str) -> Artifact {
    let mut sections = Vec::new();
    let tags: Vec<&str> = s.tags.split(',').map(|t| t.trim()).filter(|t| !t.is_empty()).collect();
    if !tags.is_empty() {
        sections.push(ArtifactSection {
            label: "Genre".into(),
            value: tags.into_iter().take(4).collect::<Vec<_>>().join(", "),
        });
    }
    if !s.country.is_empty() || !s.language.is_empty() {
        sections.push(ArtifactSection {
            label: "Origin".into(),
            value: [s.country.as_str(), s.language.as_str()]
                .iter()
                .filter(|p| !p.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" · "),
        });
    }
    if s.bitrate > 0 {
        sections.push(ArtifactSection {
            label: "Quality".into(),
            value: format!("{} kbps {}", s.bitrate, s.codec.to_uppercase()),
        });
    }

    let subtitle = [s.country.as_str(), s.tags.split(',').next().unwrap_or("").trim()]
        .iter()
        .filter(|p| !p.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ");

    Artifact {
        id: uuid::Uuid::new_v4().to_string(),
        artifact_type: "radio_station".into(),
        title: s.name.clone(),
        subtitle: if subtitle.is_empty() { None } else { Some(subtitle) },
        coordinates: None,
        sections,
        actions: vec![
            ArtifactAction {
                label: "Play".into(),
                tool: "radio_play".into(),
                params: json!({
                    "stationuuid": s.stationuuid,
                    "stream_url": stream_url,
                    "name": s.name,
                    "favicon": s.favicon,
                }),
            },
            ArtifactAction {
                label: "Stop".into(),
                tool: "radio_stop".into(),
                params: json!({}),
            },
        ],
        days: vec![],
        route: None,
        geometry: vec![],
        narrative: None,
        theme: None,
        destination: None,
    }
}

/* ── radio_search ───────────────────────────────────────────── */

pub struct RadioSearch;

#[async_trait]
impl Tool for RadioSearch {
    fn name(&self) -> &str { "radio_search" }
    fn step_label(&self) -> &str { "Searching radio stations…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `radio_search` — Search internet radio stations (Radio Browser). params: `{ query?: string, tag?: string, country?: string, language?: string, limit?: number }` — returns matching stations ranked by votes, including `stationuuid` for `radio_play`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Found {n} radio stations")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let client = RadioBrowserClient::new();
        let stations = client
            .search(&StationQuery {
                name: req.params.param_str("query").as_deref(),
                tag: req.params.param_str("tag").as_deref(),
                country: req.params.param_str("country").as_deref(),
                language: req.params.param_str("language").as_deref(),
                limit: req.params.param_u32("limit").unwrap_or(10),
            })
            .await?;

        let list: Vec<Value> = stations.iter().map(station_json).collect();
        Ok(ActionOutcome::ok(
            "radio_search",
            json!({ "stations": list, "count": list.len() }),
        ))
    }
}

/* ── radio_play ─────────────────────────────────────────────── */

pub struct RadioPlay;

#[async_trait]
impl Tool for RadioPlay {
    fn name(&self) -> &str { "radio_play" }
    fn aliases(&self) -> &[&str] { &["play_radio", "listen_radio"] }
    fn step_label(&self) -> &str { "Tuning the radio…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `radio_play` — Tune in a station and start playback in the Radio window. params: `{ query?: string, tag?: string, stationuuid?: string }` — with `stationuuid` plays exactly that station; otherwise picks the most-voted match for `query`/`tag` (e.g. query \"BBC\" or tag \"jazz\").")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let name = data.get("station").and_then(|v| v.as_str()).unwrap_or("station");
        format!("Playing {name}")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let client = RadioBrowserClient::new();

        let station = if let Some(uuid) = req.params.param_str("stationuuid") {
            client
                .by_uuid(&uuid)
                .await?
                .ok_or_else(|| AppError::NotFound("Station not found".into()))?
        } else {
            let query = req.params.param_str("query");
            let tag = req.params.param_str("tag");
            if query.is_none() && tag.is_none() {
                return Ok(ActionOutcome::error(
                    "radio_play",
                    "Tell me a station name or genre — e.g. query \"BBC\" or tag \"jazz\".",
                ));
            }
            let mut matches = client
                .search(&StationQuery {
                    name: query.as_deref(),
                    tag: tag.as_deref(),
                    country: None,
                    language: None,
                    limit: 1,
                })
                .await?;
            if matches.is_empty() {
                return Ok(ActionOutcome::error(
                    "radio_play",
                    "No working station matched — try another name or genre.",
                ));
            }
            matches.remove(0)
        };

        // Register the click with Radio Browser; the resolved URL is what the
        // player's <audio> element opens.
        let stream_url = client
            .register_click(&station.stationuuid)
            .await
            .unwrap_or_else(|_| station.url_resolved.clone());

        Ok(ActionOutcome::ok(
            "radio_play",
            json!({
                "station": station.name,
                "stationuuid": station.stationuuid,
                "stream_url": stream_url,
            }),
        )
        .with_artifact(station_artifact(&station, &stream_url)))
    }
}

/* ── radio_stop ─────────────────────────────────────────────── */

pub struct RadioStop;

#[async_trait]
impl Tool for RadioStop {
    fn name(&self) -> &str { "radio_stop" }
    fn aliases(&self) -> &[&str] { &["stop_radio"] }
    fn step_label(&self) -> &str { "Stopping the radio…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `radio_stop` — Stop radio playback. params: `{}`")
    }
    fn humanize(&self, _r: &str, _d: &Value) -> String {
        "Radio stopped".into()
    }

    async fn invoke(&self, _ctx: &PluginCtx, _req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        // Playback lives in the browser; the frontend Radio window observes
        // this action (via the agent:actions event) and stops its <audio>.
        Ok(ActionOutcome::ok("radio_stop", json!({ "stopped": true })))
    }
}
