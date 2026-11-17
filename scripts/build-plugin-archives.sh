#!/usr/bin/env bash
# Build one .tar.gz per process plugin, plus the catalogue the Plugin manager
# reads (plugins/index.json).
#
#   LITEAURA_VERSION=0.2.8 PLUGIN_BASE_URL=https://.../download/v0.2.8 \
#     scripts/build-plugin-archives.sh [out-dir]
#
# Archive: <name>-<version>-linux-<arch>.tar.gz holding a single <name>/
# folder laid out by stage-plugin.sh. The catalogue lists, for each archive,
# its size and SHA-256 and the plugin protocol versions the executable speaks,
# so a server only offers plugins it can talk to. CI signs index.json
# (index.json.sig) with the archive key; the server checks that signature.
#
# Library plugins (terminal, updates, installer) stay inside the Debian
# package: they only work with the server they were built with.
set -euo pipefail
REPO=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
OUT=${1:-${LITEAURA_DEB_BUILD:-$HOME/liteaura-iso/deb}/out/plugins}
VERSION=${LITEAURA_VERSION:?set LITEAURA_VERSION}
BASE_URL=${PLUGIN_BASE_URL:-https://github.com/askscience/shiny/releases/download/v$VERSION}
TARGET=${PLUGIN_TARGET_DIR:-$REPO/target/release}
case "$(uname -m)" in
    x86_64) ARCH=amd64 ;;
    aarch64) ARCH=arm64 ;;
    *) ARCH=$(uname -m) ;;
esac

proto=$REPO/crates/shiny-plugin-protocol/src/lib.rs
PMIN=$(sed -n 's/^pub const PROTOCOL_MIN: u32 = \([0-9]*\);/\1/p' "$proto")
PMAX=$(sed -n 's/^pub const PROTOCOL_MAX: u32 = \([0-9]*\);/\1/p' "$proto")
[ -n "$PMIN" ] && [ -n "$PMAX" ] || { echo "error: protocol range not found" >&2; exit 1; }

rm -rf "$OUT"
mkdir -p "$OUT"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

entries=()
for manifest in "$REPO"/plugins/*/plugin.toml; do
    name=$(basename "$(dirname "$manifest")")
    [ "$name" = "hello" ] && continue
    grep -Eq '^runtime[[:space:]]*=[[:space:]]*"process"' "$manifest" || continue
    version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$manifest" | head -1)
    file="$name-$version-linux-$ARCH.tar.gz"
    "$REPO/scripts/stage-plugin.sh" "$name" "$stage/$name" "$TARGET"
    # Reproducible: fixed order, owner and times.
    tar --sort=name --owner=0 --group=0 --numeric-owner \
        --mtime="@${SOURCE_DATE_EPOCH:-0}" -C "$stage" -cf - "$name" | gzip -9n > "$OUT/$file"
    size=$(stat -c %s "$OUT/$file")
    sha=$(sha256sum "$OUT/$file" | cut -d' ' -f1)
    entries+=("$(python3 -I - "$manifest" "$file" "$size" "$sha" "$BASE_URL" "$ARCH" "$PMIN" "$PMAX" <<'PY'
import json, sys, tomllib
manifest, file, size, sha, base, arch, pmin, pmax = sys.argv[1:]
with open(manifest, "rb") as f:
    m = tomllib.load(f)
print(json.dumps({
    "name": m["name"],
    "version": m["version"],
    "description": m.get("description", ""),
    "summary": m.get("summary", ""),
    "category": m.get("category", ""),
    "kind": m.get("kind", "native"),
    "arch": arch,
    "protocol": {"min": int(pmin), "max": int(pmax)},
    "file": file,
    "url": f"{base.rstrip('/')}/{file}",
    "size": int(size),
    "sha256": sha,
}))
PY
)")
    echo "  $file ($size bytes)"
done

printf '%s\n' "${entries[@]}" | python3 -I -c '
import json, sys
plugins = [json.loads(line) for line in sys.stdin if line.strip()]
print(json.dumps({"format": 1, "release": sys.argv[1], "plugins": plugins}, indent=2))
' "$VERSION" > "$OUT/index.json"
echo "catalogue: $OUT/index.json (${#entries[@]} plugins)"
