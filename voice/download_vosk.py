#!/usr/bin/env python3
"""Download and pack a Vosk model for browser use."""
import json
import os
import shutil
import sys
import tarfile
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LANG_MAP = ROOT / "voice" / "lang_map.json"
MODELS_DIR = Path(os.environ.get("VOSK_MODELS_DIR", ROOT / "data" / "vosk-models"))


def resolve_lang(lang: str) -> tuple[str, dict]:
    with open(LANG_MAP) as f:
        m = json.load(f)
    if lang not in m:
        lang = "en"
    entry = m[lang]
    stt = lang
    if "vosk_stt_fallback" in entry:
        stt = entry["vosk_stt_fallback"]
        entry = m[stt]
    return stt, entry


def download_and_pack(lang: str) -> dict:
    stt_lang, entry = resolve_lang(lang)
    if "vosk_zip" not in entry:
        raise ValueError(f"No Vosk model for language: {lang}")

    MODELS_DIR.mkdir(parents=True, exist_ok=True)
    tar_path = MODELS_DIR / f"{stt_lang}.tar.gz"
    if tar_path.exists():
        size_mb = tar_path.stat().st_size / (1024 * 1024)
        return {"status": "ready", "lang": stt_lang, "size_mb": round(size_mb, 1)}

    zip_url = entry["vosk_zip"]
    folder = entry["vosk_folder"]
    zip_path = MODELS_DIR / f"{folder}.zip"
    extract_dir = MODELS_DIR / stt_lang

    print(f"Downloading {zip_url}...", flush=True)
    urllib.request.urlretrieve(zip_url, zip_path)

    if extract_dir.exists():
        shutil.rmtree(extract_dir)
    extract_dir.mkdir(parents=True)

    with zipfile.ZipFile(zip_path, "r") as zf:
        zf.extractall(MODELS_DIR)
    src = MODELS_DIR / folder
    if src.exists() and src != extract_dir:
        if extract_dir.exists():
            shutil.rmtree(extract_dir)
        shutil.move(str(src), str(extract_dir))
    zip_path.unlink(missing_ok=True)

    with tarfile.open(tar_path, "w:gz") as tar:
        for item in extract_dir.iterdir():
            tar.add(item, arcname=item.name)

    size_mb = tar_path.stat().st_size / (1024 * 1024)
    return {"status": "ready", "lang": stt_lang, "size_mb": round(size_mb, 1)}


if __name__ == "__main__":
    lang = sys.argv[1] if len(sys.argv) > 1 else "en"
    result = download_and_pack(lang)
    print(json.dumps(result))
