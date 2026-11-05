#!/bin/bash
# Signs a build of the core with the development certificate, so that the
# Keychain sees every build as the same application (scripts/dev-cert.sh).
#
# Usage: scripts/sign-core.sh <binary>
set -euo pipefail
BIN=$1
KEYCHAIN=${KEYCHAIN:-$HOME/Library/Keychains/login.keychain-db}
if ! security find-identity -p codesigning "$KEYCHAIN" 2>/dev/null | grep -q '"NiceMailer Dev"'; then
  echo "sign-core: no \"NiceMailer Dev\" certificate. Run once: make dev-cert" >&2
  exit 1
fi
codesign --force --sign "NiceMailer Dev" --keychain "$KEYCHAIN" --identifier org.nicemailer.core "$BIN"
