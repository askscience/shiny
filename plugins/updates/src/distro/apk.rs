//! Alpine family (Alpine Linux, postmarketOS).

use crate::distro::{dedup, split_name_version, PackageManager};
use crate::exec::CmdSpec;
use crate::model::PackageUpdate;

pub struct Apk;

impl PackageManager for Apk {
    fn id(&self) -> &'static str {
        "apk"
    }
    fn label(&self) -> &'static str {
        "apk (Alpine)"
    }
    fn binary(&self) -> &'static str {
        "apk"
    }

    fn refresh(&self) -> Option<CmdSpec> {
        Some(CmdSpec::new("apk").arg("update").root())
    }

    fn list(&self) -> CmdSpec {
        // Modern apk (2.12+) lists upgradable packages as
        // `name-version arch {repo}`.
        CmdSpec::new("apk")
            .args(["list", "--upgradable"])
            .env("LC_ALL", "C")
    }

    fn parse(&self, stdout: &str) -> Vec<PackageUpdate> {
        let mut out = Vec::new();
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("WARNING") {
                continue;
            }
            let mut fields = line.split_whitespace();
            let Some(pkg) = fields.next() else { continue };
            let (name, candidate) = split_name_version(pkg);
            let repo = line
                .split('{')
                .nth(1)
                .and_then(|s| s.split('}').next())
                .map(str::to_string);
            out.push(PackageUpdate {
                name,
                current: None,
                candidate,
                repo,
            });
        }
        dedup(out)
    }

    fn upgrade_all(&self) -> CmdSpec {
        CmdSpec::new("apk")
            .args(["upgrade", "--available"])
            .root()
    }

    fn upgrade_one(&self, name: &str) -> Option<CmdSpec> {
        Some(CmdSpec::new("apk").args(["upgrade", name]).root())
    }
}
