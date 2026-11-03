//! Uploads through `gokapi-cli`, as background jobs.
//!
//! `gokapi-cli upload` handles chunking and resumable transfer; `--json`
//! makes it print the resulting `FileApiOutput` on stdout. Folders go through
//! `upload-dir`, which zips them into [`super::tmp_dir`] first. Only long flag
//! names are used except `-x` (the v1.1.x CLI has no long form for it).
//!
//! An upload can take minutes, longer than any route may hold the plugin's
//! serial worker, so [`start`] spawns it and the window polls [`job_json`].

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value as Json};
use shiny_plugin_sdk::services::PluginCtx;

use super::install::cli_bin;
use super::rest::GokapiFile;
use super::store::{self, Share};
use super::{cli_config_path, random_alnum, tmp_dir};

/// Cap on captured CLI output (it prints one JSON object).
const MAX_OUTPUT: usize = 1024 * 1024;
/// Finished jobs are forgotten after this long.
const JOB_TTL: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone)]
pub struct UploadRequest {
    pub abs_path: PathBuf,
    /// Home-relative path, as stored and shown.
    pub rel_path: String,
    pub is_dir: bool,
    /// 0 = no time limit.
    pub expiry_days: u32,
    /// 0 = unlimited.
    pub max_downloads: u32,
    pub password: Option<String>,
    /// Sender name and note shown on the download page (`theme.rs`).
    pub from: String,
    pub message: String,
    /// Encrypt locally; the key ends up after `#` in the link (`e2e.rs`).
    pub e2e: bool,
}

#[derive(Debug, Clone)]
enum JobState {
    Running,
    Done(Share),
    Failed(String),
}

struct Job {
    owner: String,
    state: JobState,
    created: Instant,
}

/// Serialises end-to-end uploads (see `start`).
static E2E_TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

static JOBS: Mutex<Option<HashMap<String, Job>>> = Mutex::new(None);

fn with_jobs<T>(f: impl FnOnce(&mut HashMap<String, Job>) -> T) -> T {
    let mut guard = JOBS.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// Arguments for one upload (without the binary).
pub fn build_args(config: &Path, req: &UploadRequest, tmp: &Path) -> Vec<OsString> {
    let mut a: Vec<OsString> = Vec::new();
    if req.is_dir {
        a.push("upload-dir".into());
        a.push("--directory".into());
        a.push(req.abs_path.clone().into());
        a.push("--tempfolder".into());
        a.push(tmp.into());
    } else {
        a.push("upload".into());
        a.push("--file".into());
        a.push(req.abs_path.clone().into());
    }
    a.push("--configuration".into());
    a.push(config.into());
    a.push("--json".into());
    if !req.e2e {
        a.push("-x".into());
    }
    if req.expiry_days > 0 {
        a.push("--expiry-days".into());
        a.push(req.expiry_days.to_string().into());
    }
    if req.max_downloads > 0 {
        a.push("--expiry-downloads".into());
        a.push(req.max_downloads.to_string().into());
    }
    if let Some(pw) = req.password.as_deref().filter(|p| !p.is_empty()) {
        a.push("--password".into());
        a.push(pw.into());
    }
    a
}

/// The CLI's JSON result: the last stdout line that parses as an object.
/// Anything else it printed is an error message.
pub fn parse_output(stdout: &str) -> Result<GokapiFile, String> {
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .filter(|l| l.starts_with('{'))
        .find_map(|l| serde_json::from_str::<GokapiFile>(l).ok())
        .filter(|f| !f.id.is_empty() && !f.url_download.is_empty())
        .ok_or_else(|| {
            let msg: Vec<&str> = stdout
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            if msg.is_empty() {
                "gokapi-cli returned no result".to_string()
            } else {
                msg.join(" ")
            }
        })
}

/// Start an upload job and return its id. Call from the plugin runtime.
pub fn start(ctx: Arc<PluginCtx>, owner: &str, req: UploadRequest) -> Result<String, String> {
    let bin = cli_bin().ok_or("gokapi-cli is not installed")?;
    let id = random_alnum(16);
    with_jobs(|jobs| {
        jobs.retain(|_, j| matches!(j.state, JobState::Running) || j.created.elapsed() < JOB_TTL);
        jobs.insert(
            id.clone(),
            Job {
                owner: owner.to_string(),
                state: JobState::Running,
                created: Instant::now(),
            },
        );
    });
    let owner = owner.to_string();
    let job_id = id.clone();
    tokio::spawn(async move {
        // The CLI rewrites one shared E2E registry per upload; two at once
        // would lose an entry (Gokapi docs), so E2E uploads queue here.
        let _e2e_turn = if req.e2e {
            Some(E2E_TURN.lock().await)
        } else {
            None
        };
        let state = match run_upload(&bin, &req).await {
            Ok(file) => {
                let share = Share {
                    id: file.id.clone(),
                    path: req.rel_path.clone(),
                    name: file.name.clone(),
                    url: file.url_download.clone(),
                    size_bytes: file.size_bytes,
                    expires_at: if file.unlimited_time {
                        0
                    } else {
                        file.expire_at
                    },
                    max_downloads: req.max_downloads as i64,
                    has_password: file.is_password_protected,
                    created_at: String::new(),
                };
                // Spawned tasks run on the plugin's runtime thread, where
                // `ctx.db()` is meant to be used.
                if let Err(e) = super::theme::write_share_meta(&share.id, &req.from, &req.message) {
                    eprintln!("[files] could not write the share card: {e}");
                }
                match store::insert(ctx.db(), &owner, &share) {
                    Ok(()) => JobState::Done(share),
                    Err(e) => {
                        JobState::Failed(format!("uploaded, but could not record the share: {e}"))
                    }
                }
            }
            Err(e) => JobState::Failed(e),
        };
        with_jobs(|jobs| {
            if let Some(j) = jobs.get_mut(&job_id) {
                j.state = state;
            }
        });
    });
    Ok(id)
}

async fn run_upload(bin: &Path, req: &UploadRequest) -> Result<GokapiFile, String> {
    let tmp = tmp_dir();
    if req.is_dir {
        std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    }
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(build_args(&cli_config_path(), req, &tmp))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let out = cmd.output().await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "gokapi-cli is not installed".to_string()
        } else {
            format!("gokapi-cli: {e}")
        }
    })?;
    let stdout = String::from_utf8_lossy(&out.stdout[..out.stdout.len().min(MAX_OUTPUT)]);
    match parse_output(&stdout) {
        Ok(f) if out.status.success() => Ok(f),
        Ok(_) => Err(format!("gokapi-cli failed ({})", out.status)),
        Err(msg) => {
            let err = String::from_utf8_lossy(&out.stderr);
            let err = err.trim();
            Err(if err.is_empty() {
                msg
            } else {
                format!("{msg} {err}")
            })
        }
    }
}

/// `{ state: "running" | "done" | "failed", share?, error? }`, or `None` for
/// an unknown job or one owned by someone else.
pub fn job_json(owner: &str, id: &str) -> Option<Json> {
    with_jobs(|jobs| {
        let j = jobs.get(id).filter(|j| j.owner == owner)?;
        Some(match &j.state {
            JobState::Running => json!({ "state": "running" }),
            JobState::Done(s) => json!({ "state": "done", "share": s }),
            JobState::Failed(e) => json!({ "state": "failed", "error": e }),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(is_dir: bool) -> UploadRequest {
        UploadRequest {
            abs_path: PathBuf::from("/home/u/Documents/a b.txt"),
            rel_path: "Documents/a b.txt".into(),
            is_dir,
            expiry_days: 7,
            max_downloads: 0,
            password: Some("pw".into()),
            from: String::new(),
            message: String::new(),
            e2e: false,
        }
    }

    fn strs(v: Vec<OsString>) -> Vec<String> {
        v.into_iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn file_upload_args() {
        let a = strs(build_args(
            Path::new("/c.json"),
            &req(false),
            Path::new("/t"),
        ));
        assert_eq!(
            a,
            [
                "upload",
                "--file",
                "/home/u/Documents/a b.txt",
                "--configuration",
                "/c.json",
                "--json",
                "-x",
                "--expiry-days",
                "7",
                "--password",
                "pw"
            ]
        );
    }

    #[test]
    fn folder_upload_args_and_unlimited_omits_flags() {
        let mut r = req(true);
        r.expiry_days = 0;
        r.password = None;
        r.max_downloads = 5;
        let a = strs(build_args(Path::new("/c.json"), &r, Path::new("/t")));
        assert_eq!(
            &a[..5],
            [
                "upload-dir",
                "--directory",
                "/home/u/Documents/a b.txt",
                "--tempfolder",
                "/t"
            ]
        );
        assert!(!a.contains(&"--expiry-days".to_string()));
        assert!(!a.contains(&"--password".to_string()));
        assert!(a.ends_with(&["--expiry-downloads".to_string(), "5".to_string()]));
    }

    #[test]
    fn e2e_upload_drops_disable_flag() {
        let mut r = req(false);
        r.e2e = true;
        let a = strs(build_args(Path::new("/c.json"), &r, Path::new("/t")));
        assert!(!a.contains(&"-x".to_string()));
        assert!(a.contains(&"--json".to_string()));
    }

    #[test]
    fn parses_json_line_and_reports_errors() {
        let ok = "Compressing...\n{\"Id\":\"x1\",\"Name\":\"a\",\"UrlDownload\":\"https://h/d?id=x1\"}\n";
        assert_eq!(parse_output(ok).unwrap().id, "x1");
        let err = parse_output("ERROR: Unauthorised API key.\n").unwrap_err();
        assert!(err.contains("Unauthorised"));
        assert_eq!(
            parse_output("").unwrap_err(),
            "gokapi-cli returned no result"
        );
    }

    #[test]
    fn jobs_are_private_to_their_owner() {
        with_jobs(|j| {
            j.insert(
                "jid".into(),
                Job {
                    owner: "alice".into(),
                    state: JobState::Running,
                    created: Instant::now(),
                },
            );
        });
        assert_eq!(job_json("alice", "jid").unwrap()["state"], "running");
        assert!(job_json("bob", "jid").is_none());
    }
}
