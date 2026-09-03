//! Calculator plugin REST routes — served through the plugin's `RouteSpec`s.
//! The Calculator window evaluates expressions and reads/clears history here;
//! DB access via the SDK's synchronous `ctx.db()`.

use std::sync::Arc;

use axum::extract::{FromRequest, FromRequestParts};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value as Json};

use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, RouteHandler, user_id_from_request};
use shiny_plugin_sdk::services::PluginCtx;

use crate::eval::{evaluate, format_number};

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    Some(match tag {
        "eval" => eval(ctx),
        "history_list" => history_list(ctx),
        "history_clear" => history_clear(ctx),
        _ => return None,
    })
}

fn user_id(req: &axum::extract::Request) -> Result<String, AppError> {
    user_id_from_request(req)
        .ok_or_else(|| AppError::Unauthorized("not authenticated".into()))
}

fn ok(data: Json) -> Response {
    axum::Json(json!({ "success": true, "data": data })).into_response()
}

fn as_text(v: &Value) -> String {
    match v {
        Value::Text(s) => s.clone(),
        Value::Int(n) => n.to_string(),
        _ => String::new(),
    }
}

async fn take_query<T: DeserializeOwned + Send + 'static>(
    req: axum::extract::Request,
) -> Result<(T, axum::extract::Request), AppError> {
    let (mut parts, body) = req.into_parts();
    let query = axum::extract::Query::<T>::from_request_parts(&mut parts, &())
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid query: {e}")))?;
    Ok((query.0, axum::extract::Request::from_parts(parts, body)))
}

fn eval(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct EvalBody {
        expression: Option<String>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let axum::Json(body) = axum::Json::<EvalBody>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let expression = body.expression.unwrap_or_default().trim().to_string();
            if expression.is_empty() {
                return Err(AppError::BadRequest("expression required".into()));
            }

            let result = evaluate(&expression).map_err(AppError::BadRequest)?;
            let result_text = format_number(result);

            ctx.db().execute(
                "INSERT INTO calculator_history (user_id, expression, result, created_at) \
                 VALUES (?1, ?2, ?3, datetime('now'))",
                &[Value::text(&uid), Value::text(&expression), Value::text(&result_text)],
            )?;

            Ok(ok(json!({ "expression": expression, "result": result, "result_text": result_text })))
        }
    })
}

fn history_list(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Q {
        limit: Option<i64>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (q, _req) = take_query::<Q>(req).await?;
            let limit = q.limit.unwrap_or(50).clamp(1, 200);
            let rows = ctx.db().query(
                "SELECT expression, result, created_at FROM calculator_history \
                 WHERE user_id = ?1 ORDER BY id DESC LIMIT ?2",
                &[Value::text(&uid), Value::Int(limit)],
            )?;
            let items: Vec<Json> = rows
                .iter()
                .map(|r| json!({ "expression": as_text(&r[0]), "result": as_text(&r[1]), "at": as_text(&r[2]) }))
                .collect();
            Ok(ok(json!({ "history": items, "count": items.len() })))
        }
    })
}

fn history_clear(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            ctx.db().execute(
                "DELETE FROM calculator_history WHERE user_id = ?1",
                &[Value::text(&uid)],
            )?;
            Ok(ok(json!({ "cleared": true })))
        }
    })
}
