#!/usr/bin/env bash
# Build qwentts.cpp with every backend this distro can actually support.
#
# The binary is built once per distribution, so whichever accelerator the
# machine turns out to have is compiled in and picked at runtime by the
# sidecar launcher (see voice/detect_accel.py → GGML_BACKEND):
#
#   CUDA toolchain present      → -DGGML_CUDA=ON     (NVIDIA, fastest)
#   Vulkan loader + glslc       → -DGGML_VULKAN=ON   (AMD / Intel / NVIDIA)
#   always                      → CPU variants (SIMD dispatch, portable fallback)
#
# Metal is macOS-only and has no flag here. Backends are DL-loaded, so a binary
# built with Vulkan still starts on a machine with no Vulkan driver — the
# runtime drops to CPU. That is why enabling everything available is safe.
#
# Usage:
#   ./voice/build_qwentts.sh              # build (or reuse an existing build)
#   ./voice/build_qwentts.sh --rebuild    # force a clean reconfigure + build
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="${QWENTTS_SRC:-$ROOT/build/qwentts.cpp}"
REPO="${QWENTTS_REPO:-https://github.com/ServeurpersoCom/qwentts.cpp.git}"
BUILD_DIR="$SRC/build"

log() { echo "build_qwentts: $*" >&2; }

REBUILD=0
for arg in "$@"; do
  [ "$arg" = "--rebuild" ] && REBUILD=1
done

if [ ! -d "$SRC/.git" ]; then
  log "cloning $REPO"
  mkdir -p "$(dirname "$SRC")"
  git clone --depth 1 --recurse-submodules --shallow-submodules "$REPO" "$SRC"
fi

# ── Local ggml fixes ────────────────────────────────────────────────────────
# col2im_1d.comp stores through `DATA_A_BF16` (the *input* type flag) where it
# needs `DATA_D_BF16` (the output type), so the F16 variant writes with the
# wrong conversion. snake.comp, the reference for the same pattern, uses
# DATA_D_BF16. Applied idempotently: the checkout is shallow and rebuilt on
# demand, so the fix lives here rather than as a local edit that a fresh clone
# would not have.
COL2IM="$SRC/ggml/src/ggml-vulkan/vulkan-shaders/col2im_1d.comp"
if [ -f "$COL2IM" ] && grep -A1 'void store_dst' "$COL2IM" | grep -q 'defined(DATA_A_BF16)'; then
  log "patching col2im_1d.comp (DATA_A_BF16 -> DATA_D_BF16 in store_dst)"
  python3 - "$COL2IM" <<'PY'
import sys
path = sys.argv[1]
src = open(path).read()
# Only the store path is wrong; the load path legitimately checks the input.
old = """void store_dst(uint32_t idx, float v) {
#if defined(DATA_A_BF16)"""
new = """void store_dst(uint32_t idx, float v) {
#if defined(DATA_D_BF16)"""
if old not in src:
    raise SystemExit("col2im_1d.comp: expected store_dst pattern not found")
open(path, "w").write(src.replace(old, new, 1))
PY
fi

# ── Backend detection ───────────────────────────────────────────────────────
cmake_args=(
  -DGGML_CPU_ALL_VARIANTS=ON
  -DGGML_BACKEND_DL=ON
  -DCMAKE_BUILD_TYPE=Release
)

if command -v nvcc >/dev/null 2>&1 || [ -x /usr/local/cuda/bin/nvcc ]; then
  log "CUDA toolkit found — enabling GGML_CUDA"
  cmake_args+=(-DGGML_CUDA=ON)
  export PATH="/usr/local/cuda/bin:$PATH"
else
  log "no CUDA toolkit — NVIDIA users get CUDA by building on a machine that has it"
fi

if command -v glslc >/dev/null 2>&1 && command -v vulkaninfo >/dev/null 2>&1; then
  log "Vulkan loader + glslc found — enabling GGML_VULKAN"
  cmake_args+=(-DGGML_VULKAN=ON)
else
  log "Vulkan loader/glslc missing — skipping GGML_VULKAN (apt: libvulkan-dev glslc)"
fi

# ── Configure + build ───────────────────────────────────────────────────────
if [ "$REBUILD" = "1" ]; then
  rm -rf "$BUILD_DIR"
fi

generator=()
command -v ninja >/dev/null 2>&1 && generator=(-G Ninja)

cmake -S "$SRC" -B "$BUILD_DIR" "${generator[@]}" "${cmake_args[@]}" >&2
cmake --build "$BUILD_DIR" --config Release -j "$(getconf _NPROCESSORS_ONLN)" >&2

log "built binaries:"
ls -1 "$BUILD_DIR"/tts-server "$BUILD_DIR"/qwen-tts "$BUILD_DIR"/qwen-codec 2>/dev/null >&2 || true
