#!/usr/bin/env bash
# Offline tests for tool/fetch_greenmail.sh using a file:// fixture and
# temporary directories. Run: bash tool/tests/fetch_greenmail.test.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FETCH="$SCRIPT_DIR/../fetch_greenmail.sh"
VERSION="9.9.9"
JAR_NAME="greenmail-standalone-${VERSION}.jar"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FIXTURES="$WORK/remote"
TOOLS="$WORK/tools"
mkdir -p "$FIXTURES" "$TOOLS"

fail_test() { echo "FAIL: $*" >&2; exit 1; }

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

printf 'greenmail-fixture-v1' > "$FIXTURES/$JAR_NAME"
DIGEST="$(sha256_of "$FIXTURES/$JAR_NAME")"

fetch() { # run the fetcher fully sandboxed
  GREENMAIL_VERSION="$VERSION" \
    GREENMAIL_SHA256="$DIGEST" \
    GREENMAIL_BASE_URL="file://$FIXTURES" \
    GREENMAIL_TOOLS_DIR="$TOOLS" \
    bash "$FETCH" "$@"
}

# 1. fresh download succeeds and verifies
fetch >/dev/null
[ -f "$TOOLS/$JAR_NAME" ] || fail_test "jar was not downloaded"
cmp -s "$FIXTURES/$JAR_NAME" "$TOOLS/$JAR_NAME" || fail_test "downloaded jar differs from fixture"
echo "ok - fresh download"

# 2. second run reuses the verified jar
out="$(fetch)"
echo "$out" | grep -q "already present" || fail_test "expected reuse message, got: $out"
echo "ok - reuse of verified jar"

# 3. corrupt jar is re-downloaded and restored
printf 'corrupt' > "$TOOLS/$JAR_NAME"
fetch >/dev/null
cmp -s "$FIXTURES/$JAR_NAME" "$TOOLS/$JAR_NAME" || fail_test "corrupt jar was not restored"
echo "ok - corrupt jar redownloaded"

# 4. checksum mismatch refuses to install and cleans up
printf 'tampered' > "$FIXTURES/$JAR_NAME"
rm -f "$TOOLS/$JAR_NAME"
if fetch >/dev/null 2>&1; then
  fail_test "tampered download should fail"
fi
[ ! -f "$TOOLS/$JAR_NAME" ] || fail_test "unverified jar was installed"
[ ! -f "$TOOLS/$JAR_NAME.tmp" ] || fail_test "tmp file left behind after failure"
echo "ok - checksum mismatch refuses install and cleans up"

# 5. --force re-downloads even when the existing jar verifies
printf 'greenmail-fixture-v1' > "$FIXTURES/$JAR_NAME"
fetch >/dev/null
fetch --force >/dev/null
cmp -s "$FIXTURES/$JAR_NAME" "$TOOLS/$JAR_NAME" || fail_test "--force did not re-download"
echo "ok - --force redownload"

echo "All fetch_greenmail tests passed."
