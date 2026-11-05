#!/bin/bash
# Builds NiceMailer.app and installs it (`make bundle`, docs/deep-links.md).
#
# `swift run` starts a bare executable: macOS gives it no URL, since only an
# app bundle can declare a URL scheme. This script wraps the release build
# in NiceMailer.app, whose Info.plist declares `nicemailer://`, so that a
# link nicemailer://m/<id> clicked in a note opens the mail.
#
# Usage: scripts/make-bundle.sh [--open] [install dir]   (default: ~/Applications)
# --open (`make open`): once built, quits the installed app if it runs,
# installs the new one and launches it.
# NICEMAILER_DATA_DIR, if set, is written into the bundle (LSEnvironment):
# an app opened by Finder or by a link gets no variable from a shell.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OPEN=0
if [ "${1:-}" = "--open" ]; then OPEN=1; shift; fi
DEST="${1:-$HOME/Applications}"
XCODE="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
PKG="$ROOT/apps/NiceMailer"

echo "Building the app (release)…"
(cd "$PKG" && DEVELOPER_DIR="$XCODE" swift build -c release --product NiceMailer)
BIN_DIR="$(cd "$PKG" && DEVELOPER_DIR="$XCODE" swift build -c release --show-bin-path)"

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
APP="$STAGE/NiceMailer.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN_DIR/NiceMailer" "$APP/Contents/MacOS/NiceMailer"
# Fonts and translations (AppResources.bundle looks here first).
cp -R "$BIN_DIR/NiceMailer_NiceMailer.bundle" "$APP/Contents/Resources/"

VERSION="$(git -C "$ROOT" describe --always --dirty 2>/dev/null || echo dev)"
ENVIRONMENT=""
if [ -n "${NICEMAILER_DATA_DIR:-}" ]; then
  ENVIRONMENT="<key>LSEnvironment</key><dict><key>NICEMAILER_DATA_DIR</key><string>${NICEMAILER_DATA_DIR}</string></dict>"
fi

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>eu.nicemailer.NiceMailer</string>
  <key>CFBundleName</key><string>NiceMailer</string>
  <key>CFBundleDisplayName</key><string>NiceMailer</string>
  <key>CFBundleExecutable</key><string>NiceMailer</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleLocalizations</key><array><string>en</string><string>fr</string></array>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>eu.nicemailer.message</string>
      <key>CFBundleURLSchemes</key><array><string>nicemailer</string></array>
    </dict>
  </array>
  ${ENVIRONMENT}
</dict>
</plist>
PLIST
plutil -lint "$APP/Contents/Info.plist" >/dev/null

# An ad-hoc signature: enough to run on this Mac (Apple Silicon requires
# signed code), not to distribute.
codesign --force --sign - "$APP" >/dev/null

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
# Tell Launch Services about the app now, so that nicemailer:// links
# open it without launching it by hand first.
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$DEST/NiceMailer.app"
echo "Installed $DEST/NiceMailer.app (nicemailer:// links open it)."
if [ -n "${NICEMAILER_DATA_DIR:-}" ]; then
  echo "Data directory: $NICEMAILER_DATA_DIR"
fi
if [ "$OPEN" = 1 ]; then
  open "$DEST/NiceMailer.app"
fi
