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


# The smallest Vosk model is tens of MB; anything under this is a truncated pack.
MIN_ARCHIVE_BYTES = 1024 * 1024


def is_valid_archive(path: Path) -> bool:
    """True when `path` is a readable gzip tarball of plausible size."""
    try:
        if path.stat().st_size < MIN_ARCHIVE_BYTES:
            return False
        with tarfile.open(path, "r:gz") as tar:
            return tar.next() is not None
    except (OSError, tarfile.TarError, EOFError):
        return False


def is_complete_model(path: Path) -> bool:
    """True when an extracted model folder holds a non-empty acoustic model and graph."""
    try:
        return (path / "am" / "final.mdl").stat().st_size > 0 and any(
            f.stat().st_size > 0 for f in (path / "graph").glob("*.fst")
        )
    except OSError:
        return False


def download_and_pack(lang: str) -> dict:
    stt_lang, entry = resolve_lang(lang)
    if "vosk_zip" not in entry:
        raise ValueError(f"No Vosk model for language: {lang}")

    MODELS_DIR.mkdir(parents=True, exist_ok=True)
    tar_path = MODELS_DIR / f"{stt_lang}.tar.gz"
    if is_valid_archive(tar_path):
        size_mb = tar_path.stat().st_size / (1024 * 1024)
        return {"status": "ready", "lang": stt_lang, "size_mb": round(size_mb, 1)}
    # A truncated archive (an interrupted pack) would otherwise count as
    # "ready" forever and fail every load in the browser.
    tar_path.unlink(missing_ok=True)

    zip_url = entry["vosk_zip"]
    folder = entry["vosk_folder"]
    zip_path = MODELS_DIR / f"{folder}.zip"
    extract_dir = MODELS_DIR / stt_lang

    # An earlier run may have extracted the model and been interrupted while
    # packing: reuse that folder instead of downloading it again.
    if not is_complete_model(extract_dir):
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

    # Pack with a top-level wrapper directory. vosk-browser's WASM model loader
    # always extracts with `strip_first_component = true`, so entries must be
    # "<wrapper>/<am|conf|graph|ivector>/…" for the subdirectories to survive.
    # Without the wrapper, the `ivector/` folder is flattened to the root and
    # Vosk can't find `ivector/final.ie` → "Ivector feature dimension mismatch".
    # Pack to a temporary name and rename at the end, so an interruption never
    # leaves a partial archive under the final name.
    tmp_path = tar_path.with_name(tar_path.name + ".part")
    try:
        with tarfile.open(tmp_path, "w:gz") as tar:
            for item in extract_dir.iterdir():
                tar.add(item, arcname=f"{extract_dir.name}/{item.name}")
        os.replace(tmp_path, tar_path)
    finally:
        tmp_path.unlink(missing_ok=True)

    size_mb = tar_path.stat().st_size / (1024 * 1024)
    return {"status": "ready", "lang": stt_lang, "size_mb": round(size_mb, 1)}


if __name__ == "__main__":
    lang = sys.argv[1] if len(sys.argv) > 1 else "en"
    result = download_and_pack(lang)
    print(json.dumps(result))
