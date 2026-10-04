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

use std::io::{BufRead, BufReader, Read, Write};
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

/// Run a command with a timeout, invoking `on_line` for each stdout line as it
/// arrives. stderr is collected and appended to the returned [`Output`].
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
        cmd.arg("-S")
            .arg("-p")
            .arg("")
            .arg("--")
            .arg(&spec.program)
            .args(&spec.args);
        sudo_pw = Some(pw.as_bytes().to_vec());
    } else {
        cmd = Command::new(&spec.program);
        cmd.args(&spec.args);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
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

    // Feed the password to sudo, then close stdin. `sudo -S` reads one line.
    if let Some(pw) = sudo_pw.take() {
        if let Some(mut si) = child.stdin.take() {
            let _ = si.write_all(&pw);
            let _ = si.write_all(b"\n");
            let _ = si.flush();
        }
    }

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let (line_tx, line_rx) = mpsc::channel::<String>();
    let out_thread = stdout.map(|out| {
        std::thread::spawn(move || {
            let reader = BufReader::new(out);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        if line_tx.send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        })
    });
    let err_thread = stderr.map(|err| {
        std::thread::spawn(move || -> String {
            let mut buf = Vec::new();
            let _ = err.take(OUTPUT_CAP as u64).read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).into_owned()
        })
    });

    let deadline = Instant::now() + timeout;
    let mut stdout_accum = String::new();
    let mut timed_out = false;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            timed_out = true;
            break;
        }
        match line_rx.recv_timeout(remaining.min(Duration::from_millis(200))) {
            Ok(line) => {
                if stdout_accum.len() < OUTPUT_CAP {
                    stdout_accum.push_str(&line);
                    stdout_accum.push('\n');
                }
                on_line(&line);
            }
            Err(RecvTimeoutError::Timeout) => {
                if Instant::now() >= deadline {
                    timed_out = true;
                    break;
                }
            }
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

    if let Some(t) = out_thread {
        let _ = t.join();
    }
    let stderr_accum = err_thread.and_then(|t| t.join().ok()).unwrap_or_default();

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
