#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
pip install -q 'supertonic[serve]' 2>/dev/null || pip install 'supertonic[serve]'
exec supertonic serve --host "${SUPERTONIC_HOST:-127.0.0.1}" --port "${SUPERTONIC_PORT:-7788}"
