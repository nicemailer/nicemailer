#!/bin/bash
# Prints the SHA-1 hash of the identity that signs NiceMailer: the core,
# the CLI, the MCP server and the app (docs/app-bundle.md, "Signature").
# scripts/make-bundle.sh and scripts/sign-core.sh both ask it, so that
# `make core`, `make bundle` and `make release` never choose differently.
#
# Usage: scripts/signing-identity.sh [--release]
#
# --release: the first valid "Developer ID Application" identity, the one
# Gatekeeper accepts outside the App Store. Fails when there is none.
#
# Without --release (development): the "Apple Development" identity of the
# same team as the Developer ID. Why the team matters: launchd ties the
# registered agent of the core to the team of its signature (a "launch
# constraint", set when the app registers the agent), and the Keychain ties
# saved passwords to it (the partition "teamid:<team>", docs/configuration.md,
# "Passwords"). With one team for both, installing a release over a
# development build, or the reverse, keeps the core startable and the
# passwords readable. With two teams, the core of the new build is refused
# by launchd until the agent is registered again, and every password must be
# entered once more.
# When no Apple Development identity shares the team of the Developer ID,
# the first Apple Development identity, with a warning on stderr; when there
# is no Developer ID, the first Apple Development identity, silently (a Mac
# that only builds for itself). Nothing printed, and exit 0, when there is
# no Apple Development identity at all: each caller has its own fallback
# (ad hoc for the bundle, "NiceMailer Dev" for `make core`).
#
# The hash rather than the name: two certificates may share a name after a
# renewal, and codesign takes either.
#
# Testing: NICEMAILER_IDENTITIES_FILE names a file of lines
# "<hash> <team> <common name>" read instead of the keychain
# (scripts/tests/test_signing_identity.py).
set -euo pipefail

RELEASE=0
if [ "${1:-}" = "--release" ]; then RELEASE=1; fi
KEYCHAIN=${KEYCHAIN:-$HOME/Library/Keychains/login.keychain-db}

# The valid identities as "<hash> <team> <common name>", one per line.
# `security find-identity` gives the hash and the common name; the team is
# the OU (organizational unit) of the certificate's subject, which only the
# certificate itself carries. The name in parentheses at the end of an
# "Apple Development" common name is the person's ID, not the team: it
# cannot be used instead.
identities() {
  if [ -n "${NICEMAILER_IDENTITIES_FILE:-}" ]; then
    cat "$NICEMAILER_IDENTITIES_FILE"
    return
  fi
  local list hash name pem team
  # Lines look like:  1) <SHA-1> "Apple Development: me@example.com (ABCDE12345)"
  list=$(security find-identity -v -p codesigning "$KEYCHAIN" 2>/dev/null \
    | sed -n 's/^ *[0-9]*) \([0-9A-F]\{40\}\) "\(.*\)"$/\1 \2/p')
  [ -n "$list" ] || return 0
  # Every certificate of the keychain once, each preceded by its
  # "SHA-1 hash: <hash>" line (-Z), in PEM (-p).
  pem=$(security find-certificate -a -Z -p "$KEYCHAIN" 2>/dev/null || true)
  while read -r hash name; do
    team=$(printf '%s\n' "$pem" \
      | awk -v h="$hash" '/^SHA-1 hash:/ { keep = ($3 == h); next } keep' \
      | openssl x509 -noout -subject -nameopt multiline 2>/dev/null \
      | sed -n 's/^ *organizationalUnitName *= *//p' | head -1)
    echo "$hash ${team:--} $name"
  done <<< "$list"
}

ALL=$(identities)
# The first identity of a kind ($1: the start of the common name), and
# optionally of a team ($2).
first() {
  printf '%s\n' "$ALL" | awk -v kind="$1" -v team="${2:-}" '
    index($0, kind) == 0 { next }
    { split($0, f, " "); rest = substr($0, length(f[1]) + length(f[2]) + 3) }
    index(rest, kind) == 1 && (team == "" || f[2] == team) { print f[1], f[2]; exit }'
}

DEVID=$(first "Developer ID Application: ")
if [ "$RELEASE" = 1 ]; then
  if [ -z "$DEVID" ]; then
    echo "signing-identity: no Developer ID Application identity in the keychain (docs/app-bundle.md, \"Signature\")." >&2
    exit 1
  fi
  echo "${DEVID%% *}"
  exit 0
fi

TEAM=${DEVID#* }
if [ -n "$DEVID" ]; then
  SAME=$(first "Apple Development: " "$TEAM")
  if [ -n "$SAME" ]; then echo "${SAME%% *}"; exit 0; fi
fi
ANY=$(first "Apple Development: ")
[ -n "$ANY" ] || exit 0
if [ -n "$DEVID" ]; then
  echo "signing-identity: warning: no Apple Development identity of team $TEAM, the team of the Developer ID;" >&2
  echo "signing with team ${ANY#* }. A release installed over this build will need its agent registered again" >&2
  echo "and its passwords entered again (docs/configuration.md, \"Passwords\")." >&2
fi
echo "${ANY%% *}"
