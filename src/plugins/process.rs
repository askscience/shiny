//! Plugins that run as their own process (`runtime = "process"`).
//!
//! A library plugin shares the server's address space and its Rust ABI: it
//! only works with the exact build it was compiled with, and a crash in it is a
//! crash of the server. A process plugin is an executable the server starts and
//! talks to over a private Unix socket with the versioned
//! `shiny-plugin-protocol`, so a plugin built for an earlier release keeps
//! working and a crash costs only that plugin.
//!
//! [`ProcessPlugin`] implements the same `Plugin` trait as a loaded library,
//! which keeps the registry, the router, activation and uninstall unchanged:
//! its tools are [`RemoteTool`]s that call the process, and its route handlers
//! forward the HTTP request to it.
//!
//! The [`Supervisor`] owns the process: it starts it on demand, restarts it
//! with a backoff after a crash (and gives up after repeated ones), stops it
//! when it has been idle for the time the plugin asked for, and writes its
//! output to the server log.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::Body;
use axum::response::{IntoResponse, Response};
use http_body_util::BodyExt;
use parking_lot::Mutex;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use shiny_plugin_protocol as proto;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::manifest::Manifest;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::plugin::Plugin;
use shiny_plugin_sdk::routes::{HttpMethod, RouteHandler, RouteSpec};
use shiny_plugin_sdk::services::{ConfigSnapshot, PluginCtx};
use shiny_plugin_sdk::tools::{RegistryBuilder, Tool, ToolRequest, UiText};

/// Where a process plugin's executable lives inside its directory.
pub fn executable_path(install_dir: &Path, name: &str) -> PathBuf {
    install_dir.join("bin").join(name)
}

/// The plugin's executable, when the directory has a usable one. Symlinks are
/// refused for the same reason `find_cdylib` refuses them: an archive must not
/// point the server at a file outside its own tree.
pub fn find_executable(install_dir: &Path, name: &str) -> Option<PathBuf> {
    let path = executable_path(install_dir, name);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    meta.file_type().is_file().then_some(path)
}

/// Make the executable runnable (a zip archive carries no mode bits).
pub fn ensure_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)?.permissions().mode();
    if mode & 0o111 != 0o111 {
        std::fs::set_permissions(
            path,
            std::fs::Permissions::from_mode((mode | 0o755) & 0o7777),
        )?;
    }
    Ok(())
}

/// Plugin processes started since the server began (restarts included).
static STARTED: AtomicU64 = AtomicU64::new(0);

/// How many plugin processes this server has started, for diagnostics.
pub fn started_processes() -> u64 {
    STARTED.load(Ordering::SeqCst)
}

const HELLO_TIMEOUT: Duration = Duration::from_secs(20);
const HOOK_TIMEOUT: Duration = Duration::from_secs(120);
const TOOL_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const UNLOAD_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_GRACE: Duration = Duration::from_secs(3);
/// More crashes than this inside [`CRASH_WINDOW`] and the plugin stays down.
const MAX_CRASHES: usize = 3;
const CRASH_WINDOW: Duration = Duration::from_secs(60);
const MAX_REPLY_BYTES: usize = 64 * 1024 * 1024;

// ─────────────────────────────────────────────────────────────
// Supervisor
// ─────────────────────────────────────────────────────────────

/// One running instance of the executable.
struct Running {
    pid: u32,
    socket: PathBuf,
    generation: u64,
    exited: tokio::sync::watch::Receiver<bool>,
}

impl Running {
    fn alive(&self) -> bool {
        !*self.exited.borrow()
    }
}

/// Starts, watches and stops one plugin's process.
pub struct Supervisor {
    name: String,
    binary: PathBuf,
    install_dir: PathBuf,
    config: Value,
    running: tokio::sync::Mutex<Option<Running>>,
    generation: AtomicU64,
    /// The generation a deliberate stop is waiting for; its exit is no crash.
    stopping: AtomicU64,
    /// `on_load` ran: a restarted process gets it again.
    loaded: AtomicBool,
    /// Uninstalled or replaced: never start again.
    retired: AtomicBool,
    /// Crashed too often: stays down until the plugin is loaded again.
    failed: AtomicBool,
    crashes: Mutex<VecDeque<Instant>>,
    in_flight: AtomicUsize,
    last_used: Mutex<Instant>,
    idle_stop: Mutex<Option<Duration>>,
    self_ref: Weak<Supervisor>,
}

/// Counts a request as in flight until dropped (the body of a streamed
/// response holds one, so an idle stop never cuts a download).
struct InFlight(Arc<Supervisor>);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
        *self.0.last_used.lock() = Instant::now();
    }
}

impl Supervisor {
    fn new(name: &str, binary: PathBuf, install_dir: PathBuf, config: Value) -> Arc<Self> {
        Arc::new_cyclic(|weak| Self {
            name: name.to_string(),
            binary,
            install_dir,
            config,
            running: tokio::sync::Mutex::new(None),
            generation: AtomicU64::new(0),
            stopping: AtomicU64::new(0),
            loaded: AtomicBool::new(false),
            retired: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            crashes: Mutex::new(VecDeque::new()),
            in_flight: AtomicUsize::new(0),
            last_used: Mutex::new(Instant::now()),
            idle_stop: Mutex::new(None),
            self_ref: weak.clone(),
        })
    }

    fn arc(&self) -> Arc<Supervisor> {
        self.self_ref
            .upgrade()
            .expect("supervisor alive while in use")
    }

    fn begin(&self) -> InFlight {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        *self.last_used.lock() = Instant::now();
        InFlight(self.arc())
    }

    /// The process id while it runs.
    pub fn pid(&self) -> Option<u32> {
        self.running
            .try_lock()
            .ok()
            .and_then(|r| r.as_ref().filter(|r| r.alive()).map(|r| r.pid))
    }

    /// Is the process up right now (for status displays)?
    pub fn is_running(&self) -> bool {
        self.running
            .try_lock()
            .map(|r| r.as_ref().is_some_and(Running::alive))
            .unwrap_or(true)
    }

    /// The socket of a live process, starting one when there is none.
    async fn socket(&self) -> Result<PathBuf, AppError> {
        let mut running = self.running.lock().await;
        if let Some(r) = running.as_ref().filter(|r| r.alive()) {
            return Ok(r.socket.clone());
        }
        if self.retired.load(Ordering::SeqCst) {
            return Err(AppError::Internal(format!(
                "plugin '{}' is not installed",
                self.name
            )));
        }
        if self.failed.load(Ordering::SeqCst) {
            return Err(AppError::Internal(format!(
                "plugin '{}' stopped after crashing repeatedly; reinstall or update it",
                self.name
            )));
        }
        let (started, welcome) = self.start().await?;
        let socket = started.socket.clone();
        *running = Some(started);
        drop(running);
        if self.loaded.load(Ordering::SeqCst) {
            // A restart: the new process starts its background work too.
            let done: proto::Done = self
                .call_at(&socket, proto::paths::LOAD, &(), HOOK_TIMEOUT)
                .await?;
            if let Some(error) = done.error {
                tracing::warn!("plugin '{}' on_load after restart: {error}", self.name);
            }
        }
        let _ = welcome;
        Ok(socket)
    }

    /// Spawn the executable, wait for its socket and say hello.
    async fn start(&self) -> Result<(Running, proto::Welcome), AppError> {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let socket = socket_path(&self.name, generation)?;
        let _ = std::fs::remove_file(&socket);

        let mut command = tokio::process::Command::new(&self.binary);
        // The working directory stays the server's: paths a library plugin
        // resolved relative to it (`data/mail.key`) must keep their meaning.
        command
            .env(proto::SOCKET_ENV, &socket)
            .env(proto::NAME_ENV, &self.name)
            .env(proto::DIR_ENV, &self.install_dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        command.env(proto::HOST_EXE_ENV, crate::application::reexec::exe_path());
        #[cfg(target_os = "linux")]
        unsafe {
            // The plugin must not outlive the server, even after a SIGKILL.
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        let mut child = command.spawn().map_err(|e| {
            AppError::Internal(format!(
                "could not start plugin '{}' ({}): {e}",
                self.name,
                self.binary.display()
            ))
        })?;
        STARTED.fetch_add(1, Ordering::SeqCst);
        let pid = child.id().unwrap_or(0);
        if let Some(out) = child.stdout.take() {
            forward_output(self.name.clone(), out);
        }
        if let Some(err) = child.stderr.take() {
            forward_output(self.name.clone(), err);
        }

        let (exit_tx, exited) = tokio::sync::watch::channel(false);
        let weak = self.self_ref.clone();
        let socket_for_exit = socket.clone();
        tokio::spawn(async move {
            let status = child.wait().await;
            let _ = exit_tx.send(true);
            let _ = std::fs::remove_file(&socket_for_exit);
            if let Some(supervisor) = weak.upgrade() {
                supervisor.exited(generation, status.ok());
            }
        });

        let running = Running {
            pid,
            socket: socket.clone(),
            generation,
            exited,
        };
        let welcome = match self.handshake(&running).await {
            Ok(welcome) => welcome,
            Err(error) => {
                self.stopping.store(generation, Ordering::SeqCst);
                kill(pid, libc::SIGKILL);
                return Err(error);
            }
        };
        *self.idle_stop.lock() = welcome.idle_stop_secs.map(Duration::from_secs);
        tracing::info!(
            "plugin '{}' process started (pid {pid}, protocol {}, sdk {})",
            self.name,
            welcome.protocol,
            welcome.sdk
        );
        Ok((running, welcome))
    }

    async fn handshake(&self, running: &Running) -> Result<proto::Welcome, AppError> {
        // The plugin binds its socket once it is up. Poll until it accepts,
        // the process dies, or the deadline passes.
        let deadline = Instant::now() + HELLO_TIMEOUT;
        // A plugin is usually listening within a few milliseconds: look again
        // quickly at first, then at the old 15 ms pace.
        let mut pause = Duration::from_millis(1);
        loop {
            if !running.alive() {
                return Err(AppError::Internal(format!(
                    "plugin '{}' exited during start-up (see the server log)",
                    self.name
                )));
            }
            if tokio::net::UnixStream::connect(&running.socket)
                .await
                .is_ok()
            {
                break;
            }
            if Instant::now() > deadline {
                return Err(AppError::Internal(format!(
                    "plugin '{}' did not open its socket in time",
                    self.name
                )));
            }
            tokio::time::sleep(pause).await;
            pause = (pause * 2).min(Duration::from_millis(15));
        }
        let hello = proto::Hello {
            protocol: proto::VersionRange::current(),
            host_version: env!("CARGO_PKG_VERSION").into(),
            config: self.config.clone(),
        };
        let welcome: proto::Welcome = self
            .call_at(&running.socket, proto::paths::HELLO, &hello, HELLO_TIMEOUT)
            .await?;
        if let Some(error) = &welcome.error {
            return Err(AppError::BadRequest(format!(
                "plugin '{}' refused the handshake: {error}",
                self.name
            )));
        }
        if welcome.protocol < proto::PROTOCOL_MIN || welcome.protocol > proto::PROTOCOL_MAX {
            return Err(AppError::BadRequest(format!(
                "plugin '{}' chose protocol {} outside {}..={}",
                self.name,
                welcome.protocol,
                proto::PROTOCOL_MIN,
                proto::PROTOCOL_MAX
            )));
        }
        let declared = welcome
            .manifest
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if declared != self.name {
            return Err(AppError::BadRequest(format!(
                "plugin manifest name mismatch: {} on disk, {declared} in the executable",
                self.name
            )));
        }
        Ok(welcome)
    }

    /// The process ended. A deliberate stop is fine; anything else is a crash.
    fn exited(&self, generation: u64, status: Option<std::process::ExitStatus>) {
        if self.stopping.load(Ordering::SeqCst) == generation || self.retired.load(Ordering::SeqCst)
        {
            tracing::info!("plugin '{}' process stopped", self.name);
            return;
        }
        let crashes = {
            let mut crashes = self.crashes.lock();
            let now = Instant::now();
            crashes.push_back(now);
            while crashes
                .front()
                .is_some_and(|t| now.duration_since(*t) > CRASH_WINDOW)
            {
                crashes.pop_front();
            }
            crashes.len()
        };
        let status = status
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown status".into());
        if crashes > MAX_CRASHES {
            self.failed.store(true, Ordering::SeqCst);
            tracing::error!(
                "plugin '{}' crashed {crashes} times in a minute ({status}); leaving it stopped",
                self.name
            );
            return;
        }
        tracing::warn!(
            "plugin '{}' process exited unexpectedly ({status})",
            self.name
        );
        // Background work (no idle stop) comes back on its own; an on-demand
        // plugin restarts with the next request.
        if self.loaded.load(Ordering::SeqCst) && self.idle_stop.lock().is_none() {
            let weak = self.self_ref.clone();
            let delay = Duration::from_millis(500 * (1 << crashes.min(4)) as u64);
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                if let Some(supervisor) = weak.upgrade() {
                    if let Err(error) = supervisor.socket().await {
                        tracing::warn!("plugin '{}' restart failed: {error}", supervisor.name);
                    }
                }
            });
        }
    }

    /// Stop the process, politely first.
    async fn stop(&self) {
        let mut running = self.running.lock().await;
        let Some(r) = running.take() else { return };
        if !r.alive() {
            return;
        }
        self.stopping.store(r.generation, Ordering::SeqCst);
        kill(r.pid, libc::SIGTERM);
        let mut exited = r.exited.clone();
        let gone = tokio::time::timeout(STOP_GRACE, exited.wait_for(|e| *e)).await;
        if gone.is_err() {
            kill(r.pid, libc::SIGKILL);
        }
    }

    /// Flush and stop for good: the server is exiting or re-executing.
    pub async fn shutdown(&self) {
        let socket = self
            .running
            .lock()
            .await
            .as_ref()
            .filter(|r| r.alive())
            .map(|r| r.socket.clone());
        if let Some(socket) = socket {
            let _: Result<proto::Done, _> = self
                .call_at(&socket, proto::paths::UNLOAD, &(), UNLOAD_TIMEOUT)
                .await;
        }
        self.retired.store(true, Ordering::SeqCst);
        self.stop().await;
    }

    /// Stop the process when idle for the time the plugin asked for.
    fn watch_idle(&self) {
        let weak = self.self_ref.clone();
        tokio::spawn(async move {
            loop {
                let Some(supervisor) = weak.upgrade() else {
                    return;
                };
                if supervisor.retired.load(Ordering::SeqCst) {
                    return;
                }
                let idle_stop = *supervisor.idle_stop.lock();
                let tick = idle_stop
                    .map(|d| (d / 2).clamp(Duration::from_secs(1), Duration::from_secs(30)))
                    .unwrap_or(Duration::from_secs(30));
                if let Some(limit) = idle_stop {
                    let idle = supervisor.last_used.lock().elapsed();
                    if idle >= limit
                        && supervisor.in_flight.load(Ordering::SeqCst) == 0
                        && supervisor.is_running()
                    {
                        tracing::info!(
                            "plugin '{}' idle for {}s, stopping its process",
                            supervisor.name,
                            idle.as_secs()
                        );
                        supervisor.stop().await;
                    }
                }
                drop(supervisor);
                tokio::time::sleep(tick).await;
            }
        });
    }

    /// JSON control call, starting the process when needed.
    async fn call<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        path: &str,
        body: &Req,
        timeout: Duration,
    ) -> Result<Resp, AppError> {
        let _flight = self.begin();
        let socket = self.socket().await?;
        self.call_at(&socket, path, body, timeout).await
    }

    async fn call_at<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        socket: &Path,
        path: &str,
        body: &Req,
        timeout: Duration,
    ) -> Result<Resp, AppError> {
        let json = serde_json::to_vec(body).map_err(|e| AppError::Internal(e.to_string()))?;
        let request = http::Request::post(path)
            .header(http::header::HOST, "plugin")
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(json))
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let name = &self.name;
        let exchange = async {
            let response = send(socket, request).await?;
            let status = response.status();
            let bytes = http_body_util::Limited::new(response.into_body(), MAX_REPLY_BYTES)
                .collect()
                .await
                .map_err(|e| AppError::Internal(format!("plugin '{name}' reply: {e}")))?
                .to_bytes();
            if !status.is_success() {
                return Err(AppError::Internal(format!(
                    "plugin '{name}' answered {status} to {path}: {}",
                    String::from_utf8_lossy(&bytes[..bytes.len().min(300)])
                )));
            }
            serde_json::from_slice(&bytes).map_err(|e| {
                AppError::Internal(format!(
                    "plugin '{name}' sent an unreadable reply to {path}: {e}"
                ))
            })
        };
        tokio::time::timeout(timeout, exchange).await.map_err(|_| {
            AppError::Internal(format!("plugin '{name}' did not answer {path} in time"))
        })?
    }

    /// Forward one user request to the process.
    async fn forward(&self, tag: &str, mut request: axum::extract::Request) -> Response {
        let flight = self.begin();
        let socket = match self.socket().await {
            Ok(socket) => socket,
            Err(error) => return unavailable(error),
        };
        match http::HeaderValue::from_str(tag) {
            Ok(value) => {
                request.headers_mut().insert(proto::ROUTE_TAG_HEADER, value);
            }
            Err(_) => return AppError::Internal("bad route tag".into()).into_response(),
        }
        match send(&socket, request).await {
            Ok(response) => {
                let (parts, body) = response.into_parts();
                // The in-flight mark lives as long as the body is streamed.
                let body = body.map_frame(move |frame| {
                    let _ = &flight;
                    frame
                });
                Response::from_parts(parts, Body::new(body))
            }
            Err(error) => unavailable(error),
        }
    }
}

fn unavailable(error: AppError) -> Response {
    let message = match &error {
        AppError::Internal(m)
        | AppError::BadRequest(m)
        | AppError::NotFound(m)
        | AppError::Unauthorized(m) => m.clone(),
        other => other.to_string(),
    };
    (
        http::StatusCode::SERVICE_UNAVAILABLE,
        axum::Json(serde_json::json!({ "success": false, "data": null, "error": message })),
    )
        .into_response()
}

/// One HTTP/1.1 exchange over a fresh connection to the plugin's socket.
async fn send(
    socket: &Path,
    request: http::Request<Body>,
) -> Result<http::Response<hyper::body::Incoming>, AppError> {
    let stream = tokio::net::UnixStream::connect(socket)
        .await
        .map_err(|e| AppError::Internal(format!("plugin socket {}: {e}", socket.display())))?;
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream))
            .await
            .map_err(|e| AppError::Internal(format!("plugin connection: {e}")))?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            tracing::debug!("plugin connection closed: {error}");
        }
    });
    sender
        .send_request(request)
        .await
        .map_err(|e| AppError::Internal(format!("plugin request: {e}")))
}

fn kill(pid: u32, signal: libc::c_int) {
    if pid > 0 {
        unsafe {
            libc::kill(pid as libc::pid_t, signal);
        }
    }
}

/// Write each line the plugin prints to the server log, tagged with its name.
fn forward_output<R: tokio::io::AsyncRead + Unpin + Send + 'static>(name: String, stream: R) {
    use tokio::io::AsyncBufReadExt;
    tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let line: String = line.chars().take(2000).collect();
            tracing::info!(target: "shiny::plugin", "[{name}] {line}");
        }
    });
}

/// A private directory for the sockets: the user's runtime dir when there is
/// one, else a 0700 directory of our own under the temp dir.
fn socket_dir() -> Result<PathBuf, AppError> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    let uid = unsafe { libc::getuid() };
    let dir = match std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        Some(runtime) => PathBuf::from(runtime).join("shiny-plugins"),
        None => std::env::temp_dir().join(format!("shiny-plugins-{uid}")),
    };
    if !dir.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .map_err(|e| AppError::Internal(format!("plugin socket dir {}: {e}", dir.display())))?;
    }
    let meta = std::fs::symlink_metadata(&dir).map_err(|e| AppError::Internal(e.to_string()))?;
    if !meta.is_dir() || meta.uid() != uid {
        return Err(AppError::Internal(format!(
            "plugin socket dir {} is not ours",
            dir.display()
        )));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(dir)
}

fn socket_path(name: &str, generation: u64) -> Result<PathBuf, AppError> {
    // sun_path holds 108 bytes; keep the name part short. The random part
    // keeps a re-executed server (same pid, generations from 1 again) from
    // ever reusing a path.
    let short: String = name.chars().take(40).collect();
    let nonce = &uuid::Uuid::new_v4().simple().to_string()[..8];
    Ok(socket_dir()?.join(format!("{short}-{generation}-{nonce}.sock")))
}

// ─────────────────────────────────────────────────────────────
// The plugin and its tools
// ─────────────────────────────────────────────────────────────

/// A process plugin, seen by the rest of the server as any other plugin.
pub struct ProcessPlugin {
    manifest: Manifest,
    welcome: proto::Welcome,
    supervisor: Arc<Supervisor>,
}

impl ProcessPlugin {
    /// Start the executable in `install_dir` and learn what it contributes.
    pub async fn start(
        install_dir: &Path,
        manifest: Manifest,
        config: &ConfigSnapshot,
    ) -> Result<Self, AppError> {
        let binary = find_executable(install_dir, &manifest.name).ok_or_else(|| {
            AppError::BadRequest(format!(
                "No executable at bin/{} for process plugin '{}'",
                manifest.name, manifest.name
            ))
        })?;
        let _ = ensure_executable(&binary);
        let config = serde_json::to_value(config).map_err(|e| AppError::Internal(e.to_string()))?;
        let supervisor = Supervisor::new(&manifest.name, binary, install_dir.to_path_buf(), config);
        let welcome = {
            let mut running = supervisor.running.lock().await;
            let (started, welcome) = supervisor.start().await?;
            *running = Some(started);
            welcome
        };
        supervisor.watch_idle();
        Ok(Self {
            manifest,
            welcome,
            supervisor,
        })
    }

    pub fn supervisor(&self) -> &Arc<Supervisor> {
        &self.supervisor
    }
}

impl Drop for ProcessPlugin {
    fn drop(&mut self) {
        // The last handle is gone: make sure the process does not linger.
        self.supervisor.retired.store(true, Ordering::SeqCst);
        if let Ok(mut running) = self.supervisor.running.try_lock() {
            if let Some(r) = running.take() {
                self.supervisor
                    .stopping
                    .store(r.generation, Ordering::SeqCst);
                kill(r.pid, libc::SIGTERM);
            }
        }
    }
}

fn parse_method(method: &str) -> Option<HttpMethod> {
    Some(match method.to_ascii_uppercase().as_str() {
        "GET" => HttpMethod::Get,
        "POST" => HttpMethod::Post,
        "PUT" => HttpMethod::Put,
        "DELETE" => HttpMethod::Delete,
        "PATCH" => HttpMethod::Patch,
        _ => return None,
    })
}

#[async_trait]
impl Plugin for ProcessPlugin {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, _ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        for spec in &self.welcome.tools {
            builder.tool_arc(Arc::new(RemoteTool::new(
                spec.clone(),
                self.supervisor.clone(),
            )));
        }
        for route in &self.welcome.routes {
            let Some(method) = parse_method(&route.method) else {
                tracing::warn!(
                    "plugin '{}': route {} has unknown method {}",
                    self.manifest.name,
                    route.path,
                    route.method
                );
                continue;
            };
            builder.route(RouteSpec {
                method,
                path: route.path.clone(),
                auth: route.auth.clone(),
                handler_tag: route.handler_tag.clone(),
            });
        }
        builder.skills(self.welcome.skills_md.clone());
        builder.persona(self.welcome.persona.clone());
        for line in &self.welcome.context_lines {
            builder.context_line(line.clone());
        }
    }

    async fn on_load(&self, _ctx: Arc<PluginCtx>) {
        self.supervisor.loaded.store(true, Ordering::SeqCst);
        match self
            .supervisor
            .call::<_, proto::Done>(proto::paths::LOAD, &(), HOOK_TIMEOUT)
            .await
        {
            Ok(proto::Done { error: Some(error) }) | Err(AppError::Internal(error)) => {
                tracing::warn!("plugin '{}' on_load: {error}", self.manifest.name)
            }
            Err(error) => tracing::warn!("plugin '{}' on_load: {error}", self.manifest.name),
            Ok(_) => {}
        }
    }

    async fn on_unload(&self, _ctx: Arc<PluginCtx>) {
        let supervisor = &self.supervisor;
        // Only a live process has state to flush; never start one to stop it.
        let socket = supervisor
            .running
            .lock()
            .await
            .as_ref()
            .filter(|r| r.alive())
            .map(|r| r.socket.clone());
        if let Some(socket) = socket {
            let flushed: Result<proto::Done, _> = supervisor
                .call_at(&socket, proto::paths::UNLOAD, &(), UNLOAD_TIMEOUT)
                .await;
            if let Err(error) = flushed {
                tracing::warn!("plugin '{}' on_unload: {error}", self.manifest.name);
            }
        }
        supervisor.retired.store(true, Ordering::SeqCst);
        supervisor.stop().await;
    }

    async fn on_user_registered(&self, _ctx: Arc<PluginCtx>, user_id: &str) {
        let body = proto::UserRegistered {
            user_id: user_id.to_string(),
        };
        match self
            .supervisor
            .call::<_, proto::Done>(proto::paths::USER_REGISTERED, &body, HOOK_TIMEOUT)
            .await
        {
            Ok(proto::Done { error: Some(error) }) => {
                tracing::warn!(
                    "plugin '{}' on_user_registered: {error}",
                    self.manifest.name
                )
            }
            Err(error) => tracing::warn!(
                "plugin '{}' on_user_registered: {error}",
                self.manifest.name
            ),
            Ok(_) => {}
        }
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        if !self.welcome.routes.iter().any(|r| r.handler_tag == tag) {
            return None;
        }
        let supervisor = self.supervisor.clone();
        let tag = tag.to_string();
        Some(Arc::new(move |request| {
            let supervisor = supervisor.clone();
            let tag = tag.clone();
            Box::pin(async move { supervisor.forward(&tag, request).await })
        }))
    }
}

/// An agent tool that runs inside the plugin's process.
pub struct RemoteTool {
    spec: proto::ToolSpec,
    /// `Tool::aliases` hands out `&[&str]`; the strings live as long as the
    /// process (one small leak per plugin load, bounded by installs).
    aliases: Vec<&'static str>,
    supervisor: Arc<Supervisor>,
    /// `humanize` and `done_ui` are synchronous, so the process computes them
    /// with the outcome and they wait here until the host asks.
    recent: Mutex<VecDeque<(u64, String, Option<UiText>)>>,
}

impl RemoteTool {
    fn new(spec: proto::ToolSpec, supervisor: Arc<Supervisor>) -> Self {
        let aliases = spec
            .aliases
            .iter()
            .map(|a| &*Box::leak(a.clone().into_boxed_str()))
            .collect();
        Self {
            spec,
            aliases,
            supervisor,
            recent: Mutex::new(VecDeque::new()),
        }
    }

    fn key(result: &str, data: &Value) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        result.hash(&mut hasher);
        data.to_string().hash(&mut hasher);
        hasher.finish()
    }

    fn remembered(&self, result: &str, data: &Value) -> Option<(String, Option<UiText>)> {
        let key = Self::key(result, data);
        self.recent
            .lock()
            .iter()
            .rev()
            .find(|(k, _, _)| *k == key)
            .map(|(_, h, d)| (h.clone(), d.clone()))
    }
}

fn ui_from_wire(ui: proto::UiText) -> UiText {
    UiText {
        key: ui.key,
        params: ui.params,
        text: ui.text,
    }
}

fn error_from_wire(error: proto::WireError) -> AppError {
    match error.kind.as_str() {
        "not_found" => AppError::NotFound(error.message),
        "unauthorized" => AppError::Unauthorized(error.message),
        "bad_request" => AppError::BadRequest(error.message),
        _ => AppError::Internal(error.message),
    }
}

#[async_trait]
impl Tool for RemoteTool {
    fn name(&self) -> &str {
        &self.spec.name
    }

    fn aliases(&self) -> &[&str] {
        &self.aliases
    }

    fn step_label(&self) -> &str {
        if self.spec.step_label.is_empty() {
            "Working…"
        } else {
            &self.spec.step_label
        }
    }

    fn humanize(&self, result: &str, data: &Value) -> String {
        self.remembered(result, data)
            .map(|(h, _)| h)
            .unwrap_or_else(|| format!("{} complete", self.spec.name))
    }

    fn doc_fragment(&self) -> Option<&str> {
        self.spec.doc_fragment.as_deref()
    }

    fn step_ui(&self) -> Option<UiText> {
        self.spec.step_ui.clone().map(ui_from_wire)
    }

    fn done_ui(&self, result: &str, data: &Value) -> Option<UiText> {
        self.remembered(result, data).and_then(|(_, d)| d)
    }

    async fn invoke(
        &self,
        _ctx: &PluginCtx,
        req: ToolRequest<'_>,
    ) -> Result<ActionOutcome, AppError> {
        let call = proto::ToolCall {
            tool: self.spec.name.clone(),
            user_id: req.user_id.to_string(),
            traveler_id: req.traveler_id.to_string(),
            params: req.params.clone(),
            context: serde_json::to_value(req.ctx).unwrap_or_default(),
            os_home: req.os_home.map(str::to_string),
        };
        let reply: proto::ToolReply = self
            .supervisor
            .call(proto::paths::TOOL, &call, TOOL_TIMEOUT)
            .await?;
        if let Some(error) = reply.error {
            return Err(error_from_wire(error));
        }
        let outcome = reply.outcome.ok_or_else(|| {
            AppError::Internal(format!("tool '{}' returned nothing", self.spec.name))
        })?;
        let parse = |v: Value| serde_json::from_value(v).ok();
        let action = ActionOutcome {
            action: outcome.action,
            result: outcome.result,
            data: outcome.data,
            artifact: outcome.artifact.and_then(parse),
            extra_artifacts: outcome
                .extra_artifacts
                .into_iter()
                .filter_map(parse)
                .collect(),
            navigation: outcome
                .navigation
                .and_then(|v| serde_json::from_value(v).ok()),
        };
        if let Some(humanized) = reply.humanized {
            let mut recent = self.recent.lock();
            recent.push_back((
                Self::key(&action.result, &action.data),
                humanized,
                reply.done_ui.map(ui_from_wire),
            ));
            while recent.len() > 32 {
                recent.pop_front();
            }
        }
        Ok(action)
    }
}
