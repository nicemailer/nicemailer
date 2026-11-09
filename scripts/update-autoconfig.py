#!/usr/bin/env python3
"""Regenerate data/providers/thunderbird.json from Thunderbird's ISP database.

`nicemailer account add` finds the servers of an address from this file
(docs/accounts.md, "Finding the servers"). It is the conversion of the
ispdb/ folder of https://github.com/thunderbird/autoconfig (MPL 2.0): one
XML file per provider, giving the domains it hosts and its servers.

What is kept, per provider:
  - its id (the file name), its display name, its domains;
  - its IMAP and SMTP servers: host, port, security, user name, the login
    methods it accepts. POP3, Exchange and the other kinds are dropped:
    NiceMailer speaks IMAP and SMTP only;
  - servers without encryption (socketType "plain") are dropped: the core
    refuses clear text beyond this Mac;
  - a provider left without any IMAP server is dropped.

Everything dropped is counted and printed, so that nothing is lost
silently. Our own providers (Infomaniak…) are not here but in
data/providers/local/, written by hand; they take precedence over this
file.

The archive is untrusted input: it is read in memory (nothing is extracted
to disk), only ispdb/*.xml is looked at, each file is size-limited, and a
file declaring a DOCTYPE or an entity is refused (entity expansion attacks).
Every value is checked before being written.

The file is rewritten only if the source answered, holds at least
MIN_PROVIDERS providers, and the list does not lose more than
MAX_REMOVED_RATIO of its providers (unless --force).

Usage: scripts/update-autoconfig.py [--force] [--dry-run] [--tarball FILE]
       (or `make autoconfig`)
"""

import argparse
import io
import json
import os
import re
import sys
import tarfile
import urllib.request
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "data" / "providers" / "thunderbird.json"

SOURCE = "https://github.com/thunderbird/autoconfig"
# The default branch, whatever its name, as a gzipped tar archive. The top
# folder of the archive is named after the commit: it is recorded.
ARCHIVE_URL = "https://github.com/thunderbird/autoconfig/archive/HEAD.tar.gz"

MIN_PROVIDERS = 100
MAX_REMOVED_RATIO = 0.10
FETCH_TIMEOUT_SECS = 60
MAX_ARCHIVE_BYTES = 20 * 1024 * 1024
MAX_FILE_BYTES = 256 * 1024

HOSTNAME = re.compile(r"^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)+$")
# Placeholders Thunderbird puts in host and user names; the core replaces
# them with the parts of the address (`providers.rs`).
PLACEHOLDERS = ("%EMAILADDRESS%", "%EMAILLOCALPART%", "%EMAILDOMAIN%")
SECURITY = {"SSL": "tls", "STARTTLS": "starttls"}
KINDS = {"incomingServer": "imap", "outgoingServer": "smtp"}


def fail(message):
    print(f"error: {message}", file=sys.stderr)
    sys.exit(1)


class Stats:
    """What was dropped, and why: printed at the end."""

    def __init__(self):
        self.reasons = {}

    def drop(self, reason, what):
        self.reasons.setdefault(reason, []).append(what)

    def report(self):
        for reason, items in sorted(self.reasons.items()):
            shown = ", ".join(sorted(items)[:8]) + (f" … (+{len(items) - 8})" if len(items) > 8 else "")
            print(f"  dropped, {reason}: {len(items)} ({shown})")


def valid_host(host):
    """A host name, placeholders replaced by a sample domain first."""
    return bool(HOSTNAME.match(host.replace("%EMAILDOMAIN%", "example.com")))


def valid_username(name):
    """A user name template: placeholders, and the characters of an address."""
    rest = name
    for p in PLACEHOLDERS:
        rest = rest.replace(p, "")
    return bool(name) and re.fullmatch(r"[A-Za-z0-9.@_+-]*", rest) is not None


def text(element, tag):
    child = element.find(tag)
    return (child.text or "").strip() if child is not None and child.text else ""


def parse_server(element, kind, provider_id, stats):
    """One <incomingServer type="imap"> or <outgoingServer type="smtp">, or None."""
    # Lower case, but the placeholder keeps its spelling.
    host = text(element, "hostname").lower().replace("%emaildomain%", "%EMAILDOMAIN%")
    port = text(element, "port")
    socket = text(element, "socketType")
    username = text(element, "username")
    auth = [a.text.strip().lower() for a in element.findall("authentication") if a.text and a.text.strip()]
    where = f"{provider_id} {kind} {host}"
    if socket not in SECURITY:
        stats.drop(f"{kind} server without encryption ({socket or 'none'})", where)
        return None
    if not valid_host(host):
        stats.drop(f"{kind} server with an invalid host", where)
        return None
    if not port.isdigit() or not 0 < int(port) < 65536:
        stats.drop(f"{kind} server with an invalid port", where)
        return None
    # No user name: Thunderbird uses the IMAP one for SMTP; filled in by
    # parse_provider, which knows it.
    if username and not valid_username(username):
        stats.drop(f"{kind} server with an invalid user name", where)
        return None
    return {"host": host, "port": int(port), "security": SECURITY[socket], "username": username, "auth": auth}


def parse_provider(name, data, stats):
    """The provider of one ispdb XML file, or None if nothing usable is left."""
    if b"<!DOCTYPE" in data or b"<!ENTITY" in data:
        stats.drop("file declaring a DOCTYPE or an entity", name)
        return None
    try:
        root = ET.fromstring(data)
    except ET.ParseError as error:
        stats.drop(f"invalid XML ({error})", name)
        return None
    provider = root.find("emailProvider")
    if root.tag != "clientConfig" or provider is None:
        stats.drop("no <emailProvider>", name)
        return None
    provider_id = (provider.get("id") or Path(name).stem).lower()
    domains = []
    for d in provider.findall("domain"):
        domain = (d.text or "").strip().lower().rstrip(".")
        if HOSTNAME.match(domain):
            if domain not in domains:
                domains.append(domain)
        else:
            stats.drop("invalid domain", f"{provider_id}: {domain}")
    servers = {"imap": [], "smtp": []}
    for tag, kind in KINDS.items():
        for element in provider.findall(tag):
            if element.get("type") != kind:
                continue
            server = parse_server(element, kind, provider_id, stats)
            if server is not None and server not in servers[kind]:
                servers[kind].append(server)
    if not domains:
        stats.drop("provider without domain", provider_id)
        return None
    if not servers["imap"]:
        stats.drop("provider without IMAP server (POP3 or Exchange only)", provider_id)
        return None
    for imap in servers["imap"]:
        if not imap["username"]:
            imap["username"] = "%EMAILADDRESS%"
    for smtp in servers["smtp"]:
        if not smtp["username"]:
            smtp["username"] = servers["imap"][0]["username"]
    name_ = text(provider, "displayName") or provider_id
    return {"id": provider_id, "name": name_, "domains": domains, "imap": servers["imap"], "smtp": servers["smtp"]}


def convert(archive_bytes, stats):
    """(commit, providers) from the gzipped tar archive of the repository."""
    providers = []
    seen_domains = {}
    commit = ""
    with tarfile.open(fileobj=io.BytesIO(archive_bytes), mode="r:gz") as tar:
        for member in tar:
            parts = member.name.split("/")
            if not commit and parts[0].startswith("autoconfig-"):
                commit = parts[0][len("autoconfig-"):]
            # <top>/ispdb/<file>.xml, a regular file: nothing else is read.
            if len(parts) != 3 or parts[1] != "ispdb" or not parts[2].endswith(".xml") or not member.isfile():
                continue
            if member.size > MAX_FILE_BYTES:
                stats.drop("file too large", parts[2])
                continue
            data = tar.extractfile(member).read()
            provider = parse_provider(parts[2], data, stats)
            if provider is None:
                continue
            # A domain claimed by two providers: the first one (in file
            # order, sorted below) keeps it.
            kept = []
            for domain in provider["domains"]:
                if domain in seen_domains:
                    stats.drop("domain already given by another provider", f"{domain} ({provider['id']}, kept in {seen_domains[domain]})")
                else:
                    seen_domains[domain] = provider["id"]
                    kept.append(domain)
            provider["domains"] = kept
            if kept:
                providers.append(provider)
    providers.sort(key=lambda p: p["id"])
    return commit, providers


def fetch():
    request = urllib.request.Request(ARCHIVE_URL, headers={"User-Agent": "niceMailer-autoconfig"})
    try:
        with urllib.request.urlopen(request, timeout=FETCH_TIMEOUT_SECS) as response:
            data = response.read(MAX_ARCHIVE_BYTES + 1)
    except OSError as error:
        fail(f"{ARCHIVE_URL}: {error}")
    if len(data) > MAX_ARCHIVE_BYTES:
        fail(f"{ARCHIVE_URL}: archive larger than {MAX_ARCHIVE_BYTES} bytes")
    return data


def render(commit, providers):
    document = {
        "_comment": "Generated by scripts/update-autoconfig.py (make autoconfig) from Thunderbird's ISP database. "
                    "Do not edit by hand: our own providers go to data/providers/local/.",
        "source": SOURCE,
        "commit": commit,
        "license": "MPL-2.0",
        "providers": providers,
    }
    return json.dumps(document, indent=1, ensure_ascii=False, sort_keys=False) + "\n"


def previous_ids():
    if not OUTPUT.exists():
        return set()
    try:
        return {p["id"] for p in json.loads(OUTPUT.read_text(encoding="utf-8"))["providers"]}
    except (ValueError, KeyError, TypeError):
        return set()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--force", action="store_true", help="write even if many providers disappear")
    parser.add_argument("--dry-run", action="store_true", help="print the changes without writing")
    parser.add_argument("--tarball", type=Path, help="read this archive instead of downloading it")
    args = parser.parse_args()

    archive = args.tarball.read_bytes() if args.tarball else fetch()
    stats = Stats()
    commit, providers = convert(archive, stats)
    print(f"{SOURCE} at {commit or '?'}: {len(providers)} providers kept, "
          f"{sum(len(p['domains']) for p in providers)} domains")
    stats.report()
    if len(providers) < MIN_PROVIDERS:
        fail(f"{len(providers)} providers, at least {MIN_PROVIDERS} expected: is the archive the right one?")

    ids = {p["id"] for p in providers}
    before = previous_ids()
    added, removed = ids - before, before - ids
    print(f"changes: {len(added)} providers added, {len(removed)} removed")
    if removed:
        print(f"  removed: {', '.join(sorted(removed))}")
    if before and len(removed) > len(before) * MAX_REMOVED_RATIO and not args.force:
        fail(f"{len(removed)} providers removed, more than {MAX_REMOVED_RATIO:.0%}: check the source, then --force")
    if args.dry_run:
        return
    text_ = render(commit, providers)
    if OUTPUT.exists() and OUTPUT.read_text(encoding="utf-8") == text_:
        print("unchanged")
        return
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    temporary = OUTPUT.with_suffix(".tmp")
    temporary.write_text(text_, encoding="utf-8")
    os.replace(temporary, OUTPUT)
    print(f"written: {OUTPUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
