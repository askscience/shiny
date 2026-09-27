#!/usr/bin/env bash
# Benchmark Qwen3-TTS: whole-text vs sentence-chunked synthesis.
#
# The question is whether subdividing text (the EOS-runaway mitigation) costs
# enough prosody/context that it is worth dropping now that the Vulkan
# integer-dot-product fix makes the model emit EOS on its own.
#
# For each passage, synthesise it
#   (a) as one request  (no subdivision), and
#   (b) sentence-by-sentence, the way src/services/qwen_tts.rs does it,
# then compare wall time, audio length, and how well the result transcribes.
#
# Usage: voice/bench_qwen_tts.sh [--port N] [--out DIR]
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PORT="${QWEN_TTS_PORT:-7787}"
STT="${WHISPER_URL:-http://127.0.0.1:7789}"
OUT="${BENCH_OUT:-/tmp/qwen-bench}"

while [ $# -gt 0 ]; do
  case "$1" in
    --port) PORT="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    *) shift ;;
  esac
done
mkdir -p "$OUT"

SPEAKER="${BENCH_SPEAKER:-vivian}"
LANG_LABEL="${BENCH_LANG:-English}"

# Passages chosen to expose prosody loss: questions, commas, and clause
# boundaries are where a subdivided read is most likely to sound wrong.
PASSAGES=(
  "prosody|Good morning. I checked the weather, and it looks like rain this afternoon, so you may want to take an umbrella when you head out."
  "long|The library opens at nine, closes at six, and stays open until eight on Thursdays. If you need a quiet place to work, the reading room on the second floor is usually the best option."
  "short|Yes, absolutely."
)

frames_budget() { # chars -> budget (mirrors src/services/qwen_tts.rs)
  local n=${#1}
  local b=$((n * 5))
  [ "$b" -lt 50 ] && b=50
  echo "$b"
}

# Split on . ! ? ; and newline, merging nothing (same as the Rust splitter).
split_sentences() {
  python3 - "$1" <<'PY'
import sys, re
text = sys.argv[1].strip()
parts, cur = [], ""
for i, ch in enumerate(text):
    cur += ch
    nxt = text[i + 1] if i + 1 < len(text) else ""
    if ch in ".!?;\n" and (nxt == "" or nxt.isspace()):
        if cur.strip():
            parts.append(cur.strip())
        cur = ""
if cur.strip():
    parts.append(cur.strip())
for p in parts:
    print(p)
PY
}

synth() { # text -> file, prints elapsed seconds
  local text="$1" dest="$2"
  local budget
  budget=$(frames_budget "$text")
  local start end
  start=$(date +%s.%N)
  curl -sS -m 600 -o "$dest" -X POST "http://127.0.0.1:$PORT/v1/audio/speech" \
    -H "Content-Type: application/json" \
    -d "$(python3 -c "
import json,sys
print(json.dumps({'input':sys.argv[1],'voice':sys.argv[2],'language':sys.argv[3],
                  'response_format':'pcm','max_new_tokens':int(sys.argv[4]),
                  'repetition_penalty':1.1}))" "$text" "$SPEAKER" "$LANG_LABEL" "$budget")" \
    >/dev/null 2>&1
  end=$(date +%s.%N)
  echo "$start $end" | awk '{printf "%.2f", $2-$1}'
}

stats() { # pcm file -> "dur peak rms transcribable"
  python3 - "$1" <<'PY'
import array, sys
try:
    a = array.array('h'); a.frombytes(open(sys.argv[1], 'rb').read())
except Exception:
    print("0 0 0 no"); raise SystemExit
n = len(a)
if n == 0:
    print("0 0 0 no"); raise SystemExit
peak = max(abs(x) for x in a)
rms = round((sum(x * x for x in a) / n) ** 0.5)
# A healthy render is loud and is not sitting at the frame cap.
print(f"{n/24000:.2f} {peak} {rms} {'yes' if peak > 15000 and rms > 1800 else 'no'}")
PY
}

transcribe() { # pcm file -> text
  curl -sS -m 240 -X POST --data-binary "@$1" \
    -H "Content-Type: application/octet-stream" \
    "$STT/stt/chunk?session=bench_$(basename "$1" .raw)&lang=en&model=tiny&final=true" 2>/dev/null \
    | python3 -c "import sys,json;print(json.load(sys.stdin).get('text',''))" 2>/dev/null
}

printf '%-9s %-10s %7s %8s %7s %6s %7s  %s\n' \
  "passage" "mode" "time" "dur" "peak" "rms" "ok" "transcript"
printf '%.0s-' {1..120}; echo

for entry in "${PASSAGES[@]}"; do
  name="${entry%%|*}"; text="${entry#*|}"

  # (a) whole text, one request
  raw="$OUT/$name-whole.raw"
  t=$(synth "$text" "$raw")
  read -r dur peak rms ok <<<"$(stats "$raw")"
  tr_=$(transcribe "$raw")
  printf '%-9s %-10s %6ss %7ss %7s %6s %7s  %s\n' \
    "$name" "whole" "$t" "$dur" "$peak" "$rms" "$ok" "${tr_:0:46}"

  # (b) sentence by sentence, concatenated (the current Rust behaviour)
  raw="$OUT/$name-chunked.raw"
  : >"$raw"
  total=0
  while IFS= read -r sentence; do
    [ -z "$sentence" ] && continue
    part="$OUT/$name-part-$total.raw"
    pt=$(synth "$sentence" "$part")
    total=$(echo "$total $pt" | awk '{printf "%.2f", $1+$2}')
    cat "$part" >>"$raw"
  done < <(split_sentences "$text")
  read -r dur peak rms ok <<<"$(stats "$raw")"
  tr_=$(transcribe "$raw")
  printf '%-9s %-10s %6ss %7ss %7s %6s %7s  %s\n' \
    "$name" "chunked" "$total" "$dur" "$peak" "$rms" "$ok" "${tr_:0:46}"
  echo
done

echo "raw PCM in $OUT (play with: ffplay -f s16le -ar 24000 -ac 1 <file>)"
