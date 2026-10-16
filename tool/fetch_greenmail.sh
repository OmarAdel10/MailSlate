#!/usr/bin/env bash
# Downloads the pinned GreenMail standalone JAR used by IMAP/SMTP integration
# tests. Policy: NO Docker anywhere (AGENTS.md §10) - GreenMail runs directly
# on the JVM.
#
# Usage:  tool/fetch_greenmail.sh [--force]
# Env:    GREENMAIL_VERSION  override the pinned version (default below)
#
# The JAR lands in .tools/ (gitignored). Integration tests are feature-gated:
#   cargo test -p mailslate-core --features greenmail
set -euo pipefail

GREENMAIL_VERSION="${GREENMAIL_VERSION:-2.1.13}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLS_DIR="$REPO_ROOT/.tools"
JAR="$TOOLS_DIR/greenmail-standalone-${GREENMAIL_VERSION}.jar"
BASE_URL="https://repo1.maven.org/maven2/com/icegreen/greenmail-standalone/${GREENMAIL_VERSION}"

if [ -f "$JAR" ] && [ "${1:-}" != "--force" ]; then
  echo "GreenMail ${GREENMAIL_VERSION} already present: $JAR"
  exit 0
fi

command -v java >/dev/null 2>&1 || echo "note: Java is not on PATH - tests using this JAR will be skipped" >&2

mkdir -p "$TOOLS_DIR"
echo "Downloading GreenMail ${GREENMAIL_VERSION}..."
curl -fL --retry 3 -o "$JAR" "${BASE_URL}/greenmail-standalone-${GREENMAIL_VERSION}.jar"

echo "Verifying checksum..."
EXPECTED="$(curl -fsSL "${BASE_URL}/greenmail-standalone-${GREENMAIL_VERSION}.jar.sha1" | tr -d '[:space:]')"
if command -v sha1sum >/dev/null 2>&1; then
  ACTUAL="$(sha1sum "$JAR" | cut -d' ' -f1)"
else
  ACTUAL="$(shasum -a 1 "$JAR" | cut -d' ' -f1)"
fi
if [ "$EXPECTED" != "$ACTUAL" ]; then
  echo "checksum mismatch for GreenMail JAR (expected ${EXPECTED}, got ${ACTUAL})" >&2
  exit 1
fi

echo "OK: $JAR"
