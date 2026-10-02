# Speech sidecars

Shiny's voice stack is served by separate processes: two Python FastAPI services
plus one optional native server. The core starts them on demand (or they can be
started by hand), and each launcher is **idempotent** — it probes its port first,
so a second server process never double-starts one.

Source: [`voice/`](../../voice); server wiring in
[`src/main.rs`](../../src/main.rs).

---

## Overview

| Sidecar | Launcher | Server | Default URL | Env |
|---|---|---|---|---|
| **faster-whisper** (STT) | [`voice/start_whisper.sh`](../../voice/start_whisper.sh) | [`voice/whisper_server.py`](../../voice/whisper_server.py) | `http://127.0.0.1:7789` | `WHISPER_*` |
| **Supertonic** (TTS) | [`voice/start_supertonic.sh`](../../voice/start_supertonic.sh) | `supertonic serve` | `http://127.0.0.1:7788` | `SUPERTONIC_*` |
| **Qwen3-TTS** (optional) | [`voice/start_qwen_tts.sh`](../../voice/start_qwen_tts.sh) | `qwentts.cpp` `tts-server` | `http://127.0.0.1:7787` | `QWEN_TTS_*` |

The server maps its config to each launcher's env when it spawns it
(`spawn_whisper_sidecar`, `spawn_supertonic_sidecar`, `spawn_qwen_tts_sidecar`),
passing the port parsed out of the configured URL.

---

## Launcher behaviour (whisper & supertonic)

Both launchers share a shape:

1. Parse `--install` / `--check` flags.
2. Find an interpreter that already has the package: explicit `*_PYTHON`
   override → the app venv (`.venv-whisper` / `.venv-supertonic`) → `python3` /
   `python` on `PATH` (whisper also probes common conda/system paths).
3. If none has it: exit with instructions unless `--install` /
   `*_AUTO_INSTALL=1`, in which case create the venv and pip-install the
   packages (`faster-whisper>=1.2,<2`; `supertonic[serve]>=1.3.1`).
4. Probe the port (`/health` for whisper, `/docs` or `/` for supertonic); if it
   is already listening, exit 0.
5. Start the server, tee'ing logs to `data/*-sidecar.log` when detached.

`--check` exits 0 only when the sidecar answers (used by tests/installers).

### Model download on first start

Neither the faster-whisper `tiny` model nor the Supertonic ONNX model is
committed (all of `data/` is gitignored). The whisper launcher fetches `tiny`
(~72 MB) once via the stdlib-only
[`voice/download_whisper.py`](../../voice/download_whisper.py) before starting,
so it works under any `python3`, before/without the venv. `small` is downloaded
on demand from Settings → Voice (`POST /api/voice/whisper/download`).

### Acceleration detection

[`voice/detect_accel.py`](../../voice/detect_accel.py) chooses
**Vulkan → CUDA → CPU** for the ONNX-based engines. Supertonic currently ships
no Vulkan execution provider, so on real hardware this resolves to CUDA (NVIDIA)
or CPU; the detector logs its reason.

---

## Qwen3-TTS (opt-in)

Unlike Supertonic, Qwen3-TTS is **off by default** because the first start may
build [`qwentts.cpp`](../../build/qwentts.cpp) (several minutes) and download
~600 MB of GGUF weights. Enable it with `AUTO_START_QWEN_TTS=true` or from
Settings → Voice. `/api/voice/qwen/download` fetches a model; the TTS route
selects the engine with `"engine": "qwen"`. The server wraps the sidecar's raw
24 kHz PCM in a WAV container because the buffered WAV path can hang the GPU on
long utterances.

---

## Lifecycles

- **Server-spawned (default).** `AUTO_START_WHISPER=true` (default) and
  `AUTO_START_SUPERTONIC=true` (default) make `main.rs` spawn the launchers
  detached. The child is reaped on a background thread so it never becomes a
  zombie when it exits after finding an existing daemon.
- **Session-spawned.** [`scripts/shiny-session`](../../scripts/shiny-session)
  starts both launchers before opening the kiosk so the engines are warm by the
  first spoken word; on a real multi-user box each user gets their own.
- **Manual.** `./voice/start_whisper.sh` / `./voice/start_supertonic.sh`.

Sidecars are **detached on purpose**: they outlive the server, and a second
server start finds them already listening and exits without starting another.

---

## Logs

| File | Process |
|---|---|
| `data/whisper-sidecar.log` | faster-whisper |
| `data/supertonic-sidecar.log` | Supertonic |
| `data/qwen-tts-sidecar.log` | Qwen3-TTS |

Override the log path with `WHISPER_LOG_FILE` / `SUPERTONIC_LOG_FILE`.

---

## Environment variables

These are consumed by the launchers (the server sets some of them when it
spawns them):

| Variable | Default | Meaning |
|---|---|---|
| `WHISPER_HOST` / `WHISPER_PORT` | `127.0.0.1` / `7789` | Bind. |
| `WHISPER_PYTHON` | — | Explicit interpreter. |
| `WHISPER_AUTO_INSTALL` | `0` | Build the venv if the package is missing. |
| `WHISPER_MODELS_DIR` | `data/whisper-models` | Model storage. |
| `WHISPER_LOG_FILE` | `data/whisper-sidecar.log` | Log file. |
| `WHISPER_BOOTSTRAP_PYTHON` | `python3` | Interpreter used only for the downloader/venv. |
| `SUPERTONIC_HOST` / `SUPERTONIC_PORT` | `127.0.0.1` / `7788` | Bind. |
| `SUPERTONIC_PYTHON` | — | Explicit interpreter. |
| `SUPERTONIC_AUTO_INSTALL` | `1` | Provision `.venv-supertonic` on demand. |
| `SUPERTONIC_LOG_FILE` | `data/supertonic-sidecar.log` | Log file. |
| `QWEN_TTS_PORT` | `7787` | Bind. |
| `QWEN_TTS_MODELS_DIR` | `data/qwen-tts-models` | GGUF storage. |

Server-side variables (`WHISPER_URL`, `SUPERTONIC_URL`, `QWEN_TTS_URL`,
`AUTO_START_*`, `*_AUTO_INSTALL`) are listed in
[configuration](../configuration.md) and [env vars](../reference/env-vars.md).

---

## Graceful degradation

- No Python package / no sidecar: whisper unavailable → the browser falls back
  to **Vosk**; Supertonic unavailable → TTS fails but STT still works.
- Low-power AI mode swaps faster-whisper → Vosk and Qwen/Supertonic →
  Supertonic (see [battery & power](../host/battery-power.md)).
- A failed `tiny` download is retried on the next start and does not abort the
  server.
