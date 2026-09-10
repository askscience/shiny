//! YouTube plugin REST routes — served through the plugin's `RouteSpec`s.

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, RouteHandler, user_id_from_request};
use shiny_plugin_sdk::services::PluginCtx;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    match tag {
        "yt_search" => Some(yt_search(ctx)),
        "yt_suggest" => Some(yt_suggest(ctx)),
        "yt_categories" => Some(yt_categories(ctx)),
        "yt_home" => Some(yt_home(ctx)),
        _ => None,
    }
}

fn user_id(req: &axum::extract::Request) -> Result<String, AppError> {
    user_id_from_request(req)
        .ok_or_else(|| AppError::Unauthorized("not authenticated".into()))
}

fn ok(data: Value) -> Response {
    axum::Json(json!({ "success": true, "data": data })).into_response()
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

fn yt_search(_ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize)]
    struct SearchQuery {
        q: String,
    }

    bridged_route(move |req: axum::extract::Request| {
        async move {
            let uid = user_id(&req)?;
            let (q, _) = take_query::<SearchQuery>(req).await?;
            let query = q.q.trim().to_string();
            if query.is_empty() {
                return Err(AppError::BadRequest("q required".into()));
            }
            // A search is a strong category signal — it feeds the homepage chips.
            crate::suggest::remember_query(&uid, &query);
            let results = crate::youtube_client::search(&query, 12).await?;
            // `{ results, count }` — the shape the AI tool and the window both
            // read. (This used to return a bare array, which is why the in-tile
            // search always said "no videos found".)
            let count = results.len();
            Ok(ok(json!({ "results": results, "count": count })))
        }
    })
}

/// Category chips for the YouTube window's idle homepage. Ranked by how much
/// the user watches/searches each topic — most-used first, at most 20.
fn yt_categories(_ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize)]
    struct CatsQuery {
        limit: Option<usize>,
    }

    bridged_route(move |req: axum::extract::Request| {
        async move {
            let uid = user_id(&req)?;
            let (q, _) = take_query::<CatsQuery>(req).await?;
            let categories = crate::suggest::categories(&uid, q.limit.unwrap_or(20));
            let count = categories.len();
            Ok(ok(json!({ "categories": categories, "count": count })))
        }
    })
}

/// The idle homepage: a shelf of videos per category, most-watched first.
/// Categories are fetched concurrently (bounded) and search results are
/// cached, so re-opening the window is quick.
fn yt_home(_ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize)]
    struct HomeQuery {
        per_category: Option<usize>,
        max_categories: Option<usize>,
    }

    bridged_route(move |req: axum::extract::Request| {
        async move {
            use futures::StreamExt;

            let uid = user_id(&req)?;
            let (q, _) = take_query::<HomeQuery>(req).await?;
            let per = q.per_category.unwrap_or(12).clamp(1, 24);
            let max = q.max_categories.unwrap_or(20).clamp(1, 20);

            let cats = crate::suggest::categories(&uid, max);
            let names: Vec<String> = cats.into_iter().map(|c| c.name).collect();
            let uid_ref = uid.as_str();
            let fetches = names.into_iter().enumerate().map(|(i, name)| async move {
                let videos = crate::suggest::videos_for(uid_ref, &name, per)
                    .await
                    .unwrap_or_default();
                (i, name, videos)
            });
            let mut got: Vec<(usize, String, Vec<crate::youtube_client::VideoResult>)> =
                futures::stream::iter(fetches)
                    .buffer_unordered(5)
                    .collect()
                    .await;
            got.sort_by_key(|(i, _, _)| *i); // keep the category ranking

            let mut sections: Vec<Value> = got
                .into_iter()
                .filter(|(_, _, videos)| !videos.is_empty())
                .map(|(_, category, videos)| {
                    json!({ "category": category, "count": videos.len(), "videos": videos })
                })
                .collect();

            // Brand-new user: no categories yet, so show what's trending.
            if sections.is_empty() {
                if let Ok((videos, seed)) = crate::suggest::suggest(&uid, None, per).await {
                    if !videos.is_empty() {
                        sections.push(json!({
                            "category": seed.title,
                            "count": videos.len(),
                            "videos": videos,
                        }));
                    }
                }
            }

            let count = sections.len();
            Ok(ok(json!({ "sections": sections, "count": count })))
        }
    })
}

/// Recommended videos for the YouTube window's "Up next" rail. The seed is the
/// video currently playing; with no seed the ranking falls back to the user's
/// most recent watch. Asking for suggestions also records the seed as watched.
fn yt_suggest(_ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize)]
    struct SuggestQuery {
        video_id: Option<String>,
        title: Option<String>,
        query: Option<String>,
        channel: Option<String>,
        limit: Option<usize>,
    }

    bridged_route(move |req: axum::extract::Request| {
        async move {
            let uid = user_id(&req)?;
            let (q, _) = take_query::<SuggestQuery>(req).await?;
            let limit = q.limit.unwrap_or(12).clamp(1, 24);
            let seed = crate::suggest::Seed {
                video_id: q.video_id.unwrap_or_default(),
                title: q.title.or(q.query).unwrap_or_default(),
                channel: q.channel.unwrap_or_default(),
            };
            let (results, seed) = crate::suggest::suggest(&uid, Some(seed), limit).await?;
            let count = results.len();
            Ok(ok(json!({
                "results": results,
                "count": count,
                "based_on": {
                    "video_id": seed.video_id,
                    "title": seed.title,
                    "channel": seed.channel,
                },
            })))
        }
    })
}
