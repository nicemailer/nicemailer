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
.PHONY: build test seed core app

build:
	cargo build --release
	cd apps/NiceMailer && DEVELOPER_DIR=$(XCODE) swift build

test:
	cargo test

# Recreate the database with fake mail (the core must be stopped first,
# since it holds the database lock).
seed:
	cargo run --release --bin nicemailer -- seed --reset --threads $(THREADS)

# Run the core in the foreground; stop it with Ctrl-C.
core:
	cargo run --release --bin nicemailer-core

app:
	cd apps/NiceMailer && DEVELOPER_DIR=$(XCODE) swift run
