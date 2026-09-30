//! Tailscale Serve/Funnel integration.
//!
//! Shiny shells out to the `tailscale` CLI rather than linking a library: it is
//! the same binary the user already manages, needs no daemon-socket coupling,
//! and a machine without Tailscale simply reports `installed: false` so the UI
//! hides the section (the Iroh path is unaffected).
//!
//! `tailscale funnel` is an L7 reverse proxy that terminates TLS on the node
//! (auto-provisioned `*.ts.net` certificate) and forwards to the local Shiny
//! server. It adds `X-Forwarded-For`, which `api::remote::is_remote` treats as
//! the remote-client signal, so host controls stay local. Only ports 443, 8443
//! and 10000 are allowed by Funnel; we use 443 with the root path.

use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

/// How long a probed status is reused. The settings panel polls every few
/// seconds; spawning the `tailscale` binary on every poll would be wasteful.
const STATUS_TTL: Duration = Duration::from_secs(5);

/// The public state of Tailscale/Funnel, as shown in the UI.
#[derive(Clone, Serialize)]
pub struct TailscaleStatus {
    /// The `tailscale` binary is present and runnable.
    pub installed: bool,
    /// `tailscaled` is up and logged in.
    pub up: bool,
    /// A Funnel route is currently serving the app.
    pub funnel: bool,
    /// The public HTTPS URL, when a Funnel is on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// This node's MagicDNS name (`host.tailnet.ts.net`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    /// Last error from a probe or action, surfaced in the UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl TailscaleStatus {
    fn absent() -> Self {
        Self {
            installed: false,
            up: false,
            funnel: false,
            url: None,
            hostname: None,
            error: None,
        }
    }
}

/// Handle stored in `AppState`. Cheap to clone; shares one status cache.
#[derive(Clone)]
pub struct TailscaleService {
    /// Loopback port Funnel forwards to (the Shiny server itself).
    port: u16,
    cache: Arc<Mutex<Option<(Instant, TailscaleStatus)>>>,
}

impl TailscaleService {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            cache: Arc::new(Mutex::new(None)),
        }
    }

    /// Current status, cached briefly.
    pub async fn status(&self) -> TailscaleStatus {
        if let Some((at, cached)) = self.cached() {
            if at.elapsed() < STATUS_TTL {
                return cached;
            }
        }
        let port = self.port;
        let status = tokio::task::spawn_blocking(move || probe(port))
            .await
            .unwrap_or_else(|_| TailscaleStatus::absent());
        self.store(status.clone());
        status
    }

    /// Turn Funnel on for the app, idempotently. Returns the fresh status.
    pub async fn enable(&self) -> Result<TailscaleStatus, String> {
        let port = self.port;
        let result = tokio::task::spawn_blocking(move || funnel(port, true))
            .await
            .map_err(|e| format!("tailscale task: {e}"))?;
        result?;
        self.refresh().await
    }

    /// Turn Funnel off, idempotently. Returns the fresh status.
    pub async fn disable(&self) -> Result<TailscaleStatus, String> {
        let port = self.port;
        let result = tokio::task::spawn_blocking(move || funnel(port, false))
            .await
            .map_err(|e| format!("tailscale task: {e}"))?;
        result?;
        self.refresh().await
    }

    /// Probe now, bypassing the cache.
    async fn refresh(&self) -> Result<TailscaleStatus, String> {
        let port = self.port;
        let status = tokio::task::spawn_blocking(move || probe(port))
            .await
            .map_err(|e| format!("tailscale task: {e}"))?;
        self.store(status.clone());
        if let Some(err) = status.error.clone() {
            return Err(err);
        }
        Ok(status)
    }

    fn cached(&self) -> Option<(Instant, TailscaleStatus)> {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn store(&self, status: TailscaleStatus) {
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((Instant::now(), status));
    }
}

/// Run one `tailscale` invocation with a hard timeout and return its combined
/// output. `tailscale funnel` blocks the first time it needs tailnet approval,
/// so an unbounded wait would hang the request forever.
fn run(args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut child = Command::new("tailscale")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "Tailscale is not installed".to_string()
            } else {
                format!("could not run tailscale: {e}")
            }
        })?;

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let out = read_output(&mut child);
                    return Err(describe(&out, "timed out waiting for tailscale"));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("could not run tailscale: {e}")),
        }
    };

    let out = read_output(&mut child);
    if status.success() {
        Ok(out)
    } else {
        Err(describe(&out, "tailscale command failed"))
    }
}

/// Drain a finished child's captured stdout and stderr into one string.
fn read_output(child: &mut std::process::Child) -> String {
    use std::io::Read;
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut out);
    }
    if let Some(mut stderr) = child.stderr.take() {
        let mut err = String::new();
        let _ = stderr.read_to_string(&mut err);
        if !err.trim().is_empty() {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&err);
        }
    }
    out.trim().to_string()
}

/// Turn captured output into an error, preferring any Tailscale approval URL
/// it contains so the UI can show the user exactly what to open.
fn describe(output: &str, fallback: &str) -> String {
    if let Some(url) = extract_approval_url(output) {
        return format!("Approve Funnel for your tailnet first: {url}");
    }
    if output.is_empty() {
        fallback.to_string()
    } else {
        output.to_string()
    }
}

/// Pull a `https://login.tailscale.com/...` link out of CLI output.
fn extract_approval_url(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|t| t.trim_end_matches([')', ',', '.', '…']))
        .find(|t| t.starts_with("https://login.tailscale.com/"))
        .map(|t| t.to_string())
}

/// Probe the node and its Funnel configuration. Blocking; run off the runtime.
fn probe(_port: u16) -> TailscaleStatus {
    let mut status = TailscaleStatus::absent();

    // Is the CLI there at all? (`tailscale version` is cheap and side-effect-free.)
    if run(&["version"], Duration::from_secs(5)).is_err() {
        return status;
    }
    status.installed = true;

    // Up + MagicDNS name from the machine-readable status.
    match run(&["status", "--json"], Duration::from_secs(10)) {
        Ok(json) => {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) {
                let backend = value
                    .get("BackendState")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                status.up = backend.eq_ignore_ascii_case("Running");
                status.hostname = value
                    .get("Self")
                    .and_then(|s| s.get("DNSName"))
                    .and_then(|v| v.as_str())
                    .map(|name| name.trim_end_matches('.').to_string());
            }
        }
        Err(e) => status.error = Some(e),
    }

    // Funnel state + URL. `funnel status` prints the serve config as text; a
    // route that is exposed publicly is tagged "(Funnel on)".
    if let Ok(text) = run(&["funnel", "status"], Duration::from_secs(15)) {
        status.funnel = text.contains("Funnel on");
        status.url = extract_url(&text);
        if status.funnel && status.url.is_none() {
            status.url = status
                .hostname
                .as_ref()
                .map(|host| format!("https://{host}"));
        }
    }

    // A Funnel route is per-port; the URL only depends on the node name, so the
    // port is not needed to report status.
    status
}

/// First `https://…` token in the status text, with trailing punctuation and
/// the Tailscale "…" ellipsis trimmed.
fn extract_url(text: &str) -> Option<String> {
    for token in text.split_whitespace() {
        let cleaned = token.trim_end_matches([')', ',', '.', '|', '…']);
        if let Some(rest) = cleaned.strip_prefix("https://") {
            if !rest.is_empty() {
                return Some(cleaned.to_string());
            }
        }
    }
    None
}

/// Enable or disable the root Funnel route pointing at the local server.
fn funnel(port: u16, on: bool) -> Result<(), String> {
    // Probe once so "not installed" is a clear error rather than a spawn failure.
    run(&["version"], Duration::from_secs(5))
        .map_err(|_| "Tailscale is not installed".to_string())?;

    let target = format!("127.0.0.1:{port}");
    let mut args: Vec<&str> = vec!["funnel", "--bg", "--https=443", "--set-path=/"];
    if on {
        args.push(target.as_str());
    } else {
        args.push("off");
    }
    run(&args, Duration::from_secs(20)).map(|_| ()).map_err(|e| {
        // The CLI's own message (or the approval URL it prints) is the most
        // useful thing we can show; add the operator hint only as a fallback.
        if e.starts_with("Approve Funnel") {
            e
        } else {
            format!("{e} (run `sudo tailscale set --operator=$USER` to allow this user to manage Funnel)")
        }
    })
}
