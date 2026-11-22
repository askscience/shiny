//! Installing what a Rust plugin needs when the machine lacks it.
//!
//! Two things can be fetched for the user, once they chose Rust on the card
//! that lists them:
//!
//! - the **Rust compiler**: `rustup-init` from the Rust project's own
//!   distribution, checked against its published SHA-256, run for the session
//!   user. It installs into their home (`~/.cargo`, `~/.rustup`): no
//!   administrator rights.
//! - the **C linker** rustc links through (`gcc` and the C library's
//!   development files): system packages, installed with `apt-get` through
//!   `sudo`. The password is the one the user types on the card: it is piped
//!   to `sudo -S` and never stored, logged or handed to the model.
//!
//! The commands are fixed. The plugin SDK is not fetched: it ships with the
//! server (`/usr/share/shiny/plugin-sdk`).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::services::agent_cancel::CancelHandle;

/// Where `rustup-init` and its checksum come from. A `file://` directory is
/// accepted, for tests.
pub const RUSTUP_DIST_ENV: &str = "SHINY_RUSTUP_DIST_URL";
const RUSTUP_DIST: &str = "https://static.rust-lang.org/rustup/dist";

const COMPILER_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const PACKAGES_TIMEOUT: Duration = Duration::from_secs(20 * 60);
/// The system packages that provide `cc` and what it links against.
const LINKER_PACKAGES: &[&str] = &["gcc", "libc6-dev"];

/// The rustup target triple of this machine, when rustup has a build for it.
fn host_triple() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("x86_64-unknown-linux-gnu"),
        "aarch64" => Some("aarch64-unknown-linux-gnu"),
        _ => None,
    }
}

async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    if let Some(path) = url.strip_prefix("file://") {
        return tokio::fs::read(path).await.map_err(|e| format!("{path}: {e}"));
    }
    let response = reqwest::Client::new()
        .get(url)
        .timeout(Duration::from_secs(300))
        .send()
        .await
        .map_err(|e| format!("could not reach {url}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("{url} answered {}", response.status()));
    }
    response.bytes().await.map(|bytes| bytes.to_vec()).map_err(|e| format!("download of {url} failed: {e}"))
}

/// Run a child to its end, or stop it with the turn or the deadline.
async fn finish(
    mut child: tokio::process::Child,
    what: &str,
    timeout: Duration,
    cancel: Option<&CancelHandle>,
) -> Result<(), String> {
    let mut stderr = child.stderr.take();
    let waited = async {
        match cancel {
            Some(handle) => tokio::select! {
                biased;
                _ = handle.cancelled() => None,
                status = child.wait() => Some(status),
            },
            None => Some(child.wait().await),
        }
    };
    let status = match tokio::time::timeout(timeout, waited).await {
        Ok(Some(Ok(status))) => status,
        Ok(Some(Err(error))) => return Err(format!("{what}: {error}")),
        Ok(None) => {
            let _ = child.kill().await;
            return Err(format!("{what} was stopped with the turn"));
        }
        Err(_) => {
            let _ = child.kill().await;
            return Err(format!("{what} ran out of time and was stopped"));
        }
    };
    if status.success() {
        return Ok(());
    }
    let mut text = String::new();
    if let Some(stderr) = stderr.as_mut() {
        use tokio::io::AsyncReadExt;
        let _ = stderr.read_to_string(&mut text).await;
    }
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(6)..].join("\n");
    Err(format!("{what} failed ({status}){}", if tail.is_empty() { String::new() } else { format!(":\n{tail}") }))
}

/// Download `rustup-init`, check it, and run it for the session user.
/// `work_dir` holds the download while it runs.
pub async fn install_compiler(work_dir: &Path, cancel: Option<&CancelHandle>) -> Result<(), String> {
    let triple = host_triple()
        .ok_or_else(|| format!("rustup has no installer for this machine ({})", std::env::consts::ARCH))?;
    let base = std::env::var(RUSTUP_DIST_ENV).ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| RUSTUP_DIST.into());
    let url = format!("{}/{triple}/rustup-init", base.trim_end_matches('/'));

    let checksum = fetch(&format!("{url}.sha256")).await?;
    let expected = String::from_utf8_lossy(&checksum)
        .split_whitespace()
        .next()
        .map(str::to_ascii_lowercase)
        .filter(|hash| hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| "the installer's published checksum could not be read".to_string())?;
    let installer = fetch(&url).await?;
    if hex::encode(Sha256::digest(&installer)) != expected {
        return Err("the downloaded installer does not match its published checksum. Nothing was run.".into());
    }

    tokio::fs::create_dir_all(work_dir).await.map_err(|e| e.to_string())?;
    let path = work_dir.join("rustup-init");
    tokio::fs::write(&path, &installer).await.map_err(|e| e.to_string())?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    let child = tokio::process::Command::new(&path)
        // The minimal profile: the compiler, cargo and the standard library.
        .args(["-y", "--profile", "minimal", "--default-toolchain", "stable", "--no-modify-path"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("could not start the Rust installer: {e}"));
    let result = match child {
        Ok(child) => finish(child, "the Rust installer", COMPILER_TIMEOUT, cancel).await,
        Err(error) => Err(error),
    };
    let _ = tokio::fs::remove_file(&path).await;
    result
}

fn program(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).map(|dir| dir.join(name)).find(|candidate| candidate.is_file())
}

/// Whether the server runs as root (it normally runs as the desktop user).
fn is_root() -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").is_ok_and(|meta| meta.uid() == 0)
}

/// Whether installing system packages will need the user's password: not as
/// root, and not when `sudo` already lets this account through.
pub async fn needs_password() -> bool {
    if is_root() {
        return false;
    }
    let Some(sudo) = program("sudo") else { return true };
    let allowed = tokio::process::Command::new(sudo)
        .args(["-n", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok_and(|status| status.success());
    !allowed
}

/// Install the C linker's packages with the system's package manager.
///
/// `password` is piped to `sudo -S` and dropped: it is in no argument, no
/// environment variable and no log line.
pub async fn install_linker(password: Option<&str>, cancel: Option<&CancelHandle>) -> Result<(), String> {
    let apt = program("apt-get").ok_or_else(|| {
        "this system has no apt-get: install a C compiler (gcc) with its own package manager, then choose Rust again"
            .to_string()
    })?;
    let mut command = if is_root() {
        tokio::process::Command::new(&apt)
    } else {
        let sudo = program("sudo").ok_or_else(|| {
            "sudo is not installed: install gcc as administrator, then choose Rust again".to_string()
        })?;
        let mut command = tokio::process::Command::new(sudo);
        // -S: the password comes on stdin; -p '': no prompt text; -k: never a
        // cached credential from another terminal, the user answers for this.
        command.args(["-S", "-k", "-p", "", "--"]).arg(&apt);
        command
    };
    command
        .args(["install", "-y", "--no-install-recommends"])
        .args(LINKER_PACKAGES)
        .env("DEBIAN_FRONTEND", "noninteractive")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|e| format!("could not start the package installation: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        if let Some(password) = password {
            let _ = stdin.write_all(password.as_bytes()).await;
            let _ = stdin.write_all(b"\n").await;
        }
        // Closed either way: a wrong password must fail, not wait for another.
        drop(stdin);
    }
    finish(child, "installing the C compiler", PACKAGES_TIMEOUT, cancel).await.map_err(|error| {
        if error.contains("incorrect password") || error.contains("Sorry, try again") || error.contains("no password was provided") {
            "the administrator password was not accepted. Nothing was installed.".to_string()
        } else {
            error
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real `sudo apt-get install` on the machine the test runs on.
    /// Ignored by default: it needs an administrator's password, given in
    /// `SHINY_TEST_SUDO_PASSWORD`, and it installs gcc when it is missing.
    #[tokio::test]
    #[ignore = "runs sudo apt-get on this machine"]
    async fn the_linker_is_installed_through_sudo_with_the_given_password() {
        let password = std::env::var("SHINY_TEST_SUDO_PASSWORD").expect("SHINY_TEST_SUDO_PASSWORD");
        let wrong = install_linker(Some("certainly-not-the-password"), None).await.unwrap_err();
        assert_eq!(wrong, "the administrator password was not accepted. Nothing was installed.");
        install_linker(Some(&password), None).await.unwrap();
        assert!(program("cc").is_some());
    }

    #[tokio::test]
    async fn a_download_that_does_not_match_its_checksum_is_never_run() {
        let dist = tempfile::tempdir().unwrap();
        let triple = host_triple().expect("a rustup triple for the test machine");
        let dir = dist.path().join(triple);
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dist.path().join("ran");
        let script = format!("#!/bin/sh\necho \"$@\" > {}\n", marker.display());
        std::fs::write(dir.join("rustup-init"), &script).unwrap();
        let work = dist.path().join("work");

        // One test, in sequence: the override is process-wide.
        std::env::set_var(RUSTUP_DIST_ENV, format!("file://{}", dist.path().display()));

        std::fs::write(dir.join("rustup-init.sha256"), format!("{} *./rustup-init\n", "0".repeat(64))).unwrap();
        let error = install_compiler(&work, None).await.unwrap_err();
        assert!(error.contains("does not match its published checksum"), "{error}");
        assert!(!marker.exists(), "a tampered installer is not run");

        std::fs::write(dir.join("rustup-init.sha256"), "not a checksum\n").unwrap();
        assert!(install_compiler(&work, None).await.unwrap_err().contains("checksum could not be read"));

        let good = hex::encode(Sha256::digest(script.as_bytes()));
        std::fs::write(dir.join("rustup-init.sha256"), format!("{good} *./rustup-init\n")).unwrap();
        install_compiler(&work, None).await.unwrap();
        let ran = std::fs::read_to_string(&marker).unwrap();
        assert_eq!(ran.trim(), "-y --profile minimal --default-toolchain stable --no-modify-path");
        assert!(!work.join("rustup-init").exists(), "the installer is removed after it ran");

        // An installer that fails says so with its own last words.
        let failing = "#!/bin/sh\necho 'no space left' >&2\nexit 3\n";
        std::fs::write(dir.join("rustup-init"), failing).unwrap();
        let hash = hex::encode(Sha256::digest(failing.as_bytes()));
        std::fs::write(dir.join("rustup-init.sha256"), format!("{hash} *./rustup-init\n")).unwrap();
        let error = install_compiler(&work, None).await.unwrap_err();
        assert!(error.contains("the Rust installer failed") && error.contains("no space left"), "{error}");

        std::env::remove_var(RUSTUP_DIST_ENV);
    }
}
