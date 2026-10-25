//! Unix-socket JSON-lines server.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use shiny_installer_protocol::{
    methods, ApplyRequest, Health, InstallPlan, JobStatus, RpcRequest, RpcResponse,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use uuid::Uuid;

use crate::probe;
use crate::engine;

pub struct Job {
    pub status: Mutex<JobStatus>,
    pub cancel: AtomicBool,
}

fn live_session_available() -> bool {
    Path::new("/run/live/medium").exists()
}

pub struct AppState {
    pub socket: PathBuf,
    pub dry_run: bool,
    pub jobs: Mutex<HashMap<String, Arc<Job>>>,
    pub hardware: shiny_installer_protocol::HardwareInfo,
}

impl AppState {
    pub fn new(socket: PathBuf, dry_run: bool) -> Self {
        Self {
            socket,
            dry_run,
            jobs: Mutex::new(HashMap::new()),
            hardware: probe::hardware(),
        }
    }
}

pub async fn run(state: Arc<AppState>) -> io::Result<()> {
    if let Ok(meta) = fs::symlink_metadata(&state.socket) {
        if meta.file_type().is_socket() {
            fs::remove_file(&state.socket)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{} exists and is not a socket; refusing to replace it",
                    state.socket.display()
                ),
            ));
        }
    }
    if let Some(parent) = state.socket.parent() {
        fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(&state.socket)?;
    let _ = fs::set_permissions(&state.socket, fs::Permissions::from_mode(0o660));
    chgrp_shiny(&state.socket);

    println!(
        "shiny-installer-helper {} listening on {} (dry_run={})",
        env!("CARGO_PKG_VERSION"),
        state.socket.display(),
        state.dry_run
    );

    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_connection(state, stream).await {
                eprintln!("installer helper connection error: {error}");
            }
        });
    }
}

async fn handle_connection(state: Arc<AppState>, stream: tokio::net::UnixStream) -> io::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<RpcRequest>(&line) {
            Ok(request) => dispatch(&state, request).await,
            Err(error) => RpcResponse::err("", format!("invalid JSON request: {error}")),
        };
        let mut bytes = serde_json::to_vec(&response)
            .unwrap_or_else(|_| br#"{"id":"","ok":false,"error":"serialization failed"}"#.to_vec());
        bytes.push(b'\n');
        writer.write_all(&bytes).await?;
        writer.flush().await?;
    }
    Ok(())
}

async fn dispatch(state: &Arc<AppState>, request: RpcRequest) -> RpcResponse {
    let id = request.id.clone();
    match request.method.as_str() {
        methods::HEALTH => {
            let health = Health {
                version: env!("CARGO_PKG_VERSION").into(),
                protocol: shiny_installer_protocol::PROTOCOL_VERSION,
                dry_run: state.dry_run,
                root: is_root(),
                socket: state.socket.display().to_string(),
                hardware: state.hardware.clone(),
            };
            serialize(id, &health)
        }
        methods::OPTIONS => serialize(id, &probe::system_options()),
        methods::PROBE => match probe::probe() {
            Ok(probe) => serialize(id, &probe),
            Err(error) => RpcResponse::err(id, error),
        },
        methods::VALIDATE => match parse_plan(&request.params) {
            Ok(plan) => match probe::validate(&plan) {
                Ok(validation) => serialize(id, &validation),
                Err(error) => RpcResponse::err(id, error),
            },
            Err(error) => RpcResponse::err(id, error),
        },
        methods::APPLY => match parse_apply(&request.params) {
            Ok(apply) => {
                let plan = apply.plan;
                match probe::validate(&plan) {
                    Ok(validation) if validation.ok => {
                        if state.dry_run {
                            start_dry_run(state, id, plan)
                        } else if live_session_available() {
                            start_apply(state, id, plan)
                        } else {
                            RpcResponse::err(
                                id,
                                "refusing to write outside the live session: /run/live/medium is missing",
                            )
                        }
                    }
                    Ok(validation) => RpcResponse::err(
                        id,
                        format!("plan validation failed: {}", validation.errors.join("; ")),
                    ),
                    Err(error) => RpcResponse::err(id, error),
                }
            }
            Err(error) => RpcResponse::err(id, error),
        },
        methods::STATUS => {
            let Some(job_id) = job_param(&request.params) else {
                return RpcResponse::err(id, "job is required");
            };
            status_for(&state, &id, &job_id)
        }
        methods::CANCEL => {
            let Some(job_id) = job_param(&request.params) else {
                return RpcResponse::err(id, "job is required");
            };
            let job = state.jobs.lock().unwrap().get(&job_id).cloned();
            match job {
                Some(job) => {
                    job.cancel.store(true, Ordering::SeqCst);
                    RpcResponse::ok(id, json!({ "cancelled": true }))
                }
                None => RpcResponse::err(id, format!("unknown installer job: {job_id}")),
            }
        }
        other => RpcResponse::err(id, format!("unknown method: {other}")),
    }
}

fn serialize<T: serde::Serialize>(id: String, value: &T) -> RpcResponse {
    match serde_json::to_value(value) {
        Ok(data) => RpcResponse::ok(id, data),
        Err(error) => RpcResponse::err(id, format!("serialization failed: {error}")),
    }
}

fn parse_plan(params: &Value) -> Result<InstallPlan, String> {
    let value = params
        .get("plan")
        .cloned()
        .unwrap_or_else(|| params.clone());
    if value.is_null() {
        return Err("plan is required".into());
    }
    serde_json::from_value(value).map_err(|error| format!("invalid install plan: {error}"))
}

fn parse_apply(params: &Value) -> Result<ApplyRequest, String> {
    let apply: ApplyRequest = serde_json::from_value(params.clone())
        .map_err(|error| format!("invalid apply request: {error}"))?;
    if apply.plan.target.serial.trim().is_empty() {
        return Err("target disk serial is required for destructive install".into());
    }
    if apply.confirm_serial.trim() != apply.plan.target.serial {
        return Err("typed disk serial does not match the plan target".into());
    }
    Ok(apply)
}

fn job_param(params: &Value) -> Option<String> {
    params
        .get("job")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn status_for(state: &Arc<AppState>, id: &str, job_id: &str) -> RpcResponse {
    let job = state.jobs.lock().unwrap().get(job_id).cloned();
    match job {
        Some(job) => {
            let status = job.status.lock().unwrap().clone();
            serialize(id.to_string(), &status)
        }
        None => RpcResponse::err(id, format!("unknown installer job: {job_id}")),
    }
}

fn start_dry_run(state: &Arc<AppState>, id: String, plan: InstallPlan) -> RpcResponse {
    let job_id = Uuid::new_v4().to_string();
    let job = Arc::new(Job {
        status: Mutex::new(JobStatus {
            job: job_id.clone(),
            stage: "queued".into(),
            progress: 0,
            done: false,
            success: false,
            error: None,
            log: vec!["dry-run job accepted".into()],
        }),
        cancel: AtomicBool::new(false),
    });
    state
        .jobs
        .lock()
        .unwrap()
        .insert(job_id.clone(), job.clone());

    let state = state.clone();
    tokio::spawn(async move {
        run_dry_job(state, plan, job).await;
    });

    RpcResponse::ok(id, json!({ "job": job_id, "dry_run": true }))
}

fn start_apply(state: &Arc<AppState>, id: String, plan: InstallPlan) -> RpcResponse {
    let job_id = Uuid::new_v4().to_string();
    let job = Arc::new(Job {
        status: Mutex::new(JobStatus {
            job: job_id.clone(),
            stage: "queued".into(),
            progress: 0,
            done: false,
            success: false,
            error: None,
            log: vec!["install job accepted".into()],
        }),
        cancel: AtomicBool::new(false),
    });
    state
        .jobs
        .lock()
        .unwrap()
        .insert(job_id.clone(), job.clone());

    let engine_job = job.clone();
    tokio::task::spawn_blocking(move || {
        let result = engine::apply(&plan, false, &engine_job.cancel, &mut |stage, progress| {
            update_status(&engine_job, |status| {
                status.stage = stage.into();
                status.progress = progress;
                status.log.push(format!("{stage}: {progress}%"));
            });
        });
        match result {
            Ok(()) => update_status(&engine_job, |status| {
                status.done = true;
                status.success = true;
                status.error = None;
                status.stage = "done".into();
                status.progress = 100;
                status.log.push("install complete; target is ready to boot".into());
            }),
            Err(error) => fail_job(&engine_job, &error),
        }
    });

    RpcResponse::ok(id, json!({ "job": job_id, "dry_run": false }))
}

async fn run_dry_job(state: Arc<AppState>, plan: InstallPlan, job: Arc<Job>) {
    let stages: [(&str, u8); 7] = [
        ("prepare", 5),
        ("partition", 20),
        ("format", 35),
        ("unpack", 55),
        ("configure", 75),
        ("bootloader", 90),
        ("finish", 100),
    ];

    update_status(&job, |status| {
        status.log.push(format!(
            "dry-run: target={} profile={} filesystem={} mode={}",
            plan.target.path, plan.profile, plan.filesystem, plan.mode
        ));
        if let Some(root) = &plan.target_partition {
            status
                .log
                .push(format!("dry-run: root partition={}", root.path));
        }
        if let Some(esp) = &plan.esp_partition {
            status
                .log
                .push(format!("dry-run: EFI system partition={}", esp.path));
        }
        if plan.nvidia.install_driver {
            status.log.push(format!(
                "dry-run: NVIDIA driver requested (CUDA={})",
                plan.nvidia.install_cuda
            ));
            for license in &plan.licenses {
                status.log.push(format!(
                    "dry-run: license {} version {} accepted at {}",
                    license.id, license.version, license.accepted_at
                ));
            }
        }
    });

    for (stage, progress) in stages {
        if job.cancel.load(Ordering::SeqCst) {
            fail_job(&job, "cancelled");
            return;
        }
        update_status(&job, |status| {
            status.stage = stage.into();
            status.progress = progress;
            status.log.push(format!("dry-run: {stage}"));
        });
        tokio::time::sleep(Duration::from_millis(350)).await;
    }

    if job.cancel.load(Ordering::SeqCst) {
        fail_job(&job, "cancelled");
        return;
    }
    update_status(&job, |status| {
        status.done = true;
        status.success = true;
        status.error = None;
        status.stage = "done".into();
        status.progress = 100;
        status
            .log
            .push("dry-run complete; no disk was modified".into());
    });

    // Keep the state alive until the job is observed; jobs are retained for
    // polling and for the small process lifetime.
    let _ = state;
}

fn update_status(job: &Job, update: impl FnOnce(&mut JobStatus)) {
    let mut status = job.status.lock().unwrap();
    update(&mut status);
}

fn fail_job(job: &Job, error: &str) {
    update_status(job, |status| {
        status.done = true;
        status.success = false;
        status.error = Some(error.into());
        status.stage = "failed".into();
        status.log.push(format!("job stopped: {error}"));
    });
}

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

fn chgrp_shiny(path: &Path) {
    use std::ffi::CString;

    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return;
    };
    let Ok(group) = CString::new("shiny") else {
        return;
    };
    unsafe {
        let entry = libc::getgrnam(group.as_ptr());
        if entry.is_null() {
            eprintln!("installer helper: group 'shiny' not found; socket stays root:root");
            return;
        }
        let gid = (*entry).gr_gid;
        if libc::chown(path.as_ptr(), u32::MAX, gid) != 0 {
            eprintln!("installer helper: could not chgrp socket to 'shiny'");
        }
    }
}
