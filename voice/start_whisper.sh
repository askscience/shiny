#!/usr/bin/env bash
# Launch the faster-whisper streaming STT sidecar.
#
# The sidecar needs the `faster-whisper` Python package (CTranslate2). This
# script finds an interpreter that already has it — the app's own venv first,
# then whatever python3 is on PATH — and only builds a private venv when asked
# to (`--install`, or WHISPER_AUTO_INSTALL=1). Probing before installing keeps
# a normal start instant and never surprises the user with a silent pip run.
#
# It also makes sure the default **tiny** model is on disk before starting: it
# is not committed (data/ is gitignored), so the first start fetches it once
# (~72 MB) via the stdlib-only voice/download_whisper.py.
#
# Usage:
#   ./voice/start_whisper.sh              # start (assumes deps are present)
#   ./voice/start_whisper.sh --install    # create .venv-whisper and install deps
#   ./voice/start_whisper.sh --check      # exit 0 when the sidecar answers
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

HOST="${WHISPER_HOST:-127.0.0.1}"
PORT="${WHISPER_PORT:-7789}"
VENV="$ROOT/.venv-whisper"
PY="${WHISPER_PYTHON:-}"
LOG="${WHISPER_LOG_FILE:-$ROOT/data/whisper-sidecar.log}"

have_deps() {
  [ -n "$1" ] && [ -x "$1" ] && "$1" - <<'PY' >/dev/null 2>&1
import faster_whisper, uvicorn, fastapi  # noqa: F401
PY
}

sidecar_up() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsS --max-time 2 "http://$HOST:$PORT/health" >/dev/null 2>&1
  else
    "$1" - "$HOST" "$PORT" <<'PY' >/dev/null 2>&1
import json, sys, urllib.request
urllib.request.urlopen(f"http://{sys.argv[1]}:{sys.argv[2]}/health", timeout=2).read()
PY
  fi
}

install_deps() {
  echo "Creating $VENV and installing faster-whisper (one-time)…"
  "${WHISPER_BOOTSTRAP_PYTHON:-python3}" -m venv "$VENV"
  "$VENV/bin/pip" install --quiet --upgrade pip
  "$VENV/bin/pip" install --quiet "faster-whisper>=1.2,<2" "fastapi>=0.110" "uvicorn>=0.27"
  PY="$VENV/bin/python"
}

# Pick the interpreter: explicit override → app venv → PATH python → common
# system/conda installs (a machine that already has CTranslate2 should not be
# made to download it twice).
if [ -n "$PY" ] && ! have_deps "$PY"; then
  echo "WHISPER_PYTHON=$PY does not provide faster-whisper" >&2
  PY=""
fi
for candidate in "$VENV/bin/python" "$(command -v python3 || true)" \
                 "$(command -v python || true)" /opt/anaconda3/bin/python3 \
                 /usr/local/bin/python3; do
  if [ -z "$PY" ] && have_deps "$candidate"; then
    PY="$candidate"
    break
  fi
done

CHECK_ONLY=0
for arg in "$@"; do
  case "$arg" in
    --install) WHISPER_AUTO_INSTALL=1 ;;
    --check) CHECK_ONLY=1 ;;
  esac
done

# Someone (the core server, or a previous run) may already own the port.
probe_py="${PY:-python3}"
if sidecar_up "$probe_py"; then
  echo "faster-whisper sidecar already listening on $HOST:$PORT"
  exit 0
fi
[ "$CHECK_ONLY" = "1" ] && exit 1

if [ -z "$PY" ]; then
  if [ "${WHISPER_AUTO_INSTALL:-0}" = "1" ]; then
    mkdir -p "$(dirname "$LOG")"
    # Detached runs keep the pip log; an interactive run still shows progress.
    if [ -t 1 ]; then
      install_deps 2>&1 | tee -a "$LOG"
    else
      install_deps >>"$LOG" 2>&1
    fi
    # The pipeline versions run in a subshell, so re-point PY here rather than
    # relying on the assignment inside install_deps.
    PY="$VENV/bin/python"
  else
    cat >&2 <<EOF
faster-whisper is not installed.
  • Install it once:   ./voice/start_whisper.sh --install
  • Or point at a Python that has it:  WHISPER_PYTHON=/path/to/python ./voice/start_whisper.sh
EOF
    exit 2
  fi
fi

export KMP_DUPLICATE_LIB_OK="${KMP_DUPLICATE_LIB_OK:-TRUE}"
export WHISPER_HOST="$HOST"
export WHISPER_PORT="$PORT"
mkdir -p "$(dirname "$LOG")"

# The default model is **tiny**. It is not committed to git (all of `data/` is
# ignored), so fetch it once here: the sidecar refuses to start without a local
# model, and "bundled" is only true after this runs. Stdlib-only downloader, so
# it works under any python3, before/without the venv.
MODELS_DIR="${WHISPER_MODELS_DIR:-$ROOT/data/whisper-models}"
if [ ! -f "$MODELS_DIR/faster-whisper-tiny/model.bin" ]; then
  echo "faster-whisper: tiny model missing — downloading to $MODELS_DIR (~72 MB, one-time)…"
  if [ -t 1 ]; then
    "${WHISPER_BOOTSTRAP_PYTHON:-python3}" "$ROOT/voice/download_whisper.py" tiny \
      || echo "faster-whisper: tiny model download failed; will retry on next start" >&2
  else
    "${WHISPER_BOOTSTRAP_PYTHON:-python3}" "$ROOT/voice/download_whisper.py" tiny \
      >>"$LOG" 2>&1 \
      || echo "faster-whisper: tiny model download failed; will retry on next start" >&2
  fi
fi

echo "Starting faster-whisper sidecar on $HOST:$PORT with $PY"
if [ -t 1 ]; then
  exec "$PY" "$ROOT/voice/whisper_server.py"
else
  # Detached (started by the core server): keep the sidecar's own diagnostics.
  exec "$PY" "$ROOT/voice/whisper_server.py" >>"$LOG" 2>&1
fi
