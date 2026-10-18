//! Detached voice daemons intentionally outlive the server. Never kill an
//! existing shared daemon during server shutdown. Launchers probe before spawn.
use crate::config::Config;
use std::process::{Child, Command, Stdio};

/// Reap a detached sidecar in the background.
///
/// The sidecars are deliberately detached: they outlive the server, and a
/// second server start finds them already listening and exits without starting
/// another. But a child whose `Child` handle is simply dropped becomes a
/// **zombie** the moment it exits, because nothing ever calls `wait()` — which
/// is exactly what the start scripts do after their health probe finds an
/// existing daemon. A thread parked on `wait()` reaps it, costs nothing, and
/// keeps the process table clean.
fn reap_in_background(mut child: Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

/// Bring up the Supertonic TTS sidecar (`voice/start_supertonic.sh`).
///
/// Detached and non-blocking, mirroring [`spawn_whisper_sidecar`]: the script
/// probes the port first, picks an interpreter that has `supertonic[serve]`
/// (the app venv, then PATH), and selects the execution provider (Vulkan →
/// CUDA → CPU via `voice/detect_accel.py`). `AUTO_START_SUPERTONIC` defaults
/// on so TTS works out of the box; `SUPERTONIC_AUTO_INSTALL` lets the launcher
/// provision the venv when the package is absent.
pub fn spawn_supertonic_sidecar(config: &Config) {
    let port = config
        .supertonic_url
        .rsplit(':')
        .next()
        .and_then(|p| p.trim_end_matches('/').parse::<u16>().ok())
        .unwrap_or(7788);

    let mut cmd = Command::new("bash");
    cmd.arg("voice/start_supertonic.sh")
        .env("SUPERTONIC_PORT", port.to_string());
    if let Some(python) = &config.supertonic_python {
        cmd.env("SUPERTONIC_PYTHON", python);
    }
    if config.auto_install_supertonic {
        cmd.env("SUPERTONIC_AUTO_INSTALL", "1");
    } else {
        cmd.env("SUPERTONIC_AUTO_INSTALL", "0");
    }

    match cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => {
            reap_in_background(child);
            tracing::info!("Starting Supertonic sidecar on port {}", port)
        }
        Err(e) => tracing::warn!("Could not auto-start Supertonic: {}", e),
    }
}

/// Bring up the faster-whisper STT sidecar (`voice/start_whisper.sh`).
///
/// Detached and non-blocking: the script probes `WHISPER_URL/health` first, so
/// a second server process never double-starts it, and it exits quietly when
/// no interpreter provides faster-whisper — in that case the app simply stays
/// on Vosk and Settings offers to install it.
pub fn spawn_whisper_sidecar(config: &Config) {
    let port = config
        .whisper_url
        .rsplit(':')
        .next()
        .and_then(|p| p.trim_end_matches('/').parse::<u16>().ok())
        .unwrap_or(7789);

    let mut cmd = Command::new("bash");
    cmd.arg("voice/start_whisper.sh")
        .env("WHISPER_PORT", port.to_string())
        .env("WHISPER_MODELS_DIR", &config.whisper_models_dir);
    if let Some(python) = &config.whisper_python {
        cmd.env("WHISPER_PYTHON", python);
    }
    if config.auto_install_whisper {
        cmd.env("WHISPER_AUTO_INSTALL", "1");
    }

    match cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => {
            reap_in_background(child);
            tracing::info!("Starting faster-whisper sidecar on port {}", port)
        }
        Err(e) => tracing::warn!("Could not auto-start faster-whisper: {}", e),
    }
}

/// Bring up the Qwen3-TTS sidecar (`voice/start_qwen_tts.sh`).
///
/// Opt-in: the launcher builds `qwentts.cpp` on first use and downloads the
/// GGUF weights, both of which are heavy. It is idempotent (the port is probed
/// first) and picks Vulkan → CUDA → CPU via `voice/detect_accel.py`.
pub fn spawn_qwen_tts_sidecar(config: &Config) {
    let port = config
        .qwen_tts_url
        .rsplit(':')
        .next()
        .and_then(|p| p.trim_end_matches('/').parse::<u16>().ok())
        .unwrap_or(7787);

    let mut cmd = Command::new("bash");
    cmd.arg("voice/start_qwen_tts.sh")
        .env("QWEN_TTS_PORT", port.to_string())
        .env("QWEN_TTS_MODELS_DIR", &config.qwen_tts_models_dir);

    match cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => {
            reap_in_background(child);
            tracing::info!("Starting Qwen3-TTS sidecar on port {}", port)
        }
        Err(e) => tracing::warn!("Could not auto-start Qwen3-TTS: {}", e),
    }
}
