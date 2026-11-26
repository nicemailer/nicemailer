"""Tests of scripts/make-appcast.py (run by `make test`).

Run alone: python3 -m unittest discover -s scripts/tests
"""

import importlib.util
import re
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

    def test_the_theme_of_an_entry_is_bold(self):
        # The bug this guards: `**Accounts:**` reached Sparkle with its stars.
        self.assertEqual(make_appcast.notes_html("- **Accounts:** Sign in."),
                         "<ul>\n<li><strong>Accounts:</strong> Sign in.</li>\n</ul>")

    def test_code_spans_and_links_are_converted(self):
        self.assertEqual(make_appcast.inline_html("Press `v` `h`."),
                         "Press <code>v</code> <code>h</code>.")
        # The Homebrew entry of CHANGELOG.md: code inside a link's label.
        self.assertEqual(
            make_appcast.inline_html("the [`nicemailer/homebrew-tap`](https://github.com/nicemailer/homebrew-tap) tap"),
            'the <a href="https://github.com/nicemailer/homebrew-tap"><code>nicemailer/homebrew-tap</code></a> tap')
        self.assertEqual(make_appcast.inline_html("**Reset `v` `r`**"),
                         "<strong>Reset <code>v</code> <code>r</code></strong>")

    def test_markdown_inside_code_stays_literal(self):
        self.assertEqual(make_appcast.inline_html("`**not bold**` and `[a](https://x)`"),
                         "<code>**not bold**</code> and <code>[a](https://x)</code>")

    def test_inline_text_is_still_escaped(self):
        self.assertEqual(make_appcast.inline_html("**<b>&</b>** `<i>`"),
                         "<strong>&lt;b&gt;&amp;&lt;/b&gt;</strong> <code>&lt;i&gt;</code>")
        self.assertEqual(make_appcast.inline_html('[x](https://e.org/?a=1&b="2")'),
                         '<a href="https://e.org/?a=1&amp;b=&quot;2&quot;">x</a>')

    def test_unknown_or_unclosed_markdown_is_left_as_typed(self):
        self.assertEqual(make_appcast.inline_html("a * b ** c"), "a * b ** c")
        self.assertEqual(make_appcast.inline_html("`open"), "`open")

    def test_only_web_links_become_links(self):
        self.assertEqual(make_appcast.inline_html("[x](javascript:alert(1))"), "[x](javascript:alert(1))")
        self.assertEqual(make_appcast.inline_html("[x](file:///etc/passwd)"), "[x](file:///etc/passwd)")

    def test_no_markdown_marker_reaches_sparkle_from_the_changelog(self):
        # Every section of the real changelog (Unreleased included), as it would ship.
        changelog = (SCRIPT.parent.parent / "CHANGELOG.md").read_text(encoding="utf-8")
        bodies = changelog.split("\n## ")[1:]
        self.assertTrue(bodies)
        for body in bodies:
            title, _, notes = body.partition("\n")
            # A marker inside a code span is meant to be shown (an entry may
            # quote `**`), so code spans are left out of the check.
            rendered = re.sub(r"<code>.*?</code>", "", make_appcast.notes_html(notes))
            for marker in ("**", "`", "]("):
                self.assertNotIn(marker, rendered, f"{marker!r} left in the notes of {title!r}")

    def test_a_build_number_that_is_not_a_number_is_refused(self):
        with self.assertRaises(ValueError):
            make_appcast.appcast("0.2.0", "v0.2.0-3-gabc", "u", "s", 1, "")


if __name__ == "__main__":
    unittest.main()
