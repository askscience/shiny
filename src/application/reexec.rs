//! Swap the running server for the binary now on disk without closing the
//! port.
//!
//! A package upgrade replaces `/usr/bin/shiny` while the old one keeps
//! running. Instead of a restart that drops the port — and with it every
//! request the page sends in between — the old process stops accepting,
//! drains what is in flight (the ordinary shutdown path), then `exec`s the
//! new binary **in the same process** with the listening socket handed over
//! as an inherited file descriptor. The kernel keeps the socket's accept
//! queue meanwhile, so the page sees a slow answer, not a refused connection.
//! The pid does not change, so the session supervisor and systemd notice
//! nothing.
//!
//! Child processes (speech sidecars) survive an `exec`; the new server finds
//! them on their ports as it does at any start.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::watch;

/// Environment variable carrying the inherited listener's descriptor.
pub const LISTEN_FD: &str = "SHINY_LISTEN_FD";

/// The request to swap, shared through `AppState`; `main` waits on it.
pub struct ReexecSignal {
    tx: watch::Sender<bool>,
    requested: AtomicBool,
}

impl Default for ReexecSignal {
    fn default() -> Self {
        Self::new()
    }
}

impl ReexecSignal {
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(false);
        Self {
            tx,
            requested: AtomicBool::new(false),
        }
    }

    /// Ask for the swap. Returns `true` the first time, `false` when one is
    /// already pending.
    pub fn request(&self) -> bool {
        let first = !self.requested.swap(true, Ordering::SeqCst);
        let _ = self.tx.send(true);
        first
    }

    pub fn requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }

    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.tx.subscribe()
    }
}

/// The path to run again: `/proc/self/exe` keeps working after the file was
/// replaced, but reads back with a ` (deleted)` suffix that `exec` would not
/// find; the clean path now holds the new binary.
pub fn exe_path() -> PathBuf {
    let current = std::env::current_exe().unwrap_or_default();
    let cleaned = strip_deleted(&current);
    if cleaned.is_file() {
        return cleaned;
    }
    std::env::args_os()
        .next()
        .map(PathBuf::from)
        .unwrap_or(cleaned)
}

fn strip_deleted(path: &std::path::Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_suffix(" (deleted)") {
        Some(clean) => PathBuf::from(clean),
        None => path.to_path_buf(),
    }
}

/// True when the binary this process started from is no longer the one on
/// disk: a package upgrade unlinked it (Linux reports the link as deleted) or
/// wrote a different file at the same path.
pub fn binary_replaced() -> bool {
    let current = std::env::current_exe().unwrap_or_default();
    if current.to_string_lossy().ends_with(" (deleted)") {
        return true;
    }
    let on_disk = crate::plugins::digest::file_digest(&current);
    on_disk != crate::api::system::build_id()
}

/// The listener handed over by the previous server, if this process was
/// started by one. The variable is cleared so a later swap starts clean.
pub fn inherited_listener() -> Option<std::net::TcpListener> {
    let raw = std::env::var(LISTEN_FD).ok()?;
    std::env::remove_var(LISTEN_FD);
    let fd: i32 = raw.trim().parse().ok()?;
    if fd < 0 {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::io::FromRawFd;
        // SAFETY: the descriptor was dup'd for exactly this purpose by the
        // previous server and nothing else in this process owns it.
        let listener = unsafe { std::net::TcpListener::from_raw_fd(fd) };
        // A listener that is not a socket (a stray number) must not become
        // the server: `local_addr` fails on anything else.
        listener.local_addr().ok()?;
        // The descriptor crossed `exec` without close-on-exec; put it back,
        // or every child this server spawns would hold the port open too.
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        Some(listener)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Replace this process with the binary on disk, passing `listener` along.
/// Only returns when the `exec` itself failed.
#[cfg(unix)]
pub fn exec_with_listener(listener: &std::net::TcpListener) -> std::io::Error {
    use std::os::unix::io::AsRawFd;
    use std::os::unix::process::CommandExt;
    let fd = listener.as_raw_fd();
    // The descriptor must outlive `exec`: clear close-on-exec.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, 0) } == -1 {
        return std::io::Error::last_os_error();
    }
    // The socket goes back to blocking: the new process sets its own mode.
    let _ = listener.set_nonblocking(false);
    let mut command = std::process::Command::new(exe_path());
    command.args(std::env::args_os().skip(1));
    command.env(LISTEN_FD, fd.to_string());
    command.exec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleted_suffix_is_stripped() {
        assert_eq!(
            strip_deleted(std::path::Path::new("/usr/bin/shiny (deleted)")),
            PathBuf::from("/usr/bin/shiny")
        );
        assert_eq!(
            strip_deleted(std::path::Path::new("/usr/bin/shiny")),
            PathBuf::from("/usr/bin/shiny")
        );
    }

    #[test]
    fn a_request_is_reported_once() {
        let signal = ReexecSignal::new();
        let mut rx = signal.subscribe();
        assert!(!signal.requested());
        assert!(signal.request());
        assert!(!signal.request());
        assert!(signal.requested());
        assert!(rx.has_changed().unwrap());
        assert!(*rx.borrow_and_update());
    }

    #[test]
    fn the_inherited_listener_comes_from_the_environment() {
        // One test for every shape of the variable: tests share the process
        // environment, so they must not run side by side.
        use std::os::unix::io::IntoRawFd;
        std::env::set_var(LISTEN_FD, "not-a-number");
        assert!(inherited_listener().is_none());
        std::env::set_var(LISTEN_FD, "-1");
        assert!(inherited_listener().is_none());

        let bound = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = bound.local_addr().unwrap().port();
        std::env::set_var(LISTEN_FD, bound.into_raw_fd().to_string());
        let inherited = inherited_listener().expect("listener from the environment");
        assert_eq!(inherited.local_addr().unwrap().port(), port);
        assert!(
            std::env::var(LISTEN_FD).is_err(),
            "the variable is consumed"
        );
        assert!(inherited_listener().is_none());
    }
}
