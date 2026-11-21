"""Tests of scripts/sign-core.sh (run by `make test`).

The real Keychain, on throwaway keychain files (never the login keychain),
and the real certificates of this Mac: a small program, built here, saves a
password with one signature and reads it with another, with every Keychain
dialog turned off, as the core does (`keychain::never_prompt`). Skipped when
the Mac lacks the certificates a case needs.

Why: the access list of a Keychain item names the program that created it
by its designated requirement. The default one of `codesign` names the kind
of certificate, so the notarized core refused the passwords saved by a
`make` build (docs/configuration.md, "Passwords").

Run alone: python3 -m unittest scripts.tests.test_sign_core (from the root),
or python3 -m unittest discover -s scripts/tests
"""

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
SIGN_CORE = ROOT / "scripts" / "sign-core.sh"
SIGNING_IDENTITY = ROOT / "scripts" / "signing-identity.sh"

# `probe write|read <keychain>`: the legacy keychain API, which the core
# uses for a file keychain too, with user interaction off. Exit 0 when the
# access succeeds.
PROBE_C = r"""
#include <Security/Security.h>
#include <stdio.h>
#include <string.h>
int main(int argc, char **argv) {
  SecKeychainSetUserInteractionAllowed(false);
  SecKeychainRef kc;
  if (SecKeychainOpen(argv[2], &kc)) return 2;
  OSStatus s;
  if (!strcmp(argv[1], "write")) {
    s = SecKeychainAddGenericPassword(kc, 10, "NiceMailer", 10, "imap:probe", 2, "pw", NULL);
  } else {
    UInt32 len; void *data;
    s = SecKeychainFindGenericPassword(kc, 10, "NiceMailer", 10, "imap:probe", &len, &data, NULL);
    if (!s && (len != 2 || memcmp(data, "pw", 2))) s = -1;
  }
  printf("build %d status=%d\n", BUILD, (int)s);
  return s ? 1 : 0;
}
"""


def identity(*args):
    """The hash chosen by signing-identity.sh, or None."""
    p = subprocess.run([str(SIGNING_IDENTITY), *args], capture_output=True, text=True)
    return p.stdout.strip() or None if p.returncode == 0 else None


def team_of(binary):
    out = subprocess.run(["codesign", "-dvv", str(binary)], capture_output=True, text=True).stderr
    for line in out.splitlines():
        if line.startswith("TeamIdentifier="):
            return line.split("=", 1)[1]
    return None


def requirement_of(binary):
    out = subprocess.run(["codesign", "-d", "-r-", str(binary)], capture_output=True, text=True).stdout
    for line in out.splitlines():
        if line.startswith("designated => "):
            return line[len("designated => "):]
    return None


DEV = identity()
DEVID = identity("--release")


@unittest.skipUnless(shutil.which("clang") and DEV, "needs clang and an Apple Development identity")
class SignCore(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.dir)
        src = self.dir / "probe.c"
        src.write_text(PROBE_C)
        self.probe = self.build(src, "probe", 1)

    def build(self, src, name, build):
        """The probe compiled with BUILD set: a different number gives a
        different binary, as a rebuild of the core after a change."""
        out = self.dir / name
        subprocess.run(
            ["clang", "-Wno-deprecated-declarations", "-framework", "Security", "-framework", "CoreFoundation",
             f"-DBUILD={build}", str(src), "-o", str(out)],
            check=True, capture_output=True)
        return out

    def signed(self, name, ident, default_requirement=False, binary=None):
        """A copy of the probe (or of `binary`) signed by sign-core.sh with
        this identity, or by plain codesign (its default requirement) for
        comparison."""
        path = self.dir / name
        shutil.copy(binary or self.probe, path)
        if default_requirement:
            cmd = ["codesign", "--force", "--sign", ident, "--identifier", "eu.nicemailer.core", "--options", "runtime", str(path)]
            subprocess.run(cmd, check=True, capture_output=True)
        else:
            env = dict(os.environ, SIGN_IDENTITY=ident)
            subprocess.run([str(SIGN_CORE), str(path)], check=True, capture_output=True, env=env)
        return path

    def keychain(self, name):
        path = self.dir / f"{name}.keychain-db"
        subprocess.run(["security", "create-keychain", "-p", "test", str(path)], check=True, capture_output=True)
        # Removed from the user's keychain list as well as from the disk.
        self.addCleanup(subprocess.run, ["security", "delete-keychain", str(path)], capture_output=True)
        subprocess.run(["security", "unlock-keychain", "-p", "test", str(path)], check=True, capture_output=True)
        return path

    def reads(self, writer, reader, name):
        kc = self.keychain(name)
        w = subprocess.run([str(writer), "write", str(kc)], capture_output=True, text=True)
        self.assertEqual(w.returncode, 0, w.stdout)
        return subprocess.run([str(reader), "read", str(kc)], capture_output=True, text=True).returncode == 0

    def test_the_requirement_names_the_identifier_and_the_team_only(self):
        core = self.signed("core", DEV)
        team = team_of(core)
        self.assertRegex(team or "", r"^[A-Z0-9]{10}$")
        self.assertIn(requirement_of(core), [
            f'identifier "eu.nicemailer.core" and anchor apple generic and certificate leaf[subject.OU] = {team}',
            f'identifier "eu.nicemailer.core" and anchor apple generic and certificate leaf[subject.OU] = "{team}"',
        ])

    def test_a_rebuild_reads_the_passwords_of_the_previous_build(self):
        # Two builds of different content, so of different hashes: the
        # partition and the access list must not depend on the hash.
        first = self.signed("first", DEV)
        other = self.build(self.dir / "probe.c", "probe2", 2)
        self.assertNotEqual(other.read_bytes(), self.probe.read_bytes())
        second = self.signed("second", DEV, binary=other)
        self.assertTrue(self.reads(first, second, "rebuild"))

    def same_team_devid(self):
        if not DEVID:
            self.skipTest("needs a Developer ID Application identity")
        a, b = self.signed("team-dev", DEV), self.signed("team-devid", DEVID)
        if team_of(a) != team_of(b):
            self.skipTest("the Apple Development and Developer ID identities are of different teams")

    def test_development_and_release_read_each_others_passwords(self):
        self.same_team_devid()
        dev, rel = self.signed("dev", DEV), self.signed("rel", DEVID)
        self.assertTrue(self.reads(dev, rel, "dev-to-rel"), "a release reads what a development build saved")
        self.assertTrue(self.reads(rel, dev, "rel-to-dev"), "a development build reads what a release saved")

    def test_the_default_requirement_is_what_refused_them(self):
        # The bug itself, kept as a test: should codesign's default ever
        # allow it, the custom requirement would no longer be needed.
        self.same_team_devid()
        dev = self.signed("plain-dev", DEV, default_requirement=True)
        rel = self.signed("plain-rel", DEVID, default_requirement=True)
        self.assertFalse(self.reads(dev, rel, "plain"))


if __name__ == "__main__":
    unittest.main()
