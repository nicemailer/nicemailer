#!/bin/bash
# Builds NiceMailer.app and installs it (`make bundle`, docs/app-bundle.md).
#
# The app carries its core and the CLI (Contents/Helpers), and the launch
# agent that declares the core (Contents/Library/LaunchAgents): at its
# launch the app registers the agent, and launchd starts the core, keeps it
# running when the app quits, and starts it at each session. The Info.plist
# declares `nicemailer://`, so that a link nicemailer://m/<id> clicked in a
# note opens the mail (docs/deep-links.md).
#
# Usage: scripts/make-bundle.sh [--open] [install dir]   (default: /Applications)
#        scripts/make-bundle.sh --release <output dir>
# --open (`make open`): once built, quits the installed app if it runs,
# installs the new one and launches it.
# --release (`make release`, through scripts/make-release.sh): builds the
# app to distribute (docs/app-bundle.md, "Signature"): signed with the
# Developer ID Application identity and a secure timestamp, from a clean
# working tree, and written to <output dir> without installing anything,
# quitting the running app or touching its core.
# NICEMAILER_DATA_DIR, if set, becomes the mailbox chosen (the file
# `data-dir` of ~/Library/Application Support/NiceMailer), as "Change
# Mailbox…" in the app would.
# NICEMAILER_FAKE_NETWORK, if set, is given to the core of the agent:
# sending is pretended (docs/sending.md).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OPEN=0
RELEASE=0
if [ "${1:-}" = "--open" ]; then OPEN=1; shift; fi
if [ "${1:-}" = "--release" ]; then RELEASE=1; shift; fi
DEST="${1:-/Applications}"
if [ "$RELEASE" = 1 ]; then
  if [ -z "${1:-}" ]; then echo "usage: make-bundle.sh --release <output dir>" >&2; exit 1; fi
  if [ -e "$DEST/NiceMailer.app" ]; then
    echo "$DEST/NiceMailer.app already exists: give a new output directory." >&2; exit 1
  fi
  # What is shipped must be what is committed: a release built from
  # uncommitted changes could never be rebuilt nor traced to a commit.
  if [ -n "$(git -C "$ROOT" status --porcelain)" ]; then
    echo "The working tree has uncommitted changes: commit them before a release." >&2; exit 1
  fi
  # A development switch must never reach users: their core would only
  # pretend to send (docs/sending.md, "Pretending to send").
  if [ -n "${NICEMAILER_FAKE_NETWORK:-}" ]; then
    echo "NICEMAILER_FAKE_NETWORK is set: unset it before a release." >&2; exit 1
  fi
fi

# The identity that signs the core, the CLI and the app (docs/app-bundle.md,
# "Signature"). Always the SHA-1 hash rather than the name: two
# certificates may share a name after a renewal.
if [ "$RELEASE" = 1 ]; then
  # Developer ID Application: the one certificate Gatekeeper accepts for an
  # app distributed outside the App Store, and the one notarization takes.
  IDENTITY="${SIGN_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null | awk '/"Developer ID Application: / { print $2; exit }')}"
  if [ -z "$IDENTITY" ]; then
    echo "No Developer ID Application identity in the keychain (docs/app-bundle.md, \"Signature\")." >&2; exit 1
  fi
  # Notarization requires a secure timestamp from Apple's server on each
  # signature (online; a development build does without).
  TIMESTAMP=--timestamp
else
  IDENTITY="${SIGN_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null | awk '/"Apple Development: / { print $2; exit }')}"
  TIMESTAMP=--timestamp=none
fi
XCODE="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
PKG="$ROOT/apps/NiceMailer"

echo "Building the app (release)…"
(cd "$PKG" && DEVELOPER_DIR="$XCODE" swift build -c release --product NiceMailer)
BIN_DIR="$(cd "$PKG" && DEVELOPER_DIR="$XCODE" swift build -c release --show-bin-path)"

echo "Building the core and the CLI (release)…"
cargo build --release --quiet --manifest-path "$ROOT/Cargo.toml" --bin nicemailer-core --bin nicemailer

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
APP="$STAGE/NiceMailer.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Helpers" "$APP/Contents/Library/LaunchAgents"
cp "$BIN_DIR/NiceMailer" "$APP/Contents/MacOS/NiceMailer"
cp "$ROOT/target/release/nicemailer-core" "$ROOT/target/release/nicemailer" "$APP/Contents/Helpers/"

# The agent of the core (architecture.md, section 6): started at once when
# the app registers it, at each session, and again after a crash; a stop
# with code 0 (Stop the Core, `make core-stop`) is not restarted.
AGENT_ENV="<key>NICEMAILER_LOG_IN_DATA_DIR</key><string>1</string>"
# (In a release, NICEMAILER_FAKE_NETWORK was refused above.)
if [ -n "${NICEMAILER_FAKE_NETWORK:-}" ]; then
  AGENT_ENV="$AGENT_ENV<key>NICEMAILER_FAKE_NETWORK</key><string>1</string>"
fi
cat > "$APP/Contents/Library/LaunchAgents/eu.nicemailer.core.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>eu.nicemailer.core</string>
  <key>BundleProgram</key><string>Contents/Helpers/nicemailer-core</string>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>AssociatedBundleIdentifiers</key><array><string>eu.nicemailer.NiceMailer</string></array>
  <key>EnvironmentVariables</key><dict>${AGENT_ENV}</dict>
  <key>ProcessType</key><string>Background</string>
</dict>
</plist>
PLIST
plutil -lint "$APP/Contents/Library/LaunchAgents/eu.nicemailer.core.plist" >/dev/null
# Fonts and translations (AppResources.bundle looks here first).
cp -R "$BIN_DIR/NiceMailer_NiceMailer.bundle" "$APP/Contents/Resources/"
# The icon (design/icon/README.md; scripts/make-icon.sh makes it).
cp "$ROOT/design/icon/app/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"

# Versions (docs/app-bundle.md, "Versions"):
# - CFBundleShortVersionString, the version users see, is the one of the
#   Rust workspace (`[workspace.package]` of Cargo.toml), so that the app,
#   the core and `nicemailer --version` always say the same;
# - CFBundleVersion, which macOS and later Sparkle compare to know which
#   build is newer, must be a number that only grows: the count of commits
#   of the history, which grows with every commit of master;
# - NMGitDescribe, our own key, names the exact commit (and "-dirty" for a
#   development build with uncommitted changes), to trace a build.
SHORT_VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml" | head -1)"
BUILD_NUMBER="$(git -C "$ROOT" rev-list --count HEAD 2>/dev/null || echo 0)"
GIT_DESCRIBE="$(git -C "$ROOT" describe --tags --always --dirty 2>/dev/null || echo unknown)"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>eu.nicemailer.NiceMailer</string>
  <key>CFBundleName</key><string>NiceMailer</string>
  <key>CFBundleDisplayName</key><string>NiceMailer</string>
  <key>CFBundleExecutable</key><string>NiceMailer</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${SHORT_VERSION}</string>
  <key>CFBundleVersion</key><string>${BUILD_NUMBER}</string>
  <key>NMGitDescribe</key><string>${GIT_DESCRIBE}</string>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleLocalizations</key><array><string>en</string><string>fr</string></array>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSContactsUsageDescription</key><string>NiceMailer shows the phone numbers and the card of the senders of a conversation among the links of the mail (l). Nothing is copied or sent.</string>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>eu.nicemailer.message</string>
      <key>CFBundleURLSchemes</key><array><string>nicemailer</string></array>
    </dict>
  </array>
</dict>
</plist>
PLIST
plutil -lint "$APP/Contents/Info.plist" >/dev/null
# The reason macOS shows when it asks for Contacts, in French (the English
# one is in Info.plist above).
mkdir -p "$APP/Contents/Resources/fr.lproj"
cat > "$APP/Contents/Resources/fr.lproj/InfoPlist.strings" <<'STRINGS'
"NSContactsUsageDescription" = "NiceMailer affiche les numéros de téléphone et la fiche des expéditeurs d'une conversation parmi les liens du mail (l). Rien n'est copié ni envoyé.";
STRINGS
plutil -lint "$APP/Contents/Resources/fr.lproj/InfoPlist.strings" >/dev/null

# Signed from the inside out, as Apple requires: the helpers first, then
# the app, whose seal records every file it contains and, for each helper,
# the requirement its signature must meet (identifier and team). All with
# the hardened runtime,
# which notarization requires; the app with its entitlements.
# In development, the Apple Development identity, as `make core` signs its
# core (scripts/sign-core.sh): the Keychain sees the core of the app and the
# one of `make core` as the same program, and the passwords saved by either
# are readable by both. Without that identity, an ad-hoc signature: it runs
# on this Mac, but the core cannot read the passwords saved by another
# build, and check-signature.sh below says so.
SIGN_IDENTITY="$IDENTITY" CODESIGN_TIMESTAMP="$TIMESTAMP" "$ROOT/scripts/sign-core.sh" "$APP/Contents/Helpers/nicemailer-core"
codesign --force --sign "${IDENTITY:--}" --identifier org.nicemailer.cli \
  --options runtime "$TIMESTAMP" "$APP/Contents/Helpers/nicemailer" >/dev/null
codesign --force --sign "${IDENTITY:--}" --identifier eu.nicemailer.NiceMailer \
  --options runtime "$TIMESTAMP" --entitlements "$PKG/NiceMailer.entitlements" "$APP" >/dev/null

if [ "$RELEASE" = 1 ]; then
  "$ROOT/scripts/check-signature.sh" --release "$APP"
  mkdir -p "$DEST"
  # ditto rather than cp: it keeps every attribute and the signatures as
  # they are, as Apple advises for signed bundles.
  ditto "$APP" "$DEST/NiceMailer.app"
  echo "Built $DEST/NiceMailer.app, version $SHORT_VERSION (build $BUILD_NUMBER, $GIT_DESCRIBE)."
  exit 0
fi
# A development build without a team (ad hoc) still installs, with a
# warning: it runs on this Mac, which is all it is for.
"$ROOT/scripts/check-signature.sh" "$APP" || echo "Warning: this build would not pass a release (see above)." >&2

# Built: only now is the running app quit, so that it stays usable during
# the build. SIGTERM rather than an Apple Event "quit", which would make
# macOS ask once for the right to control the app; the app keeps nothing
# unsaved (the core holds the data). Only the installed app is stopped,
# not one started by `make app`.
INSTALLED="$DEST/NiceMailer.app/Contents/MacOS/NiceMailer"
if [ "$OPEN" = 1 ] && pgrep -f -x "$INSTALLED" >/dev/null; then
  echo "Quitting the running NiceMailer.app…"
  pkill -TERM -f -x "$INSTALLED" || true
  for _ in $(seq 50); do
    pgrep -f -x "$INSTALLED" >/dev/null || break
    sleep 0.1
  done
  if pgrep -f -x "$INSTALLED" >/dev/null; then
    echo "NiceMailer.app does not quit: quit it (⌘Q) and run again." >&2
    exit 1
  fi
fi

mkdir -p "$DEST"
rm -rf "$DEST/NiceMailer.app"
cp -R "$APP" "$DEST/"
# An earlier build in ~/Applications (the default before the app carried
# its core): removed, so that a single NiceMailer.app answers links and
# runs the agent.
OLD="$HOME/Applications/NiceMailer.app"
if [ "$DEST" = "/Applications" ] && [ -d "$OLD" ]; then
  /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -u "$OLD" 2>/dev/null || true
  rm -rf "$OLD"
  echo "Removed the earlier build $OLD."
fi
# The mailbox given at build time, chosen as "Change Mailbox…" would.
if [ -n "${NICEMAILER_DATA_DIR:-}" ]; then
  mkdir -p "$HOME/Library/Application Support/NiceMailer"
  printf '%s\n' "$NICEMAILER_DATA_DIR" > "$HOME/Library/Application Support/NiceMailer/data-dir"
fi
# The core of the agent, if it runs, is the previous build: started again
# from the new one (and on the mailbox chosen).
launchctl kickstart -k "gui/$(id -u)/eu.nicemailer.core" 2>/dev/null || true
# Tell Launch Services about the app now, so that nicemailer:// links
# open it without launching it by hand first.
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$DEST/NiceMailer.app"
echo "Installed $DEST/NiceMailer.app (nicemailer:// links open it)."
echo "Mailbox: $(cat "$HOME/Library/Application Support/NiceMailer/data-dir" 2>/dev/null || echo "$HOME/Library/Application Support/NiceMailer")"
echo "Core log: <mailbox>/core.log; stop the core: make core-stop (or the menu Stop the Core)." 
if [ "$OPEN" = 1 ]; then
  open "$DEST/NiceMailer.app"
fi
