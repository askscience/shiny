#!/usr/bin/env bash
# Bisect Vulkan numerical mitigations for qwentts.cpp on this GPU.
#
# For each candidate env combination: start tts-server on a private port, ask
# for one fixed sentence, and score the result. A healthy render looks like the
# CPU reference (peak ~24k, rms ~2.8k, ~6 s, and transcribes).
#
# Usage: voice/bisect_vulkan.sh [--port N] [--keep]
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/build/qwentts.cpp/build/tts-server"
MODEL="$ROOT/data/qwen-tts-models/qwen-talker-0.6b-customvoice-Q4_K_M.gguf"
CODEC="$ROOT/data/qwen-tts-models/qwen-tokenizer-12hz-Q4_K_M.gguf"
OUT="${BISECT_DIR:-/tmp/qwen-bisect}"
TEXT="The quick brown fox jumps over the lazy dog. This is a test of the Qwen three text to speech engine."

PORT=7800
KEEP=0
while [ $# -gt 0 ]; do
  case "$1" in
    --port) PORT="$2"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    *) shift ;;
  esac
done

mkdir -p "$OUT"

# Candidate flag sets, cheapest hypothesis first. Every one of these disables a
# device feature this fork probes for; a broken shader behind any of them would
# corrupt the numerics the way we see.
CANDIDATES=(
  ""
  "GGML_VK_DISABLE_F16=1"
  "GGML_VK_DISABLE_COOPMAT=1 GGML_VK_DISABLE_COOPMAT2=1"
  "GGML_VK_DISABLE_BFLOAT16=1"
  "GGML_VK_DISABLE_INTEGER_DOT_PRODUCT=1"
  "GGML_VK_DISABLE_DOT2=1"
  "GGML_VK_DISABLE_COOPMAT=1 GGML_VK_DISABLE_COOPMAT2=1 GGML_VK_DISABLE_BFLOAT16=1 GGML_VK_DISABLE_INTEGER_DOT_PRODUCT=1"
  "GGML_VK_DISABLE_F16=1 GGML_VK_DISABLE_COOPMAT=1 GGML_VK_DISABLE_BFLOAT16=1 GGML_VK_DISABLE_INTEGER_DOT_PRODUCT=1 GGML_VK_DISABLE_DOT2=1"
  "GGML_VK_DISABLE_GRAPH_OPTIMIZE=1"
  "GGML_VK_DISABLE_FUSION=1"
  "GGML_VK_DISABLE_MMVQ=1"
  "GGML_VK_PREFER_HOST_MEMORY=1"
)

score() {
  # prints: dur peak rms
  python3 - "$1" <<'PY'
import array, sys
a = array.array('h')
a.frombytes(open(sys.argv[1], 'rb').read())
n = len(a)
if n == 0:
    print("0 0 0"); raise SystemExit
peak = max(abs(x) for x in a)
rms = round((sum(x * x for x in a) / n) ** 0.5)
print(f"{n/24000:.2f} {peak} {rms}")
PY
}

printf '%-72s %8s %7s %7s  %s\n' "FLAGS" "dur" "peak" "rms" "verdict"
for flags in "${CANDIDATES[@]}"; do
  port=$((PORT++))
  log="$OUT/server-$port.log"
  # shellcheck disable=SC2086
  env GGML_BACKEND=Vulkan0 $flags "$BIN" \
    --model "$MODEL" --codec "$CODEC" --alias qwen3-tts \
    --host 127.0.0.1 --port "$port" >"$log" 2>&1 &
  pid=$!

  for _ in $(seq 1 60); do
    curl -fsS -m 2 "http://127.0.0.1:$port/v1/audio/voices" >/dev/null 2>&1 && break
    sleep 1
  done

  raw="$OUT/out-$port.raw"
  curl -sS -m 240 -o "$raw" -X POST "http://127.0.0.1:$port/v1/audio/speech" \
    -H "Content-Type: application/json" \
    -d "{\"input\":\"$TEXT\",\"voice\":\"vivian\",\"language\":\"English\",\"response_format\":\"pcm\",\"max_new_tokens\":200}" \
    >/dev/null 2>&1

  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null

  read -r dur peak rms <<<"$(score "$raw" 2>/dev/null || echo '0 0 0')"
  verdict="BROKEN (quiet/garbled)"
  # CPU reference is ~6.2 s, peak ~24k, rms ~2.8k.
  if [ "${peak:-0}" -gt 15000 ] && [ "${rms:-0}" -gt 1800 ]; then
    verdict="*** OK ***"
  fi
  printf '%-72s %8s %7s %7s  %s\n' "${flags:-<none>}" "$dur" "$peak" "$rms" "$verdict"
done

[ "$KEEP" = "1" ] || echo "(outputs in $OUT)"
