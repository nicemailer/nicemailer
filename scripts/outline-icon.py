#!/usr/bin/env python3
"""Converts the <text> elements of an icon SVG to outlines (`design/icon/README.md`).

The proposals of `design/icon/` set their letters as text in Space Grotesk:
they render as designed only where the font is installed. The icon of the
app must not depend on that, so each <text> becomes a <path> of the glyphs,
at the same weight, size, letter spacing and position.

Usage:
    scripts/outline-icon.py <font.ttf> <in.svg> <out.svg>

<font.ttf>: Space Grotesk, the variable font of Google Fonts (OFL),
https://github.com/google/fonts/tree/main/ofl/spacegrotesk. Needs fontTools
(`pip install fonttools`).

Handled: the attributes the proposals use (x, y, text-anchor start|middle,
font-weight, font-size, letter-spacing, fill) and a plain text content.
Letter spacing is added after every glyph, the last one included, as
browsers do, so that `text-anchor="middle"` centres the same way.
"""

import re
import sys

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

TEXT = re.compile(r"<text([^>]*)>([^<]*)</text>")
ATTR = re.compile(r'([\w-]+)="([^"]*)"')


def instance(path, weight, cache={}):
    """The font at this weight (a static instance of the variable font)."""
    if weight not in cache:
        font = TTFont(path)
        if "fvar" in font:
            font = instantiateVariableFont(font, {"wght": weight})
        cache[weight] = font
    return cache[weight]


def outline(font_path, attrs, content):
    a = dict(ATTR.findall(attrs))
    font = instance(font_path, float(a.get("font-weight", "400")))
    glyphs = font.getGlyphSet()
    cmap = font.getBestCmap()
    upem = font["head"].unitsPerEm
    size = float(a["font-size"])
    scale = size / upem
    spacing = float(a.get("letter-spacing", "0"))
    names = [cmap[ord(c)] for c in content]
    width = sum(glyphs[n].width * scale + spacing for n in names)
    x = float(a.get("x", "0"))
    if a.get("text-anchor") == "middle":
        x -= width / 2
    elif a.get("text-anchor") == "end":
        x -= width
    y = float(a.get("y", "0"))
    pen = SVGPathPen(glyphs)
    for n in names:
        # Font units have y up; SVG has y down: flip, then place the glyph.
        glyphs[n].draw(TransformPen(pen, (scale, 0, 0, -scale, x, y)))
        x += glyphs[n].width * scale + spacing
    fill = a.get("fill", "#000")
    return f'<path d="{pen.getCommands()}" fill="{fill}"/>'


def main():
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    font_path, src, dst = sys.argv[1:]
    svg = open(src, encoding="utf-8").read()
    out = TEXT.sub(lambda m: outline(font_path, m.group(1), m.group(2)), svg)
    open(dst, "w", encoding="utf-8").write(out)


if __name__ == "__main__":
    main()
