//! Craft control-channel proxy.
//!
//! The Craft web builds cannot listen on a socket, so the vendor's desktop
//! control channel (`<app> --control <port>`) is not reachable from
//! `<app>-cli mcp --connect`. This module gives every Craft app a loopback port
//! that speaks the vendor's own line protocol and forwards each request to the
//! page showing the app, which runs it through its documented page API
//! (`window.<app>.request(method, params)`).
//!
//! **Nothing here is app-specific.** A link exists per app id; the MCP host asks
//! for the port and, when a page is attached, starts the app's CLI with
//! `--connect`. A Craft app installed later — or a future app that ships the same
//! documented page API — gets live control with no change here. When no page is
//! attached the host keeps its headless server, so the feature degrades to what
//! the app can do without a live channel.
//!
//! The page side lives in `web/js/webAppSurface.js`: an `EventSource` on
//! [`events`] receives `{"request": {…}}` and POSTs the answer to [`reply`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::Extension;
use axum::Json;
use futures::stream::Stream;
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;

/// The vendor's own bound on one control request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// One request is one line; the vendor refuses anything past 4 MiB.
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;
/// Requests buffered for one page before the link pushes back.
const PAGE_QUEUE: usize = 64;

/// One live control link per Craft app id. Cheap to clone (the links are
/// shared); `AppState` holds one for the whole server.
#[derive(Clone, Default)]
pub struct CraftControl {
    links: Arc<Mutex<HashMap<String, Arc<Link>>>>,
}

impl CraftControl {
    pub fn new() -> Self {
        Self::default()
    }

    /// The loopback port for `app`, when a page is attached and can serve it.
    /// The MCP host asks this before spawning an app's CLI: `Some(port)` means
    /// `--connect`, `None` means the headless fallback.
    pub async fn live_port(&self, app: &str) -> Option<u16> {
        let link = self.link(app).await.ok()?;
        link.has_page().then_some(link.port)
    }

    /// The link for one app, started on first use. Binding loopback here is why
    /// the API is not app-specific: the port is allocated by the OS.
    pub async fn link(&self, app: &str) -> Result<Arc<Link>, AppError> {
        if let Some(link) = self.links.lock().get(app) {
            return Ok(link.clone());
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|error| {
            AppError::Internal(format!("could not open the {app} control link: {error}"))
        })?;
        let port = listener
            .local_addr()
            .map_err(|error| AppError::Internal(error.to_string()))?
            .port();
        let link = Arc::new(Link::new(port));
        {
            let mut links = self.links.lock();
            // Another turn may have created it while we were binding.
            if let Some(existing) = links.get(app) {
                return Ok(existing.clone());
            }
            links.insert(app.to_string(), link.clone());
        }
        let accept = link.clone();
        tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else { return };
                let link = accept.clone();
                tokio::spawn(async move {
                    let _ = serve_connection(socket, link).await;
                });
            }
        });
        Ok(link)
    }
}

/// One app's control link: the vendor's line protocol on one side, the page's
/// SSE stream on the other.
pub struct Link {
    port: u16,
    /// Requests waiting for the page's answer, keyed by the vendor's request id.
    pending: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    /// Attached pages (usually one; more than one is harmless).
    pages: Mutex<Vec<(u64, mpsc::Sender<String>)>>,
    next_page: AtomicU64,
}

impl Link {
    fn new(port: u16) -> Self {
        Self {
            port,
            pending: Mutex::new(HashMap::new()),
            pages: Mutex::new(Vec::new()),
            next_page: AtomicU64::new(1),
        }
    }

    /// Is a page showing this app and able to answer?
    pub fn has_page(&self) -> bool {
        !self.pages.lock().is_empty()
    }

    /// Hand one vendor request to the page and wait for its answer.
    async fn dispatch(&self, line: &str, timeout: Duration) -> Value {
        let Ok(request) = serde_json::from_str::<Value>(line) else {
            return json!({ "ok": false, "error": "not a control request" });
        };
        let Some(id) = request.get("id").and_then(Value::as_u64) else {
            return json!({ "ok": false, "error": "a control request needs an id" });
        };
        let page = self.pages.lock().first().map(|(_, sender)| sender.clone());
        let Some(page) = page else {
            return json!({
                "id": id,
                "ok": false,
                "error": "no window is showing this app",
            });
        };
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().insert(id, sender);
        if page.send(request.to_string()).await.is_err() {
            self.pending.lock().remove(&id);
            return json!({ "id": id, "ok": false, "error": "the window went away" });
        }
        match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => json!({ "id": id, "ok": false, "error": "the window dropped the request" }),
            Err(_) => {
                self.pending.lock().remove(&id);
                json!({ "id": id, "ok": false, "error": "timeout: the window did not answer" })
            }
        }
    }

    /// The page's answer to one request, shaped the way the vendor's channel
    /// answers (`{id, ok, result}` / `{id, ok: false, error}`).
    fn complete(&self, id: u64, ok: bool, result: Option<Value>, error: Option<String>) {
        let Some(sender) = self.pending.lock().remove(&id) else { return };
        let reply = if ok {
            json!({ "id": id, "ok": true, "result": result.unwrap_or(Value::Null) })
        } else {
            json!({
                "id": id,
                "ok": false,
                "error": error.unwrap_or_else(|| "the app reported an error".into()),
            })
        };
        let _ = sender.send(reply);
    }

    fn attach(&self) -> (u64, mpsc::Receiver<String>) {
        let (sender, receiver) = mpsc::channel(PAGE_QUEUE);
        let id = self.next_page.fetch_add(1, Ordering::SeqCst);
        self.pages.lock().push((id, sender));
        (id, receiver)
    }

    fn detach(&self, page: u64) {
        self.pages.lock().retain(|(id, _)| *id != page);
    }
}

/// One connection from `<app>-cli mcp --connect`: newline-delimited JSON in,
/// one reply line per request out.
async fn serve_connection(socket: TcpStream, link: Arc<Link>) -> std::io::Result<()> {
    let (reader, mut writer): (OwnedReadHalf, OwnedWriteHalf) = socket.into_split();
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        if line.len() > MAX_LINE_BYTES {
            let reply = json!({ "ok": false, "error": "the control request is too long" });
            write_line(&mut writer, &reply).await?;
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        let reply = link.dispatch(&line, REQUEST_TIMEOUT).await;
        write_line(&mut writer, &reply).await?;
    }
    Ok(())
}

async fn write_line(writer: &mut OwnedWriteHalf, value: &Value) -> std::io::Result<()> {
    let mut line = value.to_string();
    line.push('\n');
    writer.write_all(line.as_bytes()).await?;
    writer.flush().await
}

// ── HTTP side: the page attaches and answers ────────────────────────────────

#[derive(Deserialize)]
pub struct EventsQuery {
    pub app: String,
}

/// `GET /api/plugins/craft/control/events?app=<id>` — the page's side of the
/// link (SSE). Each message is one vendor request; the page answers it through
/// [`reply`].
pub async fn events(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
    Query(query): Query<EventsQuery>,
) -> Result<
    Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>,
    AppError,
> {
    let app = query.app.trim().to_string();
    if crate::plugins::store::CraftApp::by_id(&app).is_none() {
        return Err(AppError::BadRequest(format!("unknown Craft app '{app}'")));
    }
    let link = state.craft_control.link(&app).await?;
    let (page, receiver) = link.attach();
    let guard = PageGuard { link: link.clone(), page };
    let stream = futures::stream::unfold((receiver, guard), |(mut receiver, guard)| async move {
        match receiver.recv().await {
            Some(message) => Some((
                Ok(Event::default().data(message)),
                (receiver, guard),
            )),
            None => None,
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// Removes the page from the link when its stream ends (tab closed, window
/// closed, reload).
struct PageGuard {
    link: Arc<Link>,
    page: u64,
}

impl Drop for PageGuard {
    fn drop(&mut self) {
        self.link.detach(self.page);
    }
}

#[derive(Deserialize)]
pub struct ReplyBody {
    pub app: String,
    pub id: u64,
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

/// `POST /api/plugins/craft/control/reply` — the page's answer to one request.
pub async fn reply(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
    Json(body): Json<ReplyBody>,
) -> Result<impl IntoResponse, AppError> {
    if crate::plugins::store::CraftApp::by_id(&body.app).is_none() {
        return Err(AppError::BadRequest(format!(
            "unknown Craft app '{}'",
            body.app
        )));
    }
    let link = state.craft_control.link(&body.app).await?;
    link.complete(body.id, body.ok, body.result, body.error);
    Ok(Json(json!({ "success": true })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    /// The gate: with no page attached the link answers the vendor's protocol
    /// with a clear refusal instead of hanging the agent's turn.
    #[tokio::test]
    async fn without_a_page_the_link_refuses_instead_of_hanging() {
        let control = CraftControl::new();
        let link = control.link("vectorcraft").await.expect("link");
        assert!(!link.has_page());
        assert_eq!(control.live_port("vectorcraft").await, None);

        let mut socket = TcpStream::connect(("127.0.0.1", link.port))
            .await
            .expect("connect");
        socket
            .write_all(
                b"{\"id\":7,\"method\":\"engine.execute\",\"params\":{\"command\":\"select.all\"}}\n",
            )
            .await
            .expect("write");
        let mut buffer = [0u8; 512];
        let count = socket.read(&mut buffer).await.expect("read");
        let reply: Value = serde_json::from_slice(&buffer[..count]).expect("json");
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["ok"], false);
        assert!(
            reply["error"].as_str().unwrap().contains("no window"),
            "{reply}"
        );
    }

    /// The live path: a request reaches the page verbatim, and the page's answer
    /// goes back on the vendor's wire with the same id.
    #[tokio::test]
    async fn a_request_reaches_the_page_and_its_answer_goes_back() {
        let control = CraftControl::new();
        let link = control.link("photocraft").await.expect("link");
        let mut page = link.attach().1;
        assert!(link.has_page());
        assert!(control.live_port("photocraft").await.is_some());

        let port = link.port;
        let request = tokio::spawn(async move {
            let mut socket = TcpStream::connect(("127.0.0.1", port)).await.expect("connect");
            socket
                .write_all(b"{\"id\":42,\"method\":\"ui.inspect\",\"params\":{}}\n")
                .await
                .expect("write");
            let mut buffer = [0u8; 512];
            let count = socket.read(&mut buffer).await.expect("read");
            serde_json::from_slice::<Value>(&buffer[..count]).expect("json")
        });

        let forwarded: Value = serde_json::from_str(&page.recv().await.expect("page request"))
            .expect("request json");
        assert_eq!(forwarded["id"], 42);
        assert_eq!(forwarded["method"], "ui.inspect");
        link.complete(42, true, Some(json!({ "tool": "ellipse" })), None);

        let reply = request.await.expect("reply");
        assert_eq!(reply["id"], 42);
        assert_eq!(reply["ok"], true);
        assert_eq!(reply["result"]["tool"], "ellipse");
    }

    /// The vendor's channel answers errors the same way it answers results.
    #[tokio::test]
    async fn a_page_error_comes_back_as_an_error_line() {
        let control = CraftControl::new();
        let link = control.link("wordcraft").await.expect("link");
        let mut page = link.attach().1;
        let port = link.port;
        let request = tokio::spawn(async move {
            let mut socket = TcpStream::connect(("127.0.0.1", port)).await.expect("connect");
            socket
                .write_all(b"{\"id\":1,\"method\":\"engine.execute\",\"params\":{}}\n")
                .await
                .expect("write");
            let mut buffer = [0u8; 512];
            let count = socket.read(&mut buffer).await.expect("read");
            serde_json::from_slice::<Value>(&buffer[..count]).expect("json")
        });
        let _ = page.recv().await.expect("page request");
        link.complete(1, false, None, Some("nothing selected".into()));
        let reply = request.await.expect("reply");
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["error"], "nothing selected");
    }
}
