"""Tests of scripts/bundle-languages.py and of the texts of Info.plist in
every language of the app (run by `make test`).

Run alone: python3 -m unittest discover -s scripts/tests
"""

import importlib.util
import json
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPT = ROOT / "scripts" / "bundle-languages.py"
spec = importlib.util.spec_from_file_location("bundle_languages", SCRIPT)
bundle_languages = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bundle_languages)


class BundleLanguagesTest(unittest.TestCase):
    def test_the_languages_are_those_of_the_catalog(self):
        self.assertEqual(bundle_languages.languages(), sorted([
            "da", "de", "en", "es", "fi", "fr", "it", "ja", "ko", "nb", "nl", "pl", "pt-BR", "ru", "sv", "uk", "zh-Hans", "zh-Hant",
        ]))
        with tempfile.TemporaryDirectory() as d:
            catalog = Path(d) / "Localizable.xcstrings"
            catalog.write_text(json.dumps({
                "sourceLanguage": "en",
                "strings": {"Hello": {"localizations": {"fr": {}, "de": {}}}, "Bye": {}},
            }))
            self.assertEqual(bundle_languages.languages(catalog), ["de", "en", "fr"])

    def test_every_language_translates_every_usage_description(self):
        """macOS shows these texts when it asks for an access: a language
        without them would ask in English."""
        script = (ROOT / "scripts" / "make-bundle.sh").read_text(encoding="utf-8")
        keys = set(re.findall(r"<key>(NS\w+UsageDescription)</key>", script))
        self.assertIn("NSContactsUsageDescription", keys)
        for lang in bundle_languages.languages():
            if lang == "en":
                continue
            path = ROOT / "apps" / "NiceMailer" / "Bundle" / f"{lang}.lproj" / "InfoPlist.strings"
            self.assertTrue(path.exists(), f"{path} is missing")
            parsed = json.loads(subprocess.run(
                ["plutil", "-convert", "json", "-o", "-", str(path)], check=True, capture_output=True,
            ).stdout)
            for key in keys:
                self.assertTrue(parsed.get(key), f"{lang}: {key} is not translated")


if __name__ == "__main__":
    unittest.main()
