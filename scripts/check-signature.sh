#!/bin/bash
# Checks the signature of a built NiceMailer.app (docs/app-bundle.md,
# "Signature"). scripts/make-bundle.sh runs it on every bundle it makes, so
# that a signature that notarization or the Keychain would refuse fails the
# build here, not on the Mac of a user.
#
# Checked on the app, on each of its helpers (the core, the CLI, the MCP
# server) and on the programs of Sparkle.framework:
# - the signature is valid, and the app's seal covers its helpers;
# - the hardened runtime is on (notarization refuses a program without it);
# - the signing identifier is the expected one: the Keychain items of the
#   core and, later, the rights each client gets from the core
#   (architecture.md, section 9) are tied to it;
# - all are signed by the same team: the Keychain partition of the
#   passwords is "teamid:<team>" (docs/configuration.md, "Passwords"), and
#   Sparkle's programs (docs/updates.md) are re-signed by it, as
#   notarization requires;
# - the core's designated requirement names its identifier and its team
#   only, so that every build of the team reads the passwords of the others
#   (scripts/sign-core.sh);
# - the app declares the address book entitlement (NiceMailer.entitlements),
#   without which the hardened runtime refuses the Contacts.
# With --release, also:
# - the identity is a Developer ID Application (what Gatekeeper accepts
#   outside the App Store);
# - each signature carries a secure timestamp (notarization requires it);
# - no `get-task-allow` entitlement (debugging; notarization refuses it).
#
# Usage: scripts/check-signature.sh [--release] <NiceMailer.app>
set -euo pipefail

RELEASE=0
if [ "${1:-}" = "--release" ]; then RELEASE=1; shift; fi
APP=${1:?usage: check-signature.sh [--release] <NiceMailer.app>}

FAILED=0
fail() { echo "check-signature: $*" >&2; FAILED=1; }

# The seal of the app covers every file of the bundle: a resource changed
# after the signature makes this fail. A nested program (the helpers) is
# recorded by its signature requirement (identifier and team), not by its
# hash: a helper re-signed by the same identity still passes, which is why
# each helper is checked on its own below.
if ! codesign --verify --strict --deep "$APP" 2>/dev/null; then
  fail "invalid signature: $(codesign --verify --strict --deep "$APP" 2>&1 | head -3)"
fi

TEAMS=""
# Each line: path inside the bundle, expected identifier.
while read -r REL IDENT; do
  BIN="$APP/$REL"
  if [ ! -f "$BIN" ]; then fail "$REL: missing"; continue; fi
  # `codesign -dvvv` writes the details on stderr, one `Key=value` per line.
  INFO=$(codesign -dvvv "$BIN" 2>&1) || { fail "$REL: not signed"; continue; }
  field() { printf '%s\n' "$INFO" | sed -n "s/^$1=//p" | head -1; }

  [ "$(field Identifier)" = "$IDENT" ] || fail "$REL: identifier $(field Identifier), expected $IDENT"
  # The flags line reads e.g. "CodeDirectory v=20500 size=… flags=0x10000(runtime) …".
  printf '%s\n' "$INFO" | grep -q '^CodeDirectory .*flags=.*runtime' || fail "$REL: no hardened runtime"
  TEAM=$(field TeamIdentifier)
  if [ -z "$TEAM" ] || [ "$TEAM" = "not set" ]; then
    fail "$REL: no team (ad hoc or self-signed signature)"
  fi
  TEAMS="$TEAMS $TEAM"
  lipo -archs "$BIN" 2>/dev/null | grep -qw arm64 || fail "$REL: no arm64 code"

  if [ "$RELEASE" = 1 ]; then
    # The first Authority line is the signing certificate itself.
    case "$(field Authority)" in
      "Developer ID Application: "*) ;;
      *) fail "$REL: signed by \"$(field Authority)\", not a Developer ID Application" ;;
    esac
    # A secure timestamp shows as "Timestamp=…"; without one, codesign
    # prints "Signed Time=…" (the time of the Mac, which proves nothing).
    printf '%s\n' "$INFO" | grep -q '^Timestamp=' || fail "$REL: no secure timestamp"
    if codesign -d --entitlements - --xml "$BIN" 2>/dev/null | grep -q 'get-task-allow'; then
      fail "$REL: has the get-task-allow entitlement"
    fi
  fi
done <<'LIST'
Contents/MacOS/NiceMailer eu.nicemailer.NiceMailer
Contents/Helpers/nicemailer-core eu.nicemailer.core
Contents/Helpers/nicemailer eu.nicemailer.cli
Contents/Helpers/nicemailer-mcp eu.nicemailer.mcp
Contents/Frameworks/Sparkle.framework/Versions/B/Sparkle org.sparkle-project.Sparkle
Contents/Frameworks/Sparkle.framework/Versions/B/Autoupdate Autoupdate
Contents/Frameworks/Sparkle.framework/Versions/B/Updater.app/Contents/MacOS/Updater org.sparkle-project.Sparkle.Updater
Contents/Frameworks/Sparkle.framework/Versions/B/XPCServices/Installer.xpc/Contents/MacOS/Installer org.sparkle-project.InstallerLauncher
Contents/Frameworks/Sparkle.framework/Versions/B/XPCServices/Downloader.xpc/Contents/MacOS/Downloader org.sparkle-project.DownloaderService
LIST

# One team for all: `sort -u` leaves one line when they agree.
if [ "$(printf '%s\n' $TEAMS | sort -u | wc -l | tr -d ' ')" != 1 ]; then
  fail "signed by several teams:$TEAMS"
fi

# The core's designated requirement names its team, not its kind of
# certificate (scripts/sign-core.sh): otherwise the passwords saved by a
# development build would be refused to a release, or the reverse.
CORE_TEAM=$(printf '%s\n' $TEAMS | head -1)
CORE_REQ=$(codesign -d -r- "$APP/Contents/Helpers/nicemailer-core" 2>&1 | sed -n 's/^designated => //p')
WANT_REQ="identifier \"eu.nicemailer.core\" and anchor apple generic and certificate leaf[subject.OU] = \"$CORE_TEAM\""
# codesign prints the team without quotes when it needs none: compare both.
if [ "$CORE_REQ" != "$WANT_REQ" ] && [ "$CORE_REQ" != "${WANT_REQ//\"$CORE_TEAM\"/$CORE_TEAM}" ]; then
  fail "Contents/Helpers/nicemailer-core: designated requirement \"$CORE_REQ\", expected \"$WANT_REQ\" (scripts/sign-core.sh)"
fi

if ! codesign -d --entitlements - --xml "$APP" 2>/dev/null | grep -q 'com.apple.security.personal-information.addressbook'; then
  fail "the app lacks the address book entitlement (apps/NiceMailer/NiceMailer.entitlements)"
fi

if [ "$FAILED" = 1 ]; then exit 1; fi
echo "check-signature: $APP: signed by team $(printf '%s\n' $TEAMS | head -1)$([ "$RELEASE" = 1 ] && echo ', Developer ID, timestamped'), hardened runtime."
