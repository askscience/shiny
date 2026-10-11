#!/usr/bin/env python3
"""Download the Z-Image Turbo weights (diffusion quants + encoder + VAE).

`fast` (Q3_K) is the shipped default; `balanced`/`high`/`max` are opt-in from
Settings → Images. Every tier needs the shared Qwen3-4B text encoder and the
VAE, which are fetched first (and only once). Sizes are verified after the
fetch so a truncated file is re-downloaded instead of silently breaking a
render; an interrupted fetch resumes from the `.part` file when the server
honours the Range header.

Deliberately dependency-free (stdlib only): it must run under whatever
`python3` the core server finds, like download_qwen_tts.py.

Usage:
    python3 voice/download_zimage.py fast
    python3 voice/download_zimage.py balanced
    python3 voice/download_zimage.py all
"""

import json
import os
import shutil
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

MODELS_DIR = Path(
    os.environ.get("Z_IMAGE_MODELS_DIR", ROOT / "data" / "z-image-models")
)

DIFFUSION_REPO = "leejet/Z-Image-Turbo-GGUF"

BASE = [
    (
        "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/"
        "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        2_497_281_120,
    ),
    (
        "ae.safetensors",
        "https://huggingface.co/Comfy-Org/z_image_turbo/resolve/main/"
        "split_files/vae/ae.safetensors",
        335_304_388,
    ),
]

QUANTS = {
    "fast": ("Q3_K", 3_143_559_104),
    "balanced": ("Q4_K", 3_864_250_304),
    "high": ("Q6_K", 5_263_239_104),
    "max": ("Q8_0", 6_577_440_704),
}


def human(n: int) -> str:
    return f"{n / 1e9:.1f} GB" if n >= 1_000_000_000 else f"{n // 1_000_000} MB"


def _fetch(name: str, url: str, size: int) -> None:
    """Fetch one file atomically (`.part` + rename), resuming when possible."""
    dest = MODELS_DIR / name
    if dest.is_file() and dest.stat().st_size == size:
        print(f"[z-image] {name} already present", file=sys.stderr)
        return

    tmp = dest.with_suffix(dest.suffix + ".part")
    start = tmp.stat().st_size if tmp.is_file() else 0
    headers = {"User-Agent": "shiny-imagegen/1.0"}
    if start:
        headers["Range"] = f"bytes={start}-"

    print(f"[z-image] {name} ({human(size)})", file=sys.stderr)
    req = urllib.request.Request(url + "?download=true", headers=headers)
    with urllib.request.urlopen(req) as resp, open(tmp, "ab" if start and resp.status == 206 else "wb") as out:
        if not (start and resp.status == 206):
            start = 0
        shutil.copyfileobj(resp, out, length=1024 * 1024)

    actual = tmp.stat().st_size
    if actual != size:
        raise OSError(f"{name}: expected {size} bytes, got {actual}")
    tmp.replace(dest)


def main() -> int:
    targets = sys.argv[1:] or ["fast"]
    keys: list[str] = []
    for target in targets:
        t = target.strip().lower()
        if t == "all":
            keys = list(QUANTS)
        elif t in QUANTS:
            keys.append(t)
        else:
            print(json.dumps({"status": "error", "error": f"unknown quality: {t}"}))
            return 2
    keys = list(dict.fromkeys(keys))

    MODELS_DIR.mkdir(parents=True, exist_ok=True)

    try:
        for name, url, size in BASE:
            _fetch(name, url, size)
        for key in keys:
            quant, size = QUANTS[key]
            _fetch(
                f"z_image_turbo-{quant}.gguf",
                f"https://huggingface.co/{DIFFUSION_REPO}/resolve/main/z_image_turbo-{quant}.gguf",
                size,
            )
    except OSError as exc:
        print(json.dumps({"status": "error", "error": f"download failed: {exc}"}))
        return 1

    print(json.dumps({"status": "ready", "qualities": keys}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
