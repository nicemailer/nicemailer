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
.PHONY: build test swift-test ui-test bench seed core app bundle open

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

# Run the core in the foreground; stop it with Ctrl-C.
core:
	cargo run --release --bin nicemailer-core

app:
	cd apps/NiceMailer && DEVELOPER_DIR=$(XCODE) swift run

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
