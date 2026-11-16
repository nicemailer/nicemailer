#!/bin/bash
# Builds NiceMailer.app and installs it (`make bundle`, docs/app-bundle.md).
#
# The app carries its core, the CLI and the MCP server for AI agents
# (Contents/Helpers, docs/mcp.md), and the launch
# agent that declares the core (Contents/Library/LaunchAgents): at its
# launch the app registers the agent, and launchd starts the core, keeps it
# running when the app quits, and starts it at each session. The Info.plist
# declares `nicemailer://`, so that a link nicemailer://m/<id> clicked in a
# note opens the mail (docs/deep-links.md).
#
# Usage: scripts/make-bundle.sh [--open] [install dir]   (default: /Applications)
# --open (`make open`): once built, quits the installed app if it runs,
# installs the new one and launches it.
# NICEMAILER_DATA_DIR, if set, becomes the mailbox chosen (the file
# `data-dir` of ~/Library/Application Support/NiceMailer), as "Change
# Mailbox…" in the app would.
# NICEMAILER_FAKE_NETWORK, if set, is given to the core of the agent:
# sending is pretended (docs/sending.md).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OPEN=0
if [ "${1:-}" = "--open" ]; then OPEN=1; shift; fi
DEST="${1:-/Applications}"
XCODE="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
PKG="$ROOT/apps/NiceMailer"

echo "Building the app (release)…"
(cd "$PKG" && DEVELOPER_DIR="$XCODE" swift build -c release --product NiceMailer)
BIN_DIR="$(cd "$PKG" && DEVELOPER_DIR="$XCODE" swift build -c release --show-bin-path)"

echo "Building the core, the CLI and the MCP server (release)…"
cargo build --release --quiet --manifest-path "$ROOT/Cargo.toml" --bin nicemailer-core --bin nicemailer --bin nicemailer-mcp

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
APP="$STAGE/NiceMailer.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Helpers" "$APP/Contents/Library/LaunchAgents"
cp "$BIN_DIR/NiceMailer" "$APP/Contents/MacOS/NiceMailer"
cp "$ROOT/target/release/nicemailer-core" "$ROOT/target/release/nicemailer" "$ROOT/target/release/nicemailer-mcp" "$APP/Contents/Helpers/"

# The agent of the core (architecture.md, section 6): started at once when
# the app registers it, at each session, and again after a crash; a stop
# with code 0 (Stop the Core, `make core-stop`) is not restarted.
AGENT_ENV="<key>NICEMAILER_LOG_IN_DATA_DIR</key><string>1</string>"
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

VERSION="$(git -C "$ROOT" describe --always --dirty 2>/dev/null || echo dev)"

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
  <key>CFBundleShortVersionString</key><string>0.1</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
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

# Signed with the Apple Development identity, as `make core` signs the
# core (scripts/sign-core.sh): the Keychain then sees the core of the app
# and the one of `make core` as the same program, and the passwords saved
# by either are readable by both. The helpers first, then the app around
# them. Without that identity, an ad-hoc signature: it runs on this Mac,
# but the core cannot read the passwords saved by another build.
"$ROOT/scripts/sign-core.sh" "$APP/Contents/Helpers/nicemailer-core"
IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null | awk '/"Apple Development: / { print $2; exit }')"
codesign --force --sign "${IDENTITY:--}" --identifier eu.nicemailer.cli "$APP/Contents/Helpers/nicemailer" >/dev/null
codesign --force --sign "${IDENTITY:--}" --identifier eu.nicemailer.mcp "$APP/Contents/Helpers/nicemailer-mcp" >/dev/null
codesign --force --sign "${IDENTITY:--}" "$APP" >/dev/null

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
