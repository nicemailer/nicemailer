#!/bin/bash
# Signs a build of the core, so that the Keychain sees every build as the
# same application (docs/configuration.md, "Passwords").
#
# Which identity, in order:
# 1. $SIGN_IDENTITY, when set (a name or a SHA-1 hash, as codesign takes);
# 2. the "Apple Development" identity chosen by scripts/signing-identity.sh
#    (free with an Apple ID, Xcode > Settings > Accounts > Manage
#    Certificates): of the team of the Developer ID when there is one, so
#    that development and release builds share the Keychain partition;
# 3. the self-signed "NiceMailer Dev" (scripts/dev-cert.sh), with a warning.
#
# Why Apple Development first: an item of the login keychain has, besides
# its access list, a *partition*, set by macOS from the signature of the
# program that created it. A signature with an Apple Team ID gives the
# partition "teamid:<team>", the same for every build. A signature without
# one, such as "NiceMailer Dev", gives "cdhash:<hash of this binary>": the
# next build is refused, and the password must be saved again after each
# rebuild. (A keychain made by `security create-keychain` gets no
# partition, which is why the tests on throwaway keychains did not show it.)
#
# Hardened runtime (`--options runtime`), always: notarization requires it
# for a release, and a development build signed the same way meets the same
# restrictions every day instead of on the day of a release
# (docs/app-bundle.md, "Signature"). The core needs no entitlement under it.
#
# Secure timestamp: none by default (offline, and faster); a release sets
# CODESIGN_TIMESTAMP=--timestamp, since notarization requires one
# (scripts/make-bundle.sh --release).
#
# Usage: scripts/sign-core.sh <binary>
set -euo pipefail
BIN=$1
KEYCHAIN=${KEYCHAIN:-$HOME/Library/Keychains/login.keychain-db}

IDENTITY=${SIGN_IDENTITY:-}
if [ -z "$IDENTITY" ]; then
  IDENTITY=$(KEYCHAIN="$KEYCHAIN" "$(dirname "$0")/signing-identity.sh")
fi
if [ -z "$IDENTITY" ]; then
  if ! security find-identity -p codesigning "$KEYCHAIN" 2>/dev/null | grep -q '"NiceMailer Dev"'; then
    echo "sign-core: no signing identity. Create an Apple Development certificate in Xcode" >&2
    echo "(Settings > Accounts > Manage Certificates), see docs/configuration.md, \"Passwords\"." >&2
    exit 1
  fi
  echo "sign-core: warning: no Apple Development identity, signing with \"NiceMailer Dev\":" >&2
  echo "the passwords saved by this build will not be readable by the next one." >&2
  IDENTITY="NiceMailer Dev"
fi
codesign --force --sign "$IDENTITY" --keychain "$KEYCHAIN" --identifier eu.nicemailer.core \
  --options runtime "${CODESIGN_TIMESTAMP:---timestamp=none}" "$BIN"
# The Team ID is what the Keychain partition is made of: say which.
codesign -dvv "$BIN" 2>&1 | grep -E '^(Authority|TeamIdentifier)=' | head -2 | sed 's/^/sign-core: /' >&2
