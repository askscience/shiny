//! Real Iroh remote-access service (cargo feature `iroh`).
//!
//! Binds an Iroh endpoint and transparently proxies every QUIC bidirectional
//! stream to the local axum server on `127.0.0.1:<port>` — one stream per
//! HTTP/1.1 connection, so cookies and Server-Sent Events pass through.
//!
//! Access control: a **paired-device allowlist**. While the list is empty the
//! ticket alone admits a client; pairing mode (started from the local UI) adds
//! the next connecting device's endpoint key. Once at least one device is
//! paired, every other key is rejected before any HTTP. Each `start` also mints
//! a **fresh identity**, so the ticket changes on every start/stop.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iroh::{endpoint::presets, Endpoint, EndpointAddr};
use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use crate::errors::AppError;

/// Application protocol id both ends must agree on.
pub const ALPN: &[u8] = b"shiny/http/1";

/// How long "pair a new device" stays open.
const PAIRING_WINDOW: Duration = Duration::from_secs(120);

/// Keep only what is needed to dial: the endpoint key plus its home relay, or
/// the direct addresses when there is no relay. This keeps the link short.
fn minimal_addr(addr: &EndpointAddr) -> EndpointAddr {
    let mut out = EndpointAddr::new(addr.id);
    match addr.relay_urls().next() {
        Some(relay) => out = out.with_relay_url(relay.clone()),
        None => out = out.with_addrs(addr.addrs.iter().cloned()),
    }
    out
}

/// Public snapshot of the remote-access state.
#[derive(Clone, Serialize)]
pub struct IrohStatus {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
    pub connections: u64,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    /// Paired devices (endpoint keys) allowed to connect.
    pub paired: usize,
    /// Whether pairing mode is open right now.
    pub pairing: bool,
}

impl IrohStatus {
    fn disabled(paired: usize, pairing: bool) -> Self {
        Self {
            enabled: false,
            endpoint_id: None,
            ticket: None,
            connections: 0,
            bytes: 0,
            started_at: None,
            paired,
            pairing,
        }
    }
}

/// One running endpoint and its counters.
struct Running {
    endpoint: Endpoint,
    endpoint_id: String,
    ticket: String,
    started_at: u64,
    connections: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
}

/// The service handle stored in `AppState`. Cheap to clone; all clones share one
/// endpoint and one allowlist.
#[derive(Clone)]
pub struct IrohRemote {
    inner: Arc<Mutex<Option<Running>>>,
    allowed: Arc<StdMutex<HashSet<String>>>,
    pairing_until: Arc<StdMutex<Option<Instant>>>,
}

impl IrohRemote {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            allowed: Arc::new(StdMutex::new(load_allowed())),
            pairing_until: Arc::new(StdMutex::new(None)),
        }
    }

    /// Start serving, if not already running. Idempotent.
    pub async fn start(&self, port: u16) -> Result<IrohStatus, AppError> {
        let mut guard = self.inner.lock().await;
        if let Some(running) = guard.as_ref() {
            return Ok(self.status_of(running));
        }

        let endpoint = Endpoint::builder(presets::N0)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .map_err(|e| AppError::Internal(format!("iroh bind: {e}")))?;

        // Wait briefly for a relay so the ticket is dialable off-LAN; a
        // restricted network still leaves the direct addresses.
        let _ = tokio::time::timeout(Duration::from_secs(10), endpoint.online()).await;

        let addr = endpoint.addr();
        let endpoint_id = endpoint.id().to_string();
        let ticket = shiny_iroh_proto::link(&minimal_addr(&addr));
        let connections = Arc::new(AtomicU64::new(0));
        let bytes = Arc::new(AtomicU64::new(0));

        spawn_accept(
            endpoint.clone(),
            port,
            connections.clone(),
            bytes.clone(),
            self.allowed.clone(),
            self.pairing_until.clone(),
        );

        let running = Running {
            endpoint,
            endpoint_id,
            ticket,
            started_at: now_secs(),
            connections,
            bytes,
        };
        let status = self.status_of(&running);
        tracing::info!("iroh: remote access enabled (endpoint {})", running.endpoint_id);
        *guard = Some(running);
        Ok(status)
    }

    /// Stop serving and drop the identity. Idempotent.
    pub async fn stop(&self) -> Result<(), AppError> {
        let mut guard = self.inner.lock().await;
        if let Some(running) = guard.take() {
            running.endpoint.close().await;
            tracing::info!("iroh: remote access disabled");
        }
        Ok(())
    }

    /// Stop and start again with a fresh identity (revokes the old ticket).
    pub async fn rotate(&self, port: u16) -> Result<IrohStatus, AppError> {
        self.stop().await?;
        self.start(port).await
    }

    /// Open a short window in which the next connecting device is paired.
    pub fn begin_pairing(&self) -> u64 {
        let mut until = self.pairing_until.lock().unwrap_or_else(|e| e.into_inner());
        *until = Some(Instant::now() + PAIRING_WINDOW);
        PAIRING_WINDOW.as_secs()
    }

    /// Forget every paired device (and close pairing).
    pub fn unpair_all(&self) -> usize {
        let mut allowed = self.allowed.lock().unwrap_or_else(|e| e.into_inner());
        let had = allowed.len();
        allowed.clear();
        save_allowed(&allowed);
        let mut until = self.pairing_until.lock().unwrap_or_else(|e| e.into_inner());
        *until = None;
        had
    }

    /// Current state.
    pub async fn status(&self) -> IrohStatus {
        let guard = self.inner.lock().await;
        match guard.as_ref() {
            Some(running) => self.status_of(running),
            None => IrohStatus::disabled(self.paired_count(), self.pairing_open()),
        }
    }

    fn status_of(&self, running: &Running) -> IrohStatus {
        IrohStatus {
            enabled: true,
            endpoint_id: Some(running.endpoint_id.clone()),
            ticket: Some(running.ticket.clone()),
            connections: running.connections.load(Ordering::Relaxed),
            bytes: running.bytes.load(Ordering::Relaxed),
            started_at: Some(running.started_at),
            paired: self.paired_count(),
            pairing: self.pairing_open(),
        }
    }

    fn paired_count(&self) -> usize {
        self.allowed
            .lock()
            .map(|a| a.len())
            .unwrap_or(0)
    }

    fn pairing_open(&self) -> bool {
        self.pairing_until
            .lock()
            .map(|u| u.map(|t| t > Instant::now()).unwrap_or(false))
            .unwrap_or(false)
    }
}

impl Default for IrohRemote {
    fn default() -> Self {
        Self::new()
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn paired_file() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SHINY_PAIRED_FILE") {
        return Some(PathBuf::from(p));
    }
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home).join(".local/share/shiny/paired_devices.json")
    })
}

fn load_allowed() -> HashSet<String> {
    paired_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .map(|list| list.into_iter().collect())
        .unwrap_or_default()
}

fn save_allowed(set: &HashSet<String>) {
    let Some(path) = paired_file() else { return };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let list: Vec<&String> = set.iter().collect();
    if let Ok(json) = serde_json::to_string(&list) {
        let _ = std::fs::write(path, json);
    }
}

fn spawn_accept(
    endpoint: Endpoint,
    port: u16,
    connections: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
    allowed: Arc<StdMutex<HashSet<String>>>,
    pairing_until: Arc<StdMutex<Option<Instant>>>,
) {
    tokio::spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            let conn = match incoming.await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::warn!("iroh: incoming connection failed: {e}");
                    continue;
                }
            };

            // Access control: pairing adds the next key; otherwise an unpaired
            // key is refused before any HTTP.
            let remote = Some(conn.remote_id().to_string());
            if let Some(remote) = &remote {
                let pairing = {
                    let mut until = pairing_until.lock().unwrap_or_else(|e| e.into_inner());
                    match *until {
                        Some(t) if t > Instant::now() => {
                            *until = None;
                            true
                        }
                        _ => false,
                    }
                };
                if pairing {
                    let mut set = allowed.lock().unwrap_or_else(|e| e.into_inner());
                    if set.insert(remote.clone()) {
                        save_allowed(&set);
                        tracing::info!("iroh: paired a new device ({remote})");
                    }
                } else {
                    let set = allowed.lock().unwrap_or_else(|e| e.into_inner());
                    if !set.is_empty() && !set.contains(remote) {
                        drop(set);
                        tracing::warn!("iroh: rejected unpaired device ({remote})");
                        conn.close(0u32.into(), b"unpaired device");
                        continue;
                    }
                }
            }

            connections.fetch_add(1, Ordering::Relaxed);
            let bytes = bytes.clone();
            tokio::spawn(async move {
                loop {
                    match conn.accept_bi().await {
                        Ok((send, recv)) => {
                            tokio::spawn(proxy(send, recv, port, bytes.clone()));
                        }
                        Err(_) => break,
                    }
                }
            });
        }
    });
}

/// One HTTP/1.1 connection: pipe the QUIC bidi stream to a local TCP connection.
async fn proxy(
    mut send: iroh::endpoint::SendStream,
    mut recv: iroh::endpoint::RecvStream,
    port: u16,
    bytes: Arc<AtomicU64>,
) {
    let tcp = match TcpStream::connect(("127.0.0.1", port)).await {
        Ok(tcp) => tcp,
        Err(e) => {
            tracing::warn!("iroh proxy: local connect failed: {e}");
            return;
        }
    };
    let (mut tcp_r, mut tcp_w) = tcp.into_split();

    let up = async {
        let n = tokio::io::copy(&mut recv, &mut tcp_w).await.unwrap_or(0);
        bytes.fetch_add(n, Ordering::Relaxed);
        let _ = tcp_w.shutdown().await;
    };
    let down = async {
        let n = tokio::io::copy(&mut tcp_r, &mut send).await.unwrap_or(0);
        bytes.fetch_add(n, Ordering::Relaxed);
        let _ = send.finish();
    };
    tokio::join!(up, down);
}
