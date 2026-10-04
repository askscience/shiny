//! Distribution detection and the per-package-manager strategy.
//!
//! Every supported family has its **own module** (`apt`, `dnf`, `pacman`,
//! `zypper`, `apk`, `xbps`, `emerge`, `nix`) implementing [`PackageManager`].
//! [`detect`] reads `/etc/os-release`, matches `ID` / `ID_LIKE` against each
//! family, and — if that is inconclusive — falls back to probing for the
//! package-manager binaries on `PATH`. Adding a distribution therefore means
//! adding one file and one line to the mapping; nothing else changes.

use crate::exec::{self, CmdSpec};
use crate::model::PackageUpdate;

pub mod apk;
pub mod apt;
pub mod dnf;
pub mod emerge;
pub mod nix;
pub mod pacman;
pub mod xbps;
pub mod zypper;

/// The operations a package manager must expose. All commands are returned as
/// [`CmdSpec`] so the caller (which owns the password and the timeout) decides
/// how to run them.
pub trait PackageManager: Send + Sync {
    /// Stable id, e.g. `apt`.
    fn id(&self) -> &'static str;
    /// Human label, e.g. `APT (Debian/Ubuntu)`.
    fn label(&self) -> &'static str;
    /// Primary binary used to probe for this manager on `PATH`.
    fn binary(&self) -> &'static str;
    /// False only for the [`Unknown`] fallback.
    fn supported(&self) -> bool {
        true
    }
    /// Refresh metadata (the command that needs root on most distros).
    fn refresh(&self) -> Option<CmdSpec>;
    /// List pending upgrades in a machine-parseable form.
    fn list(&self) -> CmdSpec;
    /// Parse the stdout of [`PackageManager::list`].
    fn parse(&self, stdout: &str) -> Vec<PackageUpdate>;
    /// Upgrade every package.
    fn upgrade_all(&self) -> CmdSpec;
    /// Upgrade a single package, when the manager supports it.
    fn upgrade_one(&self, name: &str) -> Option<CmdSpec>;
}

/// The result of parsing `/etc/os-release`.
#[derive(Debug, Clone, Default)]
pub struct OsRelease {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub id_like: Vec<String>,
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    let v = v.strip_prefix('"').unwrap_or(v);
    let v = v.strip_suffix('"').unwrap_or(v);
    v.strip_prefix('\'').unwrap_or(v).strip_suffix('\'').unwrap_or(v).to_string()
}

fn parse_os_release(text: &str) -> OsRelease {
    let mut os = OsRelease {
        name: "Linux".into(),
        ..Default::default()
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = unquote(value);
        match key.trim() {
            "ID" => os.id = value.to_lowercase(),
            "NAME" => os.name = value,
            "VERSION_ID" => os.version = Some(value),
            "ID_LIKE" => {
                os.id_like = value
                    .split_whitespace()
                    .map(|t| t.to_lowercase())
                    .collect();
            }
            _ => {}
        }
    }
    if os.id.is_empty() {
        os.id = "unknown".into();
    }
    os
}

/// Read `/etc/os-release` (falling back to `/usr/lib/os-release`).
pub fn read_os_release() -> OsRelease {
    for path in ["/etc/os-release", "/usr/lib/os-release"] {
        if let Ok(text) = std::fs::read_to_string(path) {
            return parse_os_release(&text);
        }
    }
    OsRelease {
        id: "unknown".into(),
        name: "Unknown Linux".into(),
        version: None,
        id_like: Vec::new(),
    }
}

/// Map one `os-release` token to a manager module.
fn manager_for_token(token: &str) -> Option<Box<dyn PackageManager>> {
    match token {
        "debian" | "ubuntu" | "linuxmint" | "pop" | "pop_os" | "kali" | "raspbian"
        | "devuan" | "elementary" | "zorin" | "neon" | "deepin" | "mx" | "parrot"
        | "pureos" | "tuxedo" | "ubuntu-core" => Some(Box::new(apt::Apt)),

        "fedora" | "rhel" | "centos" | "rocky" | "almalinux" | "ol" | "scientific"
        | "amzn" | "nobara" | "bazzite" | "centos_stream" => Some(Box::new(dnf::Dnf)),

        "arch" | "manjaro" | "endeavouros" | "garuda" | "cachyos" | "artix"
        | "arcolinux" | "archcraft" | "rebornos" => Some(Box::new(pacman::Pacman)),

        "opensuse" | "opensuse-leap" | "opensuse-tumbleweed" | "suse" | "sled"
        | "opensuse-microos" => Some(Box::new(zypper::Zypper)),

        "alpine" | "postmarketos" | "pmos" => Some(Box::new(apk::Apk)),

        "void" => Some(Box::new(xbps::Xbps)),

        "gentoo" | "funtoo" | "calculate" | "sabayon" => Some(Box::new(emerge::Emerge)),

        "nixos" | "nix" => Some(Box::new(nix::Nix)),

        _ => None,
    }
}

/// The package manager for the running system.
///
/// Detection order: `os-release` `ID`, then each `ID_LIKE` token, then a probe
/// for the known manager binaries on `PATH`, then [`Unknown`].
pub fn detect() -> Box<dyn PackageManager> {
    let os = read_os_release();

    let mut tokens = Vec::new();
    if !os.id.is_empty() {
        tokens.push(os.id.clone());
    }
    tokens.extend(os.id_like.iter().cloned());

    for token in &tokens {
        if let Some(m) = manager_for_token(token) {
            return m;
        }
    }

    // os-release gave us nothing usable — fall back to the binary that is
    // actually installed (covers minimal containers and derivatives with an
    // unusual ID).
    if exec::which("apt-get").is_some() {
        return Box::new(apt::Apt);
    }
    if exec::which("dnf").is_some() || exec::which("yum").is_some() {
        return Box::new(dnf::Dnf);
    }
    if exec::which("pacman").is_some() {
        return Box::new(pacman::Pacman);
    }
    if exec::which("zypper").is_some() {
        return Box::new(zypper::Zypper);
    }
    if exec::which("apk").is_some() {
        return Box::new(apk::Apk);
    }
    if exec::which("xbps-install").is_some() {
        return Box::new(xbps::Xbps);
    }
    if exec::which("emerge").is_some() {
        return Box::new(emerge::Emerge);
    }
    if exec::which("nix-env").is_some() || exec::which("nixos-rebuild").is_some() {
        return Box::new(nix::Nix);
    }

    Box::new(Unknown)
}

/// Last-resort manager: detection failed. Reports no updates and refuses writes.
struct Unknown;

impl PackageManager for Unknown {
    fn id(&self) -> &'static str {
        "unknown"
    }
    fn label(&self) -> &'static str {
        "Unsupported distribution"
    }
    fn binary(&self) -> &'static str {
        ""
    }
    fn supported(&self) -> bool {
        false
    }
    fn refresh(&self) -> Option<CmdSpec> {
        None
    }
    fn list(&self) -> CmdSpec {
        // A no-op that always succeeds, so a status query returns an empty
        // list rather than an error; `supported() == false` carries the
        // real signal.
        CmdSpec::new("true")
    }
    fn parse(&self, _stdout: &str) -> Vec<PackageUpdate> {
        Vec::new()
    }
    fn upgrade_all(&self) -> CmdSpec {
        CmdSpec::new("false")
    }
    fn upgrade_one(&self, _name: &str) -> Option<CmdSpec> {
        None
    }
}

// ── Shared parsing helpers used by several managers ─────────────────────────

/// Split `name-version` where `version` is defined as the first `-` followed
/// by a digit and containing a `.` or `_` somewhere after it.
///
/// Package names may themselves contain digits and dashes (`libx11`,
/// `gcc-12`), so a plain `rsplit` is wrong; versions reliably contain a dot or
/// the `_<revision>` suffix used by Alpine/Void. Returns `(name, version)`.
pub(crate) fn split_name_version(s: &str) -> (String, Option<String>) {
    let bytes = s.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if *b != b'-' {
            continue;
        }
        let rest = &s[i + 1..];
        let starts_digit = rest.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false);
        let looks_version = rest.contains('.') || rest.contains('_');
        if starts_digit && looks_version {
            return (s[..i].to_string(), Some(rest.to_string()));
        }
    }
    (s.to_string(), None)
}

/// Sort + de-duplicate a package list by name.
pub(crate) fn dedup(mut v: Vec<PackageUpdate>) -> Vec<PackageUpdate> {
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    v.dedup_by(|a, b| a.name == b.name);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_os_release() {
        let os = parse_os_release(
            "NAME=\"Debian GNU/Linux\"\nID=debian\nVERSION_ID=\"12\"\nID_LIKE=ubuntu\n",
        );
        assert_eq!(os.id, "debian");
        assert_eq!(os.name, "Debian GNU/Linux");
        assert_eq!(os.version.as_deref(), Some("12"));
        assert_eq!(os.id_like, vec!["ubuntu"]);
    }

    #[test]
    fn splits_name_and_version() {
        assert_eq!(
            split_name_version("libx11-1.8.7-r0"),
            ("libx11".to_string(), Some("1.8.7-r0".to_string()))
        );
        assert_eq!(
            split_name_version("busybox-1.36.1-r2"),
            ("busybox".to_string(), Some("1.36.1-r2".to_string()))
        );
    }
}
