#!/usr/bin/env bash
# Generates all platform icon assets for MailSlate from the master artwork.
# Contract: specs/APP_ICON.md. Art status: PENDING (owner delivers masters).
#
# Inputs (committed masters) - assets/branding/icon/source/:
#   mailslate-icon-master.png   1024x1024 light tile (required)
#   mailslate-glyph-mono.svg    monochrome glyph for tray icons (optional)
#
# Outputs (generated, committed - APP_ICON.md §4):
#   linux/runner/resources/icons/hicolor/<N>x<N>/apps/mailslate.png  (16..512)
#   windows/runner/resources/app_icon.ico                             (16..256)
#   macos/Runner/Assets.xcassets/AppIcon.appiconset/                  (10 entries)
#   linux/runner/resources/tray/tray.png (+@2x), windows/runner/resources/tray.ico
#   macos/Runner/Resources/TrayIconTemplate.png (+@2x)
#
# Run only when the masters change, then commit the outputs together with
# them (APP_ICON.md §7). Requires ImageMagick >= 7 ('magick').
# macOS icons ship via AppIcon.appiconset - Xcode compiles the .icns at build
# time; no standalone .icns is produced.
# Env: MAILSLATE_REPO_ROOT overrides the repo root (test hook).
set -euo pipefail

REPO_ROOT="${MAILSLATE_REPO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
SRC="$REPO_ROOT/assets/branding/icon/source"
MASTER="$SRC/mailslate-icon-master.png"
GLYPH="$SRC/mailslate-glyph-mono.svg"

fail() { echo "error: $*" >&2; exit 1; }

command -v magick >/dev/null 2>&1 || fail "ImageMagick 7 ('magick') is required (specs/DEVELOPMENT_ENVIRONMENT.md §1)"
[ -f "$MASTER" ] || fail "missing master artwork: $MASTER (specs/APP_ICON.md §3 - art status: pending)"

LINUX_ICONS="$REPO_ROOT/linux/runner/resources/icons/hicolor"
WIN_RES="$REPO_ROOT/windows/runner/resources"
APPICON="$REPO_ROOT/macos/Runner/Assets.xcassets/AppIcon.appiconset"
MAC_RES="$REPO_ROOT/macos/Runner/Resources"
tray_generated=0

# --- Linux hicolor PNG set (single-resize from the 1024 master) ----------
for size in 16 24 32 48 64 128 256 512; do
  dir="$LINUX_ICONS/${size}x${size}/apps"
  mkdir -p "$dir"
  magick "$MASTER" -filter Lanczos -resize "${size}x${size}" "PNG32:$dir/mailslate.png"
done

# --- Windows .ico (multi-size, PNG-compressed, 32-bit alpha) -------------
mkdir -p "$WIN_RES"
magick "$MASTER" -filter Lanczos -define icon:auto-resize=256,128,64,48,32,24,16 "$WIN_RES/app_icon.ico"

# --- macOS AppIcon.appiconset (10 entries) -------------------------------
mkdir -p "$APPICON"
emit_appicon() { # pixel-size filename
  magick "$MASTER" -filter Lanczos -resize "$1x$1" "PNG32:$APPICON/$2"
}
emit_appicon 16   icon_16x16.png
emit_appicon 32   icon_16x16@2x.png
emit_appicon 32   icon_32x32.png
emit_appicon 64   icon_32x32@2x.png
emit_appicon 128  icon_128x128.png
emit_appicon 256  icon_128x128@2x.png
emit_appicon 256  icon_256x256.png
emit_appicon 512  icon_256x256@2x.png
emit_appicon 512  icon_512x512.png
emit_appicon 1024 icon_512x512@2x.png
cat > "$APPICON/Contents.json" <<'JSON'
{
  "images" : [
    { "filename" : "icon_16x16.png",      "idiom" : "mac", "scale" : "1x", "size" : "16x16" },
    { "filename" : "icon_16x16@2x.png",   "idiom" : "mac", "scale" : "2x", "size" : "16x16" },
    { "filename" : "icon_32x32.png",      "idiom" : "mac", "scale" : "1x", "size" : "32x32" },
    { "filename" : "icon_32x32@2x.png",   "idiom" : "mac", "scale" : "2x", "size" : "32x32" },
    { "filename" : "icon_128x128.png",    "idiom" : "mac", "scale" : "1x", "size" : "128x128" },
    { "filename" : "icon_128x128@2x.png", "idiom" : "mac", "scale" : "2x", "size" : "128x128" },
    { "filename" : "icon_256x256.png",    "idiom" : "mac", "scale" : "1x", "size" : "256x256" },
    { "filename" : "icon_256x256@2x.png", "idiom" : "mac", "scale" : "2x", "size" : "256x256" },
    { "filename" : "icon_512x512.png",    "idiom" : "mac", "scale" : "1x", "size" : "512x512" },
    { "filename" : "icon_512x512@2x.png", "idiom" : "mac", "scale" : "2x", "size" : "512x512" }
  ],
  "info" : { "author" : "xcode", "version" : 1 }
}
JSON

# --- Tray icons (monochrome glyph; macOS template naming inverts itself) --
if [ -f "$GLYPH" ]; then
  tray_generated=1
  mkdir -p "$REPO_ROOT/linux/runner/resources/tray" "$MAC_RES"
  magick -background none "$GLYPH" -filter Lanczos -resize 22x22 "PNG32:$REPO_ROOT/linux/runner/resources/tray/tray.png"
  magick -background none "$GLYPH" -filter Lanczos -resize 44x44 "PNG32:$REPO_ROOT/linux/runner/resources/tray/tray@2x.png"
  magick -background none "$GLYPH" -filter Lanczos -define icon:auto-resize=32,16 "$WIN_RES/tray.ico"
  magick -background none "$GLYPH" -filter Lanczos -resize 16x16 "PNG32:$MAC_RES/TrayIconTemplate.png"
  magick -background none "$GLYPH" -filter Lanczos -resize 32x32 "PNG32:$MAC_RES/TrayIconTemplate@2x.png"
else
  echo "warn: $GLYPH missing - skipping tray icons (specs/APP_ICON.md §3)" >&2
fi

# --- Verification (fails unless every generated output exists) -----------
echo "--- Icon verification ---"
missing=0
check() {
  if [ -f "$1" ]; then
    echo "ok   ${1#"$REPO_ROOT"/}"
  else
    echo "MISS ${1#"$REPO_ROOT"/}"
    missing=1
  fi
}

check "$WIN_RES/app_icon.ico"
for size in 16 24 32 48 64 128 256 512; do
  check "$LINUX_ICONS/${size}x${size}/apps/mailslate.png"
done
check "$APPICON/Contents.json"
for name in icon_16x16.png icon_16x16@2x.png icon_32x32.png icon_32x32@2x.png \
            icon_128x128.png icon_128x128@2x.png icon_256x256.png \
            icon_256x256@2x.png icon_512x512.png icon_512x512@2x.png; do
  check "$APPICON/$name"
done

if [ "$tray_generated" -eq 1 ]; then
  check "$REPO_ROOT/linux/runner/resources/tray/tray.png"
  check "$REPO_ROOT/linux/runner/resources/tray/tray@2x.png"
  check "$WIN_RES/tray.ico"
  check "$MAC_RES/TrayIconTemplate.png"
  check "$MAC_RES/TrayIconTemplate@2x.png"
else
  echo "skip tray icons (glyph source missing - specs/APP_ICON.md §3)"
fi

[ "$missing" -eq 0 ] || fail "one or more icon outputs are missing"
echo "Done. Commit the generated outputs together with the masters (specs/APP_ICON.md §7)."
