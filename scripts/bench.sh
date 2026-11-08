#!/bin/zsh
# Times the thread lists of the core: folders, Everything, search and every
# attribute filter, through the real socket (`make bench`).
#
# By default it generates a mailbox of $THREADS conversations (100,000) in a
# temporary directory with `nicemailer seed`, starts a core on it, runs every
# scenario twice (the first run warms the cache) and prints the second.
# BENCH_DATA_DIR=<dir> runs on an existing data directory instead, for
# example a copy of a real mailbox (its core must not be running: this
# script starts its own). Each time includes starting the CLI (~5 ms).
#
# What to expect: every scenario well under 100 ms. A scenario that takes
# seconds means a query walks the whole mailbox (see the query plan tests
# in crates/nm-store/src/queries.rs).

set -euo pipefail
zmodload zsh/datetime

ROOT=${0:A:h:h}
CLI=$ROOT/target/release/nicemailer
CORE=$ROOT/target/release/nicemailer-core
THREADS=${THREADS:-100000}

cargo build --release --quiet --manifest-path "$ROOT/Cargo.toml"

# A short path: a Unix socket path is limited to about 100 characters.
if [[ -n ${BENCH_DATA_DIR:-} ]]; then
  DIR=$BENCH_DATA_DIR
else
  DIR=$(mktemp -d /tmp/nmbench.XXXXXX)
  print "Generating $THREADS conversations in $DIR..."
  "$CLI" --data-dir "$DIR" seed --threads "$THREADS" > /dev/null
fi

# Settings files of their own, in a directory of their own (the core
# creates config.toml and accounts.toml there, with no account), never the
# user's, whose accounts the core would start to synchronize.
"$CORE" --data-dir "$DIR" --config "$DIR.config/config.toml" > "$DIR.core.log" 2>&1 &
CORE_PID=$!
cleanup() {
  kill -INT $CORE_PID 2> /dev/null || true
  wait $CORE_PID 2> /dev/null || true
  rm -f "$DIR.core.log"
  rm -rf "$DIR.config"
  [[ -z ${BENCH_DATA_DIR:-} ]] && rm -rf "$DIR"
}
trap cleanup EXIT
# The first start of an older database migrates it: wait up to 10 minutes.
for i in {1..3000}; do [[ -S $DIR/core.sock ]] && break; sleep 0.2; done
[[ -S $DIR/core.sock ]] || { print -u2 "the core did not start: $(cat "$DIR.core.log")"; exit 1; }

call() { "$CLI" --data-dir "$DIR" call "$@"; }

# The biggest folder, the archives of every account, and the first account.
read -r BIG ARCHIVES ACCOUNT <<< "$(call list_folders '{}' | python3 -I -c '
import json, sys
f = json.load(sys.stdin)["items"]
big = max(f, key=lambda x: x["thread_count"])["id"]
arch = ",".join(str(x["id"]) for x in f if x["role"] == "archive")
print(big, arch, f[0]["account_id"])')"

# A sender of the generated mailbox (or the first contact of a real one).
CONTACT=$(call list_contacts '{"query":"energie-plus","limit":1}' | python3 -I -c '
import json, sys
items = json.load(sys.stdin)["items"]
print(items[0]["id"] if items else 1)')

E='"everything":{},"limit":60,"sort":"received"'
typeset -A P
P=(
  "01 biggest folder"          "{\"folder_id\":$BIG,\"limit\":60,\"sort\":\"received\"}"
  "02 biggest folder, by date of change" "{\"folder_id\":$BIG,\"limit\":60,\"sort\":\"modified\"}"
  "03 archives of all accounts" "{\"folder_ids\":[$ARCHIVES],\"limit\":60,\"sort\":\"received\"}"
  "04 everything"              "{$E}"
  "05 everything of an account" "{\"everything\":{\"account_id\":$ACCOUNT},\"limit\":60,\"sort\":\"received\"}"
  "06 everything, by date of change" "{\"everything\":{},\"limit\":60,\"sort\":\"modified\"}"
  "07 / frequent word"         "{$E,\"filter\":{\"contains\":\"facture\"}}"
  "08 / rare word"             "{$E,\"filter\":{\"contains\":\"zzyzx\"}}"
  "09 from an address"         "{$E,\"filter\":{\"from\":[\"factures@energie-plus.fr\"]}}"
  "10 from a domain"           "{$E,\"filter\":{\"from\":[\"@gmail.com\"]}}"
  "11 not from a domain"       "{$E,\"filter\":{\"not_from\":[\"@gmail.com\"]}}"
  "12 to a name"               "{$E,\"filter\":{\"to\":[\"martin\"]}}"
  "13 unread"                  "{$E,\"filter\":{\"seen\":false}}"
  "14 flagged"                 "{$E,\"filter\":{\"flagged\":true}}"
  "15 with attachments"        "{$E,\"filter\":{\"attachments\":true}}"
  "16 answered"                "{$E,\"filter\":{\"answered\":true}}"
  "17 subject"                 "{$E,\"filter\":{\"subject\":\"devis\"}}"
  "18 a month long ago"        "{$E,\"filter\":{\"after\":1230768000,\"before\":1233446400}}"
  "19 from a domain, long ago" "{$E,\"filter\":{\"from\":[\"@gmail.com\"],\"before\":1262304000}}"
  "20 biggest folder, flagged" "{\"folder_id\":$BIG,\"limit\":60,\"sort\":\"received\",\"filter\":{\"flagged\":true}}"
  "21 mails of a contact"      "{$E,\"filter\":{\"address_id\":$CONTACT}}"
  "22 a tag (none here)"       "{$E,\"filter\":{\"tag\":\"Clients\"}}"
)

OUT=$(mktemp /tmp/nmbench-out.XXXXXX)
for run in 1 2; do
  for k in ${(ok)P}; do
    s=$EPOCHREALTIME
    call list_threads "${P[$k]}" > "$OUT"
    ms=$(( (EPOCHREALTIME - s) * 1000 ))
    if (( run == 2 )); then
      rows=$(python3 -I -c 'import json, sys; print(len(json.load(open(sys.argv[1]))["items"]))' "$OUT")
      printf "%-40s %6.0f ms  %3s rows\n" "$k" $ms "$rows"
    fi
  done
done
rm -f "$OUT"
