"""Tests of scripts/update-autoconfig.py (run by `make test`).

Run alone: python3 -m unittest discover -s scripts/tests

The archives are built in memory, shaped like GitHub's: a top folder named
after the commit, the provider files in its ispdb/ folder. Nothing goes to
the network.
"""

import importlib.util
import io
import tarfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "update-autoconfig.py"
spec = importlib.util.spec_from_file_location("update_autoconfig", SCRIPT)
autoconfig = importlib.util.module_from_spec(spec)
spec.loader.exec_module(autoconfig)


def provider_xml(pid, domains, servers):
    """An ispdb file: `servers` are (tag, type, host, port, socket, username, auths)."""
    out = [f'<?xml version="1.0"?>\n<clientConfig version="1.1"><emailProvider id="{pid}">']
    out += [f"<domain>{d}</domain>" for d in domains]
    out.append(f"<displayName>{pid.upper()}</displayName>")
    for tag, kind, host, port, socket, username, auths in servers:
        out.append(f'<{tag} type="{kind}"><hostname>{host}</hostname><port>{port}</port>'
                   f"<socketType>{socket}</socketType>"
                   + (f"<username>{username}</username>" if username is not None else "")
                   + "".join(f"<authentication>{a}</authentication>" for a in auths)
                   + f"</{tag}>")
    out.append("</emailProvider></clientConfig>")
    return "".join(out).encode()


IMAP = ("incomingServer", "imap", "imap.example.com", 993, "SSL", "%EMAILADDRESS%", ["password-cleartext"])
SMTP = ("outgoingServer", "smtp", "smtp.example.com", 587, "STARTTLS", "%EMAILADDRESS%", ["password-cleartext"])


def archive(files, top="autoconfig-abc123"):
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as tar:
        for name, data in files.items():
            info = tarfile.TarInfo(f"{top}/{name}")
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
    return buffer.getvalue()


def convert(files):
    stats = autoconfig.Stats()
    commit, providers = autoconfig.convert(archive(files), stats)
    return commit, providers, stats.reasons


class ConvertTests(unittest.TestCase):
    def test_a_provider_is_kept_with_its_imap_and_smtp_servers(self):
        commit, providers, _ = convert({"ispdb/example.com.xml": provider_xml("example.com", ["Example.com", "ex.org"], [IMAP, SMTP])})
        self.assertEqual(commit, "abc123")
        self.assertEqual(providers, [{
            "id": "example.com", "name": "EXAMPLE.COM", "domains": ["example.com", "ex.org"],
            "imap": [{"host": "imap.example.com", "port": 993, "security": "tls", "username": "%EMAILADDRESS%", "auth": ["password-cleartext"]}],
            "smtp": [{"host": "smtp.example.com", "port": 587, "security": "starttls", "username": "%EMAILADDRESS%", "auth": ["password-cleartext"]}],
        }])

    def test_pop_only_and_clear_text_servers_are_dropped_and_counted(self):
        pop = ("incomingServer", "pop3", "pop.a.com", 995, "SSL", "%EMAILADDRESS%", [])
        plain_smtp = ("outgoingServer", "smtp", "smtp.b.com", 25, "plain", "%EMAILADDRESS%", [])
        _, providers, reasons = convert({
            "ispdb/a.com.xml": provider_xml("a.com", ["a.com"], [pop]),
            "ispdb/b.com.xml": provider_xml("b.com", ["b.com"], [IMAP, plain_smtp]),
        })
        self.assertEqual([p["id"] for p in providers], ["b.com"])
        self.assertEqual(providers[0]["smtp"], [])
        self.assertIn("provider without IMAP server (POP3 or Exchange only)", reasons)
        self.assertIn("smtp server without encryption (plain)", reasons)

    def test_the_domain_placeholder_of_a_host_is_kept(self):
        imap = ("incomingServer", "imap", "mail.%EMAILDOMAIN%", 993, "SSL", "%EMAILLOCALPART%", [])
        _, providers, _ = convert({"ispdb/c.com.xml": provider_xml("c.com", ["c.com"], [imap])})
        self.assertEqual(providers[0]["imap"][0]["host"], "mail.%EMAILDOMAIN%")
        self.assertEqual(providers[0]["imap"][0]["username"], "%EMAILLOCALPART%")

    def test_an_smtp_server_without_user_name_takes_the_imap_one(self):
        imap = IMAP[:5] + ("%EMAILLOCALPART%", IMAP[6])
        smtp = SMTP[:5] + (None, SMTP[6])
        _, providers, _ = convert({"ispdb/d.com.xml": provider_xml("d.com", ["d.com"], [imap, smtp])})
        self.assertEqual(providers[0]["smtp"][0]["username"], "%EMAILLOCALPART%")

    def test_invalid_values_are_refused(self):
        bad_host = ("incomingServer", "imap", "imap.example.com;rm -rf", 993, "SSL", "%EMAILADDRESS%", [])
        bad_port = ("incomingServer", "imap", "imap.example.com", 99999, "SSL", "%EMAILADDRESS%", [])
        bad_user = ("incomingServer", "imap", "imap.example.com", 993, "SSL", "a b", [])
        _, providers, reasons = convert({
            "ispdb/e.com.xml": provider_xml("e.com", ["e.com", "not a domain"], [bad_host, bad_port, bad_user, IMAP]),
        })
        self.assertEqual(providers[0]["domains"], ["e.com"])
        self.assertEqual(len(providers[0]["imap"]), 1)
        for reason in ("imap server with an invalid host", "imap server with an invalid port",
                       "imap server with an invalid user name", "invalid domain"):
            self.assertIn(reason, reasons)

    def test_a_domain_claimed_twice_stays_with_the_first_provider(self):
        _, providers, reasons = convert({
            "ispdb/a.com.xml": provider_xml("a.com", ["a.com", "shared.com"], [IMAP]),
            "ispdb/b.com.xml": provider_xml("b.com", ["b.com", "shared.com"], [IMAP]),
        })
        self.assertEqual([p["domains"] for p in providers], [["a.com", "shared.com"], ["b.com"]])
        self.assertIn("domain already given by another provider", reasons)

    def test_entities_and_files_outside_ispdb_are_never_read(self):
        bomb = b'<?xml version="1.0"?><!DOCTYPE x [<!ENTITY a "aaaa">]><clientConfig/>'
        _, providers, reasons = convert({
            "ispdb/bomb.xml": bomb,
            "other/f.com.xml": provider_xml("f.com", ["f.com"], [IMAP]),
            "ispdb/g.com.xml": provider_xml("g.com", ["g.com"], [IMAP]),
        })
        self.assertEqual([p["id"] for p in providers], ["g.com"])
        self.assertIn("file declaring a DOCTYPE or an entity", reasons)

    def test_the_real_file_is_well_formed(self):
        """data/providers/thunderbird.json, as committed: what the core reads."""
        import json
        data = json.loads(autoconfig.OUTPUT.read_text(encoding="utf-8"))
        self.assertGreaterEqual(len(data["providers"]), autoconfig.MIN_PROVIDERS)
        gmail = next(p for p in data["providers"] if "gmail.com" in p["domains"])
        # google.com is the MX of the domains hosted by Google.
        self.assertIn("google.com", gmail["domains"])
        for p in data["providers"]:
            self.assertTrue(p["imap"])
            for s in p["imap"] + p["smtp"]:
                self.assertIn(s["security"], ("tls", "starttls"))


if __name__ == "__main__":
    unittest.main()
