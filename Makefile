.DEFAULT_GOAL := help

APP  := agent-session-manager
CORE := agent-session-core
# target-dir is set globally in ~/.cargo/config.toml (shared cache under ~/Library/Caches), so ask cargo.
TARGET_DIR := $(shell cargo metadata --format-version 1 --no-deps 2>/dev/null | python3 -c 'import sys, json; print(json.load(sys.stdin)["target_directory"])')
BIN := $(TARGET_DIR)/debug/$(APP)
SHOT_DIR ?= tmp/$(shell date +%Y-%m-%d)-screenshots

help: ## List available commands
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "} {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'

setup: ## Install the Rust toolchain (mise) and the Metal Toolchain if missing
	mise install
	@xcrun -sdk macosx metal --version >/dev/null 2>&1 || xcodebuild -downloadComponent MetalToolchain

build: ## Debug build of all crates
	cargo build --workspace

run: ## Launch the existing debug binary, replacing any running instance
	@pkill -x $(APP) 2>/dev/null || true
	@for i in $$(seq 1 20); do pgrep -qx $(APP) || break; sleep 0.1; done
	@$(BIN)

run-autoplay: ## Like run, but scroll / select / sort automatically after launch
	@pkill -x $(APP) 2>/dev/null || true
	@for i in $$(seq 1 20); do pgrep -qx $(APP) || break; sleep 0.1; done
	@ASM_SPIKE_AUTOPLAY=1 $(BIN)

dev: build run ## Build and launch the app (the usual dev loop)

dv: dev ## Alias for dev

dev-autoplay: build run-autoplay ## Build and launch with the scripted interaction

release: ## Release build of the app
	cargo build --release -p $(APP)

check: ## Type-check without producing binaries
	cargo check --workspace --all-targets

test: ## Run all tests
	cargo test --workspace

clippy: ## Run clippy on all targets
	cargo clippy --workspace --all-targets

fmt: ## Format all crates with rustfmt
	cargo fmt --all

screenshot: build ## Wake the screen, launch the app and capture 3 window screenshots into $(SHOT_DIR)
	scripts/screenshot.sh "$(BIN)" "$(SHOT_DIR)"

screenshot-clicks: build ## Like screenshot, but drive dropdowns / tabs / rows with real clicks (needs cliclick)
	scripts/screenshot-clicks.sh "$(BIN)" "$(SHOT_DIR)"

screenshot-search: build ## Run the scripted search session (text, next hit, CJK, regex, Tools scope, sort, cleared) and capture 7 screenshots; types and clicks nothing
	scripts/screenshot-search.sh "$(BIN)" "$(SHOT_DIR)"

screenshot-detail: build ## Tour the Conversation tab (turn cards, steps, search hits, special cards, the 70MB session) and capture up to 8 screenshots; types and clicks nothing
	scripts/screenshot-detail.sh "$(BIN)" "$(SHOT_DIR)"

bin: ## Print the path of the debug binary
	@echo "$(BIN)"

asm: ## Run the asm CLI (release build), e.g. make asm ARGS="list -d 7"
	@cargo build --release -p $(CORE) --bin asm -q
	@$(TARGET_DIR)/release/asm $(ARGS)

transcript-reader: ## Export compact text with XML-style tags, e.g. make transcript-reader ARGS="SESSION_ID -o session.txt"
	@cargo build --release -p $(CORE) --bin transcript-reader -q
	@$(TARGET_DIR)/release/transcript-reader $(ARGS)

build-transcript-reader: ## Build a standalone release binary at dist/transcript-reader
	@cargo build --release -p $(CORE) --bin transcript-reader -q
	@mkdir -p dist
	@install -m 755 "$(TARGET_DIR)/release/transcript-reader" dist/transcript-reader

parity: ## Diff asm against the Python reference script on the real ~/.claude (outputs in tmp/<date>-python-parity)
	@cargo build --release -p $(CORE) --bin asm -q
	scripts/compare-with-python.sh "$(TARGET_DIR)/release/asm"

clean: ## Remove this project's build artifacts only (the target dir is shared across projects)
	cargo clean -p $(APP) -p $(CORE)

.PHONY: help setup build run run-autoplay dev dv dev-autoplay release check test clippy fmt screenshot screenshot-clicks screenshot-search screenshot-detail bin asm transcript-reader build-transcript-reader parity clean
