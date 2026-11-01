//! External command execution for the Updates plugin.
//!
//! Two hard rules live here:
//!
//! 1. **Elevation is password-based.** When a command needs root and the server
//!    is not already root, it is wrapped in `sudo -S` and the caller's password
//!    is written to `sudo`'s stdin. `sudo -S -p ''` keeps the prompt silent.
//!    The password is never placed on the command line, never persisted, and
//!    never logged (`CmdSpec::display` shows only the wrapped command).
//! 2. **Every child has a deadline.** A hung package manager must not pin a
//!    worker; stdout is streamed line-by-line to the callback and the child is
//!    killed once the timeout passes.

use std::io::{Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use shiny_plugin_sdk::errors::AppError;

/// Per-stream output ceiling (defensive: a hostile/broken manager can't balloon
/// plugin memory).
const OUTPUT_CAP: usize = 1024 * 1024;

/// A command the plugin wants to run.
#[derive(Debug, Clone)]
pub struct CmdSpec {
    pub program: String,
    pub args: Vec<String>,
    /// Wrap in `sudo` when the process is not already root.
    pub root: bool,
    pub env: Vec<(String, String)>,
    /// Extra exit codes to treat as success (e.g. `dnf check-update` → 100).
    pub ok_codes: Vec<i32>,
}

impl CmdSpec {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            root: false,
            env: Vec::new(),
            ok_codes: Vec::new(),
        }
    }

    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.args.push(a.into());
        self
    }

    pub fn args<I, S>(mut self, it: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for a in it {
            self.args.push(a.into());
        }
        self
    }

    pub fn root(mut self) -> Self {
        self.root = true;
        self
    }

    pub fn env(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.env.push((k.into(), v.into()));
        self
    }

    pub fn ok(mut self, code: i32) -> Self {
        self.ok_codes.push(code);
        self
    }

    /// The command as shown to the user / written to the log. Never contains a
    /// password (that travels on stdin).
    pub fn display(&self) -> String {
        let mut s = String::new();
        if self.root && !is_root() {
            s.push_str("sudo ");
        }
        s.push_str(&self.program);
        for a in &self.args {
            s.push(' ');
            if a.contains(' ') || a.contains('"') {
                s.push('"');
                s.push_str(&a.replace('"', "\\\""));
                s.push('"');
            } else {
                s.push_str(a);
            }
        }
        s
    }
}

#[derive(Debug, Clone, Default)]
pub struct Output {
    pub code: Option<i32>,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

/// Effective-uid check, so a server that already runs as root is not wrapped in
/// `sudo` (which may not even be installed).
pub fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

/// Resolve a bare binary name against `PATH` (or accept an absolute path).
pub fn which(bin: &str) -> Option<String> {
    if bin.contains('/') {
        return if std::path::Path::new(bin).is_file() {
            Some(bin.to_string())
        } else {
            None
        };
    }
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        let cand = dir.join(bin);
        if cand.is_file() {
            return Some(cand.to_string_lossy().into_owned());
        }
    }
    None
}

/// Run a command, discarding its output. Convenience wrapper over
/// [`run_streaming`].
pub fn run(spec: &CmdSpec, password: Option<&str>, timeout: Duration) -> Result<Output, AppError> {
    run_streaming(spec, password, timeout, &mut |_| {})
}

fn wait_deadline(child: &mut Child, deadline: Instant) -> Option<ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(_) => return None,
        }
    }
}

/// Lower the child's CPU and I/O priority before it starts. A system upgrade
/// (dpkg, DKMS module builds, initramfs regeneration) otherwise competes with
/// the kiosk webview at full priority and the whole desktop stutters or
/// freezes. Niceness and the I/O class are inherited through `sudo` and by
/// every process the package manager spawns.
fn lower_priority(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        cmd.pre_exec(|| {
            // Best effort: failure only means normal priority.
            libc::setpriority(libc::PRIO_PROCESS, 0, 10);
            // IOPRIO_WHO_PROCESS = 1; best-effort class (2) at its lowest level (7).
            const IOPRIO_CLASS_BE: libc::c_long = 2;
            libc::syscall(libc::SYS_ioprio_set, 1, 0, (IOPRIO_CLASS_BE << 13) | 7);
            Ok(())
        });
    }
}

/// Split a raw output chunk into lines. Both `\n` and `\r` end a line (apt and
/// wget redraw progress with `\r`), and invalid UTF-8 is replaced instead of
/// stopping the reader — a reader that stops leaves the pipe full and the
/// package manager blocked on its next write.
fn pump_lines(mut src: impl Read, tx: mpsc::Sender<Stream>, wrap: fn(String) -> Stream) {
    let mut buf = [0u8; 8192];
    let mut pending: Vec<u8> = Vec::new();
    loop {
        let n = match src.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        for &b in &buf[..n] {
            if b == b'\n' || b == b'\r' {
                if !pending.is_empty() {
                    let line = String::from_utf8_lossy(&pending).into_owned();
                    pending.clear();
                    // The receiver gone means the run is over; keep draining
                    // anyway so the child never blocks on a full pipe.
                    let _ = tx.send(wrap(line));
                }
            } else if pending.len() < 16 * 1024 {
                pending.push(b);
            }
        }
    }
    if !pending.is_empty() {
        let _ = tx.send(wrap(String::from_utf8_lossy(&pending).into_owned()));
    }
}

enum Stream {
    Out(String),
    Err(String),
}

/// Run a command with a timeout, invoking `on_line` for each output line as it
/// arrives (stdout and stderr, interleaved in arrival order). Both streams are
/// drained to the end no matter how much they print; only the copies kept in
/// the returned [`Output`] are capped.
pub fn run_streaming(
    spec: &CmdSpec,
    password: Option<&str>,
    timeout: Duration,
    on_line: &mut dyn FnMut(&str),
) -> Result<Output, AppError> {
    let use_sudo = spec.root && !is_root();

    let mut cmd;
    let mut sudo_pw: Option<Vec<u8>> = None;
    if use_sudo {
        let pw = password
            .filter(|p| !p.is_empty())
            .ok_or_else(|| {
                AppError::BadRequest(
                    "Administrator (sudo) password required to run this update".into(),
                )
            })?;
        if which("sudo").is_none() {
            return Err(AppError::Internal(
                "sudo is not installed; cannot elevate to run this update".into(),
            ));
        }
        cmd = Command::new("sudo");
        cmd.arg("-S").arg("-p").arg("").arg("--");
        // sudo's env_reset drops the caller's environment, so variables such
        // as DEBIAN_FRONTEND=noninteractive must travel through `env` on the
        // elevated side — otherwise debconf/dpkg try to ask questions on a
        // terminal that does not exist.
        if !spec.env.is_empty() {
            cmd.arg("env");
            for (k, v) in &spec.env {
                cmd.arg(format!("{k}={v}"));
            }
        }
        cmd.arg(&spec.program).args(&spec.args);
        sudo_pw = Some(pw.as_bytes().to_vec());
    } else {
        cmd = Command::new(&spec.program);
        cmd.args(&spec.args);
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
    }
    if spec.root {
        lower_priority(&mut cmd);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.stdin(if sudo_pw.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AppError::Internal(format!("`{}` is not installed", spec.program))
        } else {
            AppError::Internal(format!("failed to run `{}`: {e}", spec.program))
        }
    })?;

    // Feed the password to sudo, then close stdin (EOF): `sudo -S` reads one
    // line, and anything that later tries to prompt gets EOF instead of hanging.
    if let Some(pw) = sudo_pw.take() {
        if let Some(mut si) = child.stdin.take() {
            let _ = si.write_all(&pw);
            let _ = si.write_all(b"\n");
            let _ = si.flush();
        }
    }

    let (line_tx, line_rx) = mpsc::channel::<Stream>();
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        let tx = line_tx.clone();
        readers.push(std::thread::spawn(move || pump_lines(out, tx, Stream::Out)));
    }
    if let Some(err) = child.stderr.take() {
        let tx = line_tx.clone();
        readers.push(std::thread::spawn(move || pump_lines(err, tx, Stream::Err)));
    }
    drop(line_tx);

    let deadline = Instant::now() + timeout;
    let mut stdout_accum = String::new();
    let mut stderr_accum = String::new();
    let mut timed_out = false;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            timed_out = true;
            break;
        }
        match line_rx.recv_timeout(remaining.min(Duration::from_millis(200))) {
            Ok(Stream::Out(line)) => {
                if stdout_accum.len() < OUTPUT_CAP {
                    stdout_accum.push_str(&line);
                    stdout_accum.push('\n');
                }
                on_line(&line);
            }
            Ok(Stream::Err(line)) => {
                if stderr_accum.len() < OUTPUT_CAP {
                    stderr_accum.push_str(&line);
                    stderr_accum.push('\n');
                }
                on_line(&line);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    let mut exit_status = None;
    if !timed_out {
        exit_status = wait_deadline(&mut child, deadline);
        if exit_status.is_none() {
            timed_out = true;
        }
    }
    if timed_out {
        let _ = child.kill();
        let _ = child.wait();
    }
    // Once the child is gone its pipes close and the readers finish. A
    // grandchild that inherited the pipe could keep one open, so do not wait
    // on readers after a timeout.
    if !timed_out {
        for r in readers {
            let _ = r.join();
        }
    }

    let code = exit_status.and_then(|s| s.code());
    let mut success = false;
    if !timed_out {
        if let Some(status) = exit_status {
            success = status.success()
                || code
                    .map(|c| spec.ok_codes.contains(&c))
                    .unwrap_or(false);
        }
    }

    Ok(Output {
        code,
        success,
        stdout: stdout_accum,
        stderr: stderr_accum,
        timed_out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_invalid_utf8_and_carriage_returns() {
        let spec = CmdSpec::new("sh").args(["-c", r"printf 'a\rb\n\377c\nlast'; printf 'e1\n' >&2"]);
        let mut seen = Vec::new();
        let out = run_streaming(&spec, None, Duration::from_secs(10), &mut |l| seen.push(l.to_string())).unwrap();
        assert!(out.success);
        assert!(seen.contains(&"a".to_string()));
        assert!(seen.contains(&"b".to_string()));
        assert!(seen.contains(&"\u{FFFD}c".to_string()));
        assert!(seen.contains(&"last".to_string()));
        assert!(seen.contains(&"e1".to_string()));
        assert_eq!(out.stderr.trim(), "e1");
    }

    #[test]
    fn drains_large_stderr_without_blocking() {
        // 4 MiB on stderr: far past OUTPUT_CAP; the child must still exit 0.
        let spec = CmdSpec::new("sh").args(["-c", "head -c 4194304 /dev/zero | tr '\\0' 'x' | fold -w 100 >&2; exit 0"]);
        let out = run_streaming(&spec, None, Duration::from_secs(30), &mut |_| {}).unwrap();
        assert!(out.success, "{out:?}");
        assert!(!out.timed_out);
    }
}
