//! Shared SSE plumbing for the host-panel snapshot streams.
//!
//! Every panel (audio, battery, Bluetooth, network) follows the same contract:
//! subscribe first, read the initial snapshot, then emit that snapshot followed
//! by live changes. Subscribing before the read means an update that lands in
//! between is delivered by the live stream instead of being dropped.

use std::convert::Infallible;

use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::BoxStream;
use futures::{Stream, StreamExt};
use serde::Serialize;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

/// How a snapshot is turned into the frame's `data:` (the combined stream
/// gates host panels for remote clients, the per-panel streams do not).
type Shape = fn(serde_json::Value) -> serde_json::Value;

fn snapshot_event<T: Serialize>(event: &str, snapshot: &T, shape: Shape) -> Event {
    let value = shape(serde_json::to_value(snapshot).unwrap_or_default());
    Event::default()
        .event(event)
        .data(serde_json::to_string(&value).unwrap_or_default())
}

/// One initial snapshot followed by every broadcast change, as SSE events
/// named `event`. The building block of both the per-panel streams and the
/// combined `/api/events` stream.
pub fn snapshot_events<T>(
    event: &'static str,
    receiver: broadcast::Receiver<T>,
    initial: T,
    shape: Shape,
) -> BoxStream<'static, Event>
where
    T: Serialize + Clone + Send + 'static,
{
    let live = BroadcastStream::new(receiver).filter_map(move |message| async move {
        message.ok().map(|snapshot| snapshot_event(event, &snapshot, shape))
    });
    let first = futures::stream::once(async move { snapshot_event(event, &initial, shape) });
    first.chain(live).boxed()
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
    let stream = snapshot_events(event, receiver, initial, |value| value)
        .map(Ok::<Event, Infallible>);
    Sse::new(stream).keep_alive(KeepAlive::default())
}
