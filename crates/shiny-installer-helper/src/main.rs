//! Privileged installer helper for the live PEAK'D! session.
//!
//! The plugin talks to this process over a Unix socket. This build implements
//! read-only probing, plan validation and a **dry-run** apply; the destructive
//! engine is selected by the Phase 0 spike and is deliberately not wired yet.

mod probe;
mod server;
mod engine;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use shiny_installer_protocol::InstallPlan;

use server::AppState;

fn usage() {
    println!(
        "shiny-installer-helper {}\n\n\
         Usage: shiny-installer-helper [--socket PATH] [--dry-run]\n\n\
         Options:\n\
           --socket PATH   Unix socket to listen on (default: /run/shiny/installer.sock)\n\
           --dry-run       Simulate apply; never touches a disk\n\
           --version       Print the version and exit\n\
           --help          Show this help\n",
        env!("CARGO_PKG_VERSION")
    );
}

#[tokio::main]
async fn main() {
    let mut socket = PathBuf::from("/run/shiny/installer.sock");
    let mut dry_run = false;
    let mut apply_plan: Option<PathBuf> = None;
    let mut allow_loop = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => match args.next() {
                Some(value) => socket = PathBuf::from(value),
                None => {
                    eprintln!("--socket requires a path");
                    std::process::exit(2);
                }
            },
            "--dry-run" => dry_run = true,
            "--apply-plan" => match args.next() {
                Some(value) => apply_plan = Some(PathBuf::from(value)),
                None => {
                    eprintln!("--apply-plan requires a JSON file");
                    std::process::exit(2);
                }
            },
            "--allow-loop" => allow_loop = true,
            "--version" => {
                println!("{}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--help" | "-h" => {
                usage();
                return;
            }
            other => {
                eprintln!("unknown argument: {other}");
                usage();
                std::process::exit(2);
            }
        }
    }

    if let Some(path) = apply_plan {
        if !std::path::Path::new("/run/live/medium").exists() {
            eprintln!(
                "refusing to apply outside the live session: /run/live/medium is missing"
            );
            std::process::exit(1);
        }
        let data = match std::fs::read(&path) {
            Ok(data) => data,
            Err(error) => {
                eprintln!("cannot read plan {}: {error}", path.display());
                std::process::exit(1);
            }
        };
        let plan: InstallPlan = match serde_json::from_slice(&data) {
            Ok(plan) => plan,
            Err(error) => {
                eprintln!("invalid plan: {error}");
                std::process::exit(1);
            }
        };
        let cancel = AtomicBool::new(false);
        if let Err(error) = engine::apply(&plan, allow_loop, &cancel, &mut |stage, progress| {
            println!("{stage}: {progress}%");
        }) {
            eprintln!("install engine failed: {error}");
            std::process::exit(1);
        }
        return;
    }

    let state = Arc::new(AppState::new(socket, dry_run));
    if let Err(error) = server::run(state).await {
        eprintln!("shiny-installer-helper: {error}");
        std::process::exit(1);
    }
}
