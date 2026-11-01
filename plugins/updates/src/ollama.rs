//! Ollama detection and update.
//!
//! Ollama is installed with the official `curl … | sh` script, and the same
//! script is the upgrade path. The pipeline is run as root (via `sudo -S`,
//! password on stdin).

use std::path::Path;
use std::time::Duration;

use crate::progress::Event;
use crate::exec::{self, CmdSpec};
use crate::model::OllamaStatus;

pub const DEFAULT_INSTALL_URL: &str = "https://ollama.com/install.sh";

/// Locate the `ollama` binary. The env override is useful for non-standard
/// installs and for tests.
pub fn binary() -> Option<String> {
    if let Ok(custom) = std::env::var("OLLAMA_BIN") {
        let custom = custom.trim();
        if !custom.is_empty() && Path::new(custom).is_file() {
            return Some(custom.to_string());
        }
    }
    if let Some(p) = exec::which("ollama") {
        return Some(p);
    }
    for candidate in ["/usr/local/bin/ollama", "/usr/bin/ollama", "/opt/ollama/ollama"] {
        if Path::new(candidate).is_file() {
            return Some(candidate.to_string());
        }
    }
    None
}

/// Parse `ollama --version` → `0.5.7`.
pub fn version() -> Option<String> {
    let bin = binary()?;
    let out = exec::run(
        &CmdSpec::new(bin).arg("--version"),
        None,
        Duration::from_secs(10),
    )
    .ok()?;
    if out.timed_out {
        return None;
    }
    parse_version(&out.stdout).or_else(|| parse_version(&out.stderr))
}

fn parse_version(text: &str) -> Option<String> {
    for token in text.split_whitespace() {
        let cleaned: String = token
            .trim_matches(|c: char| !c.is_ascii_digit() && c != '.')
            .to_string();
        if cleaned.split('.').count() >= 2 && cleaned.chars().next().is_some_and(|c| c.is_ascii_digit())
        {
            return Some(cleaned);
        }
    }
    None
}

/// The systemd unit that serves Ollama, when present.
pub fn service() -> Option<String> {
    if exec::which("systemctl").is_some() && Path::new("/etc/systemd/system/ollama.service").is_file()
    {
        Some("ollama.service".into())
    } else {
        None
    }
}

pub fn status() -> OllamaStatus {
    let bin = binary();
    OllamaStatus {
        installed: bin.is_some(),
        version: version(),
        service: service(),
        update_command: update_spec().display(),
        can_update: true,
        path: bin,
    }
}

/// Check that the files the current Ollama needs are actually on disk.
///
/// The official installer deletes `/usr/local/lib/ollama` and extracts the new
/// release into it. If that download/extract is interrupted (the machine froze
/// during one such update), the command can look finished while the directory
/// holds zero-byte CUDA libraries and no `llama-server`. Ollama then starts
/// with CPU-only fallback and logs `llama-server binary not found`. Verify the
/// install here, before reporting success.
pub fn verify_installation() -> Result<String, String> {
    let bin = binary().ok_or_else(|| "the ollama binary is missing".to_string())?;
    let version = version()
        .ok_or_else(|| format!("{bin} --version did not report a version"))?;

    let lib = Path::new("/usr/local/lib/ollama");
    if !lib.is_dir() {
        return Ok(version);
    }
    if let Some(path) = zero_sized_file(lib, 0) {
        return Err(format!(
            "incomplete install: {} is empty (the download/extract was interrupted)",
            path.display()
        ));
    }

    // 0.40.11 runs the model in a separate llama-server process. An install
    // from this series without that binary silently falls back to CPU.
    let needs_server = semver::Version::parse(&version)
        .map(|v| v >= semver::Version::new(0, 40, 11))
        .unwrap_or(false);
    let server = lib.join("llama-server");
    if needs_server && !server.is_file() {
        return Err(format!(
            "incomplete install: {} is missing",
            server.display()
        ));
    }

    Ok(version)
}

/// Find a zero-byte shared library or `llama-server` under `dir`.
fn zero_sized_file(dir: &Path, depth: usize) -> Option<std::path::PathBuf> {
    if depth > 4 {
        return None;
    }
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if let Some(found) = zero_sized_file(&path, depth + 1) {
                return Some(found);
            }
            continue;
        }
        if !kind.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name != "llama-server" && !name.contains(".so") {
            continue;
        }
        if entry.metadata().map(|m| m.len() == 0).unwrap_or(false) {
            return Some(path);
        }
    }
    None
}

/// The command that installs/upgrades Ollama.
///
/// `OLLAMA_UPDATE_CMD` overrides the whole pipeline; `OLLAMA_INSTALL_URL`
/// overrides just the script URL.
pub fn update_spec() -> CmdSpec {
    if let Ok(cmd) = std::env::var("OLLAMA_UPDATE_CMD") {
        let cmd = cmd.trim();
        if !cmd.is_empty() {
            return CmdSpec::new("sh").args(["-c", cmd]).root();
        }
    }
    let url = std::env::var("OLLAMA_INSTALL_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_INSTALL_URL.to_string());
    let quoted = shell_quote(url.trim());
    let fetch = if exec::which("curl").is_some() {
        format!("curl -fsSL {quoted}")
    } else if exec::which("wget").is_some() {
        format!("wget -qO- {quoted}")
    } else {
        format!("curl -fsSL {quoted}")
    };
    // The official installer ends with `systemctl restart ollama` (an EXIT
    // trap). Restarting the service from inside an interactive session can hang
    // the GPU/desktop on some machines (notably AMD/Vulkan), which freezes the
    // whole app even though the server stays up. `OLLAMA_NO_START` is only
    // honored on macOS, so we prepend a tiny `systemctl` shim to PATH for the
    // installer: it reports an unknown systemd state, so the installer skips
    // service configuration/restart entirely. The plugin owns the service
    // lifecycle and restores the unit file the installer rewrote on its way
    // through. The new binary is on disk; the service stays stopped until the
    // user starts it (or reboots).
    let pipeline = format!(
        r#"set -u
SHIM="$(mktemp -d)"
SYSTEMCTL="$(command -v systemctl || true)"
SERVICE=/etc/systemd/system/ollama.service
SAVED=""
if [ -f "$SERVICE" ]; then
    SAVED="$(mktemp)"
    cp -p "$SERVICE" "$SAVED"
fi
cat > "$SHIM/systemctl" <<'SHIMEOF'
#!/bin/sh
# The installer configures and restarts the service through systemctl. Doing
# that mid-session can hang the GPU and hard-freeze the whole desktop (seen on
# AMD/Vulkan machines). Report an unknown systemd state so the installer skips
# its service configuration entirely; the plugin owns the service lifecycle.
echo unknown
exit 0
SHIMEOF
chmod +x "$SHIM/systemctl"
PATH="$SHIM:$PATH" OLLAMA_NO_START=1 {fetch} | PATH="$SHIM:$PATH" OLLAMA_NO_START=1 sh
rc=$?
# Even with the systemctl shim the installer rewrites ollama.service before it
# checks systemd. That copy contains this temporary shim in PATH, which stops
# existing the moment this job ends. Put the machine's unit back.
if [ -n "$SAVED" ] && [ -f "$SAVED" ]; then
    cp -p "$SAVED" "$SERVICE"
    if [ -n "$SYSTEMCTL" ] && [ -x "$SYSTEMCTL" ]; then
        "$SYSTEMCTL" daemon-reload >/dev/null 2>&1 || true
    fi
fi
rm -rf "$SHIM"
[ -n "$SAVED" ] && rm -f "$SAVED"
exit $rc"#
    );
    // On a machine with an NVIDIA/AMD GPU and no driver the installer may run
    // the distro's package manager itself: keep that unattended too.
    CmdSpec::new("sh")
        .arg("-c")
        .arg(pipeline)
        .root()
        .env("DEBIAN_FRONTEND", "noninteractive")
        .env("NEEDRESTART_MODE", "a")
        .env("LC_ALL", "C")
}

/// Structured progress from the official installer's output.
///
/// The installer announces each step on stderr as `>>> Downloading Linux amd64
/// bundle`, and `curl --progress-bar` redraws a `####…  42.7%` bar with `\r`
/// (which `exec` splits into lines — hundreds of them per download). Steps
/// become [`Event::Step`] (logged), bar ticks become [`Event::Download`] (not
/// logged), so the window shows "Downloading · 42%" instead of a log flood.
pub fn progress_event(line: &str) -> Option<Event> {
    let line = line.trim();
    if let Some(step) = line.strip_prefix(">>>") {
        let step = step.trim();
        return (!step.is_empty()).then(|| Event::Step { message: step.to_string() });
    }
    if let Some(err) = line.strip_prefix("ERROR:") {
        return Some(Event::Error { package: None, message: err.trim().to_string() });
    }
    // curl's bar: `######                 12.3%`; before the size is known it
    // draws `#=#=#`, `-=O=-` frames instead, which carry no information.
    if !line.is_empty() && line.chars().all(|c| matches!(c, '#' | '=' | 'O' | '-' | ' ')) {
        return Some(Event::Download { percent: 0.0, message: String::new() });
    }
    if let Some(num) = line.strip_suffix('%') {
        let (bar, pct) = num.rsplit_once(' ').unwrap_or(("", num));
        if bar.chars().all(|c| c == '#' || c == ' ') {
            if let Ok(percent) = pct.trim().parse::<f32>() {
                return Some(Event::Download { percent, message: String::new() });
            }
        }
    }
    None
}

/// Stop the Ollama service before replacing its files. The installer deletes
/// and rewrites `/usr/local/lib/ollama` (the GPU libraries); doing that while
/// Ollama is running can hang the GPU and freeze the machine.
pub fn stop_spec() -> Option<CmdSpec> {
    if exec::which("systemctl").is_some() {
        Some(CmdSpec::new("systemctl").args(["stop", "ollama"]).root())
    } else {
        None
    }
}

/// Whether the Ollama service is currently active (best-effort).
pub fn is_active() -> bool {
    if exec::which("systemctl").is_none() {
        return false;
    }
    exec::run(
        &CmdSpec::new("systemctl").args(["is-active", "--quiet", "ollama"]),
        None,
        Duration::from_secs(10),
    )
    .map(|o| o.success)
    .unwrap_or(false)
}
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_installer_progress() {
        assert_eq!(
            progress_event(">>> Downloading Linux amd64 bundle"),
            Some(Event::Step { message: "Downloading Linux amd64 bundle".into() })
        );
        assert_eq!(
            progress_event("##########                                        14.3%"),
            Some(Event::Download { percent: 14.3, message: String::new() })
        );
        assert_eq!(progress_event("100.0%"), Some(Event::Download { percent: 100.0, message: String::new() }));
        assert!(matches!(progress_event("#=#=#"), Some(Event::Download { .. })));
        assert!(matches!(progress_event("ERROR: no GPU"), Some(Event::Error { .. })));
        assert_eq!(progress_event("WARNING: Unable to detect NVIDIA/AMD GPU."), None);
        assert_eq!(progress_event("disk is 95% full"), None);
    }
}
