"""Tests of scripts/release-notes.py (run by `make test`).

Run alone: python3 -m unittest discover -s scripts/tests
"""

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "release-notes.py"
spec = importlib.util.spec_from_file_location("release_notes", SCRIPT)
release_notes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release_notes)

CHANGELOG = """# Changelog

Rules, before any section.

## Unreleased

## 0.2.0 — 2026-12-01

### Added

- Unsubscribe in one key.

### Fixed

- Sending no longer hangs.

## 0.1.0 — 2026-11-16

First release.
"""


class NotesFor(unittest.TestCase):
    def test_body_of_the_version_up_to_the_next_section(self):
        notes = release_notes.notes_for(CHANGELOG, "0.2.0")
        self.assertEqual(notes, "### Added\n\n- Unsubscribe in one key.\n\n### Fixed\n\n- Sending no longer hangs.\n")

    def test_last_section_runs_to_the_end(self):
        self.assertEqual(release_notes.notes_for(CHANGELOG, "0.1.0"), "First release.\n")

    def test_heading_without_date(self):
        self.assertEqual(release_notes.notes_for("## 1.0.0\n\nDone.\n", "1.0.0"), "Done.\n")

    def test_version_is_not_a_prefix_match(self):
        text = "## 0.1.0.1 — 2026-11-17\n\nPatch.\n"
        with self.assertRaises(release_notes.NotesError):
            release_notes.notes_for(text, "0.1.0")
        # A dot of the version is not a regex wildcard.
        with self.assertRaises(release_notes.NotesError):
            release_notes.notes_for("## 0x1y0\n\nNo.\n", "0.1.0")

    def test_missing_version_is_refused(self):
        with self.assertRaisesRegex(release_notes.NotesError, "no section"):
            release_notes.notes_for(CHANGELOG, "0.3.0")

    def test_empty_section_is_refused(self):
        with self.assertRaisesRegex(release_notes.NotesError, "empty"):
            release_notes.notes_for("## Unreleased\n\n## 0.2.0 — 2026-12-01\n\n## 0.1.0\n\nFirst.\n", "0.2.0")

    def test_entries_left_under_unreleased_are_refused(self):
        text = "## Unreleased\n\n- Forgotten.\n\n## 0.2.0 — 2026-12-01\n\n- Shipped.\n"
        with self.assertRaisesRegex(release_notes.NotesError, "Unreleased"):
            release_notes.notes_for(text, "0.2.0")

    def test_text_before_the_first_section_is_ignored(self):
        self.assertEqual(release_notes.sections("# Title\n\n## 0.1.0\n\nA.\n"), [("0.1.0", "A.")])


class CommandLine(unittest.TestCase):
    def run_script(self, *args):
        return subprocess.run([sys.executable, "-I", str(SCRIPT), *args], capture_output=True, text=True)

    def test_prints_the_notes(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "CHANGELOG.md"
            path.write_text(CHANGELOG, encoding="utf-8")
            result = self.run_script("0.1.0", str(path))
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "First release.\n")

    def test_refusal_exits_1_with_a_message(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "CHANGELOG.md"
            path.write_text(CHANGELOG, encoding="utf-8")
            result = self.run_script("9.9.9", str(path))
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")
        self.assertIn("no section", result.stderr)

    def test_the_repository_changelog_has_a_section_for_the_current_version(self):
        # The version of Cargo.toml is the last one released, or the one
        # about to be: its section must exist and say something. Entries
        # under "Unreleased" are not checked here: between two releases
        # they are the normal state, refused only by `make release`.
        root = SCRIPT.parent.parent
        version = next(line.split('"')[1] for line in (root / "Cargo.toml").read_text().splitlines()
                       if line.startswith("version = "))
        found = release_notes.sections((root / "CHANGELOG.md").read_text(encoding="utf-8"))
        bodies = [body for title, body in found if title.split()[0] == version]
        self.assertEqual(len(bodies), 1, f"CHANGELOG.md needs one section \"## {version}\"")
        self.assertTrue(bodies[0], f"the section \"## {version}\" of CHANGELOG.md is empty")


if __name__ == "__main__":
    unittest.main()
