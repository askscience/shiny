//! Fedora / RHEL family (Fedora, CentOS Stream, RHEL, Rocky, AlmaLinux,
//! Amazon Linux, Nobara, Bazzite, …).

use crate::distro::{dedup, PackageManager};
use crate::exec::{self, CmdSpec};
use crate::model::PackageUpdate;

pub struct Dnf;

/// Prefer `dnf`, fall back to `yum` (RHEL 7-era systems).
fn tool() -> &'static str {
    if exec::which("dnf").is_some() {
        "dnf"
    } else {
        "yum"
    }
}

impl PackageManager for Dnf {
    fn id(&self) -> &'static str {
        "dnf"
    }
    fn label(&self) -> &'static str {
        "DNF/YUM (Fedora/RHEL)"
    }
    fn binary(&self) -> &'static str {
        "dnf"
    }

    fn refresh(&self) -> Option<CmdSpec> {
        Some(
            CmdSpec::new(tool())
                .args(["-y", "makecache"])
                .root()
                .env("LC_ALL", "C"),
        )
    }

    fn list(&self) -> CmdSpec {
        // `check-update` exits 100 when updates are available — that is a
        // success, not an error.
        CmdSpec::new(tool())
            .args(["-q", "check-update"])
            .env("LC_ALL", "C")
            .ok(100)
    }

    fn parse(&self, stdout: &str) -> Vec<PackageUpdate> {
        let mut out = Vec::new();
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty()
                || line.starts_with("Last metadata")
                || line.starts_with("Obsoleting")
                || line.starts_with("Security:")
                || line.starts_with("Dependencies resolved")
                || line.starts_with("(")
            {
                continue;
            }
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 2 {
                continue;
            }
            let (name, repo) = match fields[0].rsplit_once('.') {
                Some((n, _arch)) => (n.to_string(), fields.get(2).map(|s| s.to_string())),
                None => (fields[0].to_string(), fields.get(2).map(|s| s.to_string())),
            };
            out.push(PackageUpdate {
                name,
                current: None,
                candidate: Some(fields[1].to_string()),
                repo,
            });
        }
        dedup(out)
    }

    fn upgrade_all(&self) -> CmdSpec {
        CmdSpec::new(tool())
            .args(["-y", "upgrade"])
            .root()
            .env("LC_ALL", "C")
    }

    fn upgrade_one(&self, name: &str) -> Option<CmdSpec> {
        Some(
            CmdSpec::new(tool())
                .args(["-y", "upgrade", name])
                .root()
                .env("LC_ALL", "C"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_check_update() {
        let src = "Last metadata expiration check: 0:30:00 ago.\n\
                   bash.x86_64        5.2.15-2.fc39       updates\n\
                   kernel.x86_64      6.6.2-200.fc39      updates\n\n";
        let out = Dnf.parse(src);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name, "bash");
        assert_eq!(out[0].candidate.as_deref(), Some("5.2.15-2.fc39"));
        assert_eq!(out[0].repo.as_deref(), Some("updates"));
    }
}
