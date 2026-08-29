//! YouTube plugin tools: search videos, play in the YouTube window.

use async_trait::async_trait;
use serde_json::{json, Value};

use shiny_plugin_sdk::artifacts::{Artifact, ArtifactAction};
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::youtube_client::{self, VideoResult};

const DEFAULT_LIMIT: u32 = 8;

/// Compact per-video JSON the LLM sees (and the UI reuses).
fn video_json(v: &VideoResult) -> Value {
    json!({
        "video_id": v.video_id,
        "title": v.title,
        "channel": v.channel,
        "duration": v.duration,
        "thumbnail": v.thumbnail,
    })
}

/// Tappable result card — its Play action starts playback in the YouTube window.
fn video_artifact(v: &VideoResult) -> Artifact {
    let subtitle = [v.channel.as_str(), v.duration.as_str()]
        .iter()
        .filter(|p| !p.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ");

    Artifact {
        id: uuid::Uuid::new_v4().to_string(),
        artifact_type: "youtube_video".into(),
        title: v.title.clone(),
        subtitle: if subtitle.is_empty() { None } else { Some(subtitle) },
        coordinates: None,
        sections: vec![],
        actions: vec![ArtifactAction {
            label: "Play".into(),
            tool: "youtube_play".into(),
            params: json!({ "video_id": v.video_id, "title": v.title, "thumbnail": v.thumbnail }),
        }],
        days: vec![],
        route: None,
        geometry: vec![],
        narrative: None,
        theme: None,
        destination: None,
    }
}

/* ── youtube_search ─────────────────────────────────────────── */

pub struct YoutubeSearch;

#[async_trait]
impl Tool for YoutubeSearch {
    fn name(&self) -> &str { "youtube_search" }
    fn aliases(&self) -> &[&str] { &["search_youtube", "yt_search"] }
    fn step_label(&self) -> &str { "Searching YouTube…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `youtube_search` — Search YouTube for videos. params: `{ query: string, limit?: number }` — returns title, channel, duration and `video_id` per result (also one tappable card per video).")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Found {n} YouTube videos")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let query = req
            .params
            .param_str("query")
            .or_else(|| req.params.param_str("q"))
            .ok_or_else(|| AppError::BadRequest("query required".into()))?;
        let limit = req.params.param_u32("limit").unwrap_or(DEFAULT_LIMIT) as usize;

        let results = youtube_client::search(&query, limit).await?;
        let list: Vec<Value> = results.iter().map(video_json).collect();
        let cards: Vec<Artifact> = results.iter().map(video_artifact).collect();

        Ok(ActionOutcome::ok(
            "youtube_search",
            json!({ "results": list, "count": list.len() }),
        )
        .with_extra_artifacts(cards))
    }
}

/* ── youtube_play ───────────────────────────────────────────── */

pub struct YoutubePlay;

#[async_trait]
impl Tool for YoutubePlay {
    fn name(&self) -> &str { "youtube_play" }
    fn aliases(&self) -> &[&str] { &["play_youtube", "watch_youtube"] }
    fn step_label(&self) -> &str { "Playing on YouTube…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `youtube_play` — Start playing a video in the YouTube window. params: `{ video_id?: string, query?: string }` — with `video_id` plays exactly that video; otherwise plays the first search hit for `query`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("video");
        format!("Playing {title} on YouTube")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        if let Some(video_id) = req.params.param_str("video_id") {
            let title = req
                .params
                .param_str("title")
                .unwrap_or_else(|| "YouTube video".into());
            return Ok(ActionOutcome::ok(
                "youtube_play",
                json!({ "video_id": video_id, "title": title }),
            ));
        }

        let query = req
            .params
            .param_str("query")
            .or_else(|| req.params.param_str("q"))
            .ok_or_else(|| AppError::BadRequest("video_id or query required".into()))?;

        let mut hits = youtube_client::search(&query, 1).await?;
        if hits.is_empty() {
            return Ok(ActionOutcome::error(
                "youtube_play",
                "No video matched that query — try a different search.",
            ));
        }
        let first = hits.remove(0);
        Ok(ActionOutcome::ok(
            "youtube_play",
            json!({ "video_id": first.video_id, "title": first.title, "query": query }),
        ))
    }
}
