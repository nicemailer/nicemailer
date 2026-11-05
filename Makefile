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
.PHONY: dev-cert build test swift-test ui-test bench seed core app bundle open imap-up imap-down imap-test

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

# Recreate the database with fake mail (the core must be stopped first,
# since it holds the database lock).
seed:
	cargo run --release --bin nicemailer -- seed --reset --threads $(THREADS)

# Run the core in the foreground; stop it with Ctrl-C. Signed with the
# development certificate (make dev-cert, once), so that every build reads
# the passwords the previous ones saved in the Keychain
# (docs/configuration.md, "Passwords").
core:
	cargo build --release --bin nicemailer-core
	scripts/sign-core.sh target/release/nicemailer-core
	target/release/nicemailer-core

# Creates, once, the personal code signing certificate "NiceMailer Dev" in
# the login keychain (scripts/dev-cert.sh). Does nothing if it exists.
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
# client against both Dovecot servers, and probe_account end to end. They
# are #[ignore] in the other tests, so that make test needs no server.
imap-test:
	cargo test -p nm-proto --test dovecot -- --ignored
	cargo test -p nm-daemon --test imap_rpc -- --ignored

# Stops the test IMAP servers; their mail, kept in memory, is lost.
imap-down:
	docker compose -f dev/dovecot/compose.yaml down

# Build NiceMailer.app and install it in ~/Applications, so that deep links
# nicemailer://m/<id> open it (docs/deep-links.md). NICEMAILER_DATA_DIR, if
# set, is written into the app.
bundle:
	DEVELOPER_DIR=$(XCODE) scripts/make-bundle.sh

# Same, then quit the installed app if it runs and launch the new one: the
# way to run the app when deep links are needed (the core still runs with
# `make core`).
open:
	DEVELOPER_DIR=$(XCODE) scripts/make-bundle.sh --open
