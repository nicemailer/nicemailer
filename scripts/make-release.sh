#!/bin/bash
# Builds the disk image of a release of NiceMailer, signed and notarized
# (`make release`, docs/app-bundle.md, "Releases"; plan in
# specifications/plan-release.md).
#
# 1. scripts/make-bundle.sh --release: the app, signed with the Developer ID
#    Application identity, hardened runtime, secure timestamps, checked by
#    scripts/check-signature.sh.
# 2. A disk image (DMG) holding the app and a link to /Applications, the
#    usual "drag to install" window, signed too.
# 3. Notarization: Apple scans the image and the app in it for malware and
#    checks their signatures; without it Gatekeeper refuses a downloaded
#    app. `notarytool --wait` sends it and waits for the verdict (minutes).
# 4. Stapling: the ticket of the notarization is attached to the image, so
#    that Gatekeeper accepts it even offline on the first launch.
# 5. Gatekeeper's own verdict on the result (`spctl`), as on a user's Mac.
#
# Nothing is published: tagging, pushing and the GitHub release are left to
# the person who runs it (the commands are printed at the end).
#
# Usage: scripts/make-release.sh [--no-notarize]
# --no-notarize: steps 1 and 2 only, to check the signature offline.
# NOTARY_PROFILE: the notarytool profile of the keychain, made once by
# `xcrun notarytool store-credentials` (default: nicemailer).
# SIGN_IDENTITY: the Developer ID identity, when several are in the keychain.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NOTARIZE=1
if [ "${1:-}" = "--no-notarize" ]; then NOTARIZE=0; shift; fi
PROFILE="${NOTARY_PROFILE:-nicemailer}"

VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml" | head -1)"
# A new directory for each run: a release never reuses the files of an
# earlier attempt (nor deletes anything).
OUT="$ROOT/dist/$VERSION-$(date +%Y%m%d-%H%M%S)"
DMG="$OUT/NiceMailer-$VERSION.dmg"
mkdir -p "$OUT"

# Checked before the long build: notarization would fail at the very end.
if [ "$NOTARIZE" = 1 ] && ! xcrun notarytool history --keychain-profile "$PROFILE" >/dev/null 2>&1; then
  echo "No notarytool profile \"$PROFILE\" in the keychain. Make it once with:" >&2
  echo "  xcrun notarytool store-credentials $PROFILE --apple-id <Apple ID> --team-id <team>" >&2
  exit 1
fi

"$ROOT/scripts/make-bundle.sh" --release "$OUT/app"
APP="$OUT/app/NiceMailer.app"
# The identity that signed the app signs the image (its first Authority).
IDENTITY="$(codesign -dvv "$APP" 2>&1 | sed -n 's/^Authority=//p' | head -1)"

echo "Making the disk image…"
IMAGE_SRC="$OUT/image"
mkdir -p "$IMAGE_SRC"
ditto "$APP" "$IMAGE_SRC/NiceMailer.app"
ln -s /Applications "$IMAGE_SRC/Applications"
hdiutil create -quiet -volname "NiceMailer $VERSION" -srcfolder "$IMAGE_SRC" -fs HFS+ -format UDZO -ov "$DMG"
codesign --force --sign "$IDENTITY" --timestamp "$DMG"

if [ "$NOTARIZE" = 0 ]; then
  echo "Not notarized (--no-notarize): $DMG"
  exit 0
fi

echo "Notarizing (a few minutes)…"
# The verdict, read from the JSON output rather than the exit code: an
# image refused ("Invalid") is a normal end of the command.
RESULT="$(xcrun notarytool submit "$DMG" --keychain-profile "$PROFILE" --wait --output-format json)"
STATUS="$(printf '%s' "$RESULT" | python3 -c 'import json, sys; print(json.load(sys.stdin).get("status", ""))')"
ID="$(printf '%s' "$RESULT" | python3 -c 'import json, sys; print(json.load(sys.stdin).get("id", ""))')"
if [ "$STATUS" != "Accepted" ]; then
  echo "Notarization: $STATUS. Apple's log, which names each problem:" >&2
  xcrun notarytool log "$ID" --keychain-profile "$PROFILE" >&2 || true
  exit 1
fi

xcrun stapler staple -q "$DMG"
xcrun stapler validate -q "$DMG"
# Gatekeeper's verdict on the image, then on the app in it, as a Mac that
# downloaded them would give it.
spctl --assess --type open --context context:primary-signature -v "$DMG"
MOUNT="$(mktemp -d)"
hdiutil attach -quiet -nobrowse -readonly -mountpoint "$MOUNT" "$DMG"
spctl --assess --type execute -v "$MOUNT/NiceMailer.app" || { hdiutil detach -quiet "$MOUNT"; exit 1; }
hdiutil detach -quiet "$MOUNT"

SHA="$(shasum -a 256 "$DMG" | cut -d' ' -f1)"
cat <<DONE

Release ready: $DMG
SHA-256: $SHA

Test it on another Mac or user account (specifications/plan-release.md,
section 5), then publish:
  git tag v$VERSION && git push origin v$VERSION
  gh release create v$VERSION "$DMG" --prerelease --title "NiceMailer $VERSION" --notes-file <notes>
DONE
