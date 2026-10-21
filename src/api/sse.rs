//! Shared SSE plumbing for the host-panel snapshot streams.
//!
//! Every panel (audio, battery, Bluetooth, network) follows the same contract:
//! subscribe first, read the initial snapshot, then emit that snapshot followed
//! by live changes. Subscribing before the read means an update that lands in
//! between is delivered by the live stream instead of being dropped.

use std::convert::Infallible;

use axum::response::sse::{Event, KeepAlive, Sse};
use futures::{Stream, StreamExt};
use serde::Serialize;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

fn snapshot_event<T: Serialize>(event: &str, snapshot: &T) -> Event {
    Event::default()
        .event(event)
        .data(serde_json::to_string(snapshot).unwrap_or_default())
}

/// A stream of one initial snapshot followed by every broadcast change.
pub fn snapshot_stream<T>(
    event: &'static str,
    receiver: broadcast::Receiver<T>,
    initial: T,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>>
where
    T: Serialize + Clone + Send + 'static,
{
    let live = BroadcastStream::new(receiver).filter_map(move |message| async move {
        message
            .ok()
            .map(|snapshot| Ok::<Event, Infallible>(snapshot_event(event, &snapshot)))
    });
    let first = futures::stream::once(async move {
        Ok::<Event, Infallible>(snapshot_event(event, &initial))
    });

    Sse::new(first.chain(live)).keep_alive(KeepAlive::default())
}
