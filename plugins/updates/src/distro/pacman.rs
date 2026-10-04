//! Arch family (Arch, Manjaro, EndeavourOS, Garuda, CachyOS, Artix, …).

use crate::distro::{dedup, PackageManager};
use crate::exec::{self, CmdSpec};
use crate::model::PackageUpdate;

pub struct Pacman;

impl PackageManager for Pacman {
    fn id(&self) -> &'static str {
        "pacman"
    }
    fn label(&self) -> &'static str {
        "pacman (Arch/Manjaro)"
    }
    fn binary(&self) -> &'static str {
        "pacman"
    }

    fn refresh(&self) -> Option<CmdSpec> {
        Some(CmdSpec::new("pacman").args(["-Sy"]).root())
    }

    fn list(&self) -> CmdSpec {
        // `checkupdates` (pacman-contrib) fetches an up-to-date list in a temp
        // DB without touching the live one; `pacman -Qu` reads the synced DB.
        if exec::which("checkupdates").is_some() {
            CmdSpec::new("checkupdates")
        } else {
            CmdSpec::new("pacman").args(["-Qu"]).env("LC_ALL", "C")
        }
    }

    fn parse(&self, stdout: &str) -> Vec<PackageUpdate> {
        let mut out = Vec::new();
        for line in stdout.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            // `name oldver -> newver`
            if fields.len() < 4 || fields[2] != "->" {
                continue;
            }
            out.push(PackageUpdate {
                name: fields[0].to_string(),
                current: Some(fields[1].to_string()),
                candidate: Some(fields[3].to_string()),
                repo: None,
            });
        }
        dedup(out)
    }

    fn upgrade_all(&self) -> CmdSpec {
        CmdSpec::new("pacman")
            .args(["-Syu", "--noconfirm"])
            .root()
    }

    fn upgrade_one(&self, name: &str) -> Option<CmdSpec> {
        Some(
            CmdSpec::new("pacman")
                .args(["-S", "--noconfirm", name])
                .root(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_qu() {
        let src = "linux 6.6.1.arch1-1 -> 6.6.2.arch1-1\nvim 9.0.1-1 -> 9.0.2-1\n";
        let out = Pacman.parse(src);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name, "linux");
        assert_eq!(out[0].current.as_deref(), Some("6.6.1.arch1-1"));
        assert_eq!(out[0].candidate.as_deref(), Some("6.6.2.arch1-1"));
    }
}
