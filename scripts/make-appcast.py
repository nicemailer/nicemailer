#!/usr/bin/env python3
"""Write the appcast of a release: the feed Sparkle reads to find updates.

scripts/make-release.sh runs it once the disk image is notarized and signed
for Sparkle (`sign_update`, EdDSA), and the appcast is published as an
asset of the GitHub release, next to the image (docs/updates.md). The app
reads it at `SUFeedURL`, by default
https://github.com/nicemailer/nicemailer/releases/latest/download/appcast.xml:
the appcast of the latest release, which only needs to describe that
release. One item, then: Sparkle offers it when its `sparkle:version` (the
build number, `CFBundleVersion`) is greater than the app's.

The release notes (notes.md, from CHANGELOG.md) are embedded as HTML in
the item's description, which Sparkle shows: `- ` lines become a list,
`### ` headings titles, other lines paragraphs. Every text is escaped.

Usage: scripts/make-appcast.py --version 0.2.0 --build 812 \\
         --url https://…/NiceMailer-0.2.0.dmg --signature <EdDSA> \\
         --length <bytes> --notes notes.md [--minimum-system 14.0] > appcast.xml
"""

import argparse
import html
import sys
from datetime import datetime, timezone
from email.utils import format_datetime
from pathlib import Path


def notes_html(markdown):
    """The release notes, from the Markdown of CHANGELOG.md, as HTML."""
    out = []
    in_list = False
    for line in markdown.splitlines():
        text = line.strip()
        is_item = text.startswith("- ")
        if in_list and not is_item:
            out.append("</ul>")
            in_list = False
        if not text:
            continue
        if is_item:
            if not in_list:
                out.append("<ul>")
                in_list = True
            out.append(f"<li>{html.escape(text[2:])}</li>")
        elif text.startswith("#"):
            out.append(f"<h3>{html.escape(text.lstrip('#').strip())}</h3>")
        else:
            out.append(f"<p>{html.escape(text)}</p>")
    if in_list:
        out.append("</ul>")
    return "\n".join(out)


def appcast(version, build, url, signature, length, notes, minimum_system="14.0", date=None):
    """The XML of the appcast, one item for this release."""
    if not str(build).isdigit():
        raise ValueError(f"the build number must be a whole number (CFBundleVersion), not {build!r}")
    date = date or datetime.now(timezone.utc)
    # CDATA cannot hold "]]>": split it, which the XML reader puts back.
    description = notes_html(notes).replace("]]>", "]]]]><![CDATA[>")
    e = lambda s: html.escape(str(s), quote=True)
    return f"""<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>NiceMailer</title>
    <item>
      <title>NiceMailer {e(version)}</title>
      <pubDate>{format_datetime(date)}</pubDate>
      <sparkle:version>{e(build)}</sparkle:version>
      <sparkle:shortVersionString>{e(version)}</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>{e(minimum_system)}</sparkle:minimumSystemVersion>
      <description><![CDATA[{description}]]></description>
      <enclosure url="{e(url)}" length="{e(length)}" type="application/octet-stream" sparkle:edSignature="{e(signature)}"/>
    </item>
  </channel>
</rss>
"""


def main(argv):
    p = argparse.ArgumentParser(description="Write the Sparkle appcast of a release.")
    p.add_argument("--version", required=True)
    p.add_argument("--build", required=True)
    p.add_argument("--url", required=True)
    p.add_argument("--signature", required=True)
    p.add_argument("--length", required=True, type=int)
    p.add_argument("--notes", required=True, type=Path)
    p.add_argument("--minimum-system", default="14.0")
    a = p.parse_args(argv)
    try:
        sys.stdout.write(appcast(a.version, a.build, a.url, a.signature, a.length,
                                 a.notes.read_text(encoding="utf-8"), a.minimum_system))
    except ValueError as err:
        print(f"make-appcast: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
