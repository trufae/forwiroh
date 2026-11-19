#!/usr/bin/env sh
# Render every raster asset from docs/assets/forwiroh-logo.svg.
# The SVG is the single source of truth; nothing else is hand-edited.
#
#   scripts/render-logo.sh [--check]
#
# --check renders to a temp dir and reports which outputs are stale instead
# of writing them (used by CI).
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
svg="$root/docs/assets/forwiroh-logo.svg"

check=0
[ "${1:-}" = "--check" ] && check=1

if ! command -v rsvg-convert >/dev/null 2>&1; then
  echo "render-logo: rsvg-convert not found (brew install librsvg)" >&2
  exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# One 1024-pixel RGB master; every downstream asset is a copy or a downscale.
rsvg-convert -w 1024 -h 1024 "$svg" -o "$tmp/master.png"
if command -v magick >/dev/null 2>&1; then
  magick "$tmp/master.png" -alpha off -strip -define png:color-type=2 "$tmp/master_rgb.png"
else
  cp "$tmp/master.png" "$tmp/master_rgb.png"
fi

emit() { # emit <source> <dest> [resize]
  dest="$root/$2"
  if [ -n "${3:-}" ]; then
    if command -v magick >/dev/null 2>&1; then
      magick "$1" -filter Lanczos -resize "$3" -strip -define png:color-type=2 "$tmp/out.png"
    else
      rsvg-convert -w "${3%x*}" -h "${3#*x}" "$svg" -o "$tmp/out.png"
    fi
    src="$tmp/out.png"
  else
    src="$1"
  fi

  if [ "$check" = 1 ]; then
    if cmp -s "$src" "$dest" 2>/dev/null; then
      echo "ok    $2"
    elif command -v magick >/dev/null 2>&1 && [ -f "$dest" ] &&
         [ "$(magick "$src" -format '%#' info:)" = "$(magick "$dest" -format '%#' info:)" ]; then
      echo "ok    $2 (pixels match)"
    else
      echo "stale $2"
      stale=1
    fi
    return
  fi

  mkdir -p "$(dirname -- "$dest")"
  cp "$src" "$dest"
  echo "wrote $2"
}

# README hero image (displayed at 176 px, shipped at 2x).
emit "$tmp/master_rgb.png" docs/assets/forwiroh-logo.png 512x512

# iOS app icon: opaque 1024, no alpha channel.
emit "$tmp/master_rgb.png" apps/ios/Forwiroh/Assets.xcassets/AppIcon.appiconset/AppIcon.png

# Android launcher: same 1024 master, the platform applies its own mask.
emit "$tmp/master_rgb.png" apps/android/app/src/main/res/drawable-nodpi/forwiroh_logo.png

if [ "$check" = 1 ] && [ "${stale:-0}" = 1 ]; then
  echo "render-logo: run scripts/render-logo.sh to refresh" >&2
  exit 1
fi
