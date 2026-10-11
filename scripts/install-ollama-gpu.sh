#!/bin/sh
# install-ollama-gpu.sh — give Ollama the right GPU backend for this machine.
#
#   sudo scripts/install-ollama-gpu.sh                      # auto-detect
#   sudo scripts/install-ollama-gpu.sh --backend vulkan     # force a backend
#   sudo scripts/install-ollama-gpu.sh --backend rocm --gfx-override 10.3.0
#   sudo scripts/install-ollama-gpu.sh --uninstall
#
# WHY
#   Ollama carries every backend inside /usr/local/lib/ollama (vulkan/,
#   rocm_v*/, cuda_v*/) and chooses one at runtime from environment variables
#   in a systemd drop-in. The right pick differs per machine:
#
#     vulkan  AMD, universal: RADV supports every amdgpu card, including RDNA1
#             (gfx1010-gfx1012) and older, whose gfx targets have no rocBLAS
#             kernels in Ollama's bundled ROCm.
#     rocm    AMD with bundled kernels: gfx1030+ (RDNA2/3/4) and gfx908,
#             gfx90a, gfx942 (CDNA). --gfx-override maps an unsupported target
#             onto a supported one; on RDNA1 the kernels then die with
#             HSA_STATUS_ERROR_MEMORY_APERTURE_VIOLATION at the first launch,
#             so verify with a real generation before trusting it.
#     cuda    NVIDIA; the CUDA libraries are bundled and need no pointer,
#             only the shared tuning knobs.
#     cpu     no GPU variables at all.
#
#   `auto` picks cuda on NVIDIA, vulkan on AMD, cpu otherwise. AMD also gets
#   /usr/local/bin/ollama-wait-for-amd.sh: amdgpu initializes after
#   network-online.target, and a probe too early at boot silently pins Ollama
#   to the CPU for the whole boot.
#
#   The drop-in replaces /etc/systemd/system/ollama.service.d/override.conf;
#   the file it replaces is backed up once to override.conf.bak.
set -eu

REPO_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SRC_DIR="$REPO_DIR/scripts/ollama"
DROP_IN_DIR=/etc/systemd/system/ollama.service.d
DROP_IN="$DROP_IN_DIR/override.conf"
BACKUP="$DROP_IN.bak"
WAIT_SCRIPT=/usr/local/bin/ollama-wait-for-amd.sh
SERVICE=/etc/systemd/system/ollama.service

BACKEND=auto
GFX_OVERRIDE=
UNINSTALL=0

usage() {
    awk 'NR > 1 { if ($0 !~ /^#/) exit; sub(/^# ?/, ""); print }' "$0"
    exit "${1:-0}"
}

need_root() {
    [ "$(id -u)" -eq 0 ] || { echo "error: run as root (sudo $0)" >&2; exit 1; }
}

vendor_present() {
    command -v lspci >/dev/null 2>&1 || return 1
    lspci -d "$1:" 2>/dev/null | grep -q .
}

detect_backend() {
    if vendor_present 10de; then
        echo cuda
    elif vendor_present 1002; then
        echo vulkan
    else
        echo cpu
    fi
}

apply_service() {
    [ -f "$SERVICE" ] || return 0
    systemctl daemon-reload
    systemctl try-restart ollama 2>/dev/null || true
}

while [ $# -gt 0 ]; do
    case "$1" in
        --backend)
            [ $# -ge 2 ] || { echo "error: --backend needs a value" >&2; exit 1; }
            BACKEND=$2; shift 2 ;;
        --backend=*) BACKEND=${1#--backend=}; shift ;;
        --gfx-override)
            [ $# -ge 2 ] || { echo "error: --gfx-override needs a value" >&2; exit 1; }
            GFX_OVERRIDE=$2; shift 2 ;;
        --gfx-override=*) GFX_OVERRIDE=${1#--gfx-override=}; shift ;;
        --uninstall) UNINSTALL=1; shift ;;
        -h|--help) usage 0 ;;
        *) echo "error: unknown argument: $1" >&2; usage 1 ;;
    esac
done

case "$BACKEND" in
    auto|vulkan|rocm|cuda|cpu) ;;
    *) echo "error: unknown backend: $BACKEND (auto|vulkan|rocm|cuda|cpu)" >&2; exit 1 ;;
esac

need_root

if [ "$UNINSTALL" -eq 1 ]; then
    rm -f "$DROP_IN" "$WAIT_SCRIPT"
    apply_service
    echo "Removed $DROP_IN and $WAIT_SCRIPT (if present)."
    if [ -f "$BACKUP" ]; then
        echo "Previous drop-in kept at $BACKUP."
    fi
    exit 0
fi

if [ "$BACKEND" = auto ]; then
    BACKEND=$(detect_backend)
fi

TEMPLATE=
case "$BACKEND" in
    vulkan|rocm|cuda) TEMPLATE="$SRC_DIR/$BACKEND.conf" ;;
esac

if [ -n "$TEMPLATE" ] && [ ! -f "$TEMPLATE" ]; then
    echo "error: $TEMPLATE missing" >&2
    exit 1
fi

if [ -f "$DROP_IN" ] && [ ! -f "$BACKUP" ]; then
    cp -p "$DROP_IN" "$BACKUP"
fi

if [ -z "$TEMPLATE" ]; then
    rm -f "$DROP_IN" "$WAIT_SCRIPT"
    apply_service
    echo "backend: cpu — no GPU environment installed."
    echo "Ollama will run on the CPU only."
    exit 0
fi

install -d -m 0755 "$DROP_IN_DIR"
rm -f "$DROP_IN"
install -m 0644 "$TEMPLATE" "$DROP_IN"

if [ -n "$GFX_OVERRIDE" ]; then
    if [ "$BACKEND" = rocm ]; then
        {
            echo "# Maps this card's gfx target onto a supported one (see the script header)."
            echo "Environment=\"HSA_OVERRIDE_GFX_VERSION=$GFX_OVERRIDE\""
        } >> "$DROP_IN"
    else
        echo "warning: --gfx-override only applies to --backend rocm; ignored" >&2
    fi
fi

case "$BACKEND" in
    vulkan|rocm) install -m 0755 "$SRC_DIR/ollama-wait-for-amd.sh" "$WAIT_SCRIPT" ;;
    cuda) rm -f "$WAIT_SCRIPT" ;;
esac

apply_service

echo "backend: $BACKEND"
echo "Wrote $DROP_IN"
if [ -f "$BACKUP" ]; then
    echo "Previous drop-in kept at $BACKUP."
fi
echo
echo "Verify the backend Ollama actually picked:"
echo "  journalctl -u ollama | grep 'inference compute'"
