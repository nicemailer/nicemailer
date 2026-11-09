# Development commands. The default data directory is
# ~/Library/Application Support/NiceMailer; set NICEMAILER_DATA_DIR to use
# another one (for the core, the CLI and the app alike).
#
# Variables declared with `?=` can be overridden from the command line,
# e.g. `make seed THREADS=10000`.

# The SwiftUI macros (@Observable, @State...) ship with Xcode, not with the
# Command Line Tools, so Swift commands are run with Xcode's toolchain.
XCODE ?= /Applications/Xcode.app/Contents/Developer
THREADS ?= 2500

# These targets are commands, not files to produce.
.PHONY: dev-cert build test swift-test ui-test bench seed autoconfig core core-stop core-start app bundle open imap-up imap-down imap-test volume-test

build:
	cargo build --release
	cd apps/NiceMailer && DEVELOPER_DIR=$(XCODE) swift build

# Every test that runs without a window: Rust (core), Python scripts and
# Swift unit tests.
test:
	cargo test
	python3 -m unittest discover -s scripts/tests
	cd apps/NiceMailer && DEVELOPER_DIR=$(XCODE) swift test

swift-test:
	cd apps/NiceMailer && DEVELOPER_DIR=$(XCODE) swift test

# Scripted UI tests of the app (apps/NiceMailer/UITests). Each scenario runs
# on a fresh database; the app stays in the background, off screen, and never
# takes the keyboard, so this can run while you work.
ui-test:
	scripts/ui-test.sh

# Times the thread lists of the core on a generated mailbox of 100,000
# conversations (`THREADS=…` to change it), or on an existing data directory
# with BENCH_DATA_DIR=<dir> (see scripts/bench.sh).
bench:
	scripts/bench.sh

# Downloads Thunderbird's database of mail providers and converts it to
# data/providers/thunderbird.json, which `nicemailer account add` reads to
# find the servers of an address (docs/accounts.md). Our own providers are
# in data/providers/local/, by hand. Commit the result.
autoconfig:
	scripts/update-autoconfig.py

# Recreate the database with fake mail (the core must be stopped first,
# since it holds the database lock).
seed:
	cargo run --release --bin nicemailer -- seed --reset --threads $(THREADS)

# Run the core in the foreground; stop it with Ctrl-C. Builds the CLI
# (nicemailer) too, so that the two always speak the same protocol. Signed with the
# Apple Development certificate (scripts/sign-core.sh), so that every build
# reads the passwords the previous ones saved in the Keychain
# (docs/configuration.md, "Passwords").
core:
	cargo build --release --bin nicemailer-core --bin nicemailer
	scripts/sign-core.sh target/release/nicemailer-core
	target/release/nicemailer-core

# Fallback only: creates the self-signed certificate "NiceMailer Dev" for a
# Mac without an Apple Development one (scripts/dev-cert.sh). Its passwords
# survive no rebuild of the core (docs/configuration.md, "Passwords").
dev-cert:
	scripts/dev-cert.sh

app:
	cd apps/NiceMailer && DEVELOPER_DIR=$(XCODE) swift run

# Test IMAP servers (dev/dovecot/compose.yaml), run by OrbStack or Docker:
# a complete Dovecot on 127.0.0.1:31143/31993 and a minimal one, announcing
# only IMAP4rev1 UIDPLUS IDLE LITERAL+, on 32143/32993. Any user name logs in
# with the password "nicemailer" and starts with an empty mailbox. The
# certificate is signed by a test authority, dev/dovecot/tls/ca.crt. The
# servers are made anew each time (--force-recreate): they lose their mail,
# and pick up certificates that make-tls.sh renewed.
imap-up:
	dev/dovecot/make-tls.sh
	docker compose -f dev/dovecot/compose.yaml up -d --wait --force-recreate

# The tests that need the test IMAP servers (make imap-up first): the IMAP
# client against both Dovecot servers, probe_account end to end, and the
# sending of replies through the test SMTP servers (Mailpit). They
# are #[ignore] in the other tests, so that make test needs no server.
imap-test:
	cargo test -p nm-proto --test dovecot -- --ignored
	cargo test -p nm-daemon --test imap_rpc -- --ignored
	cargo test -p nm-engine --test ingest_imap -- --ignored
	cargo test -p nm-daemon --test sync_rpc -- --ignored
	cargo test -p nm-daemon --test setup_rpc -- --ignored
	cargo test -p nm-daemon --test send_rpc -- --ignored
	cargo test -p nm-daemon --test pace_rpc -- --ignored

# The volume test of plan-imap.md, phase 8: a generated mailbox of
# NM_VOLUME mails (100,000 by default) synchronized from nothing; prints
# the durations, the memory and the latency of the lists. Minutes long.
volume-test:
	cargo test --release -p nm-daemon --test volume_rpc -- --ignored --nocapture

# Stops the test IMAP servers; their mail, kept in memory, is lost.
imap-down:
	docker compose -f dev/dovecot/compose.yaml down

# Build NiceMailer.app, with its core, its CLI and the launch agent of the
# core, and install it in /Applications (docs/app-bundle.md). Deep links
# nicemailer://m/<id> open it (docs/deep-links.md). NICEMAILER_DATA_DIR, if
# set, becomes the mailbox chosen; NICEMAILER_FAKE_NETWORK, if set, makes
# its core pretend to send.
bundle:
	DEVELOPER_DIR=$(XCODE) scripts/make-bundle.sh

# Same, then quit the installed app if it runs and launch the new one. Its
# core is started by launchd: no `make core` needed.
open:
	DEVELOPER_DIR=$(XCODE) scripts/make-bundle.sh --open

# Stops the core started by the agent of NiceMailer.app: it finishes its
# work and leaves with code 0, which launchd does not restart. It starts
# again at the next launch of the app or session (or `make core-start`).
core-stop:
	launchctl kill SIGTERM gui/$$(id -u)/eu.nicemailer.core

core-start:
	launchctl kickstart gui/$$(id -u)/eu.nicemailer.core
