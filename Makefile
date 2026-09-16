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

bin: ## Print the path of the debug binary
	@echo "$(BIN)"

clean: ## Remove this project's build artifacts only (the target dir is shared across projects)
	cargo clean -p $(APP) -p $(CORE)

.PHONY: help setup build run run-autoplay dev dv dev-autoplay release check test clippy fmt screenshot screenshot-clicks bin clean
