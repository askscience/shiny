//! Nix / NixOS.
//!
//! Nix is deliberately different: the classic `nix-env` profile and a
//! flake-based NixOS system are both common. We support the classic profile
//! (`nix-channel` + `nix-env`) and, when `nixos-rebuild` is present, upgrade
//! the system generation instead.

use crate::distro::{dedup, split_name_version, PackageManager};
use crate::exec::{self, CmdSpec};
use crate::model::PackageUpdate;

pub struct Nix;

impl PackageManager for Nix {
    fn id(&self) -> &'static str {
        "nix"
    }
    fn label(&self) -> &'static str {
        "Nix (NixOS)"
    }
    fn binary(&self) -> &'static str {
        "nix-env"
    }

    fn refresh(&self) -> Option<CmdSpec> {
        Some(CmdSpec::new("nix-channel").arg("--update").root())
    }

    fn list(&self) -> CmdSpec {
        CmdSpec::new("nix-env")
            .args(["-u", "--dry-run"])
            .env("LC_ALL", "C")
    }

    fn parse(&self, stdout: &str) -> Vec<PackageUpdate> {
        let mut out = Vec::new();
        for line in stdout.lines() {
            // `upgrading 'pkg-1.0' to 'pkg-1.1'`
            let line = line.trim();
            let Some(rest) = line.strip_prefix("upgrading '") else {
                continue;
            };
            let Some((from, to)) = rest.split_once("' to '") else {
                continue;
            };
            let to = to.trim_end_matches('\'');
            let (name, current) = split_name_version(from);
            let (_, candidate) = split_name_version(to);
            out.push(PackageUpdate {
                name,
                current,
                candidate,
                repo: None,
            });
        }
        dedup(out)
    }

    fn upgrade_all(&self) -> CmdSpec {
        if exec::which("nixos-rebuild").is_some() {
            CmdSpec::new("nixos-rebuild")
                .args(["switch", "--upgrade"])
                .root()
        } else {
            CmdSpec::new("sh")
                .args(["-c", "nix-channel --update && nix-env -u"])
                .root()
        }
    }

    fn upgrade_one(&self, name: &str) -> Option<CmdSpec> {
        Some(CmdSpec::new("nix-env").args(["-u", name]).root())
    }
}
