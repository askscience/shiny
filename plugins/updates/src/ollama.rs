//! Ollama detection and update.
//!
//! Ollama is installed with the official `curl … | sh` script, and the same
//! script is the upgrade path. The pipeline is run as root (via `sudo -S`,
//! password on stdin).

use std::path::Path;
use std::time::Duration;

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
    // installer: `is-system-running`/`daemon-reload`/`enable` pass through, but
    // start/stop/restart of `ollama` become a no-op. The new binary is on disk;
    // the running service keeps the old one until the user restarts it.
    let pipeline = format!(
        r#"set -u
SHIM="$(mktemp -d)"
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
rm -rf "$SHIM"
exit $rc"#
    );
    CmdSpec::new("sh").arg("-c").arg(pipeline).root()
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

pub fn restart_spec() -> Option<CmdSpec> {
    if exec::which("systemctl").is_some() {
        Some(CmdSpec::new("systemctl").args(["restart", "ollama"]).root())
    } else {
        None
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
