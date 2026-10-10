//! `shiny-auth` — the privileged Linux-PAM verification helper for Shiny.
//!
//! Runs as **root** (systemd socket-activated) and does exactly two things:
//! answer "is this the right password for this Linux user?" over a local Unix
//! socket, and — when the greeter asks — start that user's kiosk session. It
//! opens no session itself, touches no files, and never logs or returns the
//! password. The Shiny web server — which must never run as root — talks to it
//! through `src/services/auth_helper.rs`.
//!
//! Hardening:
//!   * the socket is mode 0660 root:shiny and `SO_PEERCRED` restricts callers
//!     to the uids in `SHINY_AUTH_ALLOW_UID` (comma-separated; root always);
//!   * per-user and global rate limits bound password guessing;
//!   * a per-connection read/write timeout bounds slow-loris clients;
//!   * request size is capped;
//!   * only `verify`, `login-session` and `ping` are implemented;
//!   * `login-session` starts `shiny-kiosk@<user>.service` for the account PAM
//!     just verified — never for an account named in the request it did not
//!     verify — and only for real login accounts (uid 1000..65533, login
//!     shell). `SHINY_AUTH_DRY_RUN=1` reports the unit instead of starting it.
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
use std::process::Command;
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
    /// Uids allowed to call the socket (`SHINY_AUTH_ALLOW_UID`, comma list).
    /// Empty means "socket permissions only". Root is always allowed.
    allow_uids: Vec<u32>,
    /// systemd unit started by `login-session`, `%s` = the verified account.
    kiosk_unit: String,
    systemctl: String,
    /// Marker written before a session unit starts. The greeter's stop script
    /// checks it: a stop with the marker present is a handover (leave the seat
    /// alone), a stop without it is a failure (restore the console).
    handover_file: String,
    /// Report the unit instead of starting it — installer/test use.
    dry_run: bool,
    timeout: Duration,
    max_line: usize,
    limiter: Limiter,
}

impl Config {
    fn from_env() -> Self {
        Self {
            sock: std::env::var("SHINY_AUTH_SOCK")
                .unwrap_or_else(|_| "/run/shiny/auth.sock".into()),
            service: std::env::var("SHINY_AUTH_PAM_SERVICE")
                .unwrap_or_else(|_| "shiny".into()),
            allow_uids: std::env::var("SHINY_AUTH_ALLOW_UID")
                .map(|v| parse_allow_uids(&v))
                .unwrap_or_default(),
            kiosk_unit: std::env::var("SHINY_AUTH_KIOSK_UNIT")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "shiny-kiosk@%s.service".into()),
            systemctl: std::env::var("SHINY_AUTH_SYSTEMCTL")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "/usr/bin/systemctl".into()),
            handover_file: std::env::var("SHINY_AUTH_HANDOVER_FILE")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "/run/shiny/handover".into()),
            dry_run: std::env::var("SHINY_AUTH_DRY_RUN")
                .map(|v| v.trim() == "1" || v.trim().eq_ignore_ascii_case("true"))
                .unwrap_or(false),
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

/// Parse `SHINY_AUTH_ALLOW_UID`: one uid or a comma-separated list.
fn parse_allow_uids(value: &str) -> Vec<u32> {
    value
        .split(',')
        .filter_map(|v| v.trim().parse().ok())
        .collect()
}

fn uid_allowed(allow: &[u32], uid: u32) -> bool {
    uid == 0 || allow.contains(&uid)
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
            if !cfg.allow_uids.is_empty() && !uid_allowed(&cfg.allow_uids, uid) {
                let _ = write_response(
                    &stream,
                    &Response::error("forbidden", "caller is not permitted"),
                );
                log(format!("rejected connection from uid {uid}"));
                return Ok(());
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
        "login-session" => login_session(&req, cfg),
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

/// `login-session` — verify the password, then start that account's kiosk
/// session. This is the one place a session is *created*, and it is always the
/// account PAM just verified: the caller cannot name a unit, only a user.
fn login_session(req: &Request, cfg: &Config) -> Response {
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
        Ok(()) => {}
        Err(PamError::Denied(code, message)) => {
            log(format!("denied: {user} (pam {code}: {message})"));
            return Response::denied(message);
        }
        Err(PamError::Unavailable(message)) => {
            log(format!("unavailable: {message}"));
            return Response::unavailable(message);
        }
    }

    // Root and service accounts have passwords too; none of them gets a seat.
    if !human_account(&user) {
        log(format!("refused session for non-human account: {user}"));
        return Response::denied("not a login account");
    }
    let Some(unit) = kiosk_unit_for(&cfg.kiosk_unit, &user) else {
        log(format!("refused session for unusable user name: {user}"));
        return Response::denied("not a valid account name");
    };

    if cfg.dry_run {
        log(format!("dry run: would start {unit}"));
        return Response::ok_with(format!("would start {unit}"));
    }

    // Tell the greeter's stop script that the seat is being handed over (it
    // skips its console fallback). The kiosk unit removes the marker when its
    // session ends, so a stale marker cannot outlive a session.
    let _ = std::fs::write(&cfg.handover_file, b"kiosk\n");

    log(format!("starting session: {unit}"));
    match Command::new(&cfg.systemctl)
        .arg("--no-block")
        .arg("start")
        .arg(&unit)
        .status()
    {
        Ok(status) if status.success() => Response::ok_with(format!("starting {unit}")),
        Ok(status) => {
            log(format!("failed to start {unit}: systemctl {status}"));
            let _ = std::fs::remove_file(&cfg.handover_file);
            Response::error("session_start", format!("could not start {unit}"))
        }
        Err(e) => {
            log(format!("failed to run {}: {e}", cfg.systemctl));
            let _ = std::fs::remove_file(&cfg.handover_file);
            Response::error("session_start", "could not start the session")
        }
    }
}

/// Build the kiosk unit name for a validated account. The username becomes the
/// systemd instance, so nothing but characters systemd accepts may get through
/// (the account already exists in NSS by this point; this is defence in depth).
fn kiosk_unit_for(template: &str, user: &str) -> Option<String> {
    let acceptable = !user.is_empty()
        && user.len() <= 64
        && user
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
    acceptable.then(|| template.replace("%s", user))
}

/// The uid/shell predicate for "a real login account", mirroring
/// `UnixUser::is_human` in the server (`src/services/unix_user.rs`).
fn is_human_account(uid: u32, shell: &str) -> bool {
    uid >= 1000
        && uid < 65534
        && !shell.is_empty()
        && !shell.ends_with("nologin")
        && !shell.ends_with("/false")
        && !shell.ends_with("/sync")
}

/// Look the account up in NSS and apply [`is_human_account`]. `getpwnam_r`
/// into a stack buffer, like the server's resolver: fails closed.
fn human_account(name: &str) -> bool {
    let Ok(name) = std::ffi::CString::new(name) else {
        return false;
    };
    let mut buf = [0 as libc::c_char; 16 * 1024];
    let mut passwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe {
        libc::getpwnam_r(
            name.as_ptr(),
            &mut passwd,
            buf.as_mut_ptr(),
            buf.len(),
            &mut result,
        )
    };
    if rc != 0 || result.is_null() {
        return false;
    }
    let shell = unsafe { std::ffi::CStr::from_ptr(passwd.pw_shell) }
        .to_string_lossy()
        .into_owned();
    is_human_account(passwd.pw_uid, &shell)
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
    if cfg.allow_uids.is_empty() {
        log("WARNING: SHINY_AUTH_ALLOW_UID is unset — relying on socket permissions only");
    }
    if cfg.dry_run {
        log(format!(
            "dry run: login-session would start `{}`",
            cfg.kiosk_unit
        ));
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

    #[test]
    fn allow_uids_accepts_one_or_a_list() {
        assert_eq!(parse_allow_uids("1000"), vec![1000]);
        assert_eq!(parse_allow_uids("1000, 999 ,"), vec![1000, 999]);
        assert!(parse_allow_uids("").is_empty());
        assert!(uid_allowed(&[1000, 999], 999));
        assert!(uid_allowed(&[1000], 0), "root is always allowed");
        assert!(!uid_allowed(&[1000], 1001));
    }

    #[test]
    fn session_unit_is_built_from_the_validated_name() {
        assert_eq!(
            kiosk_unit_for("shiny-kiosk@%s.service", "eev").as_deref(),
            Some("shiny-kiosk@eev.service")
        );
        // The username becomes a systemd instance: no injections, no paths,
        // no stray `@`.
        for bad in ["eev/../root", "eev; rm -rf /", "eev@x", "eev root", "", &"a".repeat(65)] {
            assert!(kiosk_unit_for("shiny-kiosk@%s.service", bad).is_none());
        }
    }

    #[test]
    fn human_accounts_only() {
        assert!(is_human_account(1000, "/bin/bash"));
        assert!(!is_human_account(0, "/bin/bash"), "root has no seat here");
        assert!(!is_human_account(1000, "/usr/sbin/nologin"));
        assert!(!is_human_account(1000, "/bin/false"));
        assert!(!is_human_account(1000, "/bin/sync"));
        assert!(!is_human_account(999, "/bin/bash"), "system uid");
        assert!(!is_human_account(65534, "/bin/bash"), "nobody");
    }
}
