//! SUSE family (openSUSE Leap/Tumbleweed, SLE Micro, SLES).

use crate::distro::{dedup, PackageManager};
use crate::exec::CmdSpec;
use crate::model::PackageUpdate;

pub struct Zypper;

impl PackageManager for Zypper {
    fn id(&self) -> &'static str {
        "zypper"
    }
    fn label(&self) -> &'static str {
        "zypper (openSUSE/SUSE)"
    }
    fn binary(&self) -> &'static str {
        "zypper"
    }

    fn refresh(&self) -> Option<CmdSpec> {
        Some(CmdSpec::new("zypper").args(["-n", "refresh"]).root())
    }

    fn list(&self) -> CmdSpec {
        CmdSpec::new("zypper")
            .args(["--no-refresh", "-q", "list-updates"])
            .env("LC_ALL", "C")
    }

    fn parse(&self, stdout: &str) -> Vec<PackageUpdate> {
        let mut out = Vec::new();
        for line in stdout.lines() {
            if !line.contains('|') {
                continue;
            }
            let cols: Vec<&str> = line.split('|').map(str::trim).collect();
            // S | Repository | Name | Current Version | Available Version
            if cols.len() < 5 || cols[2].eq_ignore_ascii_case("name") || cols[2].is_empty() {
                continue;
            }
            if cols[2].starts_with('-') {
                continue;
            }
            out.push(PackageUpdate {
                name: cols[2].to_string(),
                current: non_empty(cols[3]),
                candidate: non_empty(cols[4]),
                repo: non_empty(cols[1]),
            });
        }
        dedup(out)
    }

    fn upgrade_all(&self) -> CmdSpec {
        CmdSpec::new("zypper").args(["-n", "update"]).root()
    }

    fn upgrade_one(&self, name: &str) -> Option<CmdSpec> {
        Some(CmdSpec::new("zypper").args(["-n", "update", name]).root())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_list_updates_table() {
        let src = "S | Repository | Name | Current Version | Available Version\n\
                   --+------------+------+-----------------+-------------------\n\
                   v | repo-oss | bash | 5.2.15-1.1 | 5.2.15-2.1\n";
        let out = Zypper.parse(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "bash");
        assert_eq!(out[0].current.as_deref(), Some("5.2.15-1.1"));
        assert_eq!(out[0].candidate.as_deref(), Some("5.2.15-2.1"));
        assert_eq!(out[0].repo.as_deref(), Some("repo-oss"));
    }
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}
