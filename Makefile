# Overrides such as BIN_DIR or CODESIGN_IDENTITY can go in an untracked local.mk.
-include local.mk

BIN_DIR ?= $(HOME)/.local/bin
LABEL := io.github.shaunwen.readr
CODESIGN_IDENTITY ?= $(shell security find-identity -p codesigning 2>/dev/null | grep -q '"readr-dev"' && echo readr-dev)
LAUNCHD_SERVICE := gui/$(shell id -u)/$(LABEL)

.PHONY: build test lint install demo demo-data demo-tui screenshots

DEMO_DIR := $(CURDIR)/target/demo
DEMO_ENV := HOME=$(DEMO_DIR)/home XDG_CONFIG_HOME=$(DEMO_DIR)/config XDG_DATA_HOME=$(DEMO_DIR)/data

build:
	cargo build --release

test:
	cargo test --workspace

lint:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings

# Replace instead of overwrite: rewriting a signed binary in place can get it killed on Apple Silicon.
# macOS keeps Full Disk Access and Accessibility grants only while the signature stays the same;
# an ad-hoc build changes it every time, a certificate (CODESIGN_IDENTITY) keeps it stable.
install: lint test build
ifneq ($(CODESIGN_IDENTITY),)
	codesign --force --sign "$(CODESIGN_IDENTITY)" --identifier $(LABEL) target/release/readr
endif
	mkdir -p $(BIN_DIR)
	rm -f $(BIN_DIR)/readr
	cp target/release/readr $(BIN_DIR)/readr
	@echo "installed $(BIN_DIR)/readr"
	@launchctl kickstart -k $(LAUNCHD_SERVICE) 2>/dev/null && echo "restarted the daemon" || true

# The demo runs on made-up data under target/demo and never opens your own database.
demo: demo-data demo-tui

demo-data:
	rm -rf $(DEMO_DIR)
	cargo run --quiet -p readr-core --example demo_data -- $(DEMO_DIR)/data/readr/readr.db

demo-tui:
	cargo build --quiet --release
	@mkdir -p $(DEMO_DIR)/home
	@$(DEMO_ENV) target/release/readr

screenshots: demo-data
	cargo build --quiet --release
	vhs docs/demo.tape
