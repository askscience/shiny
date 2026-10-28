//! The operations a job performs. These run on the job thread and talk only to
//! [`exec`]; they never touch the database.

use std::time::Duration;

use crate::distro::PackageManager;
use crate::exec::{self, CmdSpec};
use crate::jobs::Job;
use crate::ollama;

const REFRESH_TIMEOUT: Duration = Duration::from_secs(900);
const UPGRADE_ALL_TIMEOUT: Duration = Duration::from_secs(3600);
const UPGRADE_ONE_TIMEOUT: Duration = Duration::from_secs(1800);
const REPAIR_TIMEOUT: Duration = Duration::from_secs(1800);
const AUDIT_TIMEOUT: Duration = Duration::from_secs(120);

fn run_spec(job: &Job, spec: &CmdSpec, password: Option<&str>, timeout: Duration) -> bool {
    job.log(format!("$ {}", spec.display()));
    match exec::run_streaming(spec, password, timeout, &mut |line| job.log(line)) {
        Ok(out) => {
            if out.timed_out {
                job.log("!! command timed out and was cancelled");
            }
            if !out.stderr.trim().is_empty() {
                for line in out.stderr.lines().take(100) {
                    job.log(line);
                }
            }
            out.success
        }
        Err(e) => {
            job.log(format!("!! {e}"));
            false
        }
    }
}

/// Whether the package database is consistent (`None`: no audit supported).
fn audit_package_state(
    job: &Job,
    manager: &dyn PackageManager,
    password: Option<&str>,
) -> Option<bool> {
    let spec = manager.audit()?;
    job.log(format!("$ {}", spec.display()));
    match exec::run(&spec, password, AUDIT_TIMEOUT) {
        Ok(out) => {
            if out.timed_out {
                job.log("!! package-state audit timed out");
                return Some(false);
            }
            if !out.success {
                for line in out.stderr.lines().take(40) {
                    job.log(line);
                }
                return Some(false);
            }
            let problems = out.stdout.trim();
            if problems.is_empty() {
                Some(true)
            } else {
                for line in problems.lines().take(80) {
                    job.log(line);
                }
                Some(false)
            }
        }
        Err(e) => {
            job.log(format!("!! could not audit the package state: {e}"));
            Some(false)
        }
    }
}

/// After an upgrade, make sure apt/dpkg did not leave packages half
/// configured. A failed kernel postinst used to make every later apt command
/// fail with "Sub-process /usr/bin/dpkg returned an error code (1)" — and the
/// user only found out at the next reboot, when the login screen did not come
/// back. Repair here, while the upgrade job is still open, and report failure
/// instead of silently leaving the machine in that state.
fn repair_package_state(
    job: &Job,
    manager: &dyn PackageManager,
    password: Option<&str>,
) -> bool {
    job.log("Checking that no package was left half-configured…");
    match audit_package_state(job, manager, password) {
        None => true,
        Some(true) => {
            job.log("Package state is clean.");
            true
        }
        Some(false) => {
            let Some(spec) = manager.repair() else {
                job.log("!! the package database is inconsistent and this package manager has no repair step");
                return false;
            };
            job.log("Package state is inconsistent; finishing the interrupted configuration…");
            let repaired = run_spec(job, &spec, password, REPAIR_TIMEOUT);
            if !repaired {
                job.log("!! the repair command failed");
            }
            match audit_package_state(job, manager, password) {
                Some(true) => {
                    job.log("Package state is clean again.");
                    true
                }
                _ => {
                    job.log(
                        "!! packages are still half-configured. Run \
                         `sudo dpkg --configure -a && sudo apt-get -f install` \
                         before rebooting.",
                    );
                    false
                }
            }
        }
    }
}

/// Refresh package metadata (needs root on most distros).
pub fn refresh_job(job: &Job, manager: &dyn PackageManager, password: Option<&str>) -> bool {
    match manager.refresh() {
        Some(spec) => {
            let ok = run_spec(job, &spec, password, REFRESH_TIMEOUT);
            if ok {
                crate::status::invalidate();
            }
            ok
        }
        None => {
            job.log("This package manager does not have a metadata refresh step.");
            true
        }
    }
}

/// Upgrade `packages`, or everything when `all` is set.
pub fn system_job(
    job: &Job,
    manager: &dyn PackageManager,
    password: Option<&str>,
    all: bool,
    packages: &[String],
) -> bool {
    if all {
        let spec = manager.upgrade_all();
        let upgraded = run_spec(job, &spec, password, UPGRADE_ALL_TIMEOUT);
        if !upgraded {
            job.log("The upgrade command reported failure; checking the package state…");
        }
        // Even a successful apt run can hide a failed postinst on some
        // versions; always audit once the upgrade job has finished.
        let healthy = repair_package_state(job, manager, password);
        crate::status::invalidate();
        upgraded && healthy
    } else {
        let mut ok = true;
        for name in packages {
            match manager.upgrade_one(name) {
                Some(spec) => ok &= run_spec(job, &spec, password, UPGRADE_ONE_TIMEOUT),
                None => {
                    job.log(format!(
                        "!! {name}: this package manager cannot upgrade a single package"
                    ));
                    ok = false;
                }
            }
        }
        let healthy = repair_package_state(job, manager, password);
        crate::status::invalidate();
        ok && healthy
    }
}

/// Install/upgrade Ollama with the official script, safely.
///
/// The machine hard-froze during this update because the installer replaces
/// `/usr/local/lib/ollama` (the GPU libraries) while Ollama is running, and
/// because it reconfigured/restarted the service mid-session. So we:
///   1. stop the service first (nothing is using the libraries), then
///   2. run the installer with systemd skipped entirely (see `update_spec`),
///   3. leave the service stopped so nothing restarts the GPU mid-session.
/// The new version takes effect when the user starts Ollama again (or reboots).
pub fn ollama_job(job: &Job, password: Option<&str>) -> bool {
    let was_active = ollama::is_active();
    if let Some(stop) = ollama::stop_spec() {
        if was_active {
            job.log("Stopping Ollama so its GPU libraries can be replaced safely…");
            if !run_spec(job, &stop, password, Duration::from_secs(120)) {
                job.log("!! systemctl stop ollama failed");
            }
            let mut stopped = false;
            for _ in 0..20 {
                if !ollama::is_active() {
                    stopped = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            if !stopped {
                job.log(
                    "!! Ollama is still running; refusing to replace its libraries \
                     while the GPU may be using them.",
                );
                return false;
            }
        }
    }

    let spec = ollama::update_spec();
    let ok = run_spec(job, &spec, password, Duration::from_secs(1800));
    if !ok {
        job.log(
            "!! the Ollama installer failed. The previous libraries may have been \
             removed already; run the update again before starting Ollama.",
        );
        return false;
    }

    match ollama::verify_installation() {
        Ok(version) => {
            job.log(format!(
                "Verified Ollama {version}: binary, GPU libraries and llama-server are present."
            ));
        }
        Err(e) => {
            job.log(format!("!! Ollama installation is incomplete: {e}"));
            job.log(
                "!! Run the Ollama update again (it is safe to repeat) before \
                 starting the service.",
            );
            return false;
        }
    }

    job.log(
        "Ollama updated. The service is stopped — start it with \
         `systemctl start ollama` (or reboot) to use the new version.",
    );
    crate::status::invalidate();
    true
}
