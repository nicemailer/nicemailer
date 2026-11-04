"""Tests of scripts/export-apple-mail.py (run by `make test`).

Run alone: python3 -m unittest discover -s scripts/tests

The script's name has dashes, so it cannot be imported with `import`: it is
loaded from its path with importlib.
"""

import importlib.util
import mailbox
import os
import plistlib
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "export-apple-mail.py"
spec = importlib.util.spec_from_file_location("export_apple_mail", SCRIPT)
export = importlib.util.module_from_spec(spec)
spec.loader.exec_module(export)

RAW = (b"From: Alice <alice@a.example>\r\n"
       b"Subject: Devis\r\n"
       b"Message-ID: <m1@a.example>\r\n"
       b"Date: Tue, 27 Oct 2015 09:49:25 +0100\r\n"
       b"\r\n"
       b"Bonjour\r\n"
       b"From the start of a line\r\n")


def emlx(directory, name, raw, meta):
    """Writes an .emlx file as Apple Mail does: length, message, plist."""
    path = Path(directory) / name
    path.write_bytes(str(len(raw)).encode() + b"\n" + raw + plistlib.dumps(meta))
    return path


class ExportDates(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.tmp = Path(self.dir.name)

    def export(self, *paths):
        """Exports the .emlx files to one mbox and reads it back."""
        target = self.tmp / "out.mbox"
        stats = export.write_mbox(target, [(p, ["INBOX"]) for p in paths])
        self.assertEqual(stats["errors"], 0)
        box = mailbox.mbox(str(target))
        self.addCleanup(box.close)
        return target, list(box)

    def test_from_line_carries_mails_date_received(self):
        # 1596271320 = Sat Aug  1 08:42:00 2020 UTC.
        p = emlx(self.tmp, "1.emlx", RAW, {"date-received": 1596271320, "flags": 1})
        target, msgs = self.export(p)
        self.assertEqual(msgs[0].get_from(), "MAILER-DAEMON Sat Aug  1 08:42:00 2020")
        # A single From_ line: the one given is used, not a second one.
        self.assertEqual(target.read_bytes().count(b"\nFrom MAILER-DAEMON"), 0)
        self.assertTrue(target.read_bytes().startswith(b"From MAILER-DAEMON Sat Aug  1 08:42:00 2020\n"))
        # Headers added by the export; a body line starting with "From " is
        # escaped, so it does not split the message.
        self.assertEqual(msgs[0]["Status"], "RO")
        self.assertEqual(msgs[0]["X-Mail-Folder"], "INBOX")
        self.assertIn(b">From the start of a line", target.read_bytes())

    def test_without_date_received_the_date_header_is_used(self):
        # 27 Oct 2015 09:49:25 +0100 = 08:49:25 UTC.
        p = emlx(self.tmp, "2.emlx", RAW, {"flags": 0})
        _, msgs = self.export(p)
        self.assertEqual(msgs[0].get_from(), "MAILER-DAEMON Tue Oct 27 08:49:25 2015")

    def test_without_any_date_the_file_time_is_used(self):
        raw = RAW.replace(b"Date: Tue, 27 Oct 2015 09:49:25 +0100\r\n", b"Date: n'importe quoi\r\n")
        p = emlx(self.tmp, "3.emlx", raw, {"date-received": "abc"})
        os.utime(p, (1_000_000_000, 1_000_000_000))  # Sun Sep  9 01:46:40 2001 UTC
        _, msgs = self.export(p)
        self.assertEqual(msgs[0].get_from(), "MAILER-DAEMON Sun Sep  9 01:46:40 2001")

    def test_each_message_keeps_its_own_date(self):
        a = emlx(self.tmp, "4.emlx", RAW, {"date-received": 1596271320})
        b = emlx(self.tmp, "5.emlx", RAW.replace(b"m1@", b"m2@"), {"date-received": 1300000000})
        _, msgs = self.export(a, b)
        self.assertEqual([m.get_from() for m in msgs],
                         ["MAILER-DAEMON Sat Aug  1 08:42:00 2020", "MAILER-DAEMON Sun Mar 13 07:06:40 2011"])


if __name__ == "__main__":
    unittest.main()
