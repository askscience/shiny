//! Debian family (Debian, Ubuntu, Mint, Pop!_OS, Kali, Raspberry Pi OS, …).

use crate::distro::{dedup, PackageManager};
use crate::exec::CmdSpec;
use crate::model::PackageUpdate;
use crate::progress::{Event, PkgState};

pub struct Apt;

/// Options shared by every apt run that installs or configures packages.
///
/// - `APT::Status-Fd=1`: machine-readable progress (`dlstatus:` / `pmstatus:`
///   lines) on stdout, parsed by [`Apt::progress_event`] for the window.
/// - `--force-confdef --force-confold`: when a package ships a new version of a
///   config file the user modified, keep the user's file instead of stopping at
///   dpkg's interactive "What would you like to do?" prompt — there is no
///   terminal, so that prompt either fails the upgrade or hangs it.
const APT_OPTS: [&str; 6] = [
    "-o",
    "APT::Status-Fd=1",
    "-o",
    "Dpkg::Options::=--force-confdef",
    "-o",
    "Dpkg::Options::=--force-confold",
];

/// The environment for an unattended apt run. `LC_ALL=C` keeps the status
/// messages in English so they can be parsed; the rest silence the tools that
/// would otherwise wait for an answer (debconf, needrestart, apt-listchanges).
fn unattended(spec: CmdSpec) -> CmdSpec {
    spec.env("DEBIAN_FRONTEND", "noninteractive")
        .env("NEEDRESTART_MODE", "a")
        .env("NEEDRESTART_SUSPEND", "1")
        .env("APT_LISTCHANGES_FRONTEND", "none")
        .env("LC_ALL", "C")
}

/// Map an apt install-progress message to the package's new state.
fn install_state(message: &str) -> Option<PkgState> {
    let m = message.to_ascii_lowercase();
    // Order matters: the longer prefixes share words with the shorter ones.
    if m.starts_with("installed") || m.starts_with("configured") {
        Some(PkgState::Done)
    } else if m.starts_with("completely removed") || m.starts_with("removed") {
        Some(PkgState::Removed)
    } else if m.starts_with("preparing to configure") || m.starts_with("configuring") {
        Some(PkgState::Configuring)
    } else if m.starts_with("preparing for removal")
        || m.starts_with("preparing to completely remove")
        || m.starts_with("removing")
    {
        Some(PkgState::Removing)
    } else if m.starts_with("preparing") || m.starts_with("unpacking") || m.starts_with("installing") {
        Some(PkgState::Unpacking)
    } else {
        None
    }
}

/// Parse `pmstatus:<pkg>[:<arch>]:<percent>:<message>` /
/// `pmerror:<pkg>:<percent>:<message>` / `dlstatus:<n>:<percent>:<message>`.
fn parse_status_line(line: &str) -> Option<Event> {
    let (kind, rest) = line.trim().split_once(':')?;
    if !matches!(kind, "pmstatus" | "pmerror" | "dlstatus") {
        return None;
    }
    let fields: Vec<&str> = rest.split(':').collect();
    // The percent is the first numeric field after the package. A package
    // field may carry its architecture (`libc6:amd64`), so look at fields 1
    // and 2 instead of assuming a fixed position.
    let idx = (1..fields.len().min(3)).find(|&i| fields[i].trim().parse::<f32>().is_ok())?;
    let percent: f32 = fields[idx].trim().parse().ok()?;
    let message = fields[idx + 1..].join(":").trim().to_string();
    // `libc6:amd64` → `libc6`, matching `apt list` names.
    let package = fields[0].trim();
    let package = (!package.is_empty() && package != "dpkg-exec").then(|| package.to_string());
    Some(match kind {
        "dlstatus" => Event::Download { percent, message },
        "pmerror" => Event::Error { package, message },
        _ => Event::Install {
            percent,
            state: package.as_ref().and_then(|_| install_state(&message)),
            package,
            message,
        },
    })
}

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
        Some(unattended(CmdSpec::new("apt-get").arg("update").root()))
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
        unattended(
            CmdSpec::new("apt-get")
                .args(APT_OPTS)
                .args(["-y", "full-upgrade"])
                .root(),
        )
    }

    fn upgrade_one(&self, name: &str) -> Option<CmdSpec> {
        Some(unattended(
            CmdSpec::new("apt-get")
                .args(APT_OPTS)
                .args(["-y", "install", "--only-upgrade", name])
                .root(),
        ))
    }

    fn audit(&self) -> Option<CmdSpec> {
        // `dpkg --audit` prints one paragraph per package that is unpacked but
        // not configured (or half-configured). Empty output on success.
        Some(CmdSpec::new("dpkg").arg("--audit"))
    }

    fn repair(&self) -> Option<CmdSpec> {
        // A kernel postinst (DKMS, initramfs) can fail after the packages were
        // unpacked, leaving apt's "Sub-process /usr/bin/dpkg returned an error
        // code (1)" state behind. Configure what was unpacked first, then let
        // apt resolve the rest; both steps are required for a usable system.
        Some(unattended(
            CmdSpec::new("sh")
                .args([
                    "-c",
                    "dpkg --force-confdef --force-confold --configure -a && \
                     apt-get -o APT::Status-Fd=1 \
                       -o Dpkg::Options::=--force-confdef \
                       -o Dpkg::Options::=--force-confold -y -f install",
                ])
                .root(),
        ))
    }

    fn progress_event(&self, line: &str) -> Option<Event> {
        parse_status_line(line)
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

    #[test]
    fn parses_status_fd_lines() {
        assert_eq!(
            parse_status_line("dlstatus:3:42.5:Retrieving file 3 of 12"),
            Some(Event::Download { percent: 42.5, message: "Retrieving file 3 of 12".into() })
        );
        assert_eq!(
            parse_status_line("pmstatus:libc6:amd64:20.5:Unpacking libc6:amd64"),
            Some(Event::Install {
                percent: 20.5,
                package: Some("libc6".into()),
                state: Some(PkgState::Unpacking),
                message: "Unpacking libc6:amd64".into(),
            })
        );
        assert_eq!(
            parse_status_line("pmstatus:bash:61.3:Preparing to configure bash (amd64)"),
            Some(Event::Install {
                percent: 61.3,
                package: Some("bash".into()),
                state: Some(PkgState::Configuring),
                message: "Preparing to configure bash (amd64)".into(),
            })
        );
        assert!(matches!(
            parse_status_line("pmstatus:bash:90:Installed bash (amd64)"),
            Some(Event::Install { state: Some(PkgState::Done), .. })
        ));
        assert!(matches!(
            parse_status_line("pmstatus:dpkg-exec:0:Running dpkg"),
            Some(Event::Install { package: None, state: None, .. })
        ));
        assert!(matches!(
            parse_status_line("pmerror:linux-image-6.9:80:subprocess returned error exit status 1"),
            Some(Event::Error { package: Some(_), .. })
        ));
        assert_eq!(parse_status_line("Unpacking bash (5.2) over (5.1) ..."), None);
        assert_eq!(parse_status_line("Setting up bash: done"), None);
    }
}
