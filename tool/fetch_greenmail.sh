#!/usr/bin/env bash
# Downloads the pinned GreenMail standalone JAR used by IMAP/SMTP integration
# tests. Policy: NO Docker anywhere (AGENTS.md §10) - GreenMail runs directly
# on the JVM.
#
# Usage:  tool/fetch_greenmail.sh [--force]
# Env:    GREENMAIL_VERSION    override the pinned version (default below)
#         GREENMAIL_BASE_URL   override the download base (test hook; file:// works)
#         GREENMAIL_TOOLS_DIR  override the output directory (test hook)
#         GREENMAIL_SHA256     override the pinned digest (test hook)
#
# The JAR lands in .tools/ (gitignored). The download is verified against the
# pinned SHA-256 before it is installed, and an existing JAR is re-verified on
# every reuse (a corrupted file is re-downloaded, not trusted). Integration
# tests are feature-gated:
#   cargo test -p mailslate-core --features greenmail
set -euo pipefail

GREENMAIL_VERSION="${GREENMAIL_VERSION:-2.1.13}"
GREENMAIL_SHA256="${GREENMAIL_SHA256:-21b361e46e8ffe83afe7915a2b01231c4a163c8d0fa5de607e32ed19732d908d}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLS_DIR="${GREENMAIL_TOOLS_DIR:-$REPO_ROOT/.tools}"
BASE_URL="${GREENMAIL_BASE_URL:-https://repo1.maven.org/maven2/com/icegreen/greenmail-standalone/${GREENMAIL_VERSION}}"
JAR_NAME="greenmail-standalone-${GREENMAIL_VERSION}.jar"
JAR="$TOOLS_DIR/$JAR_NAME"

fail() { echo "error: $*" >&2; exit 1; }

file_sha256() { # file
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    fail "need sha256sum or shasum to verify integrity"
  fi
}

verify_jar() { # file
  [ "$(file_sha256 "$1")" = "$GREENMAIL_SHA256" ]
}

curl_args=(--fail --location --silent --show-error --retry 3)
if [[ "$BASE_URL" == https://* ]]; then
  curl_args+=(--proto '=https' --proto-redir '=https')
fi

if [ -f "$JAR" ] && [ "${1:-}" != "--force" ]; then
  if verify_jar "$JAR"; then
    echo "GreenMail ${GREENMAIL_VERSION} already present and verified: $JAR"
    exit 0
  fi
  echo "note: existing JAR failed verification - re-downloading" >&2
fi

command -v java >/dev/null 2>&1 || echo "note: Java is not on PATH - tests using this JAR will be skipped" >&2

mkdir -p "$TOOLS_DIR"
TMP="$JAR.tmp"
trap 'rm -f "$TMP"' EXIT

echo "Downloading GreenMail ${GREENMAIL_VERSION}..."
curl "${curl_args[@]}" -o "$TMP" "${BASE_URL}/${JAR_NAME}"

echo "Verifying checksum..."
verify_jar "$TMP" || fail "checksum mismatch for GreenMail JAR - refusing to install $JAR"

mv "$TMP" "$JAR"
trap - EXIT
echo "OK: $JAR"
