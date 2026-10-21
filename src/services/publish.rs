//! Change-gated publishing for the host-panel services.

use std::sync::Mutex;

use serde::Serialize;

/// Publish a snapshot only when its serialized form changed.
///
/// The panel readers poll on an interval; this is what keeps idle SSE clients
/// from being woken for nothing.
pub(crate) fn publish_if_changed<T, F>(last_json: &Mutex<String>, status: T, publish: F)
where
    T: Serialize,
    F: FnOnce(T),
{
    let json = serde_json::to_string(&status).unwrap_or_default();
    {
        let mut last = last_json.lock().unwrap_or_else(|e| e.into_inner());
        if *last == json {
            return;
        }
        *last = json;
    }
    publish(status);
}
