#!/usr/bin/env bash
# Launch the Qwen3-TTS sidecar (qwentts.cpp `tts-server`).
#
# Unlike Supertonic, qwentts.cpp is a GGML program with a real **Vulkan**
# backend, so an AMD/Intel GPU is usable — not just CUDA or CPU. The device is
# chosen by voice/detect_accel.py: Vulkan0 → CUDA0 → CPU.
#
# The binary is built once per distribution by voice/build_qwentts.sh, which
# compiles in whatever backends that distribution can support (CUDA when an
# nvcc exists, Vulkan when glslc does, and always the portable CPU kernels).
# Backends are DL-loaded, so a Vulkan-capable build still starts on a machine
# with no Vulkan driver by falling back to CPU.
#
# Qwen3-TTS has a known model-intrinsic bug where it sometimes never emits its
# codec EOS token (QwenLM/Qwen3-TTS#118). The server is always asked for
# streaming PCM by the core, which decodes in small chunks and — unlike the
# buffered WAV path — does not build one giant vocoder graph that hangs the GPU.
#
# Usage:
#   ./voice/start_qwen_tts.sh              # start (builds if needed)
#   ./voice/start_qwen_tts.sh --check      # exit 0 when the sidecar answers
#   ./voice/start_qwen_tts.sh --rebuild    # force a rebuild first
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

HOST="${QWEN_TTS_HOST:-127.0.0.1}"
PORT="${QWEN_TTS_PORT:-7787}"
SRC="$ROOT/build/qwentts.cpp"
BIN="$SRC/build/tts-server"
LOG="${QWEN_TTS_LOG_FILE:-$ROOT/data/qwen-tts-sidecar.log}"
MODELS_DIR="${QWEN_TTS_MODELS_DIR:-$ROOT/data/qwen-tts-models}"

TALKER="${QWEN_TTS_TALKER:-qwen-talker-0.6b-customvoice-Q4_K_M.gguf}"
CODEC="${QWEN_TTS_CODEC:-qwen-tokenizer-12hz-Q4_K_M.gguf}"

CHECK_ONLY=0
REBUILD=0
for arg in "$@"; do
  case "$arg" in
    --check) CHECK_ONLY=1 ;;
    --rebuild) REBUILD=1 ;;
  esac
done

sidecar_up() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsS --max-time 2 "http://$HOST:$PORT/v1/audio/voices" >/dev/null 2>&1
  else
    python3 - "$HOST" "$PORT" <<'PY' >/dev/null 2>&1
import sys, urllib.request
urllib.request.urlopen(f"http://{sys.argv[1]}:{sys.argv[2]}/v1/audio/voices", timeout=2).read()
PY
  fi
}

# Someone (the core server, or a previous run) may already own the port.
if sidecar_up; then
  echo "Qwen3-TTS sidecar already listening on $HOST:$PORT"
  exit 0
fi
[ "$CHECK_ONLY" = "1" ] && exit 1

# Build on first use. The build is heavy, so it happens once and is reused.
if [ "$REBUILD" = "1" ] || [ ! -x "$BIN" ]; then
  mkdir -p "$(dirname "$LOG")"
  echo "Building qwentts.cpp (one-time)…" >&2
  "$ROOT/voice/build_qwentts.sh" "$@" >>"$LOG" 2>&1 || {
    echo "qwentts.cpp build failed — see $LOG" >&2
    exit 3
  }
fi

if [ ! -f "$MODELS_DIR/$TALKER" ] || [ ! -f "$MODELS_DIR/$CODEC" ]; then
  echo "Qwen3-TTS weights missing — fetch them once:" >&2
  echo "  ./voice/download_qwen_tts.py 0.6b-customvoice" >&2
  echo "(or download a model from Settings → Voice)" >&2
  exit 2
fi

# ── Device selection: Vulkan → CUDA → CPU ───────────────────────────────────
# detect_accel prints the ggml device name for this machine, preferring a GPU
# whose Vulkan driver is actually usable, and says why to the log.
DEVICE="$(python3 "$ROOT/voice/detect_accel.py" ggml 2>>"$LOG" || echo CPU)"
export GGML_BACKEND="${GGML_BACKEND:-$DEVICE}"

# Mesa's RADV exposes VK_KHR_shader_integer_dot_product on AMD parts where the
# ggml shader for it is wrong: the Q4_K dequant matmuls come out corrupt, which
# shows up as very quiet, unintelligible audio (no crash). Measured on a Navi14
# (RX 5500M): with the extension enabled the render is ~12 dB too quiet and
# transcribes as nothing; disabling it restores correct speech at the same
# speed. Only this one feature is suspect, so nothing else is disabled.
case "$GGML_BACKEND" in
  Vulkan*)
    if vulkaninfo --summary 2>/dev/null | grep -qE "RADV|AMD"; then
      export GGML_VK_DISABLE_INTEGER_DOT_PRODUCT="${GGML_VK_DISABLE_INTEGER_DOT_PRODUCT:-1}"
    fi
    ;;
esac

echo "Starting Qwen3-TTS sidecar on $HOST:$PORT with $BIN (device=$GGML_BACKEND)" >&2
mkdir -p "$(dirname "$LOG")"

exec "$BIN" \
  --model "$MODELS_DIR/$TALKER" \
  --codec "$MODELS_DIR/$CODEC" \
  --alias qwen3-tts \
  --host "$HOST" --port "$PORT" >>"$LOG" 2>&1
