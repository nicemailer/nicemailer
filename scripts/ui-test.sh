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
    NICEMAILER_DATA_DIR="$dir" "$CLI" seed --threads 300 --seed 1 >/dev/null
    # No request leaves the machine (one-click unsubscribe).
    NICEMAILER_DATA_DIR="$dir" NICEMAILER_FAKE_NETWORK=1 "$CORE" 2>"$dir/core.log" &
    core=$!
    for _ in $(seq 50); do [ -S "$dir/core.sock" ] && break; sleep 0.1; done
    NICEMAILER_DATA_DIR="$dir" "$CLI" import-mbox "$APP/UITests/fixtures/sample.mbox" \
        --account fixture --address me@example.com >/dev/null 2>&1
    mkdir -p "$dir/exports"
    # perl's alarm gives the app 120 s at most (macOS has no `timeout`).
    # `-AppleLanguages (en)`: English UI whatever the machine's language, so
    # that expected messages do not depend on it.
    if NICEMAILER_DATA_DIR="$dir" NICEMAILER_UI_SCRIPT="$script" NICEMAILER_UI_OUT="$OUT/$name" \
        NICEMAILER_EXPORT_DIR="$dir/exports" perl -e 'alarm shift; exec @ARGV' 120 "$BIN" -AppleLanguages "(en)" 2>"$OUT/$name.log"; then
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
