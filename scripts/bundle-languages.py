#!/usr/bin/env python3
"""The languages of NiceMailer.app: the source language of the String
Catalog and every language it has a translation for, sorted, one per line.

scripts/make-bundle.sh lists them in CFBundleLocalizations and copies
their InfoPlist.strings (apps/NiceMailer/Bundle/<language>.lproj), so that
a language added to the catalog is offered by macOS without editing the
script (specifications/localisation.md, section 3.7).

Usage: scripts/bundle-languages.py [catalog]
"""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CATALOG = ROOT / "apps/NiceMailer/Sources/NiceMailer/Resources/Localizable.xcstrings"


def languages(catalog: Path = CATALOG) -> list[str]:
    """The source language and the languages of the translations."""
    data = json.loads(catalog.read_text(encoding="utf-8"))
    found = {data["sourceLanguage"]}
    for entry in data["strings"].values():
        found.update(entry.get("localizations", {}).keys())
    return sorted(found)


if __name__ == "__main__":
    print("\n".join(languages(Path(sys.argv[1]) if len(sys.argv) > 1 else CATALOG)))
