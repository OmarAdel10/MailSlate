#!/usr/bin/env bash
# Hermetic tests for tool/generate_icons.sh failure/skip paths using a stub
# 'magick' and MAILSLATE_REPO_ROOT. Run: bash tool/tests/generate_icons.test.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GEN="$SCRIPT_DIR/../generate_icons.sh"
BASH_BIN="$(command -v bash)"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fail_test() { echo "FAIL: $*" >&2; exit 1; }

FAKE_REPO="$WORK/repo"
mkdir -p "$FAKE_REPO"

EMPTY_BIN="$WORK/empty-bin"
STUB_BIN="$WORK/stub-bin"
mkdir -p "$EMPTY_BIN" "$STUB_BIN"
cat > "$STUB_BIN/magick" <<'SH'
#!/bin/sh
# writes nothing on purpose - exercises the verification path
exit 0
SH
chmod +x "$STUB_BIN/magick"

# 1. missing magick -> clear failure
if MAILSLATE_REPO_ROOT="$FAKE_REPO" PATH="$EMPTY_BIN" "$BASH_BIN" "$GEN" >"$WORK/out1" 2>&1; then
  fail_test "expected failure when magick is missing"
fi
grep -q "ImageMagick 7" "$WORK/out1" || fail_test "expected magick error message, got: $(cat "$WORK/out1")"
echo "ok - missing magick fails cleanly"

# 2. missing master artwork -> clear failure
if MAILSLATE_REPO_ROOT="$FAKE_REPO" PATH="$STUB_BIN:$PATH" "$BASH_BIN" "$GEN" >"$WORK/out2" 2>&1; then
  fail_test "expected failure when master artwork is missing"
fi
grep -q "missing master artwork" "$WORK/out2" || fail_test "expected master-artwork error, got: $(cat "$WORK/out2")"
echo "ok - missing master artwork fails cleanly"

# 3. stub run without glyph -> tray skipped, verification fails on missing outputs
MASTER_DIR="$FAKE_REPO/assets/branding/icon/source"
mkdir -p "$MASTER_DIR"
printf 'stub' > "$MASTER_DIR/mailslate-icon-master.png"
if MAILSLATE_REPO_ROOT="$FAKE_REPO" PATH="$STUB_BIN:$PATH" "$BASH_BIN" "$GEN" >"$WORK/out3" 2>&1; then
  fail_test "expected verification failure with a no-op magick"
fi
grep -q "MISS" "$WORK/out3" || fail_test "expected MISS entries, got: $(cat "$WORK/out3")"
grep -q "skip tray icons" "$WORK/out3" || fail_test "expected tray skip note"
echo "ok - no-op magick fails verification with MISS entries"

# 4. glyph present -> tray outputs are part of verification
printf '<svg/>' > "$MASTER_DIR/mailslate-glyph-mono.svg"
if MAILSLATE_REPO_ROOT="$FAKE_REPO" PATH="$STUB_BIN:$PATH" "$BASH_BIN" "$GEN" >"$WORK/out4" 2>&1; then
  fail_test "expected verification failure with a no-op magick"
fi
grep -q "tray/tray.png" "$WORK/out4" || fail_test "expected tray outputs in verification, got: $(cat "$WORK/out4")"
echo "ok - tray outputs verified when the glyph exists"

echo "All generate_icons tests passed."
