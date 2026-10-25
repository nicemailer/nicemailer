#!/usr/bin/env python3
"""Régénère data/public-domains/public-domains.txt.

La liste sert à interdire les règles de domaine sur les messageries publiques
(gmail.com, orange.fr…) : approuver un tel domaine approuverait des millions
d'inconnus.

Calcul :
  + la liste HubSpot des domaines de messagerie gratuits ;
  - les domaines morts relevés chaque jour par DNS dans Kikobeats/free-email-domains (MIT) ;
  + data/public-domains/extra.txt, tenu à la main ;
  - data/public-domains/exclude.txt.

Les listes de domaines jetables (que Kikobeats agrège aussi) sont volontairement
écartées : personne ne demandera une règle de domaine pour mailinator.com, et ces
milliers de domaines éphémères rendraient la liste illisible.

Le fichier n'est réécrit que si toutes les sources ont répondu et si la liste
ne perd pas plus de MAX_REMOVED_RATIO de ses entrées (sauf --force).

Usage : scripts/update-public-domains.py [--force] [--dry-run]
"""

import argparse
import json
import os
import re
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DATA_DIR = ROOT / "data" / "public-domains"
OUTPUT = DATA_DIR / "public-domains.txt"
EXTRA = DATA_DIR / "extra.txt"
EXCLUDE = DATA_DIR / "exclude.txt"

FREE_DOMAINS = {
    "name": "HubSpot free domains",
    "url": "https://f.hubspotusercontent40.net/hubfs/2832391/Marketing/Lead-Capture/free-domains-2.csv",
    "format": "csv",
    "min_domains": 3000,
}

INACTIVE_DOMAINS = {
    "name": "Kikobeats inactive domains",
    "url": "https://raw.githubusercontent.com/Kikobeats/free-email-domains/master/inactive-domains.json",
    "format": "json",
    "min_domains": 100,
}

MAX_REMOVED_RATIO = 0.05
FETCH_TIMEOUT_SECS = 30
HOSTNAME = re.compile(r"^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)+$")

HEADER = """\
# Domaines de messagerie publics : une règle de domaine ne peut pas les viser.
# Fichier généré par scripts/update-public-domains.py, ne pas modifier à la main.
# Ajouts : data/public-domains/extra.txt. Retraits : data/public-domains/exclude.txt.
"""


def fail(message):
    print(f"erreur : {message}", file=sys.stderr)
    sys.exit(1)


def read_list(path):
    """Lit un fichier d'un domaine par ligne, en ignorant commentaires et lignes vides."""
    if not path.exists():
        return set()
    domains = set()
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        entry = line.strip()
        if not entry or entry.startswith("#"):
            continue
        if not HOSTNAME.match(entry):
            fail(f"{path.relative_to(ROOT)}:{number} : domaine invalide « {entry} »")
        domains.add(entry)
    return domains


def fetch(source):
    request = urllib.request.Request(source["url"], headers={"User-Agent": "niceMailer-public-domains"})
    try:
        with urllib.request.urlopen(request, timeout=FETCH_TIMEOUT_SECS) as response:
            text = response.read().decode("utf-8-sig")
    except OSError as error:
        fail(f"{source['name']} : {error}")

    if "<html" in text[:1000].lower():
        fail(f"{source['name']} : page HTML reçue au lieu d'une liste")

    if source["format"] == "json":
        entries = json.loads(text)
    else:
        entries = re.split(r"[,\r\n]+", text)

    domains = {entry.strip().lower() for entry in entries}
    domains = {domain for domain in domains if HOSTNAME.match(domain)}
    if len(domains) < source["min_domains"]:
        fail(f"{source['name']} : {len(domains)} domaines, au moins {source['min_domains']} attendus")
    return domains


def sample(domains, limit=15):
    shown = sorted(domains)[:limit]
    suffix = f" … (+{len(domains) - limit})" if len(domains) > limit else ""
    return ", ".join(shown) + suffix


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--force", action="store_true", help="écrire même si la liste perd beaucoup d'entrées")
    parser.add_argument("--dry-run", action="store_true", help="afficher les changements sans écrire")
    args = parser.parse_args()

    extra = read_list(EXTRA)
    exclude = read_list(EXCLUDE)

    free = fetch(FREE_DOMAINS)
    inactive = fetch(INACTIVE_DOMAINS)
    upstream = free - inactive
    print(f"{FREE_DOMAINS['name']} : {len(free)} domaines, dont {len(free & inactive)} morts écartés")

    missing_upstream = extra - upstream
    if missing_upstream:
        print(f"absents des sources, ajoutés par extra.txt ({len(missing_upstream)}) : {sample(missing_upstream)}")

    domains = (upstream | extra) - exclude
    previous = read_list(OUTPUT)
    added = domains - previous
    removed = previous - domains

    print(f"total : {len(domains)} domaines, {len(added)} ajoutés, {len(removed)} retirés")
    if added:
        print(f"  ajoutés : {sample(added)}")
    if removed:
        print(f"  retirés : {sample(removed)}")

    if previous and len(removed) > len(previous) * MAX_REMOVED_RATIO and not args.force:
        fail(
            f"{len(removed)} domaines retirés, soit plus de {MAX_REMOVED_RATIO:.0%} de la liste ; "
            "vérifier les sources puis relancer avec --force"
        )

    if args.dry_run or (not added and not removed):
        return

    temporary = OUTPUT.with_suffix(".tmp")
    temporary.write_text(HEADER + "".join(f"{domain}\n" for domain in sorted(domains)), encoding="utf-8")
    os.replace(temporary, OUTPUT)
    print(f"écrit : {OUTPUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
