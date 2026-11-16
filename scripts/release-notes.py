#!/usr/bin/env python3
"""Print the release notes of a version, taken from CHANGELOG.md.

scripts/make-release.sh runs it before building a release, and writes its
output next to the disk image (notes.md), the file given to
`gh release create --notes-file` (docs/app-bundle.md, "Release notes").

The notes of a version are the body of its section in CHANGELOG.md: the
lines after the heading `## <version>` (optionally followed by a date,
`## 0.2.0 — 2026-12-01`) up to the next `## ` heading.

It refuses, with a message saying what to fix, and exit code 1:
  - a version without a section, or whose section is empty: a release
    would be published without notes;
  - entries left under `## Unreleased`: at a release, that section is
    renamed to the version (CHANGELOG.md, "How to write it"), so entries
    still there are changes shipped in this release but missing from its
    notes.

Usage: scripts/release-notes.py <version> [CHANGELOG.md]
"""

import re
import sys
from pathlib import Path

UNRELEASED = "Unreleased"


class NotesError(Exception):
    """A changelog that cannot give the notes of a release."""


def sections(text):
    """The `## ` sections of a changelog: a list of (title, body).

    The title is the heading without `## `; the body, the lines up to the
    next `## ` heading, stripped of blank lines around it. Headings of
    other levels (`#`, `###`) belong to the body of their section. What
    precedes the first `## ` (the title and the rules) is not a section.
    """
    found = []
    title = None
    body = []
    for line in text.splitlines():
        if line.startswith("## "):
            if title is not None:
                found.append((title, "\n".join(body).strip()))
            title = line[3:].strip()
            body = []
        elif title is not None:
            body.append(line)
    if title is not None:
        found.append((title, "\n".join(body).strip()))
    return found


def notes_for(text, version):
    """The notes of `version` in the changelog `text`; NotesError if none."""
    # The version, then the end of the heading or a space before the date:
    # "0.1.0" must not match the heading of "0.1.0.1" nor of "0.1.01".
    heading = re.compile(re.escape(version) + r"(\s|$)")
    found = sections(text)
    for title, body in found:
        if title == UNRELEASED and body:
            raise NotesError(
                f"CHANGELOG.md has entries under \"## {UNRELEASED}\": rename that "
                f"section to \"## {version} — <date>\" and start a new, empty "
                f"\"## {UNRELEASED}\" above it.")
    for title, body in found:
        if heading.match(title):
            if not body:
                raise NotesError(f"The section \"## {title}\" of CHANGELOG.md is empty.")
            return body + "\n"
    raise NotesError(
        f"CHANGELOG.md has no section \"## {version}\": rename \"## {UNRELEASED}\" "
        f"to \"## {version} — <date>\" in the commit that sets the version.")


def main(argv):
    if len(argv) not in (2, 3):
        print("usage: release-notes.py <version> [CHANGELOG.md]", file=sys.stderr)
        return 2
    path = Path(argv[2]) if len(argv) == 3 else Path(__file__).resolve().parent.parent / "CHANGELOG.md"
    try:
        sys.stdout.write(notes_for(path.read_text(encoding="utf-8"), argv[1]))
    except (NotesError, OSError) as error:
        print(f"release-notes: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
