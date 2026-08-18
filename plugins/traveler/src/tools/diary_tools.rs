//! Diary tools: list, get by date, search, generate.

use async_trait::async_trait;
use serde_json::json;

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::diary;
use crate::models::DiaryEntry;

/* ── list_diary ─────────────────────────────────────────────── */

pub struct ListDiary;

#[async_trait]
impl Tool for ListDiary {
    fn name(&self) -> &str { "list_diary" }
    fn step_label(&self) -> &str { "Reading diary…" }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let limit = req.params.get("limit").and_then(|v| v.as_i64()).unwrap_or(20);
        let entries = sqlx::query_as::<_, DiaryEntry>(
            "SELECT * FROM diary_entries WHERE traveler_id = ?1 ORDER BY date DESC LIMIT ?2",
        )
        .bind(req.traveler_id)
        .bind(limit)
        .fetch_all(ctx.pool().await)
        .await?;
        Ok(ActionOutcome::ok("list_diary", json!({ "entries": entries })))
    }
}

/* ── get_diary ──────────────────────────────────────────────── */

pub struct GetDiary;

#[async_trait]
impl Tool for GetDiary {
    fn name(&self) -> &str { "get_diary" }
    fn step_label(&self) -> &str { "Loading diary entry…" }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let date = req.params.require_str("date")?;
        let entry = sqlx::query_as::<_, DiaryEntry>(
            "SELECT * FROM diary_entries WHERE traveler_id = ?1 AND date = ?2",
        )
        .bind(req.traveler_id)
        .bind(&date)
        .fetch_optional(ctx.pool().await)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("No diary entry for {}", date)))?;
        Ok(ActionOutcome::ok("get_diary", json!({ "entry": entry })))
    }
}

/* ── search_diary ───────────────────────────────────────────── */

pub struct SearchDiary;

#[async_trait]
impl Tool for SearchDiary {
    fn name(&self) -> &str { "search_diary" }
    fn step_label(&self) -> &str { "Searching diary…" }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let q = req.params.require_str("q")?;
        let limit = req.params.get("limit").and_then(|v| v.as_i64()).unwrap_or(20);
        let pattern = format!("%{}%", q);
        let entries = sqlx::query_as::<_, DiaryEntry>(
            "SELECT * FROM diary_entries WHERE traveler_id = ?1 AND \
             (content_markdown LIKE ?2 OR title LIKE ?2 OR summary LIKE ?2 OR tags LIKE ?2) \
             ORDER BY date DESC LIMIT ?3",
        )
        .bind(req.traveler_id)
        .bind(&pattern)
        .bind(limit)
        .fetch_all(ctx.pool().await)
        .await?;
        Ok(ActionOutcome::ok("search_diary", json!({ "entries": entries })))
    }
}

/* ── generate_diary ─────────────────────────────────────────── */

pub struct GenerateDiary;

#[async_trait]
impl Tool for GenerateDiary {
    fn name(&self) -> &str { "generate_diary" }
    fn step_label(&self) -> &str { "Writing today's diary…" }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let date = req
            .params
            .param_str("date")
            .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
        let entry = diary::generate_for_date(ctx, req.traveler_id, &date).await?;
        Ok(ActionOutcome::ok("generate_diary", json!({ "entry": entry })))
    }
}
