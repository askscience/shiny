//! YouTube plugin tools: search videos, play in the YouTube window.

use async_trait::async_trait;
use serde_json::{json, Value};

use shiny_plugin_sdk::artifacts::{Artifact, ArtifactAction};
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::suggest::{self, Seed};
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

        // The AI's searches are category signals too, same as the user's.
        suggest::remember_query(req.user_id, &query);
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
            let channel = req.params.param_str("channel").unwrap_or_default();
            let thumbnail = req.params.param_str("thumbnail").unwrap_or_default();
            // Remember the watch so `youtube_suggest` can personalise later.
            suggest::remember(
                req.user_id,
                &Seed { video_id: video_id.clone(), title: title.clone(), channel: channel.clone() },
            );
            return Ok(ActionOutcome::ok(
                "youtube_play",
                json!({
                    "video_id": video_id,
                    "title": title,
                    "channel": channel,
                    "thumbnail": thumbnail,
                }),
            ));
        }

        let query = req
            .params
            .param_str("query")
            .or_else(|| req.params.param_str("q"))
            .ok_or_else(|| AppError::BadRequest("video_id or query required".into()))?;

        suggest::remember_query(req.user_id, &query);
        let mut hits = youtube_client::search(&query, 1).await?;
        if hits.is_empty() {
            return Ok(ActionOutcome::error(
                "youtube_play",
                "No video matched that query — try a different search.",
            ));
        }
        let first = hits.remove(0);
        suggest::remember(
            req.user_id,
            &Seed {
                video_id: first.video_id.clone(),
                title: first.title.clone(),
                channel: first.channel.clone(),
            },
        );
        Ok(ActionOutcome::ok(
            "youtube_play",
            json!({
                "video_id": first.video_id,
                "title": first.title,
                "channel": first.channel,
                "thumbnail": first.thumbnail,
                "query": query,
            }),
        ))
    }
}

/* ── youtube_suggest ────────────────────────────────────────── */

pub struct YoutubeSuggest;

#[async_trait]
impl Tool for YoutubeSuggest {
    fn name(&self) -> &str { "youtube_suggest" }
    fn aliases(&self) -> &[&str] { &["suggest_videos", "recommend_youtube", "youtube_recommend"] }
    fn step_label(&self) -> &str { "Finding videos you might like…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `youtube_suggest` — Recommend videos to watch next, using a simple keyword/channel similarity ranker over YouTube search results. params: `{ video_id?: string, title?: string, channel?: string, query?: string, limit?: number }` — seed it with the video in question, or with a `query` for a topic; with no seed it falls back to what the user watched most recently (and to trending videos for a brand-new user). Returns title, channel, duration and `video_id` per result (also one tappable card per video).")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        let based_on = data
            .get("based_on")
            .and_then(|b| b.get("title"))
            .and_then(|t| t.as_str())
            .unwrap_or("");
        if based_on.is_empty() {
            format!("Suggested {n} videos")
        } else {
            format!("Suggested {n} videos based on “{based_on}”")
        }
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let limit = req.params.param_u32("limit").unwrap_or(DEFAULT_LIMIT) as usize;
        let seed = Seed {
            video_id: req.params.param_str("video_id").unwrap_or_default(),
            title: req
                .params
                .param_str("title")
                .or_else(|| req.params.param_str("query"))
                .unwrap_or_default(),
            channel: req.params.param_str("channel").unwrap_or_default(),
        };

        let (results, seed) = suggest::suggest(req.user_id, Some(seed), limit).await?;
        let list: Vec<Value> = results.iter().map(video_json).collect();
        let cards: Vec<Artifact> = results.iter().map(video_artifact).collect();

        Ok(ActionOutcome::ok(
            "youtube_suggest",
            json!({
                "results": list,
                "count": list.len(),
                "based_on": {
                    "video_id": seed.video_id,
                    "title": seed.title,
                    "channel": seed.channel,
                },
            }),
        )
        .with_extra_artifacts(cards))
    }
}
