//! Debian family (Debian, Ubuntu, Mint, Pop!_OS, Kali, Raspberry Pi OS, …).

use crate::distro::{dedup, PackageManager};
use crate::exec::CmdSpec;
use crate::model::PackageUpdate;

pub struct Apt;

impl PackageManager for Apt {
    fn id(&self) -> &'static str {
        "apt"
    }
    fn label(&self) -> &'static str {
        "APT (Debian/Ubuntu)"
    }
    fn binary(&self) -> &'static str {
        "apt-get"
    }

    fn refresh(&self) -> Option<CmdSpec> {
        Some(
            CmdSpec::new("apt-get")
                .arg("update")
                .root()
                .env("DEBIAN_FRONTEND", "noninteractive"),
        )
    }

    fn list(&self) -> CmdSpec {
        // `apt list --upgradable` reads the cached package lists and works
        // without root. Output: `name/suite version arch [upgradable from: old]`.
        CmdSpec::new("apt")
            .args(["list", "--upgradable"])
            .env("LC_ALL", "C")
    }

    fn parse(&self, stdout: &str) -> Vec<PackageUpdate> {
        let mut out = Vec::new();
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("Listing") || !line.contains('/') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let Some(first) = parts.next() else { continue };
            let mut slash = first.splitn(2, '/');
            let name = slash.next().unwrap_or(first).to_string();
            let repo = slash.next().map(str::to_string);
            let candidate = parts.next().map(str::to_string);
            let rest: Vec<&str> = parts.collect();
            let current = rest
                .iter()
                .position(|p| *p == "from:")
                .and_then(|i| rest.get(i + 1))
                .map(|s| s.trim_end_matches(']').to_string());
            out.push(PackageUpdate {
                name,
                current,
                candidate,
                repo,
            });
        }
        dedup(out)
    }

    fn upgrade_all(&self) -> CmdSpec {
        // `full-upgrade` (not `upgrade`): plain `upgrade` silently keeps back
        // packages that need new dependencies or a removal — most notably
        // kernel updates — and still exits 0, so the UI would say "finished"
        // while the update stayed pending.
        CmdSpec::new("apt-get")
            .args(["-y", "full-upgrade"])
            .root()
            .env("DEBIAN_FRONTEND", "noninteractive")
    }

    fn upgrade_one(&self, name: &str) -> Option<CmdSpec> {
        Some(
            CmdSpec::new("apt-get")
                .args(["-y", "install", "--only-upgrade", name])
                .root()
                .env("DEBIAN_FRONTEND", "noninteractive"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_upgradable_list() {
        let src = "Listing... Done\n\
                   bash/stable 5.2.15-2+b7 amd64 [upgradable from: 5.2.15-2+b2]\n\
                   curl/stable 7.88.1-10 amd64 [upgradable from: 7.88.1-9]\n";
        let out = Apt.parse(src);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name, "bash");
        assert_eq!(out[0].candidate.as_deref(), Some("5.2.15-2+b7"));
        assert_eq!(out[0].current.as_deref(), Some("5.2.15-2+b2"));
        assert_eq!(out[0].repo.as_deref(), Some("stable"));
    }
}
