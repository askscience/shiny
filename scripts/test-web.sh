#!/bin/sh
# No bundler or npm install: syntax-check first-party modules, then test behavior.
set -eu
cd "$(dirname "$0")/.."
find web/js web/ui plugins -type f -name '*.js' -not -path '*/vendor/*' -not -path '*/node_modules/*' -print0 |
    xargs -0 -n 1 node --check
for file in web/js/tests/*.test.mjs; do
    printf '== %s\n' "$file"
    node "$file"
done
for file in plugins/*/web/plugin.smoke.mjs; do
    [ -f "$file" ] || continue
    printf '== %s\n' "$file"
    node --experimental-vm-modules "$file"
done
