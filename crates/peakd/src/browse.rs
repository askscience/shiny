//! Browser-plugin view protocol and state for the Qt shell.
//!
//! The Browser plugin renders each tab in a native child web view at the
//! page's true origin (its reason to exist: Cloudflare sees an ordinary
//! browser). The page drives the shell over `window.ipc` with JSON messages
//! prefixed [`IPC_PREFIX`]; this module owns that protocol, the command and
//! event queues, and the view state machine. The Qt objects live behind
//! [`ViewHost`], implemented by `host::QtHost` with the shim's C ABI.
//!
//! Threading: every callback from Qt runs on the Qt main thread; the queues
//! therefore only guard against the pump ticking while a callback pushes.
//! Commands are drained and applied on the pump (100 ms), and events are
//! reported back by evaluating `window.__peakdViewEvent` on the main view.

use std::collections::HashSet;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{json, Value};

/// Prefix that marks an IPC message as a browser-view command.
pub const IPC_PREFIX: &str = "peakd:view:";

/// The viewport rectangle as the page measured it: CSS pixels relative to the
/// page, plus the device ratio so the shell can map it to native pixels without
/// assuming the kiosk's page zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CssRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub dpr: f64,
}

impl CssRect {
    fn parse(value: &Value) -> Option<Self> {
        let rect = value.get("rect")?;
        let num = |key: &str| rect.get(key).and_then(Value::as_f64);
        Some(Self {
            x: num("x")?,
            y: num("y")?,
            w: num("w")?,
            h: num("h")?,
            // A page that reports a nonsense ratio gets the identity rather
            // than a native view with zero size.
            dpr: num("dpr").filter(|d| *d > 0.0).unwrap_or(1.0),
        })
    }

    /// Qt logical pixels for a child widget.
    ///
    /// The page's `devicePixelRatio` already includes the shell's page zoom
    /// (the shell forces Qt's own scale factor to 1), so the mapping is the
    /// same as the GTK shell's physical-pixel one: CSS × dpr.
    #[must_use]
    pub fn to_logical(self) -> (i32, i32, i32, i32) {
        let x = (self.x * self.dpr).round() as i32;
        let y = (self.y * self.dpr).round() as i32;
        let w = (self.w * self.dpr).round().max(0.0) as i32;
        let h = (self.h * self.dpr).round().max(0.0) as i32;
        (x, y, w, h)
    }
}

/// One command from the page.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Open {
        id: String,
        url: String,
        rect: Option<CssRect>,
        visible: bool,
        incognito: bool,
    },
    Navigate {
        id: String,
        url: String,
    },
    SetBounds {
        id: String,
        rect: CssRect,
    },
    SetVisible {
        id: String,
        visible: bool,
    },
    Back {
        id: String,
    },
    Forward {
        id: String,
    },
    Reload {
        id: String,
    },
    Close {
        id: String,
    },
    Focus {
        id: String,
    },
}

impl Command {
    fn parse(value: &Value) -> Option<Self> {
        let id = value.get("id")?.as_str()?.to_string();
        let op = value.get("op")?.as_str()?;
        let rect = CssRect::parse(value);
        Some(match op {
            "open" => Self::Open {
                id,
                url: value.get("url")?.as_str()?.to_string(),
                rect,
                visible: value
                    .get("visible")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                incognito: value
                    .get("incognito")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            },
            "navigate" => Self::Navigate {
                id,
                url: value.get("url")?.as_str()?.to_string(),
            },
            "setBounds" => Self::SetBounds { id, rect: rect? },
            "setVisible" => Self::SetVisible {
                id,
                visible: value
                    .get("visible")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            },
            "back" => Self::Back { id },
            "forward" => Self::Forward { id },
            "reload" => Self::Reload { id },
            "close" => Self::Close { id },
            "focus" => Self::Focus { id },
            _ => return None,
        })
    }
}

/// The queue between the page (which may only post messages) and the pump
/// (which may touch views). Cheap to clone; every clone shares the queues.
#[derive(Clone)]
pub struct ViewBus {
    commands: Arc<Mutex<Vec<Command>>>,
    events: Arc<Mutex<Vec<Value>>>,
}

impl ViewBus {
    #[must_use]
    pub fn new() -> Self {
        Self {
            commands: Arc::new(Mutex::new(Vec::new())),
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Handle one IPC message. Returns `true` when it was a browser-view
    /// command (or a malformed one), `false` when the caller should keep
    /// looking.
    pub fn handle_ipc(&self, body: &str) -> bool {
        let Some(raw) = body.strip_prefix(IPC_PREFIX) else {
            return false;
        };
        if let Ok(value) = serde_json::from_str::<Value>(raw) {
            if let Some(command) = Command::parse(&value) {
                self.commands.lock().push(command);
            }
        }
        true
    }

    /// Queue one event for `window.__peakdViewEvent` on the main view.
    pub fn push_event(&self, event: Value) {
        self.events.lock().push(event);
    }

    #[must_use]
    pub fn view_event(id: &str, kind: &str, payload: &str) -> Value {
        match kind {
            "title" => json!({ "id": id, "type": "title", "title": payload }),
            "url" => json!({ "id": id, "type": "url", "url": payload }),
            "new-window" => json!({ "id": id, "type": "new-window", "url": payload }),
            "load" => json!({ "id": id, "type": "load", "phase": payload }),
            other => json!({ "id": id, "type": other, "payload": payload }),
        }
    }
}

impl Default for ViewBus {
    fn default() -> Self {
        Self::new()
    }
}

/// The native side of a child view. Implemented by `host::QtHost` (shim) in
/// the shell; a stub keeps the state machine testable without Qt.
pub trait ViewHost {
    fn create(&mut self, id: &str, url: &str, rect: Option<CssRect>, visible: bool, incognito: bool);
    fn navigate(&mut self, id: &str, url: &str);
    fn set_bounds(&mut self, id: &str, rect: CssRect);
    fn set_visible(&mut self, id: &str, visible: bool);
    fn back(&mut self, id: &str);
    fn forward(&mut self, id: &str);
    fn reload(&mut self, id: &str);
    fn focus(&mut self, id: &str);
    fn close(&mut self, id: &str);
}

/// Owns the live child views and drains [`ViewBus`] on the shell's pump.
pub struct Views<H: ViewHost> {
    bus: ViewBus,
    host: H,
    live: HashSet<String>,
}

impl<H: ViewHost> Views<H> {
    pub fn new(bus: ViewBus, host: H) -> Self {
        Self {
            bus,
            host,
            live: HashSet::new(),
        }
    }

    /// Drain the queues. `evaluate` runs JS on the main view.
    pub fn pump(&mut self, mut evaluate: impl FnMut(&str)) {
        let commands: Vec<Command> = self.bus.commands.lock().drain(..).collect();
        for command in commands {
            self.apply(command);
        }
        let events: Vec<Value> = self.bus.events.lock().drain(..).collect();
        for event in events {
            // `window.__peakdViewEvent` is defined by the plugin module; the
            // guard means a missing handler is a no-op, not an error page.
            evaluate(&format!(
                "window.__peakdViewEvent && window.__peakdViewEvent({event})"
            ));
        }
    }

    fn apply(&mut self, command: Command) {
        match command {
            Command::Open {
                id,
                url,
                rect,
                visible,
                incognito,
            } => {
                if self.live.contains(&id) {
                    self.host.navigate(&id, &url);
                    if let Some(rect) = rect {
                        self.host.set_bounds(&id, rect);
                    }
                    self.host.set_visible(&id, visible);
                } else {
                    self.host.create(&id, &url, rect, visible, incognito);
                    self.live.insert(id);
                }
            }
            Command::Navigate { id, url } => {
                if self.live.contains(&id) {
                    self.host.navigate(&id, &url);
                }
            }
            Command::SetBounds { id, rect } => {
                if self.live.contains(&id) {
                    self.host.set_bounds(&id, rect);
                }
            }
            Command::SetVisible { id, visible } => {
                if self.live.contains(&id) {
                    self.host.set_visible(&id, visible);
                }
            }
            Command::Back { id } => {
                if self.live.contains(&id) {
                    self.host.back(&id);
                }
            }
            Command::Forward { id } => {
                if self.live.contains(&id) {
                    self.host.forward(&id);
                }
            }
            Command::Reload { id } => {
                if self.live.contains(&id) {
                    self.host.reload(&id);
                }
            }
            Command::Close { id } => {
                if self.live.remove(&id) {
                    self.host.close(&id);
                }
            }
            Command::Focus { id } => {
                if self.live.contains(&id) {
                    self.host.focus(&id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_rect_scales_by_device_ratio() {
        let rect = CssRect {
            x: 10.0,
            y: 20.0,
            w: 300.0,
            h: 200.0,
            dpr: 2.0,
        };
        assert_eq!(rect.to_logical(), (20, 40, 600, 400));
    }

    #[test]
    fn missing_device_ratio_is_identity() {
        let value: Value = serde_json::from_str(r#"{"rect":{"x":0,"y":0,"w":5,"h":5}}"#).unwrap();
        assert_eq!(CssRect::parse(&value).unwrap().dpr, 1.0);
    }

    #[test]
    fn parses_open_command() {
        let value: Value = serde_json::from_str(
            r#"{"op":"open","id":"t1","url":"https://example.com/","rect":{"x":1,"y":2,"w":3,"h":4,"dpr":1},"visible":true}"#,
        )
        .unwrap();
        match Command::parse(&value).unwrap() {
            Command::Open {
                id,
                url,
                rect,
                visible,
                incognito,
            } => {
                assert_eq!(id, "t1");
                assert_eq!(url, "https://example.com/");
                assert!(visible);
                assert!(!incognito);
                assert_eq!(rect.unwrap().w, 3.0);
            }
            other => panic!("wrong command: {other:?}"),
        }
    }

    #[test]
    fn parses_an_incognito_open_command() {
        let value: Value = serde_json::from_str(
            r#"{"op":"open","id":"t2","url":"https://example.com/","visible":true,"incognito":true}"#,
        )
        .unwrap();
        match Command::parse(&value).unwrap() {
            Command::Open { incognito, .. } => assert!(incognito),
            other => panic!("wrong command: {other:?}"),
        }
    }

    #[test]
    fn unknown_operation_is_ignored() {
        let value: Value = serde_json::from_str(r#"{"op":"destroy","id":"t1"}"#).unwrap();
        assert!(Command::parse(&value).is_none());
    }

    /// A host that records calls, so the state machine is testable without Qt.
    #[derive(Default)]
    struct RecordingHost {
        calls: Vec<String>,
    }

    impl ViewHost for RecordingHost {
        fn create(&mut self, id: &str, url: &str, _rect: Option<CssRect>, visible: bool, incognito: bool) {
            self.calls.push(format!("create {id} {url} {visible} {incognito}"));
        }
        fn navigate(&mut self, id: &str, url: &str) {
            self.calls.push(format!("navigate {id} {url}"));
        }
        fn set_bounds(&mut self, id: &str, rect: CssRect) {
            self.calls.push(format!("bounds {id} {}x{}", rect.w, rect.h));
        }
        fn set_visible(&mut self, id: &str, visible: bool) {
            self.calls.push(format!("visible {id} {visible}"));
        }
        fn back(&mut self, id: &str) {
            self.calls.push(format!("back {id}"));
        }
        fn forward(&mut self, id: &str) {
            self.calls.push(format!("forward {id}"));
        }
        fn reload(&mut self, id: &str) {
            self.calls.push(format!("reload {id}"));
        }
        fn focus(&mut self, id: &str) {
            self.calls.push(format!("focus {id}"));
        }
        fn close(&mut self, id: &str) {
            self.calls.push(format!("close {id}"));
        }
    }

    #[test]
    fn commands_reach_the_host_in_order() {
        let bus = ViewBus::new();
        assert!(bus.handle_ipc(
            r#"peakd:view:{"op":"open","id":"t1","url":"https://a/","rect":{"x":1,"y":2,"w":3,"h":4,"dpr":1},"visible":true}"#
        ));
        assert!(bus.handle_ipc(r#"peakd:view:{"op":"navigate","id":"t1","url":"https://b/"}"#));
        assert!(bus.handle_ipc(r#"peakd:view:{"op":"close","id":"t1"}"#));
        // A command for a closed view is dropped.
        assert!(bus.handle_ipc(r#"peakd:view:{"op":"back","id":"t1"}"#));

        let mut views = Views::new(bus, RecordingHost::default());
        views.pump(|_| {});
        assert_eq!(
            views.host.calls,
            vec![
                "create t1 https://a/ true false",
                "navigate t1 https://b/",
                "close t1",
            ]
        );
    }
}
