//! Native web views for the in-app Browser plugin.
//!
//! The Browser plugin used to render every page in an `<iframe>` whose origin
//! was the filter proxy, which forced the proxy to rewrite the document and its
//! cookies for a `127.0.0.1` origin. That is fine for ordinary sites but breaks
//! anti-bot systems: Cloudflare's JS challenge loops forever (its `__cf_bm` /
//! `cf_clearance` cookies are re-scoped, and the challenge runs at the wrong
//! origin) and some engines answer 403 outright.
//!
//! This module renders each tab in a real WebKitGTK **child webview** instead.
//! The child is created with `wry`'s `build_as_child` over the kiosk window and
//! positioned with `set_bounds`, so the page has its true origin, its own
//! cookies and its own TLS — Cloudflare sees an ordinary browser. Traffic still
//! goes through the filter engine: `route_through` sets the process-wide
//! `http_proxy`, and WebKitGTK applies it to every webview in the process;
//! inside a `CONNECT` tunnel the engine filters by hostname.
//!
//! The page drives this over the existing IPC bridge (`window.ipc`): JSON
//! messages prefixed with [`IPC_PREFIX`]. Commands are queued and drained on the
//! event loop — the only thread allowed to touch a `WebView` — and the shell
//! reports load/title/url back by calling `window.__peakdViewEvent` on the main
//! webview.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{json, Value};
use tao::dpi::{PhysicalPosition, PhysicalSize, Position, Size};
use tao::event_loop::EventLoopProxy;
use tao::window::Window;
use wry::{
    NewWindowResponse, PageLoadEvent, PermissionKind, PermissionResponse, Rect, WebView,
    WebViewBuilder,
};

use crate::bench::BenchStep;

/// Prefix that marks an IPC message as a browser-view command.
pub const IPC_PREFIX: &str = "peakd:view:";

/// The User-Agent a native view presents to the open web.
///
/// The main webview pins a `Peakd/…` UA for the app, and a child webview can
/// inherit the shared WebKit settings. An unknown UA riding on WebKitGTK's TLS
/// is exactly the mismatch Cloudflare's bot management flags, so native views
/// pin the ordinary Linux Safari UA instead — the same engine family, a UA the
/// web already knows.
const CHILD_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 \
(KHTML, like Gecko) Version/17.0 Safari/605.1.15";

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

    /// Native pixels for `wry::Rect`. `dpr` is the page's own ratio (page zoom
    /// times display scale), so no assumption about the kiosk is baked in here.
    fn to_rect(self) -> Rect {
        let x = (self.x * self.dpr).round() as i32;
        let y = (self.y * self.dpr).round() as i32;
        let w = (self.w * self.dpr).round().max(0.0) as u32;
        let h = (self.h * self.dpr).round().max(0.0) as u32;
        Rect {
            position: Position::Physical(PhysicalPosition::new(x, y)),
            size: Size::Physical(PhysicalSize::new(w, h)),
        }
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

/// The queue between the page (which may only post messages) and the event loop
/// (which may touch a `WebView`). Cheap to clone; every clone shares the queues.
#[derive(Clone)]
pub struct ViewBus {
    commands: Arc<Mutex<VecDeque<Command>>>,
    events: Arc<Mutex<VecDeque<Value>>>,
    proxy: EventLoopProxy<BenchStep>,
}

impl ViewBus {
    pub fn new(proxy: EventLoopProxy<BenchStep>) -> Self {
        Self {
            commands: Arc::new(Mutex::new(VecDeque::new())),
            events: Arc::new(Mutex::new(VecDeque::new())),
            proxy,
        }
    }

    /// Handle one IPC message. Returns `true` when it was a browser-view
    /// command (and was consumed), `false` when the caller should keep looking.
    pub fn handle_ipc(&self, body: &str) -> bool {
        let Some(raw) = body.strip_prefix(IPC_PREFIX) else {
            return false;
        };
        if let Ok(value) = serde_json::from_str::<Value>(raw) {
            if let Some(command) = Command::parse(&value) {
                self.commands.lock().push_back(command);
                self.wake();
            }
        }
        true
    }

    /// Leave the kiosk (the page's Cmd/Alt+Q shortcut).
    pub fn exit(&self) {
        let _ = self.proxy.send_event(BenchStep::Exit);
    }

    fn wake(&self) {
        let _ = self.proxy.send_event(BenchStep::View);
    }

    fn push_event(&self, event: Value) {
        self.events.lock().push_back(event);
        self.wake();
    }
}

/// Owns the live child webviews and drains [`ViewBus`] on the event loop.
pub struct Views {
    bus: ViewBus,
    live: HashMap<String, WebView>,
}

impl Views {
    pub fn new(bus: ViewBus) -> Self {
        Self {
            bus,
            live: HashMap::new(),
        }
    }

    /// Drain the queues. Called from the event loop when `BenchStep::View`
    /// arrives; every `WebView` method must run on that thread.
    pub fn pump(&mut self, window: &Window, main: &WebView) {
        let commands: Vec<Command> = self.bus.commands.lock().drain(..).collect();
        for command in commands {
            self.apply(command, window);
        }
        let events: Vec<Value> = self.bus.events.lock().drain(..).collect();
        for event in events {
            // `window.__peakdViewEvent` is defined by the plugin module; the
            // guard means a missing handler is a no-op, not an error page.
            let js = format!("window.__peakdViewEvent && window.__peakdViewEvent({event})");
            if let Err(err) = main.evaluate_script(&js) {
                tracing::debug!("peakd: could not report a browser-view event: {err}");
            }
        }
    }

    fn apply(&mut self, command: Command, window: &Window) {
        match command {
            Command::Open {
                id,
                url,
                rect,
                visible,
            } => {
                if let Some(view) = self.live.get(&id) {
                    let _ = view.load_url(&url);
                    if let Some(rect) = rect {
                        let _ = view.set_bounds(rect.to_rect());
                    }
                    let _ = view.set_visible(visible);
                } else if let Some(view) = self.create(window, &id, &url, rect, visible) {
                    self.live.insert(id, view);
                }
            }
            Command::Navigate { id, url } => {
                if let Some(view) = self.live.get(&id) {
                    let _ = view.load_url(&url);
                }
            }
            Command::SetBounds { id, rect } => {
                if let Some(view) = self.live.get(&id) {
                    let _ = view.set_bounds(rect.to_rect());
                }
            }
            Command::SetVisible { id, visible } => {
                if let Some(view) = self.live.get(&id) {
                    let _ = view.set_visible(visible);
                }
            }
            Command::Back { id } => {
                if let Some(view) = self.live.get(&id) {
                    let _ = view.go_back();
                }
            }
            Command::Forward { id } => {
                if let Some(view) = self.live.get(&id) {
                    let _ = view.go_forward();
                }
            }
            Command::Reload { id } => {
                if let Some(view) = self.live.get(&id) {
                    let _ = view.reload();
                }
            }
            Command::Close { id } => {
                // Dropping the `WebView` closes its native window.
                self.live.remove(&id);
            }
            Command::Focus { id } => {
                if let Some(view) = self.live.get(&id) {
                    let _ = view.focus();
                }
            }
        }
    }

    fn create(
        &self,
        window: &Window,
        id: &str,
        url: &str,
        rect: Option<CssRect>,
        visible: bool,
    ) -> Option<WebView> {
        let bus = self.bus.clone();

        let nav = {
            let bus = bus.clone();
            let id = id.to_string();
            move |url: String| {
                bus.push_event(json!({ "id": id, "type": "url", "url": url }));
                true
            }
        };
        let title = {
            let bus = bus.clone();
            let id = id.to_string();
            move |title: String| {
                bus.push_event(json!({ "id": id, "type": "title", "title": title }));
            }
        };
        let load = {
            let bus = bus.clone();
            let id = id.to_string();
            move |event: PageLoadEvent, url: String| {
                let phase = match event {
                    PageLoadEvent::Started => "started",
                    PageLoadEvent::Finished => "finished",
                };
                bus.push_event(json!({ "id": id, "type": "load", "phase": phase, "url": url }));
            }
        };
        let ipc = {
            let bus = bus.clone();
            move |request: wry::http::Request<String>| {
                // The only message a child view is given a bridge for is the
                // way out of the kiosk; everything else is ignored so a page
                // cannot drive the view manager from inside.
                if request.body() == crate::EXIT_MESSAGE {
                    bus.exit();
                }
            }
        };
        let new_window = {
            let bus = bus.clone();
            let id = id.to_string();
            move |url: String, _features| {
                // Never hand a popup to the OS browser (that would leave the
                // filtered kiosk); the plugin opens a tab instead.
                bus.push_event(json!({ "id": id, "type": "new-window", "url": url }));
                NewWindowResponse::Deny
            }
        };

        let mut builder = WebViewBuilder::new()
            .with_url(url)
            .with_visible(visible)
            .with_user_agent(CHILD_USER_AGENT)
            .with_navigation_handler(nav)
            .with_document_title_changed_handler(title)
            .with_on_page_load_handler(load)
            // Same grant as the main webview: the mic/camera must work on the
            // pages the user browses, not only in the app chrome.
            .with_permission_handler(|kind| match kind {
                PermissionKind::Microphone | PermissionKind::Camera => PermissionResponse::Allow,
                _ => PermissionResponse::Default,
            })
            .with_initialization_script(crate::EXIT_SHORTCUT_JS)
            .with_ipc_handler(ipc)
            .with_new_window_req_handler(new_window);
        if let Some(rect) = rect {
            builder = builder.with_bounds(rect.to_rect());
        }

        match builder.build_as_child(window) {
            Ok(view) => Some(view),
            Err(err) => {
                tracing::warn!("peakd: could not create a browser view: {err}");
                None
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
        }
        .to_rect();
        match rect.position {
            Position::Physical(p) => {
                assert_eq!((p.x, p.y), (20, 40));
            }
            other => panic!("expected a physical position, got {other:?}"),
        }
        match rect.size {
            Size::Physical(s) => {
                assert_eq!((s.width, s.height), (600, 400));
            }
            other => panic!("expected a physical size, got {other:?}"),
        }
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
            } => {
                assert_eq!(id, "t1");
                assert_eq!(url, "https://example.com/");
                assert!(visible);
                assert_eq!(rect.unwrap().w, 3.0);
            }
            other => panic!("wrong command: {other:?}"),
        }
    }

    #[test]
    fn unknown_operation_is_ignored() {
        let value: Value = serde_json::from_str(r#"{"op":"destroy","id":"t1"}"#).unwrap();
        assert!(Command::parse(&value).is_none());
    }
}
