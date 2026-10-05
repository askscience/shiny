#!/usr/bin/env python3
"""fetch-infinity.py — download the curated Infinity sources.

Reads ``mapping-infinity.json`` and downloads every referenced SVG from the
Infinity repository (github.com/rogts/infinity-icon-theme, GPL-3.0) into
``assets/iconsets/Infinity/{infinity,infinity-dark}/``, following the theme's
symlinks (``actions/symbolic/*`` → ``actions/16/*``, ``folder-music`` →
``folder-sound``).  The raw tree is git-ignored; only the generated icons under
``web/ui/iconsets/`` are committed.

Run:  python3 scripts/kde-icons/fetch-infinity.py
"""

from __future__ import annotations

import json
import threading
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RAW = "https://raw.githubusercontent.com/rogts/infinity-icon-theme/main"
OUT = ROOT / "assets" / "iconsets" / "Infinity"
MAPPING = ROOT / "scripts" / "kde-icons" / "mapping-infinity.json"

_lock = threading.Lock()
_cache: dict[str, str] = {}
_failures: list[str] = []


def _get(path: str) -> str | None:
    with _lock:
        if path in _cache:
            return _cache[path]
    try:
        req = urllib.request.Request(f"{RAW}/{path}", headers={"User-Agent": "curl/8"})
        with urllib.request.urlopen(req, timeout=180) as res:
            text = res.read().decode("utf-8", "replace")
    except Exception:  # noqa: BLE001 — recorded by the caller, not here
        return None
    with _lock:
        _cache[path] = text
    return text


def _resolve(path: str) -> tuple[str, str] | None:
    """(resolved content, final path) following in-repo symlinks, or None."""
    seen = set()
    for _ in range(8):
        if path in seen:
            return None
        seen.add(path)
        text = _get(path)
        if text is None:
            return None
        stripped = text.strip()
        if stripped.startswith("<"):  # a real SVG document
            return text, path
        if stripped.endswith(".svg") and len(stripped) < 200:
            base = path.rsplit("/", 1)[0]
            path = str(Path(base) / stripped)
            continue
        return None
    return None


def _fetch_one(variant: str, rel: str) -> tuple[str, str, bool]:
    dest = OUT / variant / rel
    if dest.exists() and dest.stat().st_size > 0:
        return f"{variant}/{rel}", "cached", True
    url = f"{variant}/{rel}"
    resolved = _resolve(url)
    if resolved is None:
        _failures.append(f"{variant}/{rel}")
        return url, "failed", False
    text, _final = resolved
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(text, encoding="utf-8")
    return url, "ok", True


def _add(jobs: list[tuple[str, str]], seen: set[str], variant: str, rel: str) -> None:
    job = (variant, rel)
    if job not in seen:
        seen.add(job)
        jobs.append(job)


def main() -> int:
    mapping = json.loads(MAPPING.read_text(encoding="utf-8"))
    jobs: list[tuple[str, str]] = []
    seen: set[str] = set()
    for group in ("hud", "ui", "apps"):
        for entry in mapping[group]:
            src = entry.get("src")
            if src:
                # `src` lives in the dark tree only when flagged, else light.
                _add(jobs, seen, "infinity-dark" if entry.get("src_is_dark") else "infinity", src)
            if entry.get("src_dark"):
                _add(jobs, seen, "infinity-dark", entry["src_dark"])

    print(f"downloading {len(jobs)} file(s) …")
    done = 0
    warnings: list[str] = []
    with ThreadPoolExecutor(max_workers=8) as pool:
        for url, status, ok in pool.map(lambda j: _fetch_one(*j), jobs):
            done += 1
            if ok:
                continue
            # A dark-only symlink may point outside the repo (e.g. ../breeze/…).
            # That is not fatal when the light copy of the same path exists:
            # the converter falls back to it.
            variant, rel = url.split("/", 1)
            if variant == "infinity-dark" and (OUT / "infinity" / rel).exists():
                warnings.append(url)
                continue
            print(f"  ! {url}")
    # Keep a copy of the license next to the raw set.
    (OUT / "LICENSE").parent.mkdir(parents=True, exist_ok=True)
    license_text = _get("LICENSE")
    if license_text:
        (OUT / "LICENSE").write_text(license_text, encoding="utf-8")
    if _failures:
        real = [f for f in _failures
                if not (f.startswith("infinity-dark/")
                        and (OUT / "infinity" / f.split("/", 1)[1]).exists())]
        if real:
            print(f"\n{len(real)} failure(s):")
            for f in real[:30]:
                print("  ", f)
            return 1
    if warnings:
        print(f"({len(warnings)} dark symlink(s) resolve via the light fallback)")
    print(f"done — {done} source file(s) under {OUT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
