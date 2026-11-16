"""Tests of scripts/signing-identity.sh (run by `make test`).

The script reads the identities from NICEMAILER_IDENTITIES_FILE instead of
the keychain, so that every case runs without certificates.

Run alone: python3 -m unittest discover -s scripts/tests
"""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "signing-identity.sh"

DEV_PERSONAL = "1" * 40 + " PERSONAL01 Apple Development: me@example.com (M9YTW2AHFD)"
DEV_TEAM = "2" * 40 + " TEAM000001 Apple Development: Me (YD97X2XXTB)"
DEVID = "3" * 40 + " TEAM000001 Developer ID Application: Company (TEAM000001)"
DEVID_OTHER = "4" * 40 + " OTHER00001 Developer ID Application: Other (OTHER00001)"


def run(lines, *args):
    """Runs the script on these identities; (exit code, stdout, stderr)."""
    with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
        f.write("".join(line + "\n" for line in lines))
        path = f.name
    try:
        env = dict(os.environ, NICEMAILER_IDENTITIES_FILE=path)
        p = subprocess.run([str(SCRIPT), *args], env=env, capture_output=True, text=True)
        return p.returncode, p.stdout.strip(), p.stderr
    finally:
        os.unlink(path)


class Development(unittest.TestCase):
    def test_prefers_the_team_of_the_developer_id(self):
        # The personal certificate comes first, as in the keychain where the
        # bug was seen: it must not be chosen.
        code, out, err = run([DEV_PERSONAL, DEVID, DEV_TEAM])
        self.assertEqual((code, out, err), (0, "2" * 40, ""))

    def test_other_team_falls_back_with_a_warning(self):
        code, out, err = run([DEV_PERSONAL, DEVID])
        self.assertEqual((code, out), (0, "1" * 40))
        self.assertIn("no Apple Development identity of team TEAM000001", err)

    def test_without_developer_id_takes_the_first_silently(self):
        code, out, err = run([DEV_PERSONAL, DEV_TEAM])
        self.assertEqual((code, out, err), (0, "1" * 40, ""))

    def test_the_first_developer_id_sets_the_team(self):
        code, out, _ = run([DEV_PERSONAL, DEVID, DEVID_OTHER, DEV_TEAM])
        self.assertEqual((code, out), (0, "2" * 40))

    def test_no_apple_development_prints_nothing(self):
        # Each caller has its own fallback (ad hoc, "NiceMailer Dev").
        self.assertEqual(run([DEVID]), (0, "", ""))
        self.assertEqual(run([]), (0, "", ""))

    def test_kind_is_matched_at_the_start_of_the_name(self):
        # "Apple Development: " inside another name is not an identity of
        # that kind.
        odd = "5" * 40 + " TEAM000001 Mac Developer: Apple Development: x (TEAM000001)"
        self.assertEqual(run([odd])[1], "")


class Release(unittest.TestCase):
    def test_takes_the_developer_id(self):
        self.assertEqual(run([DEV_TEAM, DEVID], "--release"), (0, "3" * 40, ""))

    def test_fails_without_developer_id(self):
        code, out, err = run([DEV_TEAM], "--release")
        self.assertEqual((code, out), (1, ""))
        self.assertIn("no Developer ID Application identity", err)


if __name__ == "__main__":
    unittest.main()
