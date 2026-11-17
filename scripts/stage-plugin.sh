#!/usr/bin/env bash
# Lay out one built plugin the way the server loads it:
#
#   <dest>/plugin.toml, web/, migrations/, skills/
#   <dest>/bin/<name>                       runtime = "process"
#   <dest>/libshiny_<name>_plugin.so        library plugins
#
#   scripts/stage-plugin.sh <name> <dest> [target-dir]
#
# target-dir defaults to target/release. Used by the Debian package, the
# per-plugin archives and the development install.
set -euo pipefail
REPO=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
name=${1:?plugin name}
dest=${2:?destination}
target=${3:-$REPO/target/release}
src="$REPO/plugins/$name"
die() { echo "error: $*" >&2; exit 1; }
[ -f "$src/plugin.toml" ] || die "no plugin $name"

install -d "$dest"
cp -a "$src/plugin.toml" "$dest/"
for part in web migrations skills; do
    [ -e "$src/$part" ] && cp -a "$src/$part" "$dest/"
done
if grep -Eq '^runtime[[:space:]]*=[[:space:]]*"process"' "$src/plugin.toml"; then
    exe="$target/shiny-plugin-$name"
    [ -x "$exe" ] || die "missing plugin executable for $name: $exe"
    install -d "$dest/bin"
    # Copy, then rename: a running server never sees a half-written file.
    cp "$exe" "$dest/bin/.$name.new"
    chmod 0755 "$dest/bin/.$name.new"
    mv -f "$dest/bin/.$name.new" "$dest/bin/$name"
else
    so="$target/libshiny_${name}_plugin.so"
    [ -f "$so" ] || die "missing plugin cdylib for $name: $so"
    cp "$so" "$dest/.lib.new"
    mv -f "$dest/.lib.new" "$dest/libshiny_${name}_plugin.so"
fi
