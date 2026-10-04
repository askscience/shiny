//! Gentoo family (Gentoo, Funtoo).

use crate::distro::{dedup, split_name_version, PackageManager};
use crate::exec::CmdSpec;
use crate::model::PackageUpdate;

pub struct Emerge;

impl PackageManager for Emerge {
    fn id(&self) -> &'static str {
        "emerge"
    }
    fn label(&self) -> &'static str {
        "Portage (Gentoo)"
    }
    fn binary(&self) -> &'static str {
        "emerge"
    }

    fn refresh(&self) -> Option<CmdSpec> {
        Some(
            CmdSpec::new("emerge")
                .args(["--sync", "--color=n"])
                .root(),
        )
    }

    fn list(&self) -> CmdSpec {
        // Pretend a `@world` update. Long-running (dependency resolution), so
        // the caller uses a generous timeout.
        CmdSpec::new("emerge")
            .args(["-uDNp", "--color=n", "--nospinner", "@world"])
            .env("LC_ALL", "C")
    }

    fn parse(&self, stdout: &str) -> Vec<PackageUpdate> {
        let mut out = Vec::new();
        for line in stdout.lines() {
            let line = line.trim();
            if !line.starts_with("[ebuild") {
                continue;
            }
            let Some(close) = line.find(']') else { continue };
            let after = line[close + 1..].trim();
            let Some(atom) = after.split_whitespace().next() else {
                continue;
            };
            let Some((category, pkg)) = atom.split_once('/') else {
                continue;
            };
            let (name, candidate) = split_name_version(pkg);
            // The old version (if any) is shown at the end of the line as
            // `[oldversion]`.
            let current = line
                .rfind('[')
                .and_then(|i| line[i + 1..].strip_suffix(']').map(|s| s.to_string()))
                .filter(|s| !s.is_empty());
            out.push(PackageUpdate {
                name: format!("{category}/{name}"),
                current,
                candidate,
                repo: None,
            });
        }
        dedup(out)
    }

    fn upgrade_all(&self) -> CmdSpec {
        CmdSpec::new("emerge")
            .args(["-uDN", "--color=n", "--nospinner", "@world"])
            .root()
            .env("LC_ALL", "C")
    }

    fn upgrade_one(&self, name: &str) -> Option<CmdSpec> {
        Some(
            CmdSpec::new("emerge")
                .args(["-u", "--color=n", "--nospinner", name])
                .root(),
        )
    }
}
