"""Tests of scripts/make-appcast.py (run by `make test`).

Run alone: python3 -m unittest discover -s scripts/tests
"""

import importlib.util
import unittest
import xml.etree.ElementTree as ET
from datetime import datetime, timezone
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "make-appcast.py"
spec = importlib.util.spec_from_file_location("make_appcast", SCRIPT)
make_appcast = importlib.util.module_from_spec(spec)
spec.loader.exec_module(make_appcast)

SPARKLE = "{http://www.andymatuschak.org/xml-namespaces/sparkle}"
NOTES = """### Added

- Updates by Sparkle: <b>no</b> more downloads by hand.
- A second line & more.

### Fixed

A paragraph ]]> with the end of a CDATA.
"""


class AppcastTests(unittest.TestCase):
    def item(self, **kw):
        args = dict(version="0.2.0", build="812", url="https://example.org/NiceMailer-0.2.0.dmg?a=1&b=2",
                    signature="c2lnbmF0dXJl==", length=12345, notes=NOTES,
                    date=datetime(2026, 12, 1, 10, 0, tzinfo=timezone.utc))
        args.update(kw)
        root = ET.fromstring(make_appcast.appcast(**args))
        return root.find("channel/item")

    def test_the_item_is_what_sparkle_reads(self):
        item = self.item()
        self.assertEqual(item.find(f"{SPARKLE}version").text, "812")
        self.assertEqual(item.find(f"{SPARKLE}shortVersionString").text, "0.2.0")
        self.assertEqual(item.find(f"{SPARKLE}minimumSystemVersion").text, "14.0")
        self.assertEqual(item.find("pubDate").text, "Tue, 01 Dec 2026 10:00:00 +0000")
        enclosure = item.find("enclosure")
        self.assertEqual(enclosure.get("url"), "https://example.org/NiceMailer-0.2.0.dmg?a=1&b=2")
        self.assertEqual(enclosure.get("length"), "12345")
        self.assertEqual(enclosure.get(f"{SPARKLE}edSignature"), "c2lnbmF0dXJl==")

    def test_the_notes_become_escaped_html(self):
        description = self.item().find("description").text
        self.assertIn("<h3>Added</h3>", description)
        self.assertIn("<ul>\n<li>Updates by Sparkle: &lt;b&gt;no&lt;/b&gt; more downloads by hand.</li>", description)
        self.assertIn("<li>A second line &amp; more.</li>\n</ul>", description)
        self.assertIn("<p>A paragraph ]]&gt; with the end of a CDATA.</p>", description)

    def test_a_build_number_that_is_not_a_number_is_refused(self):
        with self.assertRaises(ValueError):
            make_appcast.appcast("0.2.0", "v0.2.0-3-gabc", "u", "s", 1, "")


if __name__ == "__main__":
    unittest.main()
