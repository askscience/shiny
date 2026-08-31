//! Impress plugin REST routes — served through the plugin's `RouteSpec`s.
//! DB access via the SDK's synchronous `ctx.db()`.

use std::sync::Arc;

use axum::extract::{FromRequest, FromRequestParts, Multipart};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value as Json};

use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::odp;
use shiny_plugin_sdk::routes::{bridged_route, RouteHandler, user_id_from_request, path_params_from_request};
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::Slide;

const MIME_ODP: &str = "application/vnd.oasis.opendocument.presentation";
const MAX_SLIDES: usize = 200;
const MAX_TEXT_LEN: usize = 10_000;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    Some(match tag {
        "deck_list" => deck_list(ctx),
        "deck_create" => deck_create(ctx),
        "deck_get" => deck_get(ctx),
        "deck_save" => deck_save(ctx),
        "deck_delete" => deck_delete(ctx),
        "deck_export" => deck_export(ctx),
        "deck_import" => deck_import(ctx),
        _ => return None,
    })
}

/* ── helpers ────────────────────────────────────────────────── */

fn user_id(req: &axum::extract::Request) -> Result<String, AppError> {
    user_id_from_request(req)
        .ok_or_else(|| AppError::Unauthorized("not authenticated".into()))
}

fn ok(data: Json) -> Response {
    axum::Json(json!({ "success": true, "data": data })).into_response()
}

fn clean_title(t: &str) -> String {
    let t = t.trim();
    if t.is_empty() { "Untitled".into() } else { t.chars().take(120).collect() }
}

fn filename_for_title(t: &str) -> String {
    let clean: String = t
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') { c } else { '-' })
        .collect();
    let clean = clean.trim().trim_matches('.').to_string();
    let clean = if clean.is_empty() { "presentation".to_string() } else { clean };
    format!("{clean}.odp")
}

fn as_text(v: &Value) -> String {
    match v {
        Value::Text(s) => s.clone(),
        Value::Int(n) => n.to_string(),
        _ => String::new(),
    }
}

fn take_path(req: &axum::extract::Request) -> Result<String, AppError> {
    let mut params = path_params_from_request(req)
        .ok_or_else(|| AppError::BadRequest("no path parameter found".into()))?;
    if params.len() != 1 {
        return Err(AppError::BadRequest("expected exactly one path parameter".into()));
    }
    Ok(params.remove(0).1)
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

fn parse_slides(json: &str) -> Vec<Slide> {
    if json.trim().is_empty() {
        return Vec::new();
    }
    serde_json::from_str::<Vec<Slide>>(json)
        .unwrap_or_default()
        .into_iter()
        .take(MAX_SLIDES)
        .collect()
}

fn validate_slides(slides: &[Slide]) -> Result<(), AppError> {
    if slides.len() > MAX_SLIDES {
        return Err(AppError::BadRequest(format!("Too many slides (max {MAX_SLIDES})")));
    }
    for s in slides {
        let text = [&s.title, &s.subtitle, &s.body, &s.attribution, &s.notes];
        if text.iter().any(|t| t.chars().count() > MAX_TEXT_LEN)
            || s.bullets.iter().chain(s.columns.iter().flatten()).any(|b| b.chars().count() > MAX_TEXT_LEN)
        {
            return Err(AppError::BadRequest(format!("Slide text is too long (max {MAX_TEXT_LEN} chars)")));
        }
    }
    Ok(())
}

/* ── handlers ───────────────────────────────────────────────── */

fn deck_list(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let rows = ctx.db().query(
                "SELECT id, title, slides, theme, updated_at FROM presentations \
                 WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT 200",
                &[Value::text(&uid)],
            )?;
            let decks: Vec<Json> = rows
                .iter()
                .map(|r| json!({ "id": as_text(&r[0]), "title": as_text(&r[1]), "slide_count": parse_slides(&as_text(&r[2])).len(), "theme": as_text(&r[3]), "updated_at": as_text(&r[4]) }))
                .collect();
            Ok(ok(json!(decks)))
        }
    })
}

fn deck_create(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Create {
        title: Option<String>,
        theme: Option<String>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let axum::Json(body) = axum::Json::<Create>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let title = clean_title(body.title.as_deref().unwrap_or("Untitled"));
            let theme = odp::normalize_theme(body.theme.as_deref().unwrap_or("aurora"));
            let id = uuid::Uuid::new_v4().to_string();

            ctx.db().execute(
                "INSERT INTO presentations (id, user_id, title, slides, theme, aspect, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, '[]', ?4, '16x9', datetime('now'), datetime('now'))",
                &[Value::text(&id), Value::text(&uid), Value::text(&title), Value::text(&theme)],
            )?;

            Ok(ok(json!({ "id": id, "title": title, "theme": theme, "aspect": "16x9", "slides": [], "updated_at": "now" })))
        }
    })
}

fn deck_get(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let rows = ctx.db().query(
                "SELECT title, slides, theme, aspect, updated_at FROM presentations \
                 WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let row = rows.first().ok_or_else(|| AppError::NotFound("Presentation not found".into()))?;
            let title = as_text(&row[0]);
            let slides = parse_slides(&as_text(&row[1]));
            let theme = as_text(&row[2]);
            let aspect = as_text(&row[3]);
            let updated_at = as_text(&row[4]);
            Ok(ok(json!({ "id": id, "title": title, "theme": theme, "aspect": aspect, "slides": slides, "updated_at": updated_at })))
        }
    })
}

fn deck_save(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Save {
        title: Option<String>,
        theme: Option<String>,
        slides: Option<Vec<Slide>>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let axum::Json(body) = axum::Json::<Save>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let slides = body.slides.unwrap_or_default();
            validate_slides(&slides)?;

            let current = ctx.db().query(
                "SELECT title, theme FROM presentations WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let cur = current.first().ok_or_else(|| AppError::NotFound("Presentation not found".into()))?;
            let new_title = match body.title {
                Some(t) => clean_title(&t),
                None => as_text(&cur[0]),
            };
            let new_theme = match body.theme {
                Some(t) => odp::normalize_theme(&t),
                None => as_text(&cur[1]),
            };
            let slides_json = serde_json::to_string(&slides)?;

            let changed = ctx.db().execute(
                "UPDATE presentations SET title = ?1, theme = ?2, slides = ?3, updated_at = datetime('now') \
                 WHERE id = ?4 AND user_id = ?5",
                &[Value::text(&new_title), Value::text(&new_theme), Value::text(&slides_json), Value::text(&id), Value::text(&uid)],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound("Presentation not found".into()));
            }
            Ok(ok(json!({ "success": true })))
        }
    })
}

fn deck_delete(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let changed = ctx.db().execute(
                "DELETE FROM presentations WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound("Presentation not found".into()));
            }
            Ok(ok(json!({ "success": true })))
        }
    })
}

fn deck_export(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let rows = ctx.db().query(
                "SELECT title, slides, theme FROM presentations WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let row = rows.first().ok_or_else(|| AppError::NotFound("Presentation not found".into()))?;
            let title = as_text(&row[0]);
            let slides = parse_slides(&as_text(&row[1]));
            let theme = as_text(&row[2]);
            let bytes = odp::slides_to_odp(&theme, &slides)?;
            let filename = filename_for_title(&title);
            Ok(Response::builder()
                .header(CONTENT_TYPE, MIME_ODP)
                .header(CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename.replace('"', "")))
                .body(axum::body::Body::from(bytes))
                .map_err(|e| AppError::Internal(format!("Export failed: {e}")))?)
        }
    })
}

fn deck_import(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct ImportQuery {
        name: Option<String>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (q, req) = take_query::<ImportQuery>(req).await?;
            let mut multipart = Multipart::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?;

            let mut bytes: Option<Vec<u8>> = None;
            let mut original_name: Option<String> = None;
            while let Some(field) = multipart
                .next_field()
                .await
                .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
            {
                if field.name() == Some("file") {
                    original_name = field.file_name().map(|f| f.to_string()).or(original_name);
                    let data = field
                        .bytes()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("read error: {e}")))?;
                    bytes = Some(data.to_vec());
                }
            }
            let data = bytes.ok_or_else(|| AppError::BadRequest("missing 'file' field".into()))?;

            let stem = original_name
                .as_deref()
                .map(|n| n.rsplit('.').nth(1).unwrap_or(n))
                .unwrap_or("Imported presentation");
            let title = clean_title(q.name.as_deref().unwrap_or(stem));

            let slides = odp::odp_to_slides(&data)?;
            let slides_json = serde_json::to_string(&slides)?;
            let id = uuid::Uuid::new_v4().to_string();
            ctx.db().execute(
                "INSERT INTO presentations (id, user_id, title, slides, theme, aspect, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, 'aurora', '16x9', datetime('now'), datetime('now'))",
                &[Value::text(&id), Value::text(&uid), Value::text(&title), Value::text(&slides_json)],
            )?;

            Ok(ok(json!({ "title": title })))
        }
    })
}
