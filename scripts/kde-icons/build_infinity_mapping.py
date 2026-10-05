#!/usr/bin/env python3
"""build_infinity_mapping.py — derive mapping-infinity.json for the Infinity
icon set from a candidate table.

Infinity (github.com/rogts/infinity-icon-theme, GPL-3.0) is a Breeze-derived
KDE icon theme with a symlink-heavy layout (``actions/symbolic/*`` points at
``actions/16/*``, ``places/48/folder-music.svg`` at ``folder-sound.svg``).
This script validates every candidate path against the two variants
(``infinity/`` and ``infinity-dark/``) and writes the mapping the converter
consumes. Light sources are preferred; when a name only exists in the dark
tree the dark file is recorded as the fallback.

Run:  python3 scripts/kde-icons/build_infinity_mapping.py
"""

from __future__ import annotations

import json
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REPO = "rogts/infinity-icon-theme"
TREES = ROOT / "assets" / "iconsets" / "Infinity" / "trees"
MAPPING = ROOT / "scripts" / "kde-icons" / "mapping-infinity.json"


def _fetch_json(url: str) -> dict:
    req = urllib.request.Request(url, headers={"User-Agent": "curl/8"})
    with urllib.request.urlopen(req, timeout=120) as res:
        return json.load(res)


def _variant_paths(name: str, root_sha: str) -> set[str]:
    """Every blob path of a variant (cached under assets/iconsets)."""
    cache = TREES / f"{name}.json"
    if cache.exists():
        data = json.loads(cache.read_text(encoding="utf-8"))
    else:
        data = _fetch_json(
            f"https://api.github.com/repos/{REPO}/git/trees/{root_sha}?recursive=1")
        cache.parent.mkdir(parents=True, exist_ok=True)
        cache.write_text(json.dumps(data), encoding="utf-8")
    return {e["path"] for e in data.get("tree", [])
            if e.get("type") == "blob" and e["path"].endswith(".svg")}


# Root tree SHAs of the two variants (stable while the repo is unchanged).
LIGHT = _variant_paths("infinity", "84c6d0758d98d24e231cec405b81be4a588790b8")
DARK = _variant_paths("infinity-dark", "0ea29e711163dc90c002aea927801c58705b51a3")


def _exists(tree: set[str], rel: str) -> bool:
    return rel in tree


# name -> candidates (relative to a variant root), first hit wins.
ACTIONS = "actions/16"
UI: dict[str, list[str]] = {
    "archive": ["mimetypes/32/package-x-generic.svg"],
    "arranger": [f"{ACTIONS}/view-list-details.svg"],
    "arrow-left": [f"{ACTIONS}/arrow-left.svg"],
    "arrow-up": [f"{ACTIONS}/arrow-up.svg"],
    "blur": [f"{ACTIONS}/blur.svg"],
    "bold": [f"{ACTIONS}/format-text-bold.svg"],
    "brightness": ["actions/22/brightness-high.svg", "actions/24/brightness-high.svg"],
    "brush": [f"{ACTIONS}/draw-brush.svg", f"{ACTIONS}/tool_brush.svg"],
    "burn": [f"{ACTIONS}/tools-media-optical-burn.svg"],
    "calc": ["mimetypes/32/x-office-spreadsheet.svg"],
    "calculator": ["apps/48/accessories-calculator.svg", "apps/22/accessories-calculator.svg"],
    "calendar": [f"{ACTIONS}/view-calendar.svg"],
    "channels": ["actions/22/channelmixer.svg", "actions/24/channelmixer.svg"],
    "check": [f"{ACTIONS}/dialog-ok.svg"],
    "chevron-down": [f"{ACTIONS}/go-down.svg"],
    "chevron-left": [f"{ACTIONS}/go-previous.svg"],
    "chevron-right": [f"{ACTIONS}/go-next.svg"],
    "chevron-up": [f"{ACTIONS}/go-up.svg"],
    "clone-stamp": [f"{ACTIONS}/edit-clone.svg"],
    "close": [f"{ACTIONS}/window-close.svg"],
    "compress": [f"{ACTIONS}/transform-scale.svg"],
    "contrast": ["actions/22/contrast.svg", "actions/24/contrast.svg"],
    "copy": [f"{ACTIONS}/edit-copy.svg"],
    "crop": [f"{ACTIONS}/transform-crop.svg"],
    "curves": ["actions/22/adjustcurves.svg", "actions/24/adjustcurves.svg"],
    "deselect": [f"{ACTIONS}/edit-select-none.svg"],
    "doc": ["mimetypes/32/x-office-document.svg"],
    "dodge": [f"{ACTIONS}/draw-highlight.svg"],
    "download": [f"{ACTIONS}/download.svg"],
    "droplet": [f"{ACTIONS}/color-picker.svg"],
    "emboss": ["actions/22/embosstool.svg", "actions/24/embosstool.svg"],
    "eraser": [f"{ACTIONS}/draw-eraser.svg"],
    "expand": [f"{ACTIONS}/expand.svg"],
    "eye": [f"{ACTIONS}/view-visible.svg"],
    "eyedropper": [f"{ACTIONS}/color-picker.svg"],
    "file": ["mimetypes/32/text-x-generic.svg"],
    "fill": [f"{ACTIONS}/color-fill.svg"],
    "filter": [f"{ACTIONS}/view-filter.svg"],
    "flip-h": [f"{ACTIONS}/object-flip-horizontal.svg"],
    "flip-v": [f"{ACTIONS}/object-flip-vertical.svg"],
    "folder": ["places/48/folder.svg"],
    "folder-open": ["places/32/folder-open.svg"],
    "folder-plus": [f"{ACTIONS}/folder-new.svg"],
    "forward": ["emblems/16/emblem-symbolic-link.svg"],
    "fx": [f"{ACTIONS}/tool_imageeffects.svg"],
    "gradient": [f"{ACTIONS}/color-gradient.svg"],
    "grayscale": [f"{ACTIONS}/color-mode-black-white.svg"],
    "grid": [f"{ACTIONS}/view-grid.svg"],
    "grip": [f"{ACTIONS}/transform-move.svg"],
    "hand": [f"{ACTIONS}/tool-pointer.svg"],
    "history": [f"{ACTIONS}/view-history.svg"],
    "home": [f"{ACTIONS}/go-home.svg"],
    "image": ["mimetypes/32/image-x-generic.svg"],
    "incognito": [f"{ACTIONS}/view-private.svg"],
    "info": ["status/16/dialog-information.svg"],
    "inverse": [f"{ACTIONS}/edit-select-invert.svg"],
    "invert": [f"{ACTIONS}/color-mode-invert-image.svg"],
    "italic": [f"{ACTIONS}/format-text-italic.svg"],
    "keyboard": ["devices/16/input-keyboard.svg"],
    "lasso": ["actions/22/edit-select-lasso.svg", "actions/24/edit-select-lasso.svg"],
    "launcher": [f"{ACTIONS}/application-menu.svg"],
    "layers": [f"{ACTIONS}/dialog-layers.svg"],
    "link": [f"{ACTIONS}/link.svg"],
    "list": [f"{ACTIONS}/view-list-text.svg"],
    "lock": [f"{ACTIONS}/lock.svg"],
    "loop": [f"{ACTIONS}/media-playlist-repeat.svg"],
    "magic-wand": [f"{ACTIONS}/tools-wizard.svg"],
    "mail": [f"{ACTIONS}/mail-message.svg"],
    "marquee-ellipse": [f"{ACTIONS}/tool_elliptical_selection.svg"],
    "marquee-rect": [f"{ACTIONS}/select-rectangular.svg"],
    "mask": [f"{ACTIONS}/path-mask-edit.svg"],
    "message-circle": [f"{ACTIONS}/view-conversation-balloon.svg"],
    "metronome": [f"{ACTIONS}/chronometer.svg"],
    "mic": ["devices/16/audio-input-microphone.svg"],
    "mic-off": ["status/16/mic-off.svg"],
    "minus": [f"{ACTIONS}/list-remove.svg"],
    "monitor": ["devices/16/monitor.svg"],
    "more": [f"{ACTIONS}/overflow-menu.svg"],
    "move": [f"{ACTIONS}/transform-move.svg"],
    "music": ["mimetypes/32/audio-x-generic.svg"],
    "navigator": [f"{ACTIONS}/view-sidetree.svg"],
    "network": [f"{ACTIONS}/network-connect.svg"],
    "noise": ["actions/22/noisereduction.svg", "actions/24/noisereduction.svg"],
    "paint-bucket": [f"{ACTIONS}/tool_flood_fill.svg"],
    "palette": [f"{ACTIONS}/color-management.svg"],
    "paths": [f"{ACTIONS}/editpath.svg"],
    "pause": [f"{ACTIONS}/media-playback-pause.svg"],
    "pencil": [f"{ACTIONS}/document-edit.svg"],
    "pin": [f"{ACTIONS}/pin.svg"],
    "play": [f"{ACTIONS}/media-playback-start.svg"],
    "plus": [f"{ACTIONS}/list-add.svg"],
    "power": ["actions/22/system-shutdown.svg", "actions/32/system-shutdown.svg"],
    "present": [f"{ACTIONS}/view-presentation.svg"],
    "properties": [f"{ACTIONS}/document-properties.svg"],
    "puzzle": [f"{ACTIONS}/plugins.svg"],
    "redo": [f"{ACTIONS}/edit-redo.svg"],
    "refresh": [f"{ACTIONS}/view-refresh.svg"],
    "reply": [f"{ACTIONS}/mail-reply-sender.svg"],
    "rotate-left": [f"{ACTIONS}/object-rotate-left.svg"],
    "rotate-right": [f"{ACTIONS}/object-rotate-right.svg"],
    "save": [f"{ACTIONS}/document-save.svg"],
    "search": [f"{ACTIONS}/search.svg"],
    "select-all": [f"{ACTIONS}/edit-select-all.svg"],
    "send": [f"{ACTIONS}/mail-send.svg"],
    "settings": [f"{ACTIONS}/configure.svg"],
    "shape": [f"{ACTIONS}/shapes.svg"],
    "sharpen": ["actions/22/sharpenimage.svg", "actions/24/sharpenimage.svg"],
    "shield": ["status/16/security-high.svg"],
    "sponge": [f"{ACTIONS}/draw-watercolor.svg"],
    "star": ["emblems/16/rating.svg"],
    "stop": [f"{ACTIONS}/media-playback-stop.svg"],
    "stroke": [f"{ACTIONS}/object-stroke.svg"],
    "suspend": ["actions/22/system-suspend.svg", "actions/32/system-suspend.svg"],
    "swatches": [f"{ACTIONS}/paint-swatch.svg"],
    "threshold": [f"{ACTIONS}/view-object-histogram-linear.svg"],
    "trash": ["places/48/user-trash.svg"],
    "type": [f"{ACTIONS}/draw-text.svg"],
    "underline": [f"{ACTIONS}/format-text-underline.svg"],
    "undo": [f"{ACTIONS}/edit-undo.svg"],
    "upload": [f"{ACTIONS}/cloud-upload.svg"],
    "user": [f"{ACTIONS}/user.svg"],
    "video": ["mimetypes/32/video-x-generic.svg"],
    "volume": [f"{ACTIONS}/player-volume.svg"],
    "warning": ["status/16/dialog-warning.svg"],
    "youtube": [f"{ACTIONS}/im-youtube.svg"],
    "zoom-in": [f"{ACTIONS}/zoom-in.svg"],
    "zoom-out": [f"{ACTIONS}/zoom-out.svg"],
}

# Coloured folder variants used by the Files plugin.
FOLDERS = {
    "folder-documents": ["places/48/folder-documents.svg"],
    "folder-downloads": ["places/48/folder-downloads.svg"],
    "folder-desktop": ["places/48/user-desktop.svg"],
    "folder-music": ["places/48/folder-music.svg"],
    "folder-pictures": ["places/48/folder-pictures.svg"],
    "folder-public": ["places/32/folder-public.svg", "places/48/folder-public.svg"],
    "folder-templates": ["places/32/folder-templates.svg"],
    "folder-videos": ["places/48/folder-videos.svg"],
}

HUD = {
    "wifi-0": ["devices/16/network-wireless-connected-00.svg"],
    "wifi-1": ["devices/16/network-wireless-connected-25.svg"],
    "wifi-2": ["devices/16/network-wireless-connected-50.svg"],
    "wifi-3": ["devices/16/network-wireless-connected-75.svg"],
    "wifi-4": ["devices/16/network-wireless-connected-100.svg"],
    "wifi-off": ["devices/16/network-wireless-disconnected.svg"],
    "ethernet": ["devices/16/network-wired.svg"],
    "bluetooth": ["status/22/network-bluetooth-activated.svg", "devices/22/network-bluetooth.svg"],
    "bluetooth-off": ["devices/16/network-bluetooth.svg"],
    "volume-0": ["status/16/audio-volume-muted.svg"],
    "volume-1": ["status/16/audio-volume-low.svg"],
    "volume-2": ["status/16/audio-volume-medium.svg"],
    "volume-3": ["status/16/audio-volume-high.svg"],
    "volume-muted": ["status/16/audio-volume-muted.svg"],
    "headphones": ["devices/16/audio-headphones.svg", "devices/64/audio-headphones.svg"],
    "battery-0": ["status/16/battery-000.svg"],
    "battery-1": ["status/16/battery-020.svg"],
    "battery-2": ["status/16/battery-040.svg"],
    "battery-3": ["status/16/battery-070.svg"],
    "battery-4": ["status/16/battery-100.svg"],
    "battery-charging": ["status/16/battery-060-charging.svg"],
    "clock": [f"{ACTIONS}/clock.svg"],
}

# Coloured plugin identity icons. Names fall back to the base symbolic set when
# no source is found in either variant.
APPS = {
    "browser": ["apps/48/chromium.svg"],
    "calc": ["mimetypes/32/x-office-spreadsheet.svg"],
    "calculator": ["apps/48/accessories-calculator.svg", "apps/22/accessories-calculator.svg"],
    "calendar": ["apps/48/calindori.svg"],
    "files": ["apps/48/system-file-manager.svg"],
    "hello": ["emotes/22/face-smile.svg", f"{ACTIONS}/smiley.svg"],
    "image": ["apps/48/gwenview.svg", "apps/48/inkscape-logo.svg"],
    "impress": ["mimetypes/32/x-office-presentation.svg"],
    "keyboard": ["preferences/32/preferences-desktop-keyboard.svg", "devices/16/input-keyboard.svg"],
    "mail": ["mimetypes/32/application-vnd.stardivision.mail.svg", f"{ACTIONS}/mail-message.svg"],
    "pdf": ["mimetypes/64/application-pdf.svg", "mimetypes/32/application-pdf.svg"],
    "radio": ["devices/64/multimedia-player.svg", f"{ACTIONS}/media-playback-start.svg"],
    "studio": ["devices/64/audio-card.svg", f"{ACTIONS}/view-media-equalizer.svg"],
    "terminal": ["apps/48/utilities-terminal.svg", "apps/64/utilities-terminal.svg"],
    "traveler": [f"{ACTIONS}/mark-location.svg"],
    "word": ["apps/48/libreoffice-writer.svg"],
    "youtube": [f"{ACTIONS}/im-youtube.svg"],
}

COLOR = {
    "folder", "folder-open", "folder-documents", "folder-downloads",
    "folder-desktop", "folder-music", "folder-pictures", "folder-public",
    "folder-templates", "folder-videos", "trash",
    "archive", "calc", "doc", "file", "image", "music", "video",
    "calculator",
    "browser", "calendar", "files", "hello", "impress", "keyboard", "mail",
    "pdf", "radio", "studio", "terminal", "word",
}


def pick(name: str, candidates: list[str], report: list[str]) -> dict | None:
    light = next((c for c in candidates if _exists(LIGHT, c)), None)
    dark = next((c for c in candidates if _exists(DARK, c)), None)
    if not light and not dark:
        report.append(name)
        return None
    entry = {"name": name, "dst": f"{name}.svg"}
    if light:
        entry["src"] = light
    if dark and (dark != light or name in COLOR):
        if light:
            # Same path, but the dark variant ships its own coloured artwork.
            entry["src_dark"] = dark
        else:
            # Only the dark variant ships this file; record its true root.
            entry["src"] = dark
            entry["src_is_dark"] = True
    if name in COLOR:
        entry["color"] = True
    return entry


def main() -> int:
    missing: list[str] = []
    groups = {"hud": HUD, "ui": {**UI, **FOLDERS}, "apps": APPS}
    out = {"_comment": (
        "Infinity icon set (github.com/rogts/infinity-icon-theme, GPL-3.0). "
        "Generated by scripts/kde-icons/build_infinity_mapping.py — do not edit. "
        "`src` is relative to the light variant root, `src_dark` to the dark one; "
        "the converter falls back across roots when one lacks a file. "
        "`color: true` keeps original fills instead of rewriting to currentColor."
    )}
    for group, table in groups.items():
        entries = []
        for name, candidates in table.items():
            entry = pick(name, candidates, missing)
            if entry:
                # Group prefix: hud/* stays, ui/* stays, apps/* stays.
                entry["dst"] = f"{group}/{name.split('/', 1)[-1]}.svg"
                entries.append(entry)
        out[group] = sorted(entries, key=lambda e: e["dst"])
    if missing:
        print("!! no source found for:", ", ".join(sorted(missing)))
    MAPPING.write_text(json.dumps(out, indent=2) + "\n", encoding="utf-8")
    counts = {g: len(out[g]) for g in ("hud", "ui", "apps")}
    print(f"wrote {MAPPING} — {counts}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
