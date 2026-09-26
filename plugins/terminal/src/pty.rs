//! PTY-backed shell sessions for the Terminal window.
//!
//! Each session owns a real pseudo-terminal running the host's login shell.
//! Output is kept in a bounded scrollback and fanned out to any number of SSE
//! subscribers, so a reconnecting window replays what it missed instead of
//! losing the session.
//!
//! Nothing here needs Tokio: the PTY reader runs on a plain OS thread and
//! publishes into an executor-agnostic `futures::channel::mpsc` queue, which
//! the core's runtime may poll safely (PLUGINS.md §15 — no plugin runtime
//! values cross the dlopen boundary).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use base64::Engine as _;
use futures::channel::mpsc;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use uuid::Uuid;

use shiny_plugin_sdk::errors::AppError;

/// Raw output kept per session so a reconnecting client can catch up.
const SCROLLBACK_BYTES: usize = 256 * 1024;

/// How often an idle session emits an SSE comment-ish ping.
const PING_INTERVAL: Duration = Duration::from_secs(20);

fn base64_std() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// One message on a session's output channel.
#[derive(Clone, Debug)]
pub enum Event {
    Ready {
        session: String,
        shell: String,
        cwd: String,
        cols: u16,
        rows: u16,
    },
    /// Terminal output, base64-encoded so arbitrary bytes survive SSE.
    Out(String),
    Exit {
        code: Option<u32>,
    },
    Ping,
}

impl Event {
    /// Serialize as one SSE frame (`data: {json}\n\n`).
    pub fn to_sse(&self) -> String {
        let value = match self {
            Event::Ready { session, shell, cwd, cols, rows } => serde_json::json!({
                "type": "ready",
                "session": session,
                "shell": shell,
                "cwd": cwd,
                "cols": cols,
                "rows": rows,
            }),
            Event::Out(data) => serde_json::json!({ "type": "out", "data": data }),
            Event::Exit { code } => serde_json::json!({ "type": "exit", "code": code }),
            Event::Ping => serde_json::json!({ "type": "ping" }),
        };
        format!("data: {value}\n\n")
    }
}

/// State guarded by one lock so subscribing is atomic with the scrollback
/// snapshot: no output can slip between "send me history" and "send me live".
struct Inner {
    scrollback: Vec<u8>,
    subscribers: Vec<mpsc::UnboundedSender<Event>>,
    cols: u16,
    rows: u16,
}

pub struct Session {
    pub id: String,
    pub user_id: String,
    pub shell: String,
    pub cwd: String,
    inner: Mutex<Inner>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    alive: AtomicBool,
}

static SESSIONS: LazyLock<Mutex<HashMap<String, Arc<Session>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn get(id: &str) -> Option<Arc<Session>> {
    SESSIONS.lock().ok()?.get(id).cloned()
}

pub fn list_for(user_id: &str) -> Vec<Arc<Session>> {
    SESSIONS
        .lock()
        .map(|map| {
            map.values()
                .filter(|s| s.user_id == user_id)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/root".into())
}

fn shell_path() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| std::path::Path::new(s).is_file())
        .unwrap_or_else(|| "/bin/bash".into())
}

/// Spawn a new PTY session running the host's login shell.
pub fn create(user_id: &str, cols: u16, rows: u16) -> Result<Arc<Session>, AppError> {
    let cols = cols.clamp(2, 1000);
    let rows = rows.clamp(1, 500);
    let shell = shell_path();
    let cwd = home_dir();

    let pair = native_pty_system()
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| AppError::Internal(format!("openpty failed: {e}")))?;

    let mut cmd = CommandBuilder::new(&shell);
    cmd.arg("-l");
    cmd.cwd(&cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("LANG", std::env::var("LANG").unwrap_or_else(|_| "C.UTF-8".into()));

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| AppError::Internal(format!("spawn shell failed: {e}")))?;
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| AppError::Internal(format!("pty reader failed: {e}")))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| AppError::Internal(format!("pty writer failed: {e}")))?;

    let id = Uuid::new_v4().to_string();
    let session = Arc::new(Session {
        id: id.clone(),
        user_id: user_id.to_string(),
        shell: shell.clone(),
        cwd: cwd.clone(),
        inner: Mutex::new(Inner { scrollback: Vec::new(), subscribers: Vec::new(), cols, rows }),
        master: Mutex::new(pair.master),
        writer: Mutex::new(writer),
        child: Mutex::new(child),
        alive: AtomicBool::new(true),
    });

    // PTY reader: copy output into the scrollback and fan it out, then reap
    // the child and let every subscriber know the shell is gone.
    {
        let session = session.clone();
        let tag = short_tag(&id);
        std::thread::Builder::new()
            .name(format!("terminal-{tag}"))
            .spawn(move || {
                let mut reader = reader;
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => session.fanout(&buf[..n]),
                        Err(_) => break,
                    }
                }
                let code = session
                    .child
                    .lock()
                    .ok()
                    .and_then(|mut child| child.wait().ok())
                    .map(|status| status.exit_code());
                session.finish(code);
            })
            .map_err(|e| AppError::Internal(format!("spawn pty reader failed: {e}")))?;
    }

    // Keepalive so an idle SSE connection stays warm and disconnects are seen.
    {
        let session = session.clone();
        let tag = short_tag(&id);
        std::thread::Builder::new()
            .name(format!("terminal-ping-{tag}"))
            .spawn(move || loop {
                std::thread::sleep(PING_INTERVAL);
                if !session.is_alive() {
                    break;
                }
                session.broadcast(Event::Ping);
            })
            .ok();
    }

    SESSIONS
        .lock()
        .map_err(|_| AppError::Internal("session registry poisoned".into()))?
        .insert(id, session.clone());
    Ok(session)
}

fn short_tag(id: &str) -> String {
    id.chars().take(8).collect()
}

impl Session {
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    pub fn meta(&self) -> serde_json::Value {
        let (cols, rows) = self
            .inner
            .lock()
            .map(|inner| (inner.cols, inner.rows))
            .unwrap_or((80, 24));
        serde_json::json!({
            "id": self.id,
            "shell": self.shell,
            "cwd": self.cwd,
            "cols": cols,
            "rows": rows,
            "alive": self.is_alive(),
        })
    }

    pub fn ready_event(&self) -> Event {
        let (cols, rows) = self
            .inner
            .lock()
            .map(|inner| (inner.cols, inner.rows))
            .unwrap_or((80, 24));
        Event::Ready {
            session: self.id.clone(),
            shell: self.shell.clone(),
            cwd: self.cwd.clone(),
            cols,
            rows,
        }
    }

    /// Hand a subscriber the current scrollback and the live feed, atomically.
    /// Returns `(scrollback, receiver, alive)`.
    pub fn subscribe(&self) -> (Vec<u8>, mpsc::UnboundedReceiver<Event>, bool) {
        let (tx, rx) = mpsc::unbounded();
        let Ok(mut inner) = self.inner.lock() else {
            return (Vec::new(), rx, false);
        };
        let snapshot = inner.scrollback.clone();
        let alive = self.is_alive();
        if alive {
            inner.subscribers.push(tx);
        }
        (snapshot, rx, alive)
    }

    /// Append output to the scrollback and deliver it to every subscriber.
    fn fanout(&self, bytes: &[u8]) {
        let encoded = base64_std().encode(bytes);
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        inner.scrollback.extend_from_slice(bytes);
        let len = inner.scrollback.len();
        if len > SCROLLBACK_BYTES {
            inner.scrollback.drain(..len - SCROLLBACK_BYTES);
        }
        inner
            .subscribers
            .retain(|tx| tx.unbounded_send(Event::Out(encoded.clone())).is_ok());
    }

    fn broadcast(&self, event: Event) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        inner.subscribers.retain(|tx| tx.unbounded_send(event.clone()).is_ok());
    }

    pub fn write_input(&self, data: &[u8]) -> Result<(), AppError> {
        let Ok(mut writer) = self.writer.lock() else {
            return Err(AppError::Internal("pty writer poisoned".into()));
        };
        writer
            .write_all(data)
            .and_then(|_| writer.flush())
            .map_err(|e| AppError::Internal(format!("pty write failed: {e}")))
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), AppError> {
        let cols = cols.clamp(2, 1000);
        let rows = rows.clamp(1, 500);
        let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
        let Ok(master) = self.master.lock() else {
            return Err(AppError::Internal("pty master poisoned".into()));
        };
        master
            .resize(size)
            .map_err(|e| AppError::Internal(format!("pty resize failed: {e}")))?;
        if let Ok(mut inner) = self.inner.lock() {
            inner.cols = cols;
            inner.rows = rows;
        }
        Ok(())
    }

    /// Kill the shell (if still running) and drop the session.
    pub fn close(&self) -> Result<(), AppError> {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
        self.finish(None);
        Ok(())
    }

    /// Mark the session dead, wake every subscriber with the exit, and remove
    /// it from the registry. Idempotent.
    fn finish(&self, code: Option<u32>) {
        let was_alive = self.alive.swap(false, Ordering::SeqCst);
        if !was_alive {
            return;
        }
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        for tx in inner.subscribers.drain(..) {
            let _ = tx.unbounded_send(Event::Exit { code });
        }
        drop(inner);
        if let Ok(mut registry) = SESSIONS.lock() {
            registry.remove(&self.id);
        }
    }
}
