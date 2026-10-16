BIN_DIR ?= $(HOME)/workspace/bin

.PHONY: build test lint install

build:
	cargo build --release

test:
	cargo test --workspace

lint:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings

# Replace instead of overwrite: rewriting a signed binary in place can get it killed on Apple Silicon.
install: lint test build
	mkdir -p $(BIN_DIR)
	rm -f $(BIN_DIR)/readr
	cp target/release/readr $(BIN_DIR)/readr
	@echo "installed $(BIN_DIR)/readr"
