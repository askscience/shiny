#!/usr/bin/env bash
# Launch the Supertonic TTS sidecar (`supertonic serve`).
#
# Mirrors voice/start_whisper.sh: it probes an interpreter that already has the
# package (the app venv first, then whatever python3 is on PATH) and only builds
# a private venv when asked to (`--install`, or SUPERTONIC_AUTO_INSTALL=1). The
# app passes SUPERTONIC_AUTO_INSTALL=1 by default so a fresh install provisions
# itself; set it to 0 to keep startup instant and install on demand.
#
# Acceleration is chosen by voice/detect_accel.py: it prefers a usable Vulkan
# device, then CUDA, then CPU. ONNX Runtime currently ships no Vulkan execution
# provider, so on real hardware this resolves to CUDA (NVIDIA) or CPU; the
# detector reports the reason to the log.
#
# Usage:
#   ./voice/start_supertonic.sh              # start (assumes deps are present)
#   ./voice/start_supertonic.sh --install    # create .venv-supertonic and install deps
#   ./voice/start_supertonic.sh --check      # exit 0 when the sidecar answers
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

HOST="${SUPERTONIC_HOST:-127.0.0.1}"
PORT="${SUPERTONIC_PORT:-7788}"
VENV="$ROOT/.venv-supertonic"
PY="${SUPERTONIC_PYTHON:-}"
LOG="${SUPERTONIC_LOG_FILE:-$ROOT/data/supertonic-sidecar.log}"

have_deps() {
  [ -n "$1" ] && [ -x "$1" ] && "$1" - <<'PY' >/dev/null 2>&1
import supertonic, fastapi, uvicorn  # noqa: F401
PY
}

sidecar_up() {
  if command -v curl >/dev/null 2>&1; then
    # `/v1/health` answers 503 while the model is still loading, so "bound but
    # not ready" is not mistaken for ready (and `/docs` is served that early).
    curl -fsS --max-time 2 "http://$HOST:$PORT/v1/health" >/dev/null 2>&1
  else
    "$1" - "$HOST" "$PORT" <<'PY' >/dev/null 2>&1
import sys, urllib.request
urllib.request.urlopen(f"http://{sys.argv[1]}:{sys.argv[2]}/v1/health", timeout=2).read()
PY
  fi
}

install_deps() {
  echo "Creating $VENV and installing supertonic[serve] (one-time)…"
  "${SUPERTONIC_BOOTSTRAP_PYTHON:-python3}" -m venv "$VENV"
  "$VENV/bin/pip" install --quiet --upgrade pip
  "$VENV/bin/pip" install --quiet "supertonic[serve]>=1.3.1"
  PY="$VENV/bin/python"
}

# Pick the interpreter: explicit override → app venv → PATH python.
if [ -n "$PY" ] && ! have_deps "$PY"; then
  echo "SUPERTONIC_PYTHON=$PY does not provide supertonic" >&2
  PY=""
fi
for candidate in "$VENV/bin/python" "$(command -v python3 || true)" \
                 "$(command -v python || true)"; do
  if [ -z "$PY" ] && have_deps "$candidate"; then
    PY="$candidate"
    break
  fi
done

CHECK_ONLY=0
for arg in "$@"; do
  case "$arg" in
    --install) SUPERTONIC_AUTO_INSTALL=1 ;;
    --check) CHECK_ONLY=1 ;;
  esac
done

# Someone (the core server, or a previous run) may already own the port.
probe_py="${PY:-python3}"
if sidecar_up "$probe_py"; then
  echo "Supertonic sidecar already listening on $HOST:$PORT"
  exit 0
fi
[ "$CHECK_ONLY" = "1" ] && exit 1

# Two launchers run on every login — shiny-session and the core server — and a
# start is slow enough (bind, fetch/load the ONNX model) that both could get
# past the probe above and race for the port: the loser died on EADDRINUSE
# after paying the model load. The starter holds this lock for as long as the
# sidecar lives (the descriptor is inherited through the exec below), so a
# launcher that arrives during a start waits for health instead of racing it.
# The lock is per-port runtime state, never a file in the install tree — a
# distro package may ship that read-only. flock is a Debian staple but not on
# macOS; without it the port probe is the fallback.
runtime_dir="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
[ -d "$runtime_dir" ] || runtime_dir="${TMPDIR:-/tmp}"
LOCK="${SUPERTONIC_LOCK_FILE:-$runtime_dir/shiny-supertonic-$PORT.lock}"
if command -v flock >/dev/null 2>&1 && : >>"$LOCK" 2>/dev/null; then
  exec 9>>"$LOCK"
  if ! flock -n 9; then
    echo "Supertonic sidecar is starting on $HOST:$PORT"
    took_over=0
    for _ in $(seq 1 120); do  # ~60 s: a cold model load is slow
      sleep 0.5
      if sidecar_up "$probe_py"; then
        echo "Supertonic sidecar ready on $HOST:$PORT"
        exit 0
      fi
      if flock -n 9; then  # the starter died — take over and start ours
        took_over=1
        break
      fi
    done
    if [ "$took_over" != "1" ]; then
      echo "Supertonic sidecar did not answer on $HOST:$PORT" >&2
      exit 1
    fi
  fi
fi

if [ -z "$PY" ]; then
  if [ "${SUPERTONIC_AUTO_INSTALL:-1}" = "1" ]; then
    mkdir -p "$(dirname "$LOG")"
    if [ -t 1 ]; then
      install_deps 2>&1 | tee -a "$LOG"
    else
      install_deps >>"$LOG" 2>&1
    fi
    PY="$VENV/bin/python"
  else
    cat >&2 <<EOF
supertonic is not installed.
  • Install it once:   ./voice/start_supertonic.sh --install
  • Or point at a Python that has it:  SUPERTONIC_PYTHON=/path/to/python ./voice/start_supertonic.sh
EOF
    exit 2
  fi
fi

export SUPERTONIC_HOST="$HOST"
export SUPERTONIC_PORT="$PORT"
mkdir -p "$(dirname "$LOG")"

# Vulkan → CUDA → CPU (see detect_accel.py). Falls back to CPU if the probe
# itself fails, so a broken detector can never take TTS down.
if ! accel="$(SUPERTONIC_ONNX_PROVIDERS="${SUPERTONIC_ONNX_PROVIDERS:-}" \
    "$PY" "$ROOT/voice/detect_accel.py" supertonic 2>>"$LOG")"; then
  accel="export SUPERTONIC_ONNX_PROVIDERS='CPUExecutionProvider'"
fi
# shellcheck disable=SC1090
eval "$accel"

echo "Starting Supertonic sidecar on $HOST:$PORT with $PY (providers=$SUPERTONIC_ONNX_PROVIDERS)"
if [ -t 1 ]; then
  exec "$PY" "$ROOT/voice/supertonic_server.py"
else
  # Detached (started by the core server): keep the sidecar's own diagnostics.
  exec "$PY" "$ROOT/voice/supertonic_server.py" >>"$LOG" 2>&1
fi
