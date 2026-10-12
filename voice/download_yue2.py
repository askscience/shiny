#!/usr/bin/env python3
"""Download the YuE2 song-generation weights (LM quants + VAE + companion + tokenizer).

`q4` (Q4_K_M-imat) is the shipped default; `q8` (Q8_0) is the opt-in
near-lossless tier from Settings → Music. Every tier needs the shared VAE, the
v9 decoder companion and the BPE tokenizer, which are fetched first (and only
once). Sizes are verified after the fetch so a truncated file is re-downloaded
instead of silently breaking a render; an interrupted fetch resumes from the
`.part` file when the server honours the Range header.

After the VAE arrives, its GGUF metadata is patched in place —
`yue2vae.decode_core_frames` 1024 → 128. The engine clamps its decode tiles to
512 frames; on 8 GB cards that tile builds a ~3.2 GB compute graph whose
execution hangs RADV (measured on a Navi14 — see `benchmarks/yue-v2/README.md`).
128-frame tiles decode faster there (~9 s for a 75 s song) and are harmless on
larger GPUs.

Deliberately dependency-free (stdlib only): it must run under whatever
`python3` the core server finds, like download_zimage.py.

Usage:
    python3 voice/download_yue2.py q4
    python3 voice/download_yue2.py q8
    python3 voice/download_yue2.py all
"""

import hashlib
import json
import os
import shutil
import struct
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

MODELS_DIR = Path(os.environ.get("YUE2_MODELS_DIR", ROOT / "data" / "yue2-models"))
STORE = MODELS_DIR / "yue2"

GGUF_BASE = "https://huggingface.co/scragnog/YuE2-GGUF/resolve/main"
COMPANION_BASE = (
    "https://huggingface.co/Mothersuperior/"
    "yue2-mothersuperior-realaudio-tokenizer-v4/resolve/main"
)

# (path under STORE, url, bytes[, sha256])
BASE = [
    ("yue2-vae-standard-f32.gguf", f"{GGUF_BASE}/yue2-vae-standard-f32.gguf", 530_348_768),
    (
        "nar_lora_joint_v9.safetensors",
        f"{COMPANION_BASE}/nar_lora_joint_v9.safetensors",
        140_560_592,
        "585f303da1d5252d228d1e8ac6d4c4d11d970df9297406935cc8bdafa49cfa7e",
    ),
    ("tokenizer/vocab.json", f"{GGUF_BASE}/tokenizer/vocab.json", 3_080_121),
    ("tokenizer/merges.txt", f"{GGUF_BASE}/tokenizer/merges.txt", 1_823_241),
    (
        "tokenizer/special_tokens.json",
        f"{GGUF_BASE}/tokenizer/special_tokens.json",
        14_141,
    ),
    ("tokenizer/pretokenizer.json", f"{GGUF_BASE}/tokenizer/pretokenizer.json", 900),
    (
        "tokenizer/expected-ids-corpus.json",
        f"{GGUF_BASE}/tokenizer/expected-ids-corpus.json",
        24_681,
    ),
]

TIERS = {
    "q4": ("yue2-lm-Q4_K_M-imat.gguf", 2_429_623_904),
    "q8": ("yue2-lm-q8_0.gguf", 3_916_164_544),
}

VAE_FILE = "yue2-vae-standard-f32.gguf"
VAE_TILE_KEY = b"yue2vae.decode_core_frames"
VAE_TILE_CORE = 128

_SCALAR_SIZES = {0: 1, 1: 1, 2: 2, 3: 2, 4: 4, 5: 4, 6: 4, 7: 1}


def human(n: int) -> str:
    return f"{n / 1e9:.1f} GB" if n >= 1_000_000_000 else f"{n // 1_000_000} MB"


def _sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(8 * 1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def _fetch(name: str, url: str, size: int, sha256: str | None = None) -> None:
    """Fetch one file atomically (`.part` + rename), resuming when possible."""
    dest = STORE / name
    dest.parent.mkdir(parents=True, exist_ok=True)
    if dest.is_file() and dest.stat().st_size == size:
        if sha256 and _sha256(dest) != sha256:
            print(f"[yue2] {name} hash mismatch — refetching", file=sys.stderr)
            dest.unlink()
        else:
            print(f"[yue2] {name} already present", file=sys.stderr)
            return

    tmp = dest.with_suffix(dest.suffix + ".part")
    start = tmp.stat().st_size if tmp.is_file() else 0
    headers = {"User-Agent": "shiny-songgen/1.0"}
    if start:
        headers["Range"] = f"bytes={start}-"

    print(f"[yue2] {name} ({human(size)})", file=sys.stderr)
    req = urllib.request.Request(url + "?download=true", headers=headers)
    with urllib.request.urlopen(req) as resp, open(tmp, "ab" if start and resp.status == 206 else "wb") as out:
        if not (start and resp.status == 206):
            start = 0
        shutil.copyfileobj(resp, out, length=1024 * 1024)

    actual = tmp.stat().st_size
    if actual != size:
        raise OSError(f"{name}: expected {size} bytes, got {actual}")
    tmp.replace(dest)
    if sha256 and _sha256(dest) != sha256:
        dest.unlink()
        raise OSError(f"{name}: sha256 mismatch")


def _find_tile_value(f) -> tuple[int, int]:
    """(file offset, value) of the decode_core_frames u32 in an open GGUF."""
    if f.read(4) != b"GGUF":
        raise OSError("not a GGUF file")
    _, _, n_kv = struct.unpack("<IQQ", f.read(20))  # version, tensors, kvs
    pos = 24
    for _ in range(n_kv):
        (klen,) = struct.unpack("<Q", f.read(8))
        key = f.read(klen)
        pos += 8 + klen
        (vtype,) = struct.unpack("<I", f.read(4))
        pos += 4
        if vtype == 8:  # string
            (slen,) = struct.unpack("<Q", f.read(8))
            f.seek(slen, 1)
            pos += 8 + slen
        elif vtype == 9:  # array
            (etype,) = struct.unpack("<I", f.read(4))
            (alen,) = struct.unpack("<Q", f.read(8))
            pos += 12
            if etype == 8:
                for _ in range(alen):
                    (slen,) = struct.unpack("<Q", f.read(8))
                    f.seek(slen, 1)
                    pos += 8 + slen
            elif etype in _SCALAR_SIZES:
                skip = _SCALAR_SIZES[etype] * alen
                f.seek(skip, 1)
                pos += skip
            else:
                raise OSError(f"unhandled array element type {etype}")
        elif vtype in _SCALAR_SIZES:
            if key == VAE_TILE_KEY:
                if vtype != 4:
                    raise OSError("decode_core_frames is not a uint32")
                (val,) = struct.unpack("<I", f.read(4))
                return pos, val
            f.seek(_SCALAR_SIZES[vtype], 1)
            pos += _SCALAR_SIZES[vtype]
        else:
            raise OSError(f"unhandled GGUF value type {vtype}")
    raise OSError("decode_core_frames not found")


def _patch_vae_tile(path: Path) -> None:
    """Set the decode tile to VAE_TILE_CORE in place (idempotent)."""
    with open(path, "r+b") as f:
        off, val = _find_tile_value(f)
        if val == VAE_TILE_CORE:
            print(f"[yue2] vae decode tile already {VAE_TILE_CORE}", file=sys.stderr)
            return
        f.seek(off)
        f.write(struct.pack("<I", VAE_TILE_CORE))
        f.flush()
        os.fsync(f.fileno())
    with open(path, "rb") as f:
        _, check = _find_tile_value(f)
    if check != VAE_TILE_CORE:
        raise OSError("vae tile patch did not stick")
    print(f"[yue2] vae decode tile {val} -> {VAE_TILE_CORE}", file=sys.stderr)


def main() -> int:
    targets = sys.argv[1:] or ["q4"]
    keys: list[str] = []
    for target in targets:
        t = target.strip().lower()
        if t == "all":
            keys = list(TIERS)
        elif t in TIERS:
            keys.append(t)
        else:
            print(json.dumps({"status": "error", "error": f"unknown quality: {t}"}))
            return 2
    keys = list(dict.fromkeys(keys))

    STORE.mkdir(parents=True, exist_ok=True)

    try:
        for entry in BASE:
            _fetch(*entry)
        _patch_vae_tile(STORE / VAE_FILE)
        for key in keys:
            lm_file, size = TIERS[key]
            _fetch(lm_file, f"{GGUF_BASE}/{lm_file}", size)
    except OSError as exc:
        print(json.dumps({"status": "error", "error": f"download failed: {exc}"}))
        return 1

    print(json.dumps({"status": "ready", "qualities": keys}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
