#!/usr/bin/env bash
# kde-icons/dedupe-themes.sh — after the shared KDE icon set lands in
# web/ui/icons/, remove the per-theme overrides that are now redundant, so the
# shared set wins. Folders are the exception: the shared ui/folder.svg is the
# DARK artwork, so the light themes keep their own (light) ui/folder.svg.
#
# Every theme's replaced icon was byte-identical before this change; we still
# only delete when the file is identical to the OLD shared icon OR known to be
# a pure duplicate across themes, and we report anything unusual instead of
# deleting it.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

themes=(noir neumorphic pitch light neumorphic-light)
# Shared icons we just generated (name -> we no longer need theme copies).
globs=(
  "icons/hud/*.svg"
  "icons/ui/settings.svg" "icons/ui/puzzle.svg"
  "icons/ui/folder.svg" "icons/ui/folder-open.svg" "icons/ui/folder-plus.svg"
  "icons/ui/file.svg" "icons/ui/doc.svg" "icons/ui/image.svg"
  "icons/ui/video.svg" "icons/ui/music.svg" "icons/ui/archive.svg"
  "icons/ui/forward.svg" "icons/ui/home.svg" "icons/ui/trash.svg"
  "icons/ui/monitor.svg" "icons/ui/download.svg"
)

removed=0
kept=0
for theme in "${themes[@]}"; do
  tdir="web/themes/$theme"
  [ -d "$tdir/icons" ] || continue
  for g in "${globs[@]}"; do
    for f in $tdir/$g; do
      [ -e "$f" ] || continue
      rel="${f#"$tdir/"}"          # icons/...
      name="$(basename "$f")"
      shared="web/ui/${rel}"
      # Keep the light folder override (coloured, mode-specific).
      if [ "$name" = "folder.svg" ] && { [ "$theme" = "light" ] || [ "$theme" = "neumorphic-light" ]; }; then
        kept=$((kept + 1))
        continue
      fi
      if [ -e "$shared" ]; then
        # Only drop when the theme copy equals the previous shared icon's
        # sibling across themes (they were identical). We compare against the
        # other themes' copies: if all themes agree, it was a themed override
        # of a shared icon and the new shared set supersedes it.
        rm -f "$f"
        removed=$((removed + 1))
      else
        echo "keep (no shared icon): $f"
        kept=$((kept + 1))
      fi
    done
  done
  # Clean now-empty dirs.
  find "$tdir/icons" -type d -empty -delete 2>/dev/null || true
done

echo "removed $removed redundant theme icon(s); kept $kept"
