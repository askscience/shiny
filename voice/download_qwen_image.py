#!/usr/bin/env python3
"""Download the Qwen-Image 2.1 Turbo weights (turbo quants + shared files).

`fast` (Q3_K) is the shipped default; `balanced`/`high`/`max` are opt-in from
Settings → Images. Every tier needs the shared Qwen3-VL-8B text encoder, the
2.1 VAE, the TAEQI2.1 tiny decoder and the vision tower (mmproj, image
editing), which are fetched first (and only once). Sizes are verified after
the fetch so a truncated file is re-downloaded instead of silently breaking a
render; an interrupted fetch resumes from the `.part` file when the server
honours the Range header.

The weights are under the Qwen Research License (non-commercial use) — unlike
Z-Image's Apache-2.0.

Deliberately dependency-free (stdlib only): it must run under whatever
`python3` the core server finds, like download_zimage.py.

Usage:
    python3 voice/download_qwen_image.py fast
    python3 voice/download_qwen_image.py balanced
    python3 voice/download_qwen_image.py all
"""

import json
import os
import shutil
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

MODELS_DIR = Path(
    os.environ.get("QWEN_IMAGE_MODELS_DIR", ROOT / "data" / "qwen-image-models")
)

DIFFUSION_REPO = "DogukanUrker/Qwen-Image-2.1-Turbo-GGUF"

BASE = [
    (
        "Qwen3VL-8B-Instruct-Q4_K_M.gguf",
        "https://huggingface.co/Qwen/Qwen3-VL-8B-Instruct-GGUF/resolve/main/"
        "Qwen3VL-8B-Instruct-Q4_K_M.gguf",
        5_027_784_800,
    ),
    (
        "qwen_image_2.1_vae_bf16.safetensors",
        "https://huggingface.co/Comfy-Org/Qwen-Image-2.1/resolve/main/"
        "vae/qwen_image_2.1_vae_bf16.safetensors",
        675_509_688,
    ),
    (
        "taeqi2_1.safetensors",
        "https://huggingface.co/madebyollin/taeqi2_1/resolve/main/taeqi2_1.safetensors",
        30_591_264,
    ),
    (
        "mmproj-Qwen3VL-8B-Instruct-F16.gguf",
        "https://huggingface.co/Qwen/Qwen3-VL-8B-Instruct-GGUF/resolve/main/"
        "mmproj-Qwen3VL-8B-Instruct-F16.gguf",
        1_159_029_824,
    ),
]

QUANTS = {
    "fast": ("Q3_K", 3_110_431_776),
    "balanced": ("Q4_K", 4_050_916_384),
    "high": ("Q6_K", 5_876_562_976),
    "max": ("Q8_0", 7_591_558_176),
}


def human(n: int) -> str:
    return f"{n / 1e9:.1f} GB" if n >= 1_000_000_000 else f"{n // 1_000_000} MB"


def _fetch(name: str, url: str, size: int) -> None:
    """Fetch one file atomically (`.part` + rename), resuming when possible."""
    dest = MODELS_DIR / name
    if dest.is_file() and dest.stat().st_size == size:
        print(f"[qwen-image] {name} already present", file=sys.stderr)
        return

    tmp = dest.with_suffix(dest.suffix + ".part")
    start = tmp.stat().st_size if tmp.is_file() else 0
    headers = {"User-Agent": "shiny-imagegen/1.0"}
    if start:
        headers["Range"] = f"bytes={start}-"

    print(f"[qwen-image] {name} ({human(size)})", file=sys.stderr)
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
                f"qwen-image-2.1-turbo-{quant}.gguf",
                f"https://huggingface.co/{DIFFUSION_REPO}/resolve/main/"
                f"qwen-image-2.1-turbo-{quant}.gguf",
                size,
            )
    except OSError as exc:
        print(json.dumps({"status": "error", "error": f"download failed: {exc}"}))
        return 1

    print(json.dumps({"status": "ready", "qualities": keys}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
