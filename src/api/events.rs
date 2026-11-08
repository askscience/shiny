//! `GET /api/events?topics=<a,b,…>` — every live stream the shell needs, on
//! one connection.
//!
//! Chromium (and so the LiteAura kiosk) opens at most six HTTP/1.1
//! connections per origin. Each permanent stream used to hold one: the four
//! top-bar panels plus one per open Craft window filled the pool, and every
//! other request (a HUD menu, the browser's suggestions, an app's WebAssembly)
//! queued behind them for up to a minute after login. This stream multiplexes
//! them: each SSE event is named after its topic.
//!
//! Topics:
//! - `audio`, `battery`, `bluetooth`, `network`: the panel snapshot, first the
//!   current one, then every change (same payload as `/api/<panel>/status`'s
//!   `data`, gated for remote clients the same way).
//! - `craft:<app>`: the page side of that Craft app's live-control link; each
//!   event is one vendor request, answered through
//!   `/api/plugins/craft/control/reply`.
//!
//! Unknown topics are skipped, so one stale name never takes the others down.
//! A changed topic set is a new connection; the old one ends when dropped.

use std::convert::Infallible;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::BoxStream;
use futures::{Stream, StreamExt};
use serde::Deserialize;

use crate::api::sse::snapshot_events;
use crate::api::AppState;

#[derive(Deserialize)]
pub struct EventsQuery {
    #[serde(default)]
    topics: String,
}

fn gate_remote(mut value: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = value.as_object_mut() {
        obj.insert("available".into(), serde_json::json!(false));
        obj.insert(
            "reason".into(),
            serde_json::json!("host control is unavailable to remote clients"),
        );
    }
    value
}

fn keep(value: serde_json::Value) -> serde_json::Value {
    value
}

/// GET /api/events
pub async fn events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let shape = if crate::api::remote::is_remote(&headers) { gate_remote } else { keep };
    let mut seen = std::collections::HashSet::new();
    let mut streams: Vec<BoxStream<'static, Event>> = Vec::new();

    for topic in query.topics.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        if !seen.insert(topic.to_string()) {
            continue;
        }
        let stream = match topic {
            "audio" => snapshot_events("audio", state.audio.subscribe(), state.audio.status(), shape),
            "battery" => {
                snapshot_events("battery", state.battery.subscribe(), state.battery.status(), shape)
            }
            "bluetooth" => snapshot_events(
                "bluetooth",
                state.bluetooth.subscribe(),
                state.bluetooth.status(),
                shape,
            ),
            "network" => {
                snapshot_events("network", state.network.subscribe(), state.network.status(), shape)
            }
            _ => match topic.strip_prefix("craft:") {
                Some(app) => {
                    match crate::plugins::craft_control::page_events(&state, app, Some(topic.to_string()))
                        .await
                    {
                        Ok(stream) => stream,
                        Err(error) => {
                            tracing::debug!("events: skipping {topic}: {error}");
                            continue;
                        }
                    }
                }
                None => {
                    tracing::debug!("events: unknown topic {topic}");
                    continue;
                }
            },
        };
        streams.push(stream);
    }

    // `select_all` of nothing ends at once; keep an idle stream open instead so
    // the client does not spin on reconnects.
    let merged: BoxStream<'static, Event> = if streams.is_empty() {
        futures::stream::pending().boxed()
    } else {
        futures::stream::select_all(streams).boxed()
    };
    Sse::new(merged.map(Ok)).keep_alive(KeepAlive::default())
}
