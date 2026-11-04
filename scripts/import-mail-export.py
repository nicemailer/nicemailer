#!/usr/bin/env python3
"""Import every mbox of mail-export/ (made by scripts/export-apple-mail.py)
into NiceMailer, through the `nicemailer import-mbox` CLI.

Each mbox needs an archive account name and the address of the mailbox
(`--account`, `--address`, see docs/import-mbox.md). The script works in two
phases so that the user only has to be there at the start:

1. Plan (interactive). Every mbox is looked at:
   - "<Account> (<address>).mbox" (one per IMAP account): the address is in
     the file name and the name comes from the account; nothing to ask unless
     the name is taken or the account was a deleted one;
   - "Sur mon Mac/Messages récupérés (<Account>).mbox": goes with <Account>;
   - a file name holding an address ("HEY-emails-me@hey.com.mbox"): the
     address is known, the name is asked (with a suggestion);
   - any other local folder: local folders mix several of the user's
     addresses, so the script counts which of them sent the messages and asks
     once per top-level local folder ("Sur mon Mac/Pro"...), with that count
     and a suggestion; the user can also decide file by file.
   Answers are saved to <export-dir>/import-plan.json and reused on the next
   run (only new files are asked about). A recap is shown, then one
   confirmation.
2. Import (unattended). The core runs one import at a time, so the files are
   imported one after the other; a failure is logged and the next file goes
   on. The full CLI output goes to <export-dir>/import.log.

Running the script again is safe: the core skips the messages already
imported (same bytes), so an interrupted run is resumed by running it again.

The core must be running (`make core`).

Usage: scripts/import-mail-export.py [--export-dir DIR] [--data-dir DIR]
           [--cli PATH] [--replan] [--dry-run] [--yes]
"""

import argparse
import json
import os
import re
import sqlite3
import subprocess
import sys
import time
import unicodedata
from collections import Counter, defaultdict
from dataclasses import asdict, dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOCAL_ROOT = "Sur mon Mac"  # folder made by export-apple-mail.py for "On My Mac"
# Same rule as Store::prepare_archive_account (nm-store/src/import.rs).
NAME_RE = re.compile(r"^[A-Za-z0-9._-]{1,64}$")
ADDR_RE = re.compile(rb"[\w.+-]+@[\w-]+(?:\.[\w-]+)+")
TOP_FILE_RE = re.compile(r"^(?P<label>.*) \((?P<addr>[^()\s]+@[^()\s]+)\)\.mbox$")
RECOVERED_RE = re.compile(r"^Messages récupérés \((?P<what>.+)\)\.mbox$")
SKIP = "-"


@dataclass
class Item:
    """One mbox file and what it will be imported as."""
    path: str                 # relative to the export dir
    size: int
    account: str = ""
    address: str = ""
    evident: bool = False     # decided by the script, nothing to ask
    reason: str = ""          # why, shown in the recap
    group: str = ""           # local folders: top-level local folder
    senders: dict = field(default_factory=dict)  # own address -> messages sent
    follows: str = ""         # path of the account mbox this file goes with

    @property
    def skipped(self):
        return self.account == SKIP


# --- Small helpers ----------------------------------------------------------

def slug(text):
    """'Achille Sh' -> 'achille-sh', 'gérance' -> 'gerance': account names
    only allow ASCII letters, digits, - _ ."""
    text = unicodedata.normalize("NFKD", text).encode("ascii", "ignore").decode()
    text = re.sub(r"[^a-z0-9._-]+", "-", text.lower()).strip("-.")
    return text[:64] or "archive"


def nfc(text):
    # macOS gives decomposed file names ('e' + accent); compare in NFC.
    return unicodedata.normalize("NFC", text)


def human(n):
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024 or unit == "GB":
            return f"{n:.0f} {unit}" if unit == "B" else f"{n:.1f} {unit}"
        n /= 1024


def ask(prompt, default=""):
    try:
        answer = input(f"{prompt} [{default}]: " if default else f"{prompt}: ").strip()
    except EOFError:
        # Input closed (piped answers ran out, Ctrl-D): stop rather than
        # loop on an invalid default. Nothing has been imported yet.
        print()
        sys.exit("Input closed before all questions were answered: nothing imported.")
    return answer or default


# --- What NiceMailer already has -------------------------------------------

def data_dir(arg):
    if arg:
        return Path(arg)
    if os.environ.get("NICEMAILER_DATA_DIR"):
        return Path(os.environ["NICEMAILER_DATA_DIR"])
    return Path.home() / "Library" / "Application Support" / "NiceMailer"


def existing_accounts(ddir):
    """config_key -> (kind, address). Read-only: the core owns the database."""
    db = ddir / "nicemailer.db"
    if not db.exists():
        return {}
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    try:
        return {k: (kind, (a or "").lower()) for k, kind, a in
                con.execute("SELECT config_key, kind, address FROM account")}
    finally:
        con.close()


# --- Reading the mbox files -------------------------------------------------

def count_senders(path, own):
    """Count who sent the messages of an mbox (From header): among the user's
    own addresses, and among all addresses (a hint when none of the own
    addresses shows up). Also returns the number of messages. Only header
    blocks are looked at; the file is read as a stream."""
    sent, anyone, total = Counter(), Counter(), 0
    with open(path, "rb") as f:
        in_head, head = False, []
        for line in f:
            if line.startswith(b"From "):
                total += 1
                in_head, head = True, []
            elif in_head:
                if line in (b"\n", b"\r\n"):
                    in_head = False
                    block = b"".join(head)
                    m = re.search(rb"^from:(.*(?:\n[ \t].*)*)", block, re.I | re.M)
                    if m:
                        found = [a.decode("ascii", "replace").lower() for a in ADDR_RE.findall(m.group(1))]
                        if found:
                            anyone[found[0]] += 1
                        for a in found:
                            if a in own:
                                sent[a] += 1
                                break
                elif len(head) < 400:  # a header block is never that long
                    head.append(line)
    return sent, anyone, total


def header_addresses(path, limit=300):
    """Every address of the To/Cc/Delivered-To/X-Original-To/From headers of
    the first `limit` messages, with their count."""
    seen, n = Counter(), 0
    with open(path, "rb") as f:
        in_head, head = False, []
        for line in f:
            if line.startswith(b"From "):
                n += 1
                if n > limit:
                    break
                in_head, head = True, []
            elif in_head:
                if line in (b"\n", b"\r\n"):
                    in_head = False
                    block = b"".join(head)
                    for m in re.finditer(rb"^(?:to|cc|delivered-to|x-original-to|from):(.*(?:\n[ \t].*)*)",
                                         block, re.I | re.M):
                        seen.update(a.decode("ascii", "replace").lower() for a in ADDR_RE.findall(m.group(1)))
                elif len(head) < 400:
                    head.append(line)
    return seen


def address_in_name(stem, path):
    """The address in a file name such as 'HEY-emails-me@hey.com', or None.

    The local part of an address may contain '-', so the regex alone takes
    'HEY-emails-me@hey.com'. Each suffix after a '-', '_' or space is a
    candidate ('me@hey.com', 'emails-me@hey.com'...); the one that appears
    most in the messages' headers wins, the shortest one if none appears."""
    m = ADDR_RE.search(stem.encode())
    if not m:
        return None
    full = m.group(0).decode().lower()
    local = full.split("@", 1)[0]
    # Suffixes of `full` starting after each separator of the local part.
    starts = [0] + [i + 1 for i, c in enumerate(local) if c in "-_ "]
    candidates = [full[s:] for s in starts if full[s] != "@"]
    seen = header_addresses(path)
    return max(candidates, key=lambda c: (seen[c], -len(c)))

class Planner:
    def __init__(self, export_dir, accounts):
        self.export_dir = export_dir
        self.accounts = accounts          # existing NiceMailer accounts
        self.planned = {}                 # account name -> address, this plan

    def name_problem(self, name, address):
        """Why `name` cannot be used for `address`, or None."""
        if not NAME_RE.match(name):
            return "letters, digits, - _ . only, 64 characters at most"
        kind, addr = self.accounts.get(name, (None, None))
        if kind and kind != "archive":
            return f'"{name}" is already used by an account of kind {kind} in NiceMailer'
        if kind == "archive" and addr != address:
            return f'archive account "{name}" already has the address {addr}'
        if self.planned.get(name, address) != address:
            return f'"{name}" is already planned with the address {self.planned[name]}'
        return None

    def archive_for(self, address):
        """Name of an existing (or planned) archive account with this address."""
        for name, (kind, addr) in self.accounts.items():
            if kind == "archive" and addr == address:
                return name
        for name, addr in self.planned.items():
            if addr == address:
                return name
        return None

    def free_name(self, base, address):
        """`base`, or `base-archive`, `base-archive2`... if taken."""
        for cand in [base, f"{base}-archive"] + [f"{base}-archive{i}" for i in range(2, 50)]:
            if not self.name_problem(cand, address):
                return cand
        return base

    def commit(self, item):
        if not item.skipped:
            self.planned[item.account] = item.address


def propose(items, planner):
    """Fill account/address for what can be decided without the user."""
    by_label = {}  # account label or address -> (name, address), for "Messages récupérés"
    top = [i for i in items if "/" not in i.path]
    local = [i for i in items if "/" in i.path]

    for it in top:
        m = TOP_FILE_RE.match(nfc(it.path))
        if not m:
            it.reason = "no address in the file name"
            continue
        label, addr = m["label"], m["addr"].lower()
        it.address = addr
        existing = planner.archive_for(addr)
        if existing:
            it.account, it.evident, it.reason = existing, True, "existing archive account with this address"
        elif label.startswith("Deleted account"):
            it.account = planner.free_name(slug(addr.replace("@", "-")), addr)
            it.reason = "account deleted from Apple Mail: confirm the name"
        else:
            base = slug(label)
            it.account = planner.free_name(base, addr)
            it.evident = it.account == base
            it.reason = "Apple Mail account" if it.evident else f'"{base}" is taken'
        if it.evident:
            planner.commit(it)
        by_label[label.lower()] = by_label[addr] = it

    for it in local:
        name = nfc(Path(it.path).name)
        parts = nfc(it.path).split("/")
        # Group = first folder under "Sur mon Mac"; loose files form their own group.
        it.group = "/".join(parts[:2]) if len(parts) > 2 else f"{LOCAL_ROOT} (loose files)"
        m = RECOVERED_RE.match(name)
        if m and m["what"].lower() in by_label:
            src = by_label[m["what"].lower()]
            # Same account as `src`, whose name may still change if the user
            # is asked about it: copied again after the questions.
            it.account, it.address, it.follows = src.account, src.address, src.path
            it.evident = True
            it.reason = f"recovered messages of {m['what']}"
            it.group = ""
            continue
        stem = name[:-5]  # without ".mbox", which would look like a domain
        addr = address_in_name(stem, planner.export_dir / it.path)
        if addr:
            it.address = addr
            it.group = ""
            existing = planner.archive_for(addr)
            if existing:
                it.account, it.evident, it.reason = existing, True, "existing archive account with this address"
            else:
                base = slug(re.sub(re.escape(addr), "", stem, flags=re.I)) or slug(addr.replace("@", "-"))
                it.account = planner.free_name(base, addr)
                it.reason = "address in the file name: confirm the name and the address"
            continue
    for it in local:
        if it.evident and not it.follows:
            planner.commit(it)


def ask_one(planner, title, account, address, senders, allow_per_file=False, others=None):
    """Ask account and address. Returns (account, address) or the string
    'per-file'. Loops until the answer is valid."""
    print(f"\n{title}")
    if senders is not None:  # None: the file was not read
        if senders:
            print("    sent from: " + ", ".join(f"{a} ({n})" for a, n in senders.most_common(5)))
        else:
            print("    sent from: none of your known addresses")
            if others:
                print("    most frequent senders: " + ", ".join(f"{a} ({n})" for a, n in others.most_common(3)))
    extra = ", 'f' to decide file by file" if allow_per_file else ""
    while True:
        acc = ask(f"    account name ('{SKIP}' to skip{extra})", account)
        if acc == SKIP:
            return SKIP, ""
        if allow_per_file and acc == "f":
            return "per-file"
        addr = ask("    address", address).lower()
        if "@" not in addr or " " in addr:
            print("    ! not an address")
            continue
        problem = planner.name_problem(acc, addr)
        if problem:
            print(f"    ! {problem}")
            account = planner.free_name(acc if NAME_RE.match(acc) else slug(acc), addr)
            address = addr
            continue
        return acc, addr


def interview(items, planner, export_dir, own):
    """Ask about everything that is not evident."""
    # Single files first (top-level accounts, files named after an address).
    singles = [i for i in items if not i.evident and not i.group]
    groups = defaultdict(list)
    for i in items:
        if not i.evident and i.group:
            groups[i.group].append(i)

    if singles or groups:
        print(f"\n{len(singles) + len(groups)} question(s). Press Enter to accept the suggestion in brackets.")

    for it in singles:
        title = f"{it.path}  ({human(it.size)})  - {it.reason}"
        it.account, it.address = ask_one(planner, title, it.account, it.address, None)
        it.evident = True
        planner.commit(it)
    by_path = {i.path: i for i in items}
    for it in items:
        if it.follows and it.follows in by_path:
            src = by_path[it.follows]
            it.account, it.address = src.account, src.address

    for g, members in groups.items():
        print(f"\nReading {g} ({len(members)} files)...", end="", flush=True)
        total_senders, total_others, total_msgs, others = Counter(), Counter(), 0, {}
        for it in members:
            sent, anyone, n = count_senders(export_dir / it.path, own)
            it.senders, others[it.path] = dict(sent), anyone
            total_senders.update(sent)
            total_others.update(anyone)
            total_msgs += n
        print(" done")
        best = total_senders or total_others
        address = best.most_common(1)[0][0] if best else ""
        base = slug(g.split("/")[-1].replace("(loose files)", "local"))
        account = planner.archive_for(address) or planner.free_name(base, address) if address else base
        title = (f"{g}: {len(members)} files, {total_msgs} messages, "
                 f"{human(sum(i.size for i in members))}")
        answer = ask_one(planner, title, account, address, total_senders, True, total_others)
        if answer == "per-file":
            for it in members:
                s = Counter(it.senders)
                addr = s.most_common(1)[0][0] if s else address
                acc = planner.archive_for(addr) or planner.free_name(base, addr)
                it.account, it.address = ask_one(planner, f"{it.path}  ({human(it.size)})", acc, addr, s,
                                                 others=others[it.path])
                it.evident = True
                planner.commit(it)
        else:
            for it in members:
                it.account, it.address = answer
                it.evident = True
                planner.commit(it)


def recap(items):
    print("\nPlan:")
    by_acc = defaultdict(list)
    for it in items:
        by_acc[(it.account, it.address)].append(it)
    for (acc, addr), its in sorted(by_acc.items()):
        size = human(sum(i.size for i in its))
        if acc == SKIP:
            print(f"  skipped: {len(its)} file(s), {size}")
            continue
        print(f"  {acc:28} {addr:30} {len(its):4} file(s)  {size:>9}")


# --- Phase 2: the imports ---------------------------------------------------

def cli_command(args):
    if args.cli:
        return [args.cli]
    exe = ROOT / "target" / "release" / "nicemailer"
    # Always (re)build: cargo does nothing when the binary is up to date, and
    # an old binary could speak an older protocol.
    print("Building the CLI (cargo build --release --bin nicemailer)...")
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "nicemailer"], cwd=ROOT, check=True)
    return [str(exe)]


def cli(base, args, *extra, **kw):
    cmd = base + (["--data-dir", str(args.data_dir)] if args.data_dir else []) + list(extra)
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def import_running(base, args):
    r = cli(base, args, "call", "import_status")
    if r.returncode != 0:
        return False
    try:
        status = json.loads(r.stdout)
    except json.JSONDecodeError:
        return False
    return bool(status) and status.get("running") is True


def run_imports(items, base, args, export_dir):
    todo = [i for i in items if not i.skipped]
    log_path = export_dir / "import.log"
    failures, started = [], time.monotonic()
    print(f"\nImporting {len(todo)} file(s); details in {log_path}\n")
    with open(log_path, "a", encoding="utf-8") as log:
        log.write(f"\n=== {time.strftime('%Y-%m-%d %H:%M:%S')} ===\n")
        for n, it in enumerate(todo, 1):
            print(f"[{n}/{len(todo)}] {it.account} <- {it.path} ({human(it.size)})", flush=True)
            for attempt in range(3):
                # One import at a time in the core: wait for a running one
                # (e.g. left by an earlier interrupted run) to finish.
                while import_running(base, args):
                    time.sleep(5)
                r = cli(base, args, "import-mbox", str(export_dir / it.path),
                        "--account", it.account, "--address", it.address)
                log.write(f"--- {it.path} -> {it.account} <{it.address}> (exit {r.returncode})\n")
                log.write(r.stdout + r.stderr.replace("\r", "\n") + "\n")
                log.flush()
                # The core's BUSY message is in French ("déjà en cours").
                if r.returncode == 0 or not re.search(r"already running|déjà en cours|1004", r.stderr):
                    break
            # The CLI's summary is the line after "Import N (finished)".
            lines = r.stdout.strip().splitlines()
            summary = lines[1].strip() if len(lines) > 1 else ""
            if r.returncode == 0:
                print(f"      {summary}")
            else:
                err = (r.stderr.strip().splitlines() or ["unknown error"])[-1]
                print(f"      FAILED: {err}")
                failures.append((it, err))
    minutes = (time.monotonic() - started) / 60
    print(f"\nDone in {minutes:.1f} min: {len(todo) - len(failures)} file(s) done, {len(failures)} failed.")
    for it, err in failures:
        print(f"  FAILED {it.path}: {err}")
    if failures:
        print("Run the script again to retry them (already imported messages are skipped).")
    return 1 if failures else 0


# --- Main -------------------------------------------------------------------

def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--export-dir", type=Path, default=ROOT / "mail-export")
    ap.add_argument("--data-dir", type=Path, help="NiceMailer data directory (default: NICEMAILER_DATA_DIR or ~/Library/Application Support/NiceMailer)")
    ap.add_argument("--cli", help="path of the nicemailer binary (default: build target/release/nicemailer)")
    ap.add_argument("--replan", action="store_true", help="ignore the saved plan and ask again")
    ap.add_argument("--dry-run", action="store_true", help="make the plan, do not import")
    ap.add_argument("--yes", action="store_true", help="do not ask for the final confirmation")
    args = ap.parse_args()
    export_dir = args.export_dir.resolve()
    plan_path = export_dir / "import-plan.json"

    # Check the core before asking anything: the user's time is the scarce
    # resource here.
    base = cli_command(args)
    if not args.dry_run:
        r = cli(base, args, "import-status")
        if r.returncode != 0:
            sys.exit(f"The core does not answer ({r.stderr.strip()}). Start it with `make core`.")

    files = sorted(p for p in export_dir.rglob("*.mbox") if p.is_file())
    items = [Item(path=str(p.relative_to(export_dir)), size=p.stat().st_size) for p in files]
    if not items:
        sys.exit(f"No .mbox file in {export_dir}")

    accounts = existing_accounts(data_dir(args.data_dir))
    planner = Planner(export_dir, accounts)

    saved = {}
    if plan_path.exists() and not args.replan:
        saved = {d["path"]: d for d in json.loads(plan_path.read_text())}
        print(f"Reusing the answers of {plan_path} (--replan to ask again).")
    for it in items:
        if it.path in saved:
            s = saved[it.path]
            it.account, it.address, it.evident, it.reason = s["account"], s["address"], True, s.get("reason", "saved plan")
            planner.commit(it)

    fresh = [i for i in items if i.path not in saved]
    propose(fresh, planner)
    # The user's own addresses: those of the accounts, used to tell who sent
    # the messages of local folders.
    own = {i.address for i in items if i.address} | {a for k, a in accounts.values() if a}
    interview(fresh, planner, export_dir, own)

    plan_path.write_text(json.dumps([asdict(i) for i in items], indent=1, ensure_ascii=False))
    recap(items)
    print(f"\nPlan saved to {plan_path}")
    if args.dry_run:
        return 0
    if not args.yes and ask("\nStart the imports now? You can leave, nothing more will be asked (y/n)", "y").lower() not in ("y", "yes", "o", "oui"):
        print("Nothing imported. Run the script again to start (the plan is kept).")
        return 0
    return run_imports(items, base, args, export_dir)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("\nStopped. An import already started goes on in the core; run the script again to resume.")
        sys.exit(130)
