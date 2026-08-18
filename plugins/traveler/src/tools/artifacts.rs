//! Artifact card tools: show_artifact (render a card) and update_artifact
//! (merge edits into a saved card).

use async_trait::async_trait;
use serde_json::json;

use shiny_plugin_sdk::artifacts::{self, ArtifactAction, ArtifactSection, Coordinates};
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::artifact_store;

/* ── show_artifact ──────────────────────────────────────────── */

pub struct ShowArtifact;

#[async_trait]
impl Tool for ShowArtifact {
    fn name(&self) -> &str { "show_artifact" }
    fn step_label(&self) -> &str { "Preparing card…" }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let artifact = artifacts::build_from_params(req.params);
        Ok(ActionOutcome::ok("show_artifact", json!({ "artifact": artifact }))
            .with_artifact(artifact))
    }
}

/* ── update_artifact ────────────────────────────────────────── */

pub struct UpdateArtifact;

#[async_trait]
impl Tool for UpdateArtifact {
    fn name(&self) -> &str { "update_artifact" }
    fn step_label(&self) -> &str { "Updating card…" }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let artifact_id = req.params.require_str("artifact_id")?;
        let mut artifact = artifact_store::load_artifact(ctx.pool().await, req.traveler_id, &artifact_id).await?;

        if let Some(title) = req.params.param_str("title") {
            artifact.title = title;
        }
        if let Some(subtitle) = req.params.get("subtitle").and_then(|v| v.as_str()).map(String::from) {
            artifact.subtitle = Some(subtitle);
        }
        if let Some(sections) = req
            .params
            .get("sections")
            .and_then(|s| serde_json::from_value::<Vec<ArtifactSection>>(s.clone()).ok())
        {
            artifact.sections = sections;
        }
        if let Some(actions) = req
            .params
            .get("actions")
            .and_then(|s| serde_json::from_value::<Vec<ArtifactAction>>(s.clone()).ok())
        {
            artifact.actions = actions;
        }
        if let Some(coords) = req.params.get("coordinates").and_then(|c| {
            Some(Coordinates {
                lat: c.get("lat")?.as_f64()?,
                lon: c.get("lon")?.as_f64()?,
            })
        }) {
            artifact.coordinates = Some(coords);
        }

        let artifact = artifact_store::save_artifact(ctx.pool().await, req.traveler_id, &artifact).await?;
        Ok(ActionOutcome::ok("update_artifact", json!({ "artifact": artifact }))
            .with_artifact(artifact))
    }
}
