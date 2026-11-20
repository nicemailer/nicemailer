#!/usr/bin/env bash
# Runs the scripted UI tests of the app (apps/NiceMailer/UITests/*.uiscript).
#
# For each scenario: a fresh data directory with fake mail (seed) and a small
# imported mbox, a core, then the app in UI script mode. The app never comes
# to the front and never takes the keyboard: it can run while you work.
#
# Usage: scripts/ui-test.sh [scenario.uiscript...]   (default: all)
# Results and snapshots: apps/NiceMailer/.build/ui-test/<scenario>/
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
APP="$ROOT/apps/NiceMailer"
XCODE=${XCODE:-/Applications/Xcode.app/Contents/Developer}
OUT="$APP/.build/ui-test"

cargo build --release --quiet --manifest-path "$ROOT/Cargo.toml"
(cd "$APP" && DEVELOPER_DIR="$XCODE" swift build --quiet)

CLI="$ROOT/target/release/nicemailer"
# The MCP server, for the menu "AI Agents" (agents.uiscript, docs/mcp.md).
MCP="$ROOT/target/release/nicemailer-mcp"
CORE="$ROOT/target/release/nicemailer-core"
# NICEMAILER_UI_BIN plays the scenarios with another build of the app, such
# as the executable of NiceMailer.app (`make bundle`).
BIN="${NICEMAILER_UI_BIN:-$APP/.build/debug/NiceMailer}"

rm -rf "$OUT"
mkdir -p "$OUT"
scripts=("$@")
if [ ${#scripts[@]} -eq 0 ]; then scripts=("$APP"/UITests/*.uiscript); fi

failed=0
for script in "${scripts[@]}"; do
    name=$(basename "$script" .uiscript)
    # A bare name ("pickers.uiscript") is looked up in UITests/. A scenario
    # that does not exist fails: the app would play nothing and pass.
    if [ ! -f "$script" ] && [ -f "$APP/UITests/$script" ]; then script="$APP/UITests/$script"; fi
    if [ ! -f "$script" ]; then
        echo "FAIL $name: no scenario at $script"
        failed=1
        continue
    fi
    script=$(cd "$(dirname "$script")" && pwd)/$(basename "$script")
    # Short path: the core's socket path must stay under 104 characters.
    dir=$(mktemp -d /tmp/nmuit.XXXXXX)
    # A scenario may ask for a bigger mailbox (fixtures/<scenario>.seed:
    # a number of conversations), to go deep into a list.
    threads=300
    if [ -f "$APP/UITests/fixtures/$name.seed" ]; then threads=$(cat "$APP/UITests/fixtures/$name.seed"); fi
    NICEMAILER_DATA_DIR="$dir" "$CLI" seed --threads "$threads" --seed 1 >/dev/null
    # A scenario may have settings of its own (fixtures/<scenario>.config.toml),
    # read by the core when it starts.
    # Without one, the text sizes are 12 pt: the size the layout is
    # written for, so that the widths, heights and line breaks the
    # scenarios check do not move with the default sizes (16 and 18 pt),
    # which textsize.uiscript checks on its own.
    if [ -f "$APP/UITests/fixtures/$name.config.toml" ]; then
        cp "$APP/UITests/fixtures/$name.config.toml" "$dir/config.toml"
    else
        printf '[ui]\ntext_size = 12\nstatus_bar_text_size = 12\n' >"$dir/config.toml"
    fi
    # ... and keys of its own (fixtures/<scenario>.keymap.toml, docs/keymap.md),
    # next to config.toml, where the core reads them.
    if [ -f "$APP/UITests/fixtures/$name.keymap.toml" ]; then
        cp "$APP/UITests/fixtures/$name.keymap.toml" "$dir/keymap.toml"
    fi
    # A scenario may answer the AI prompts from a table
    # (fixtures/<scenario>.ai.json, `NICEMAILER_FAKE_AI`, docs/ai.md): no
    # AI provider is ever called by the UI tests.
    if [ -f "$APP/UITests/fixtures/$name.ai.json" ]; then
        export NICEMAILER_FAKE_AI="$APP/UITests/fixtures/$name.ai.json"
    else
        unset NICEMAILER_FAKE_AI
    fi
    # No request leaves the machine (one-click unsubscribe).
    NICEMAILER_DATA_DIR="$dir" NICEMAILER_CONFIG="$dir/config.toml" NICEMAILER_FAKE_NETWORK=1 "$CORE" 2>"$dir/core.log" &
    # Mailboxes for "Change Mailbox…" (mailbox.uiscript): one of an earlier
    # data format, an early development one, one of a newer version, an
    # empty folder. The app checks and upgrades them with this CLI.
    mkdir -p "$dir/mailboxes/old" "$dir/mailboxes/early" "$dir/mailboxes/newer" "$dir/mailboxes/empty"
    sqlite3 "$dir/mailboxes/old/nicemailer.db" < "$ROOT/crates/nm-store/tests/fixtures/schema_v1.sql" >/dev/null
    sqlite3 "$dir/mailboxes/early/nicemailer.db" "CREATE TABLE account (id INTEGER PRIMARY KEY); PRAGMA user_version = 1;"
    sqlite3 "$dir/mailboxes/newer/nicemailer.db" "PRAGMA user_version = 99;"
    core=$!
    for _ in $(seq 50); do [ -S "$dir/core.sock" ] && break; sleep 0.1; done
    # A scenario may keep the seeded account alone
    # (fixtures/<scenario>.single-account, an empty file): no account
    # "fixture", neither the sample mbox nor the scenario's own.
    single=false
    if [ -f "$APP/UITests/fixtures/$name.single-account" ]; then single=true; fi
    if [ "$single" = false ]; then
        NICEMAILER_DATA_DIR="$dir" "$CLI" import-mbox "$APP/UITests/fixtures/sample.mbox" \
            --account fixture --address me@example.com >/dev/null 2>&1
    fi
    # A scenario may add mail of its own (fixtures/<scenario>.mbox), so the
    # mail the others count stays the same.
    if [ "$single" = false ] && [ -f "$APP/UITests/fixtures/$name.mbox" ]; then
        NICEMAILER_DATA_DIR="$dir" "$CLI" import-mbox "$APP/UITests/fixtures/$name.mbox" \
            --account fixture --address me@example.com >/dev/null 2>&1
    fi
    mkdir -p "$dir/exports"
    # `-AppleLanguages (en)`: English UI whatever the machine's language, so
    # that expected messages do not depend on it. A scenario may ask for
    # another language (fixtures/<scenario>.language: `fr`) and for the
    # region settings of a Mac (fixtures/<scenario>.locale: `en_US`), which
    # give the order of dates and the clock (docs/localization.md); without
    # it, fr_FR, so that the dates expected do not depend on the machine.
    language=en
    if [ -f "$APP/UITests/fixtures/$name.language" ]; then language=$(cat "$APP/UITests/fixtures/$name.language"); fi
    locale=fr_FR
    if [ -f "$APP/UITests/fixtures/$name.locale" ]; then locale=$(cat "$APP/UITests/fixtures/$name.locale"); fi
    # perl's alarm gives the app 120 s at most (macOS has no `timeout`).
    if NICEMAILER_DATA_DIR="$dir" NICEMAILER_CLI="$CLI" NICEMAILER_MCP="$MCP" NICEMAILER_UI_SCRIPT="$script" NICEMAILER_UI_OUT="$OUT/$name" \
        NICEMAILER_EXPORT_DIR="$dir/exports" perl -e 'alarm shift; exec @ARGV' 120 "$BIN" -AppleLanguages "($language)" -AppleLocale "$locale" 2>"$OUT/$name.log"; then
        echo "PASS $name"
    else
        echo "FAIL $name"
        grep -E "^FAIL" "$OUT/$name/result.txt" 2>/dev/null || tail -5 "$OUT/$name.log"
        failed=1
    fi
    kill "$core" 2>/dev/null || true
    wait "$core" 2>/dev/null || true
    rm -rf "$dir"
done
exit $failed
