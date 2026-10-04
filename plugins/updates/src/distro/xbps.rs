//! Void Linux (XBPS).

use crate::distro::{dedup, split_name_version, PackageManager};
use crate::exec::CmdSpec;
use crate::model::PackageUpdate;

pub struct Xbps;

impl PackageManager for Xbps {
    fn id(&self) -> &'static str {
        "xbps"
    }
    fn label(&self) -> &'static str {
        "XBPS (Void Linux)"
    }
    fn binary(&self) -> &'static str {
        "xbps-install"
    }

    fn refresh(&self) -> Option<CmdSpec> {
        Some(CmdSpec::new("xbps-install").arg("-S").root())
    }

    fn list(&self) -> CmdSpec {
        // `-u` update, `-n` dry-run: prints the transaction without applying it.
        CmdSpec::new("xbps-install")
            .args(["-un"])
            .env("LC_ALL", "C")
    }

    fn parse(&self, stdout: &str) -> Vec<PackageUpdate> {
        let mut out = Vec::new();
        for line in stdout.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.is_empty() {
                continue;
            }
            // Forms seen in the wild:
            //   `name-1.0_1 update name-1.1_1`
            //   `name-1.0_1 -> name-1.1_1`
            let (left, right) = if let Some(i) = fields.iter().position(|f| *f == "update" || *f == "->")
            {
                if i == 0 || i + 1 >= fields.len() {
                    continue;
                }
                (fields[i - 1], fields[i + 1])
            } else if fields.len() == 2 {
                (fields[0], fields[1])
            } else {
                continue;
            };
            let (name, current) = split_name_version(left);
            let (_, candidate) = split_name_version(right);
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
        CmdSpec::new("xbps-install").args(["-Suy"]).root()
    }

    fn upgrade_one(&self, name: &str) -> Option<CmdSpec> {
        Some(CmdSpec::new("xbps-install").args(["-Syu", name]).root())
    }
}
