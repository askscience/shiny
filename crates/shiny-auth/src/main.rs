//! `shiny-auth` — the privileged Linux-PAM verification helper for Shiny.
//!
//! Runs as **root** (systemd socket-activated) and does exactly one thing:
//! answer "is this the right password for this Linux user?" over a local Unix
//! socket. It opens no session, touches no files, and never logs or returns the
//! password. The Shiny web server — which must never run as root — talks to it
//! through `src/services/auth_helper.rs`.
//!
//! Hardening:
//!   * the socket is mode 0660 root:shiny and `SO_PEERCRED` restricts callers
//!     to `SHINY_AUTH_ALLOW_UID` (the server user) or root;
//!   * per-user and global rate limits bound password guessing;
//!   * a per-connection read/write timeout bounds slow-loris clients;
//!   * request size is capped;
//!   * only `verify` and `ping` are implemented.
//!
//! Build/install with `scripts/install-linux-auth.sh`. Run by hand for a test:
//!   SHINY_AUTH_SOCK=/tmp/auth.sock SHINY_AUTH_ALLOW_UID=$(id -u) \
//!     target/debug/shiny-auth

mod pam;
mod protocol;

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pam::PamError;
use protocol::{Request, Response};

fn log(message: impl AsRef<str>) {
    eprintln!("[shiny-auth] {}", message.as_ref());
}

/// Bounds how often one username (and the socket as a whole) may be tried.
struct Limiter {    per_user: Mutex<HashMap<String, Vec<Instant>>>,
    global: Mutex<Vec<Instant>>,
    window: Duration,
    per_user_max: usize,
    global_max: usize,
}

impl Limiter {
    fn new(window: Duration, per_user_max: usize, global_max: usize) -> Self {
        Self {
            per_user: Mutex::new(HashMap::new()),
            global: Mutex::new(Vec::new()),
            window,
            per_user_max,
            global_max,
        }
    }

    fn allow(&self, user: &str) -> bool {
        let now = Instant::now();
        {
            let mut map = self.per_user.lock().unwrap_or_else(|e| e.into_inner());
            // Keep the map from growing without bound if a caller sends
            // arbitrary usernames: drop stale keys when it gets large.
            if map.len() > 4096 {
                map.retain(|_, times| {
                    times.retain(|t| now.duration_since(*t) < self.window);
                    !times.is_empty()
                });
            }
            let times = map.entry(user.to_string()).or_default();
            times.retain(|t| now.duration_since(*t) < self.window);
            if times.len() >= self.per_user_max {
                return false;
            }
            times.push(now);
        }
        let mut global = self.global.lock().unwrap_or_else(|e| e.into_inner());
        global.retain(|t| now.duration_since(*t) < self.window);
        if global.len() >= self.global_max {
            return false;
        }
        global.push(now);
        true
    }
}

struct Config {
    sock: String,
    service: String,
    allow_uid: Option<u32>,
    timeout: Duration,
    max_line: usize,
    limiter: Limiter,
}

impl Config {
    fn from_env() -> Self {
        let allow_uid = std::env::var("SHINY_AUTH_ALLOW_UID")
            .ok()
            .and_then(|v| v.trim().parse().ok());
        Self {
            sock: std::env::var("SHINY_AUTH_SOCK")
                .unwrap_or_else(|_| "/run/shiny/auth.sock".into()),
            service: std::env::var("SHINY_AUTH_PAM_SERVICE")
                .unwrap_or_else(|_| "shiny".into()),
            allow_uid,
            timeout: Duration::from_secs(
                std::env::var("SHINY_AUTH_TIMEOUT_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(10),
            ),
            max_line: 8 * 1024,
            limiter: Limiter::new(
                Duration::from_secs(
                    std::env::var("SHINY_AUTH_WINDOW_SECS")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(60),
                ),
                std::env::var("SHINY_AUTH_MAX_ATTEMPTS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(10),
                std::env::var("SHINY_AUTH_GLOBAL_MAX_ATTEMPTS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(120),
            ),
        }
    }
}

/// Use the systemd-passed listening socket when socket-activated, otherwise
/// bind `SHINY_AUTH_SOCK` ourselves (manual runs / testing).
fn build_listener(cfg: &Config) -> io::Result<UnixListener> {
    if let Some(listener) = systemd_listener() {
        return Ok(listener);
    }
    let _ = std::fs::remove_file(&cfg.sock);
    if let Some(parent) = Path::new(&cfg.sock).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let listener = UnixListener::bind(&cfg.sock)?;
    let _ = std::fs::set_permissions(&cfg.sock, std::fs::Permissions::from_mode(0o660));
    Ok(listener)
}

/// The socket systemd passes as fd 3 when `Accept=no` and the unit is
/// socket-activated (`LISTEN_FDS=1`).
fn systemd_listener() -> Option<UnixListener> {
    let pid: i32 = std::env::var("LISTEN_PID").ok()?.parse().ok()?;
    if pid != std::process::id() as i32 {
        return None;
    }
    let fds: i32 = std::env::var("LISTEN_FDS").ok()?.parse().ok()?;
    if fds < 1 {
        return None;
    }
    Some(unsafe { UnixListener::from_raw_fd(3) })
}

/// The uid of the process on the other end of `stream` (`SO_PEERCRED`).
fn peer_uid(stream: &UnixStream) -> Option<u32> {
    let fd = stream.as_raw_fd();
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if rc == 0 {
        Some(cred.uid)
    } else {
        None
    }
}

fn write_response(stream: &UnixStream, resp: &Response) -> io::Result<()> {
    let mut s = stream;
    s.write_all(resp.to_line().as_bytes())?;
    s.flush()
}

fn handle(stream: UnixStream, cfg: &Config) -> io::Result<()> {
    match peer_uid(&stream) {
        Some(uid) => {
            if let Some(allow) = cfg.allow_uid {
                if uid != allow && uid != 0 {
                    let _ = write_response(
                        &stream,
                        &Response::error("forbidden", "caller is not permitted"),
                    );
                    log(format!("rejected connection from uid {uid}"));
                    return Ok(());
                }
            }
        }
        None => {
            let _ = write_response(
                &stream,
                &Response::error("forbidden", "cannot read peer credentials"),
            );
            return Ok(());
        }
    }

    stream.set_read_timeout(Some(cfg.timeout)).ok();
    stream.set_write_timeout(Some(cfg.timeout)).ok();

    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    let read = reader.read_line(&mut line)?;
    if read == 0 {
        return Ok(());
    }
    if read > cfg.max_line {
        return write_response(&stream, &Response::error("too_large", "request too large"));
    }

    let req: Request = match serde_json::from_str(line.trim()) {
        Ok(req) => req,
        Err(_) => return write_response(&stream, &Response::error("bad_request", "invalid JSON")),
    };

    let response = match req.op.as_deref().unwrap_or("verify") {
        "ping" => {
            if pam::available() {
                Response::ok()
            } else {
                Response::unavailable(pam::unavailable_reason().unwrap_or_default())
            }
        }
        "verify" => verify(&req, cfg),
        other => Response::error("unknown_op", format!("unknown op `{other}`")),
    };
    write_response(&stream, &response)
}

fn verify(req: &Request, cfg: &Config) -> Response {
    let user = req.user.as_deref().unwrap_or("").trim().to_string();
    let password = req.password.as_deref().unwrap_or("");

    if user.is_empty() || user.len() > 64 {
        return Response::error("invalid_user", "missing or invalid user name");
    }
    if password.len() > 1024 {
        return Response::error("invalid_password", "password too long");
    }
    if !cfg.limiter.allow(&user) {
        log(format!("rate limited: {user}"));
        return Response::error("rate_limited", "too many attempts — wait and retry");
    }

    match pam::verify(&cfg.service, &user, password) {
        Ok(()) => {
            log(format!("verified: {user}"));
            Response::ok()
        }
        Err(PamError::Denied(code, message)) => {
            log(format!("denied: {user} (pam {code}: {message})"));
            Response::denied(message)
        }
        Err(PamError::Unavailable(message)) => {
            log(format!("unavailable: {message}"));
            Response::unavailable(message)
        }
    }
}

fn main() {
    let cfg = Arc::new(Config::from_env());

    if pam::available() {
        log("libpam loaded — verification enabled");
    } else {
        log(format!(
            "WARNING: PAM unavailable: {}",
            pam::unavailable_reason().unwrap_or_default()
        ));
    }
    if cfg.allow_uid.is_none() {
        log("WARNING: SHINY_AUTH_ALLOW_UID is unset — relying on socket permissions only");
    }

    let listener = match build_listener(&cfg) {
        Ok(listener) => listener,
        Err(e) => {
            log(format!("fatal: cannot listen: {e}"));
            std::process::exit(1);
        }
    };
    log(format!("listening on {}", cfg.sock));

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let cfg = cfg.clone();
                std::thread::spawn(move || {
                    if let Err(e) = handle(stream, &cfg) {
                        log(format!("connection error: {e}"));
                    }
                });
            }
            Err(e) => log(format!("accept failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_blocks_one_user_after_the_cap() {
        let limiter = Limiter::new(Duration::from_secs(60), 2, 100);
        assert!(limiter.allow("alice"));
        assert!(limiter.allow("alice"));
        assert!(!limiter.allow("alice"));
        // A different user is unaffected by alice's cap.
        assert!(limiter.allow("bob"));
    }

    #[test]
    fn limiter_enforces_a_global_cap() {
        let limiter = Limiter::new(Duration::from_secs(60), 100, 2);
        assert!(limiter.allow("a"));
        assert!(limiter.allow("b"));
        assert!(!limiter.allow("c"));
    }

    #[test]
    fn protocol_never_echoes_the_password() {
        let line = Response::denied("Authentication failure").to_line();
        assert!(line.contains("denied"));
        assert!(!line.to_lowercase().contains("password"));
    }
}
