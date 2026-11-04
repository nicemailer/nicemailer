#!/usr/bin/env python3
"""Export every Apple Mail store on this Mac to mbox files, straight from disk.

Why: Apple Mail's own "Export Mailbox" fails on IMAP accounts whose server no
longer exists (it apparently tries to fetch what it never downloaded). This
script reads Mail's files directly and never contacts any server. It never
writes into ~/Library/Mail.

Output (default: <project>/mail-export/):
  - one mbox per IMAP account, all its folders merged:
        <Account name> (<address>).mbox
    each message carries an "X-Mail-Folder:" header per folder it was found
    in, so the original folder structure is not lost;
  - one mbox per local folder ("On My Mac" / "Sur mon Mac"), the folder tree
    being reproduced as directories:
        Sur mon Mac/Pro/Clients/briq.mbox
        Sur mon Mac/Pro/Clients/briq/vinot.mbox

How Mail stores messages (~/Library/Mail/V10/<account-uuid>/<Folder>.mbox/):
  - <id>.emlx: a byte count line, the raw RFC 822 message, then an XML plist
    holding Mail's flags (read, flagged...). Written as is.
  - <id>.partial.emlx: same, but Mail removed the body of some MIME parts
    (usually attachments) and marked them with "X-Apple-Content-Length". If
    Mail downloaded the attachment, it sits in
    Attachments/<id>/<IMAP part number>/<file name>; it is put back. If not,
    the attachment is lost for good (the server is gone): the part becomes a
    short text note and gets an "X-Missing-Attachment: true" header, so the
    loss is visible instead of an empty attachment.

Dates: the plist of each .emlx holds "date-received", when the server
received the message (IMAP's INTERNALDATE). It is written in the mbox
"From " separator line, which mbox readers (and NiceMailer's import) take as
the reception date. Without it, Python's mailbox module would write the time
of the export there, and every message would look received today.

Duplicates: inside one IMAP account the same message often exists several
times (Gmail's "All Mail" holds a copy of every message, Archive/INBOX
copies...). Copies sharing their Message-ID, Date and Subject headers are
written once (Message-ID alone is not unique in practice), choosing the most
complete one (a full .emlx over a partial one, then the one with the most
attachments on disk). Messages without a Message-ID are always kept. Local
folders are not deduplicated: each is its own mbox.

Usage: scripts/export-apple-mail.py [--mail-dir DIR] [--output DIR] [--only UUID ...]
"""

import argparse
import base64
import email
import email.policy
import email.utils
import mailbox
import plistlib
import re
import sqlite3
import sys
import time
import unicodedata
from collections import Counter, defaultdict
from email.generator import BytesGenerator
from io import BytesIO
from pathlib import Path
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parent.parent
HOME = Path.home()
DEFAULT_MAIL_DIR = HOME / "Library" / "Mail" / "V10"
ACCOUNTS_DB = HOME / "Library" / "Accounts" / "Accounts4.sqlite"
UUID_RE = re.compile(r"^[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}$")


# --- Reading Mail's files -------------------------------------------------

def read_emlx(path):
    """Return (raw RFC 822 bytes, metadata dict) of an .emlx file."""
    data = path.read_bytes()
    nl = data.index(b"\n")
    length = int(data[:nl].strip())
    raw = data[nl + 1 : nl + 1 + length]
    try:
        meta = plistlib.loads(data[nl + 1 + length :])
    except Exception:
        meta = {}  # a damaged trailer only loses the read/flagged state
    return raw, meta


def dedup_key(path):
    """(Message-ID, Date, Subject) read from the header block only, without parsing the
    body: this pass runs on every message and must stay cheap.

    Message-ID alone is not enough: some clients (old Outlook, the
    "<!&!AAAA...>" ids) reuse one id for different messages, and a sender may
    even reuse id and Date for two versions of a newsletter. Whitespace is
    removed so that a header folded differently still matches. When a server
    rewrote the subject of one copy ("{Spam?} ...") both copies are kept: a
    duplicate is harmless, a lost message is not. None without Message-ID."""
    with path.open("rb") as f:
        f.readline()  # byte count line
        head = b""
        while len(head) < 256_000:
            chunk = f.read(16_384)
            if not chunk:
                break
            head += chunk
            if b"\n\n" in head or b"\r\n\r\n" in head:
                break
    head = re.split(rb"\r?\n\r?\n", head, maxsplit=1)[0]

    def header(name):
        # A header value may continue on indented lines ("folding").
        m = re.search(rb"^" + name + rb":[ \t]*(.*(?:\r?\n[ \t].*)*)", head, re.I | re.M)
        return re.sub(rb"\s+", b"", m.group(1)) if m else b""

    mid = header(b"message-id")
    return (mid, header(b"date"), header(b"subject")) if mid else None


def leaves(part, pid=""):
    """Yield (IMAP part number, part) for each non-multipart part. IMAP numbers
    parts 1, 2, 2.1... which is the naming of Mail's Attachments/ folders."""
    if part.is_multipart():
        for i, sub in enumerate(part.get_payload(), 1):
            yield from leaves(sub, f"{pid}.{i}" if pid else str(i))
    else:
        yield pid or "1", part


def attachment_dir(path):
    return path.parent.parent / "Attachments" / path.name.split(".")[0]


def attachments_on_disk(path):
    d = attachment_dir(path)
    return sum(1 for f in d.rglob("*") if f.is_file()) if d.is_dir() else 0


def repair_partial(raw, attdir, stats):
    """Put back the attachments Mail stored aside; mark the lost ones."""
    msg = email.message_from_bytes(raw, policy=email.policy.compat32)
    changed = False
    for pid, part in leaves(msg):
        size = part.get("X-Apple-Content-Length")
        if size is None:
            continue
        changed = True
        del part["X-Apple-Content-Length"]
        d = attdir / pid
        files = sorted(f for f in d.iterdir() if f.is_file()) if d.is_dir() else []
        if files:
            # The file on disk is decoded content: re-encode it in base64,
            # whatever the original transfer encoding was.
            del part["Content-Transfer-Encoding"]
            part["Content-Transfer-Encoding"] = "base64"
            part.set_payload(base64.encodebytes(files[0].read_bytes()).decode("ascii"))
            stats["restored"] += 1
        else:
            name = part.get_filename() or part.get_content_type()
            for h in ("Content-Type", "Content-Transfer-Encoding", "Content-Disposition", "Content-Id"):
                del part[h]
            part["Content-Type"] = "text/plain"
            part["X-Missing-Attachment"] = "true"
            # Passing a charset lets the email package pick a safe transfer
            # encoding for non-ASCII file names.
            part.set_payload(
                f"[Attachment not available: {name} ({size} bytes). "
                "Apple Mail never downloaded it and the server no longer exists.]\n",
                charset="utf-8",
            )
            stats["missing"] += 1
    if not changed:
        return raw
    buf = BytesIO()
    BytesGenerator(buf, mangle_from_=False, policy=email.policy.compat32).flatten(msg)
    return buf.getvalue()


def to_mbox_bytes(path, folders, stats):
    raw, meta = read_emlx(path)
    if path.name.endswith(".partial.emlx"):
        raw = repair_partial(raw, attachment_dir(path), stats)
    # Mail flags: bit 0 = read, bit 4 = flagged. Status/X-Status are the
    # headers mbox readers (Mail, Thunderbird, mutt) understand.
    flags = int(meta.get("flags", 0))
    extra = [b"Status: " + (b"RO" if flags & 1 else b"O")]
    if flags & (1 << 4):
        extra.append(b"X-Status: F")
    extra += [b"X-Mail-Folder: " + f.encode("utf-8") for f in folders]
    # The bytes start with their own "From " line: mailbox.mbox.add() then
    # writes it as the separator instead of one dated now.
    return from_line(reception_time(path, raw, meta)) + b"\n".join(extra) + b"\n" + raw.replace(b"\r\n", b"\n")


def reception_time(path, raw, meta):
    """Unix time the message was received: Mail's "date-received", else the
    Date header, else the time Mail wrote the file (close to the download)."""
    try:
        t = int(meta.get("date-received", 0))
        if t > 0:
            return t
    except (TypeError, ValueError):
        pass
    head = re.split(rb"\r?\n\r?\n", raw, maxsplit=1)[0]
    m = re.search(rb"^date:[ \t]*(.*(?:\r?\n[ \t].*)*)", head, re.I | re.M)
    if m:
        try:
            value = re.sub(rb"\s+", b" ", m.group(1)).decode("ascii", "replace").strip()
            return int(email.utils.parsedate_to_datetime(value).timestamp())
        except (TypeError, ValueError, IndexError, OverflowError):
            pass
    return int(path.stat().st_mtime)


def from_line(t):
    """mbox separator line dated t, in the asctime format, in UTC."""
    return b"From MAILER-DAEMON " + time.asctime(time.gmtime(t)).encode("ascii") + b"\n"


# --- Discovering stores and folders --------------------------------------

def folder_name(mbox_dir, store):
    """'Pro.mbox/Clients.mbox' -> 'Pro/Clients' (NFC: macOS file names are
    decomposed, 'é' would otherwise be 'e' + combining accent)."""
    rel = mbox_dir.relative_to(store)
    return unicodedata.normalize("NFC", "/".join(p[:-5] for p in rel.parts))


def folders(store):
    """Every *.mbox folder of a store, nested ones included."""
    return sorted(store.rglob("*.mbox"))


def message_files(mbox_dir):
    """Messages of this folder only, not of its nested *.mbox children."""
    for p in mbox_dir.rglob("*.emlx"):
        rel = p.relative_to(mbox_dir)
        if not any(x.endswith(".mbox") for x in rel.parts[:-1]):
            yield p


def message_number(p):
    return int(p.name.split(".")[0])


def account_names():
    try:
        con = sqlite3.connect(f"file:{ACCOUNTS_DB}?mode=ro", uri=True)
        rows = con.execute("select ZIDENTIFIER, ZACCOUNTDESCRIPTION, ZUSERNAME from ZACCOUNT").fetchall()
    except sqlite3.Error:
        return {}
    return {r[0]: (r[1], r[2]) for r in rows}


def local_store_ids(mail_dir):
    """Stores whose folders Mail indexes as local:// ("On My Mac")."""
    try:
        con = sqlite3.connect(f"file:{mail_dir / 'MailData' / 'Envelope Index'}?mode=ro", uri=True)
        urls = [r[0] for r in con.execute("select url from mailboxes where url like 'local://%'")]
    except sqlite3.Error:
        return set()
    return {u.split("/")[2] for u in urls}


def guess_address(store, sample=200):
    """For a deleted account (no longer in Accounts4): the most common
    Delivered-To / X-Original-To / To address of its messages."""
    counts = Counter()
    for i, p in enumerate(store.rglob("*.emlx")):
        if i >= sample:
            break
        try:
            head = p.read_bytes()[:20_000]
        except OSError:
            continue
        m = re.search(rb"^(?:Delivered-To|X-Original-To|To):.*?([\w.+-]+@[\w.-]+)", head, re.I | re.M)
        if m:
            counts[m.group(1).decode("ascii", "replace").lower()] += 1
    return counts.most_common(1)[0][0] if counts else None


def safe(name):
    return re.sub(r'[/:\\]', "_", name).strip() or "unnamed"


# --- Export ----------------------------------------------------------------

def write_mbox(target, items):
    """items: iterable of (emlx path, folder names). Returns the stats."""
    stats = Counter()
    target.parent.mkdir(parents=True, exist_ok=True)
    tmp = target.with_name(target.name + ".tmp")
    if tmp.exists():
        tmp.unlink()
    mb = mailbox.mbox(str(tmp), create=True)
    mb.lock()
    try:
        for path, names in items:
            try:
                mb.add(to_mbox_bytes(path, names, stats))
                stats["messages"] += 1
            except Exception as e:  # one bad file must not stop the export
                stats["errors"] += 1
                print(f"  ERROR {path}: {e}", file=sys.stderr)
        mb.flush()
    finally:
        mb.unlock()
        mb.close()
    # Write to a temporary name then rename: an interrupted run never leaves
    # a truncated mbox that looks complete.
    tmp.replace(target)
    return stats


def export_account(store, target):
    """All folders of an IMAP account in one mbox, deduplicated."""
    by_id = defaultdict(list)  # (Message-ID, Date, Subject) -> [(path, folder)]
    singles = []
    for d in folders(store):
        name = folder_name(d, store)
        for p in message_files(d):
            key = dedup_key(p)
            (by_id[key].append((p, name)) if key else singles.append((p, name)))
    items, dupes = [], 0
    for copies in by_id.values():
        best = max(copies, key=lambda c: (not c[0].name.endswith(".partial.emlx"),
                                          attachments_on_disk(c[0]), c[0].stat().st_size))
        names = sorted({c[1] for c in copies})
        items.append((best[0], names))
        dupes += len(copies) - 1
    items += [(p, [n]) for p, n in singles]
    # Stable order: by folder, then by Mail's message number (≈ arrival order).
    items.sort(key=lambda it: (it[1][0], message_number(it[0])))
    stats = write_mbox(target, items)
    stats["duplicates"] = dupes
    return stats


def export_local(store, out_dir):
    """One mbox per local folder, the tree reproduced as directories."""
    total = Counter()
    for d in folders(store):
        files = sorted(message_files(d), key=message_number)
        if not files:
            continue
        name = folder_name(d, store)
        target = out_dir.joinpath(*[safe(x) for x in name.split("/")])
        target = target.with_name(target.name + ".mbox")
        stats = write_mbox(target, [(p, [name]) for p in files])
        print(f"    {name:60} {stats['messages']:6} msgs")
        total.update(stats)
    return total


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--mail-dir", type=Path, default=DEFAULT_MAIL_DIR)
    ap.add_argument("--output", type=Path, default=ROOT / "mail-export")
    ap.add_argument("--only", nargs="*", help="store UUIDs to export (default: all)")
    args = ap.parse_args()

    names = account_names()
    local_ids = local_store_ids(args.mail_dir)
    stores = sorted(p for p in args.mail_dir.iterdir() if p.is_dir() and UUID_RE.match(p.name))
    if args.only:
        stores = [s for s in stores if s.name in args.only]
    args.output.mkdir(parents=True, exist_ok=True)

    grand = Counter()
    for store in stores:
        uuid = store.name
        desc, user = names.get(uuid, (None, None))
        if uuid in local_ids:
            label = desc or "On My Mac"
            print(f"{label} [{uuid}] (local folders)")
            stats = export_local(store, args.output / safe(label))
        else:
            if desc:
                label = f"{desc} ({user})" if user else desc
            else:
                addr = guess_address(store)
                label = f"Deleted account {uuid[:8]}" + (f" ({addr})" if addr else "")
            stats = export_account(store, args.output / f"{safe(label)}.mbox")
            print(f"{label} [{uuid}]")
        print(f"    -> {stats['messages']} msgs, {stats['duplicates']} duplicates skipped, "
              f"{stats['restored']} attachments restored, {stats['missing']} missing, "
              f"{stats['errors']} errors")
        grand.update(stats)
    print("TOTAL", dict(grand))
    return 1 if grand["errors"] else 0


if __name__ == "__main__":
    sys.exit(main())
