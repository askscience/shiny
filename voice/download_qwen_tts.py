#!/usr/bin/env python3
"""Download the GGUF weights for the Qwen3-TTS sidecar (qwentts.cpp).

The bundled 0.6B CustomVoice model is fetched on first run; the optional 1.7B
is an opt-in download from Settings → Voice, the same shape as faster-whisper's
`tiny` (bundled) and `small` (on demand).

Two files are needed and the codec is shared by every talker:
  qwen-talker-<size>-customvoice-Q4_K_M.gguf   the talker LM
  qwen-tokenizer-12hz-Q4_K_M.gguf              the 12 Hz RVQ codec

Q4_K_M is deliberate: the RVQ codec paths stay F32 and the talker keeps
K-quant precision, so it is a fraction of the F16 size with no meaningful
quality loss for this use.

Deliberately dependency-free (stdlib only): it must run under whatever
`python3` the core server finds, which is not necessarily an interpreter with
`huggingface_hub` installed.

Usage:
    python3 voice/download_qwen_tts.py 0.6b-customvoice
    python3 voice/download_qwen_tts.py 1.7b-customvoice
    python3 voice/download_qwen_tts.py all
"""

import json
import os
import shutil
import sys
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

REPO = "Serveurperso/Qwen3-TTS-GGUF"
CODEC = "qwen-tokenizer-12hz-Q4_K_M.gguf"

MODELS = {
    "0.6b-customvoice": {
        "file": "qwen-talker-0.6b-customvoice-Q4_K_M.gguf",
        "size": "~605 MB",
    },
    "1.7b-customvoice": {
        "file": "qwen-talker-1.7b-customvoice-Q4_K_M.gguf",
        "size": "~1.2 GB",
    },
}

MODELS_DIR = Path(
    os.environ.get("QWEN_TTS_MODELS_DIR", ROOT / "data" / "qwen-tts-models")
)


def _url(filename: str) -> str:
    return f"https://huggingface.co/{REPO}/resolve/main/{filename}?download=true"


def _download(filename: str) -> None:
    """Fetch one file atomically, streaming so progress is observable."""
    dest = MODELS_DIR / filename
    tmp = dest.with_suffix(dest.suffix + ".part")
    # A partial file from a previous attempt is kept only while it matches what
    # was already written; restarting is simpler and HF supports ranged resume
    # only via the CLI, so a clean fetch is the honest behaviour here.
    req = urllib.request.Request(_url(filename), headers={"User-Agent": "peakd-voice/1.0"})
    with urllib.request.urlopen(req) as resp, open(tmp, "wb") as out:
        shutil.copyfileobj(resp, out, length=1024 * 1024)
    tmp.replace(dest)


def model_ready(key: str) -> bool:
    return (
        (MODELS_DIR / MODELS[key]["file"]).is_file()
        and (MODELS_DIR / CODEC).is_file()
    )


def main() -> int:
    target = sys.argv[1] if len(sys.argv) > 1 else "0.6b-customvoice"
    if target == "all":
        keys = list(MODELS)
    elif target in MODELS:
        keys = [target]
    else:
        print(json.dumps({"status": "error", "error": f"unknown model: {target}"}))
        return 2

    MODELS_DIR.mkdir(parents=True, exist_ok=True)

    # The codec is shared, so fetch it once before any talker.
    try:
        if not (MODELS_DIR / CODEC).is_file():
            print(f"[qwen-tts] codec {CODEC}", file=sys.stderr)
            _download(CODEC)
        for key in keys:
            filename = MODELS[key]["file"]
            if (MODELS_DIR / filename).is_file():
                print(f"[qwen-tts] {filename} already present", file=sys.stderr)
                continue
            print(f"[qwen-tts] {filename} ({MODELS[key]['size']})", file=sys.stderr)
            _download(filename)
    except urllib.error.URLError as exc:
        print(json.dumps({"status": "error", "error": f"download failed: {exc}"}))
        return 1
    except OSError as exc:
        print(json.dumps({"status": "error", "error": f"write failed: {exc}"}))
        return 1

    ready = [k for k in keys if model_ready(k)]
    print(json.dumps({"status": "ready", "models": ready}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
