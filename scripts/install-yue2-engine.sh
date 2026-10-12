#!/usr/bin/env bash
# Install the YuE2 song engine — HOT-Step-CPP's `ace-server`, the Linux Vulkan
# release — into build/hot-step/, where the app's song service looks for it
# (first in this list; `YUE2_ENGINE` overrides, and the benchmark kit copy at
# benchmarks/yue-v2/build/hot-step/ is the last fallback).
#
# The engine is a ~390 MB binary release; the model weights (~3 GB) are fetched
# separately — Settings → Music, or the assistant's own ask-first download when
# `generate_song` finds them missing (`voice/download_yue2.py`).
#
# Pinned to v1.3.6 (the release the song service was validated against on the
# 8 GB Navi14): sha256 below is GitHub's digest for the asset.
#
# Usage:
#   scripts/install-yue2-engine.sh
#   YUE2_ENGINE_DIR=/opt/hot-step scripts/install-yue2-engine.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="${YUE2_ENGINE_DIR:-$ROOT/build/hot-step}"
VER="v1.3.6"
ASSET="HOT-Step-CPP-$VER-linux-x64-vulkan.tar.gz"
URL="https://github.com/scragnog/HOT-Step-CPP/releases/download/$VER/$ASSET"
SHA256="f57e882266a9b3e95cb5ae4405101a6dffb3f09368643eb8b90d6047dc1555c4"

log() { echo "yue2-engine: $*" >&2; }

case "$(uname -m)" in
  x86_64) ;;
  *)
    log "no prebuilt engine for $(uname -m) — build HOT-Step-CPP from source"
    log "(https://github.com/scragnog/HOT-Step-CPP) and point YUE2_ENGINE at its engine/ace-server"
    exit 2
    ;;
esac

if [ -x "$DEST/engine/ace-server" ]; then
  log "already installed: $DEST/engine/ace-server"
  exit 0
fi

for tool in curl tar sha256sum; do
  command -v "$tool" >/dev/null 2>&1 || { log "missing $tool"; exit 2; }
done

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

log "fetching $ASSET (~391 MB, resumable)"
curl -fL --retry 3 --retry-delay 2 -C - -o "$TMP/$ASSET" "$URL"
got="$(sha256sum "$TMP/$ASSET" | cut -d' ' -f1)"
if [ "$got" != "$SHA256" ]; then
  log "sha256 mismatch: got $got — delete $TMP/$ASSET and rerun"
  exit 3
fi

log "extracting into $DEST"
mkdir -p "$TMP/extract"
tar -xzf "$TMP/$ASSET" -C "$TMP/extract"
src="$TMP/extract"
if [ "$(find "$src" -mindepth 1 -maxdepth 1 | wc -l)" = 1 ] \
   && [ -d "$(find "$src" -mindepth 1 -maxdepth 1 | head -1)" ]; then
  src="$(find "$src" -mindepth 1 -maxdepth 1 | head -1)"
fi
mkdir -p "$(dirname "$DEST")"
rm -rf "$DEST"
mv "$src" "$DEST"

[ -x "$DEST/engine/ace-server" ] || { log "engine binary not found after extraction"; exit 3; }
log "installed: $DEST/engine/ace-server"
