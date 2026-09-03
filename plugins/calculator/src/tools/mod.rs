//! Calculator plugin tools: evaluate expressions, recall and clear history.

use async_trait::async_trait;
use serde_json::{json, Value};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::eval::{evaluate, format_number};

fn expression_param(req: &ToolRequest<'_>) -> Result<String, AppError> {
    req.params
        .param_str("expression")
        .or_else(|| req.params.param_str("expr"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest("expression required".into()))
}

/* ── calculator_eval ────────────────────────────────────────── */

pub struct CalculatorEval;

#[async_trait]
impl Tool for CalculatorEval {
    fn name(&self) -> &str { "calculator_eval" }
    fn aliases(&self) -> &[&str] { &["calculate", "compute", "evaluate"] }
    fn step_label(&self) -> &str { "Calculating…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `calculator_eval` — Evaluate a math expression and return the result. params: `{ expression: string }` — supports + - * / % ^ ! and parentheses; scientific functions sin/cos/tan (radians), sind/cosd/tand (degrees), asin/acos/atan/atan2, sinh/cosh/tanh, sqrt/cbrt, ln/log/log2, exp, abs, floor/ceil/round/trunc/sign, deg/rad, pow/mod/hypot, fact (factorial); constants pi, e, tau, phi. Returns `result` and `result_text`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let expr = data.get("expression").and_then(|v| v.as_str()).unwrap_or("");
        let result = data.get("result_text").and_then(|v| v.as_str()).unwrap_or("");
        if expr.is_empty() {
            "Calculated".into()
        } else {
            format!("{expr} = {result}")
        }
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let expression = expression_param(&req)?;
        let result = evaluate(&expression).map_err(AppError::BadRequest)?;
        let result_text = format_number(result);

        sqlx::query(
            "INSERT INTO calculator_history (user_id, expression, result, created_at) \
             VALUES (?1, ?2, ?3, datetime('now'))",
        )
        .bind(req.traveler_id)
        .bind(&expression)
        .bind(&result_text)
        .execute(ctx.pool().await)
        .await?;

        Ok(ActionOutcome::ok(
            "calculator_eval",
            json!({ "expression": expression, "result": result, "result_text": result_text }),
        ))
    }
}

/* ── calculator_history ─────────────────────────────────────── */

pub struct CalculatorHistory;

#[async_trait]
impl Tool for CalculatorHistory {
    fn name(&self) -> &str { "calculator_history" }
    fn aliases(&self) -> &[&str] { &["calculation_history", "calc_history"] }
    fn step_label(&self) -> &str { "Reading calculator history…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `calculator_history` — List recent calculations. params: `{ limit?: number }` — returns `history` (each with `expression`, `result`, `at`) and `count`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Found {n} recent calculations")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let limit = req.params.param_u32("limit").unwrap_or(20).clamp(1, 100) as i64;
        let rows = sqlx::query_as::<_, (String, String, String)>(
            "SELECT expression, result, created_at FROM calculator_history \
             WHERE user_id = ?1 ORDER BY id DESC LIMIT ?2",
        )
        .bind(req.traveler_id)
        .bind(limit)
        .fetch_all(ctx.pool().await)
        .await?;

        let items: Vec<Value> = rows
            .iter()
            .map(|(e, r, at)| json!({ "expression": e, "result": r, "at": at }))
            .collect();
        Ok(ActionOutcome::ok(
            "calculator_history",
            json!({ "history": items, "count": items.len() }),
        ))
    }
}

/* ── calculator_clear_history ───────────────────────────────── */

pub struct CalculatorClearHistory;

#[async_trait]
impl Tool for CalculatorClearHistory {
    fn name(&self) -> &str { "calculator_clear_history" }
    fn aliases(&self) -> &[&str] { &["clear_calculator_history", "clear_calc_history"] }
    fn step_label(&self) -> &str { "Clearing calculator history…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `calculator_clear_history` — Clear the user's calculation history. params: `{}`")
    }
    fn humanize(&self, _r: &str, _d: &Value) -> String {
        "Calculator history cleared".into()
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        sqlx::query("DELETE FROM calculator_history WHERE user_id = ?1")
            .bind(req.traveler_id)
            .execute(ctx.pool().await)
            .await?;
        Ok(ActionOutcome::ok("calculator_clear_history", json!({ "cleared": true })))
    }
}
