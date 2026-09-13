#!/usr/bin/env python3
"""Download a faster-whisper (CTranslate2) model for the PEAK'D! STT sidecar.

The tiny model is bundled with the app; this script exists so the optional
small model can be fetched on demand from Settings → Voice, and so a fresh
checkout can (re)install the bundled tiny model if it is missing.

Deliberately dependency-free (stdlib only): it must run under whatever
`python3` the core server finds, which is not necessarily the interpreter that
has faster-whisper installed.

Usage:
    python3 voice/download_whisper.py tiny
    python3 voice/download_whisper.py small
"""

import json
import os
import shutil
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

MODELS = {
    "tiny": {
        "dir": "faster-whisper-tiny",
        "repo": "Systran/faster-whisper-tiny",
        "size": "~75 MB",
    },
    "small": {
        "dir": "faster-whisper-small",
        "repo": "Systran/faster-whisper-small",
        "size": "~480 MB",
    },
}

# Everything CTranslate2 needs to load the model directory. README/.gitattributes
# are intentionally skipped.
FILES = ["config.json", "model.bin", "tokenizer.json", "vocabulary.txt"]

MODELS_DIR = Path(
    os.environ.get("WHISPER_MODELS_DIR", ROOT / "data" / "whisper-models")
)


def _download(url: str, dest: Path) -> None:
    """Fetch url → dest atomically, so a killed download is never half-loaded."""
    tmp = dest.with_suffix(dest.suffix + ".part")
    req = urllib.request.Request(url, headers={"User-Agent": "peakd-voice/1.0"})
    with urllib.request.urlopen(req) as resp, open(tmp, "wb") as out:
        shutil.copyfileobj(resp, out, length=1024 * 1024)
    tmp.replace(dest)


def model_ready(key: str) -> bool:
    path = MODELS_DIR / MODELS[key]["dir"]
    return (path / "model.bin").is_file() and (path / "config.json").is_file()


def download(key: str, force: bool = False) -> dict:
    key = (key or "tiny").strip().lower()
    if key not in MODELS:
        raise ValueError(f"Unknown whisper model: {key}")

    spec = MODELS[key]
    target = MODELS_DIR / spec["dir"]

    if model_ready(key) and not force:
        size_mb = (target / "model.bin").stat().st_size / (1024 * 1024)
        return {
            "status": "ready",
            "model": key,
            "dir": str(target),
            "size_mb": round(size_mb, 1),
            "downloaded": False,
        }

    MODELS_DIR.mkdir(parents=True, exist_ok=True)
    target.mkdir(parents=True, exist_ok=True)

    base = f"https://huggingface.co/{spec['repo']}/resolve/main"
    for name in FILES:
        dest = target / name
        if dest.is_file() and not force and name != "model.bin":
            continue
        print(f"Downloading {spec['repo']}/{name}…", flush=True)
        _download(f"{base}/{name}?download=true", dest)

    if not model_ready(key):
        raise RuntimeError(f"Model {key} is incomplete at {target}")

    size_mb = (target / "model.bin").stat().st_size / (1024 * 1024)
    return {
        "status": "ready",
        "model": key,
        "dir": str(target),
        "size_mb": round(size_mb, 1),
        "downloaded": True,
    }


if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    force = "--force" in sys.argv
    model_key = args[0] if args else "tiny"
    try:
        print(json.dumps(download(model_key, force=force)))
    except Exception as exc:  # noqa: BLE001 - reported to the server as-is
        print(json.dumps({"status": "error", "error": str(exc)}))
        sys.exit(1)
