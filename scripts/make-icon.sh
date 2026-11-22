#!/bin/bash
# Builds the icon of NiceMailer.app, design/icon/app/AppIcon.icns, from the
# outlined SVGs of design/icon/app (design/icon/README.md).
#
# Each size of the .icns is drawn from the SVG made for it, as Apple's icons
# do: the small sizes lose the details they cannot show (the trail, the
# "nm" of the scale bar), so that the letters stay readable in the Finder.
#   16, 32 px        AppIcon-32.svg
#   64 px            AppIcon-64.svg
#   128 px           AppIcon-128.svg
#   256 to 1024 px   AppIcon.svg
#
# Needs rsvg-convert (`brew install librsvg`). The .icns is committed, so
# that `make bundle` needs neither; run this again after changing an SVG.
# Usage: scripts/make-icon.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/design/icon/app"
SET="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$SET"
trap 'rm -rf "$(dirname "$SET")"' EXIT

svg_for() {
  if [ "$1" -le 32 ]; then echo "$SRC/AppIcon-32.svg"
  elif [ "$1" -le 64 ]; then echo "$SRC/AppIcon-64.svg"
  elif [ "$1" -le 128 ]; then echo "$SRC/AppIcon-128.svg"
  else echo "$SRC/AppIcon.svg"; fi
}

render() { rsvg-convert -w "$1" -h "$1" "$(svg_for "$1")" -o "$SET/$2"; }

for size in 16 32 128 256 512; do
  render "$size" "icon_${size}x${size}.png"
  render "$((size * 2))" "icon_${size}x${size}@2x.png"
done
iconutil -c icns "$SET" -o "$SRC/AppIcon.icns"
echo "Built $SRC/AppIcon.icns"
