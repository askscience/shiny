//! Headless bootstrap, supervision and public exposure of the Gokapi server.
//!
//! Gokapi normally starts a web setup wizard when `config.json` is missing
//! (`setup.RunIfFirstStart`). Shiny skips it entirely; checked against the
//! v2.2.4 sources:
//!
//! 1. write `config.json` ourselves ([`config_json`], `ConfigVersion` 22,
//!    internal auth, SQLite, loopback port, encryption level 0);
//! 2. run `gokapi --deployment-password <random>`: on an empty database
//!    `EditSuperAdmin` creates the super-admin, the schema is created on
//!    connect, and the process exits 0;
//! 3. insert an API key into `ApiKeys` ([`insert_api_key`]). Gokapi stores the
//!    key in clear as the row id (`isValidApiKey` → `database.GetApiKey`).
//!
//! The server then runs as a supervised child bound to `127.0.0.1`. Links are
//! made public either through `tailscale funnel` on :8443 (Shiny itself uses
//! :443) or behind a reverse proxy the user already has ("custom" URL). Both
//! point at the share gate ([`super::gate`]), never at Gokapi itself, so only
//! the download pages are public.

use std::path::Path;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use shiny_plugin_sdk::db::{Db, Value};
use tokio::sync::oneshot;

use super::install::server_bin;
use super::{
    cli_config_path, config_dir, data_dir, gokapi_config_path, gokapi_db_path, log_path,
    random_alnum, shiny_state_path, state_dir, write_private,
};

/// Gokapi's `configupgrade.CurrentConfigVersion` for v2.2.4.
const CONFIG_VERSION: i64 = 22;
/// `ApiPermView | ApiPermUpload | ApiPermDelete | ApiPermEdit`
/// (`models.ApiPermDefault`; bits 1, 2, 4 and 16).
const API_PERMISSIONS: i64 = 1 | 2 | 4 | 16;
/// Lengths Gokapi itself uses (`api.LengthApiKey`, `api.LengthPublicId`).
const API_KEY_LEN: usize = 30;
const PUBLIC_ID_LEN: usize = 35;
/// Name Gokapi shows in page titles and link previews (`og:title`).
pub const PUBLIC_NAME: &str = "PEAK'D!";
/// Funnel allows 443, 8443 and 10000; 443 is taken by Shiny's own Funnel.
pub const FUNNEL_PORT: u16 = 8443;

/// Shiny's record of the managed instance (`shiny.json`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Instance {
    /// Loopback port Gokapi listens on.
    pub port: u16,
    pub api_key: String,
    /// Base URL recipients open, with a trailing slash.
    pub public_url: String,
    /// `"funnel"` or `"custom"`.
    pub mode: String,
    /// Start the server with Shiny. Cleared by "disable".
    pub enabled: bool,
    /// Loopback port of the public gate in front of Gokapi; what Funnel or the
    /// user's reverse proxy forwards to. 0 on instances created before the
    /// gate existed, assigned by [`ensure_gate`].
    #[serde(default)]
    pub gate_port: u16,
    /// End-to-end encryption has been turned on (`e2e::enable`).
    #[serde(default)]
    pub e2e: bool,
}

impl Instance {
    pub fn local_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

pub fn load_instance() -> Option<Instance> {
    let raw = std::fs::read(shiny_state_path()).ok()?;
    serde_json::from_slice(&raw).ok()
}

pub fn save_instance(inst: &Instance) -> Result<(), String> {
    let raw = serde_json::to_vec_pretty(inst).map_err(|e| e.to_string())?;
    write_private(&shiny_state_path(), &raw).map_err(|e| format!("save state: {e}"))
}

/// Normalise a public base URL: http(s) only, trailing slash.
pub fn normalize_url(url: &str) -> Result<String, String> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("the public URL must start with https:// or http://".into());
    }
    if url.contains(char::is_whitespace) {
        return Err("the public URL may not contain spaces".into());
    }
    Ok(if url.ends_with('/') {
        url.to_string()
    } else {
        format!("{url}/")
    })
}

/// Gokapi's `config.json` for a fresh headless instance.
pub fn config_json(port: u16, server_url: &str, salt_admin: &str, salt_files: &str) -> Json {
    let data = data_dir();
    json!({
        "Authentication": {
            "Method": 0,
            "SaltAdmin": salt_admin,
            "SaltFiles": salt_files,
            "Username": "shiny",
            "HeaderKey": "",
            "OauthProvider": "",
            "OAuthClientId": "",
            "OAuthClientSecret": "",
            "OauthGroupScope": "",
            "OAuthRecheckInterval": 12,
            "OAuthGroups": [],
            "OnlyRegisteredUsers": false
        },
        "Port": format!("127.0.0.1:{port}"),
        "ServerUrl": server_url,
        "RedirectUrl": "",
        "PublicName": PUBLIC_NAME,
        "DataDir": data.to_string_lossy(),
        "DatabaseUrl": format!("sqlite://{}", gokapi_db_path().to_string_lossy()),
        "ConfigVersion": CONFIG_VERSION,
        "MaxFileSizeMB": 102400,
        "MaxMemory": 50,
        "ChunkSize": 45,
        "MaxParallelUploads": 4,
        "Encryption": { "Level": 0, "Cipher": null, "Salt": "", "Checksum": "", "ChecksumSalt": "" },
        "UseSsl": false,
        "PicturesAlwaysLocal": false,
        "SaveIp": false,
        "IncludeFilename": false
    })
}

/// `gokapi-cli`'s login file, written directly instead of the interactive
/// `gokapi-cli login`. The CLI talks to the loopback port, never the tunnel.
pub fn cli_config_json(port: u16, api_key: &str) -> Json {
    json!({ "Url": format!("http://127.0.0.1:{port}"), "Apikey": api_key, "E2Ekey": null })
}

/// A free loopback port, picked by the kernel.
fn free_port() -> Result<u16, String> {
    let l = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| format!("pick port: {e}"))?;
    l.local_addr().map(|a| a.port()).map_err(|e| e.to_string())
}

/// Environment every `gokapi` invocation runs with.
fn gokapi_command(bin: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(bin);
    cmd.current_dir(state_dir())
        .env("GOKAPI_CONFIG_DIR", config_dir())
        .env("GOKAPI_CONFIG_FILE", "config.json")
        .env("GOKAPI_DATA_DIR", data_dir())
        .stdin(Stdio::null());
    cmd
}

/// Create the instance once; a no-op when `shiny.json` and `config.json`
/// already exist. `public_url` must already be normalised.
pub async fn bootstrap(public_url: &str, mode: &str) -> Result<Instance, String> {
    if let Some(inst) = load_instance() {
        if gokapi_config_path().is_file() {
            return Ok(inst);
        }
    }
    let bin = server_bin().ok_or("Gokapi is not installed")?;
    for dir in [state_dir(), config_dir(), data_dir()] {
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    // A half-finished earlier attempt would make `--deployment-password` see
    // an existing user without a super-admin and abort: start clean.
    let _ = std::fs::remove_file(gokapi_db_path());

    let port = free_port()?;
    let config = config_json(port, public_url, &random_alnum(30), &random_alnum(30));
    write_private(
        &gokapi_config_path(),
        &serde_json::to_vec_pretty(&config).unwrap(),
    )
    .map_err(|e| format!("write config.json: {e}"))?;

    // The admin password is never needed by Shiny (it uses the API key); it
    // is kept only so an operator can recover the instance by hand.
    let password = random_alnum(32);
    write_private(&state_dir().join("admin.pass"), password.as_bytes())
        .map_err(|e| format!("write admin.pass: {e}"))?;
    let mut cmd = gokapi_command(&bin);
    cmd.arg("--deployment-password")
        .arg(&password)
        .kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(60), cmd.output())
        .await
        .map_err(|_| "gokapi --deployment-password timed out".to_string())?
        .map_err(|e| format!("run gokapi: {e}"))?;
    if !out.status.success() {
        let text = String::from_utf8_lossy(&out.stdout).to_string()
            + &String::from_utf8_lossy(&out.stderr);
        let tail: String = text.lines().rev().take(5).collect::<Vec<_>>().join(" | ");
        return Err(format!("gokapi bootstrap failed ({}): {tail}", out.status));
    }

    let db_path = gokapi_db_path();
    let api_key = tokio::task::spawn_blocking(move || insert_api_key(&db_path))
        .await
        .map_err(|e| e.to_string())??;
    write_private(
        &cli_config_path(),
        &serde_json::to_vec(&cli_config_json(port, &api_key)).unwrap(),
    )
    .map_err(|e| format!("write gokapi-cli.json: {e}"))?;

    let inst = Instance {
        port,
        api_key,
        public_url: public_url.to_string(),
        mode: mode.to_string(),
        enabled: false,
        gate_port: free_port()?,
        e2e: false,
    };
    save_instance(&inst)?;
    Ok(inst)
}

/// Insert a full-permission API key for the super-admin into Gokapi's
/// database and return it. The server must not be running.
pub fn insert_api_key(db_path: &Path) -> Result<String, String> {
    let db = Db::open(&format!("sqlite://{}", db_path.to_string_lossy()))
        .map_err(|e| format!("open gokapi db: {e}"))?;
    let rows = db
        .query("SELECT Id FROM Users WHERE Userlevel = 0 LIMIT 1", &[])
        .map_err(|e| format!("read gokapi users: {e}"))?;
    let admin_id = match rows.first().and_then(|r| r.first()) {
        Some(Value::Int(id)) => *id,
        _ => return Err("Gokapi super-admin was not created".into()),
    };
    let key = random_alnum(API_KEY_LEN);
    db.execute(
        "INSERT INTO ApiKeys (Id, FriendlyName, LastUsed, Permissions, Expiry, IsSystemKey, \
         UserId, PublicId, UploadRequestId) VALUES (?, 'Shiny', 0, ?, 0, 0, ?, ?, '')",
        &[
            Value::text(key.clone()),
            Value::Int(API_PERMISSIONS),
            Value::Int(admin_id),
            Value::text(random_alnum(PUBLIC_ID_LEN)),
        ],
    )
    .map_err(|e| format!("insert gokapi api key: {e}"))?;
    Ok(key)
}

/// Point generated links at a new public base URL (Gokapi builds
/// `UrlDownload` from `ServerUrl`). Takes effect on the next server start.
pub fn set_server_url(url: &str) -> Result<(), String> {
    let path = gokapi_config_path();
    let raw = std::fs::read(&path).map_err(|e| format!("read config.json: {e}"))?;
    let mut cfg: Json =
        serde_json::from_slice(&raw).map_err(|e| format!("parse config.json: {e}"))?;
    cfg["ServerUrl"] = Json::String(url.to_string());
    write_private(&path, &serde_json::to_vec_pretty(&cfg).unwrap())
        .map_err(|e| format!("write config.json: {e}"))
}

/// Set Gokapi's `PublicName` when it differs (read at start).
fn rename_public(name: &str) -> Result<(), String> {
    let path = gokapi_config_path();
    let raw = std::fs::read(&path).map_err(|e| e.to_string())?;
    let mut cfg: Json = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
    if cfg["PublicName"] == name {
        return Ok(());
    }
    cfg["PublicName"] = Json::String(name.to_string());
    write_private(&path, &serde_json::to_vec_pretty(&cfg).unwrap()).map_err(|e| e.to_string())
}

/* ── supervision ──────────────────────────────────────────── */

#[derive(Default)]
struct Supervisor {
    stop: Option<oneshot::Sender<()>>,
    running: bool,
    last_error: Option<String>,
}

static SUP: Mutex<Supervisor> = Mutex::new(Supervisor {
    stop: None,
    running: false,
    last_error: None,
});

fn sup() -> std::sync::MutexGuard<'static, Supervisor> {
    SUP.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn is_running() -> bool {
    sup().running
}

pub fn last_error() -> Option<String> {
    sup().last_error.clone()
}

/// Start the supervisor task if it is not already running. Must be called on
/// the plugin runtime (bridged route, or inside `rt::spawn`).
pub fn start_supervisor() {
    let rx = {
        let mut s = sup();
        if s.stop.is_some() {
            return;
        }
        let (tx, rx) = oneshot::channel();
        s.stop = Some(tx);
        s.last_error = None;
        rx
    };
    tokio::spawn(supervise(rx));
}

/// Give an instance from before the gate existed a gate port (saved), then
/// make sure the gate is listening.
pub fn ensure_gate(inst: &mut Instance) -> Result<(), String> {
    if inst.gate_port == 0 {
        inst.gate_port = free_port()?;
        save_instance(inst)?;
    }
    super::gate::start(inst.gate_port, inst.port)
}

/// Stop the server, its supervisor and the public gate.
pub async fn stop_server() {
    super::gate::stop();
    let tx = sup().stop.take();
    if let Some(tx) = tx {
        let _ = tx.send(());
    }
    // Give the supervisor a moment to reap the child.
    for _ in 0..50 {
        if !is_running() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn spawn_server() -> Result<tokio::process::Child, String> {
    let bin = server_bin().ok_or("Gokapi is not installed")?;
    // Gokapi reads custom/ only at start: refresh the PEAK'D! theme first. A
    // failure only costs the look, never the links.
    if let Err(e) = super::theme::write_theme() {
        eprintln!("[files] could not write the share theme: {e}");
    }
    // Instances made before the rename still say "Shiny" in titles/previews.
    let _ = rename_public(PUBLIC_NAME);
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
        .map_err(|e| format!("open gokapi.log: {e}"))?;
    let log_err = log.try_clone().map_err(|e| e.to_string())?;
    let mut cmd = gokapi_command(&bin);
    cmd.stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .kill_on_drop(true);
    // If Shiny dies without unloading the plugin, take the server with it
    // rather than leaving an orphan holding the port.
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    cmd.spawn().map_err(|e| format!("start gokapi: {e}"))
}

async fn supervise(mut stop: oneshot::Receiver<()>) {
    let mut backoff = 1u64;
    loop {
        let mut child = match spawn_server() {
            Ok(c) => c,
            Err(e) => {
                let mut s = sup();
                s.last_error = Some(e);
                s.stop = None;
                return;
            }
        };
        sup().running = true;
        let started = Instant::now();
        tokio::select! {
            _ = &mut stop => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                sup().running = false;
                return;
            }
            status = child.wait() => {
                let mut s = sup();
                s.running = false;
                s.last_error = Some(match status {
                    Ok(st) => format!("gokapi exited ({st}); see {}", log_path().display()),
                    Err(e) => format!("gokapi wait failed: {e}"),
                });
            }
        }
        backoff = if started.elapsed() > Duration::from_secs(60) {
            1
        } else {
            (backoff * 2).min(60)
        };
        tokio::select! {
            _ = &mut stop => return,
            _ = tokio::time::sleep(Duration::from_secs(backoff)) => {}
        }
    }
}

/// Bring an enabled instance back after Shiny starts: gate, server, and the
/// Funnel re-pointed at the gate. The last step also moves an instance set up
/// before the gate existed (its Funnel still targets Gokapi's own port).
/// Must run on the plugin runtime; spawn it, since `tailscale funnel` can take
/// a few seconds.
pub async fn resume() {
    let Some(mut inst) = load_instance().filter(|i| i.enabled) else {
        return;
    };
    if let Err(e) = ensure_gate(&mut inst) {
        sup().last_error = Some(e);
        return;
    }
    start_supervisor();
    if inst.mode == "funnel" {
        if let Err(e) = funnel(inst.gate_port, true).await {
            sup().last_error = Some(e);
        }
    }
}

/// Start the server if needed and wait until its API answers.
pub async fn ensure_running(inst: &Instance) -> Result<(), String> {
    if inst.gate_port != 0 {
        super::gate::start(inst.gate_port, inst.port)?;
    }
    start_supervisor();
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if super::rest::healthy(inst).await {
            return Ok(());
        }
        if !is_running() {
            if let Some(e) = last_error() {
                return Err(e);
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Err(format!(
        "Gokapi did not become ready on port {}; see {}",
        inst.port,
        log_path().display()
    ))
}

/* ── Tailscale Funnel ─────────────────────────────────────── */

async fn run_cmd(bin: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(args).stdin(Stdio::null()).kill_on_drop(true);
    let out = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| format!("`{bin} {}` timed out", args.join(" ")))?
        .map_err(|e| format!("{bin}: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if out.status.success() {
        Ok(stdout)
    } else {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if msg.is_empty() {
            stdout.trim().to_string()
        } else {
            msg
        })
    }
}

pub fn tailscale_installed() -> bool {
    super::install::which("tailscale").is_some()
}

/// This node's MagicDNS name (`Self.DNSName` without the trailing dot).
pub async fn tailnet_hostname() -> Result<String, String> {
    let out = run_cmd("tailscale", &["status", "--json"], Duration::from_secs(10)).await?;
    parse_dns_name(&out).ok_or_else(|| "Tailscale is not logged in (no MagicDNS name)".into())
}

fn parse_dns_name(status_json: &str) -> Option<String> {
    let v: Json = serde_json::from_str(status_json).ok()?;
    let name = v
        .get("Self")?
        .get("DNSName")?
        .as_str()?
        .trim_end_matches('.');
    (!name.is_empty()).then(|| name.to_string())
}

pub fn funnel_url(hostname: &str) -> String {
    format!("https://{hostname}:{FUNNEL_PORT}/")
}

/// Turn the :8443 Funnel to `port` (the share gate) on or off. Mirrors the error
/// handling of core's `services::tailscale::funnel`.
pub async fn funnel(port: u16, on: bool) -> Result<(), String> {
    let https = format!("--https={FUNNEL_PORT}");
    let target = format!("127.0.0.1:{port}");
    let mut args = vec!["funnel", "--bg", https.as_str()];
    args.push(if on { target.as_str() } else { "off" });
    run_cmd("tailscale", &args, Duration::from_secs(20)).await.map(|_| ()).map_err(|e| {
        if e.contains("login.tailscale.com") || e.starts_with("Approve") {
            e
        } else {
            format!("{e} (run `sudo tailscale set --operator=$USER` to allow this user to manage Funnel)")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_url_adds_slash_and_rejects_other_schemes() {
        assert_eq!(
            normalize_url("https://x.example").unwrap(),
            "https://x.example/"
        );
        assert_eq!(
            normalize_url(" https://x.example/s/ ").unwrap(),
            "https://x.example/s/"
        );
        assert!(normalize_url("ftp://x").is_err());
        assert!(normalize_url("https://a b").is_err());
    }

    #[test]
    fn config_json_matches_gokapi_v22_shape() {
        let c = config_json(4242, "https://h.ts.net:8443/", "sa", "sf");
        assert_eq!(c["ConfigVersion"], 22);
        assert_eq!(c["Port"], "127.0.0.1:4242");
        assert_eq!(c["ServerUrl"], "https://h.ts.net:8443/");
        assert_eq!(c["Authentication"]["Method"], 0);
        assert_eq!(c["Authentication"]["SaltFiles"], "sf");
        assert_eq!(c["Encryption"]["Level"], 0);
        assert!(c["DatabaseUrl"].as_str().unwrap().starts_with("sqlite://"));
    }

    #[test]
    fn cli_config_uses_loopback() {
        let c = cli_config_json(4242, "k");
        assert_eq!(c["Url"], "http://127.0.0.1:4242");
        assert_eq!(c["Apikey"], "k");
        assert!(c["E2Ekey"].is_null());
    }

    #[test]
    fn parses_magicdns_name() {
        let s = r#"{"Self":{"DNSName":"box.tail1234.ts.net."}}"#;
        assert_eq!(parse_dns_name(s).as_deref(), Some("box.tail1234.ts.net"));
        assert_eq!(parse_dns_name(r#"{"Self":{"DNSName":""}}"#), None);
        assert_eq!(funnel_url("box.ts.net"), "https://box.ts.net:8443/");
    }

    #[test]
    fn inserts_api_key_for_super_admin() {
        let dir = std::env::temp_dir().join(format!("shiny-gokapi-db-{}", random_alnum(8)));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("g.sqlite");
        let db = Db::open(&format!("sqlite://{}", path.display())).unwrap();
        // Subset of Gokapi v2.2.4's schema that insert_api_key touches.
        db.execute(
            "CREATE TABLE Users (Id INTEGER PRIMARY KEY AUTOINCREMENT, Name TEXT NOT NULL UNIQUE, \
             Password TEXT, Permissions INTEGER NOT NULL, Userlevel INTEGER NOT NULL, \
             LastOnline INTEGER NOT NULL DEFAULT 0, ResetPassword INTEGER NOT NULL DEFAULT 0)",
            &[],
        )
        .unwrap();
        db.execute(
            "CREATE TABLE ApiKeys (Id TEXT NOT NULL UNIQUE, FriendlyName TEXT NOT NULL, \
             LastUsed INTEGER NOT NULL, Permissions INTEGER NOT NULL DEFAULT 0, Expiry INTEGER, \
             IsSystemKey INTEGER, UserId INTEGER NOT NULL, PublicId TEXT NOT NULL UNIQUE, \
             UploadRequestId TEXT NOT NULL, PRIMARY KEY(Id)) WITHOUT ROWID",
            &[],
        )
        .unwrap();
        db.execute(
            "INSERT INTO Users (Name, Password, Permissions, Userlevel) VALUES ('shiny', 'x', 255, 0)",
            &[],
        )
        .unwrap();
        drop(db);

        let key = insert_api_key(&path).unwrap();
        assert_eq!(key.len(), API_KEY_LEN);
        let db = Db::open(&format!("sqlite://{}", path.display())).unwrap();
        let rows = db
            .query(
                "SELECT Id, Permissions, UserId, length(PublicId) FROM ApiKeys",
                &[],
            )
            .unwrap();
        assert_eq!(
            rows,
            vec![vec![
                Value::Text(key),
                Value::Int(API_PERMISSIONS),
                Value::Int(1),
                Value::Int(PUBLIC_ID_LEN as i64)
            ]]
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
