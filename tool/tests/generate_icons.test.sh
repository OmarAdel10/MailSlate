#!/usr/bin/env bash
# Hermetic tests for tool/generate_icons.sh using stub 'magick' binaries and
# MAILSLATE_REPO_ROOT. Covers success paths (stub writes outputs) and
# verification failure paths (no-op stub). Run: bash tool/tests/generate_icons.test.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GEN="$SCRIPT_DIR/../generate_icons.sh"
BASH_BIN="$(command -v bash)"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fail_test() { echo "FAIL: $*" >&2; exit 1; }

FAKE_REPO="$WORK/repo"
mkdir -p "$FAKE_REPO"

EMPTY_BIN="$WORK/empty-bin" # no magick at all
BAD_BIN="$WORK/bad-bin"     # magick that writes nothing
GOOD_BIN="$WORK/good-bin"   # magick that writes its output argument
mkdir -p "$EMPTY_BIN" "$BAD_BIN" "$GOOD_BIN"

cat > "$BAD_BIN/magick" <<'SH'
#!/bin/sh
# writes nothing on purpose - exercises the verification failure path
exit 0
SH

cat > "$GOOD_BIN/magick" <<'SH'
#!/bin/sh
# writes the output path (last argument; may be prefixed PNG32:)
last=''
for arg in "$@"; do last="$arg"; done
case "$last" in
  PNG32:*) out="${last#PNG32:}" ;;
  *) out="$last" ;;
esac
printf 'stub' > "$out"
SH

chmod +x "$BAD_BIN/magick" "$GOOD_BIN/magick"

MASTER_DIR="$FAKE_REPO/assets/branding/icon/source"

# 1. missing magick -> clear failure
if MAILSLATE_REPO_ROOT="$FAKE_REPO" PATH="$EMPTY_BIN" "$BASH_BIN" "$GEN" >"$WORK/out1" 2>&1; then
  fail_test "expected failure when magick is missing"
fi
grep -q "ImageMagick 7" "$WORK/out1" || fail_test "expected magick error message, got: $(cat "$WORK/out1")"
echo "ok - missing magick fails cleanly"

# 2. missing master artwork -> clear failure
if MAILSLATE_REPO_ROOT="$FAKE_REPO" PATH="$GOOD_BIN:$PATH" "$BASH_BIN" "$GEN" >"$WORK/out2" 2>&1; then
  fail_test "expected failure when master artwork is missing"
fi
grep -q "missing master artwork" "$WORK/out2" || fail_test "expected master-artwork error, got: $(cat "$WORK/out2")"
echo "ok - missing master artwork fails cleanly"

mkdir -p "$MASTER_DIR"
printf 'stub' > "$MASTER_DIR/mailslate-icon-master.png"

# 3. no-op magick, no glyph -> verification fails, tray skipped
if MAILSLATE_REPO_ROOT="$FAKE_REPO" PATH="$BAD_BIN:$PATH" "$BASH_BIN" "$GEN" >"$WORK/out3" 2>&1; then
  fail_test "expected verification failure with a no-op magick"
fi
grep -q "MISS" "$WORK/out3" || fail_test "expected MISS entries, got: $(cat "$WORK/out3")"
grep -q "skip tray icons" "$WORK/out3" || fail_test "expected tray skip note"
echo "ok - no-op magick fails verification with MISS entries"

# 4. no-op magick with glyph -> tray outputs part of verification
printf '<svg/>' > "$MASTER_DIR/mailslate-glyph-mono.svg"
if MAILSLATE_REPO_ROOT="$FAKE_REPO" PATH="$BAD_BIN:$PATH" "$BASH_BIN" "$GEN" >"$WORK/out4" 2>&1; then
  fail_test "expected verification failure with a no-op magick"
fi
grep -q "tray/tray.png" "$WORK/out4" || fail_test "expected tray outputs in verification, got: $(cat "$WORK/out4")"
echo "ok - tray outputs verified when the glyph exists"

# 5. success path without glyph -> completes, tray skipped, outputs verified
rm "$MASTER_DIR/mailslate-glyph-mono.svg"
MAILSLATE_REPO_ROOT="$FAKE_REPO" PATH="$GOOD_BIN:$PATH" "$BASH_BIN" "$GEN" >"$WORK/out5" 2>&1 || fail_test "expected success without glyph, got: $(cat "$WORK/out5")"
grep -q "skip tray icons" "$WORK/out5" || fail_test "expected tray skip note on success run"
grep -q "Done." "$WORK/out5" || fail_test "expected Done line"
[ -f "$FAKE_REPO/windows/runner/resources/app_icon.ico" ] || fail_test "expected app_icon.ico"
[ -f "$FAKE_REPO/linux/runner/resources/icons/hicolor/512x512/apps/mailslate.png" ] || fail_test "expected 512px linux icon"
[ -f "$FAKE_REPO/macos/Runner/Assets.xcassets/AppIcon.appiconset/Contents.json" ] || fail_test "expected Contents.json"
echo "ok - success path without glyph"

# 6. success path with glyph -> tray outputs generated and verified
printf '<svg/>' > "$MASTER_DIR/mailslate-glyph-mono.svg"
MAILSLATE_REPO_ROOT="$FAKE_REPO" PATH="$GOOD_BIN:$PATH" "$BASH_BIN" "$GEN" >"$WORK/out6" 2>&1 || fail_test "expected success with glyph, got: $(cat "$WORK/out6")"
grep -q "Done." "$WORK/out6" || fail_test "expected Done line"
if grep -q "skip tray icons" "$WORK/out6"; then
  fail_test "did not expect the tray skip note when the glyph exists"
fi
[ -f "$FAKE_REPO/linux/runner/resources/tray/tray.png" ] || fail_test "expected linux tray png"
[ -f "$FAKE_REPO/linux/runner/resources/tray/tray@2x.png" ] || fail_test "expected linux tray @2x png"
[ -f "$FAKE_REPO/windows/runner/resources/tray.ico" ] || fail_test "expected windows tray ico"
[ -f "$FAKE_REPO/macos/Runner/Resources/TrayIconTemplate.png" ] || fail_test "expected macos tray template png"
echo "ok - success path with glyph"

echo "All generate_icons tests passed."
