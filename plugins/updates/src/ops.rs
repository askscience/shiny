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
        run_spec(job, &spec, password, UPGRADE_ALL_TIMEOUT)
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
        if ok {
            crate::status::invalidate();
        }
        ok
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
    if let Some(stop) = ollama::stop_spec() {
        job.log("Stopping Ollama so its GPU libraries can be replaced safely…");
        if run_spec(job, &stop, password, Duration::from_secs(120)) {
            for _ in 0..20 {
                if !ollama::is_active() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }

    let spec = ollama::update_spec();
    let ok = run_spec(job, &spec, password, Duration::from_secs(1800));
    if ok {
        job.log(
            "Ollama updated. The service is stopped — start it with \
             `systemctl start ollama` (or reboot) to use the new version.",
        );
        crate::status::invalidate();
    }
    ok
}
