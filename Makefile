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

screenshot-clicks: build ## Like screenshot, but drive dropdowns / tabs / rows / turn cards / row context menus with real clicks (needs cliclick)
	scripts/screenshot-clicks.sh "$(BIN)" "$(SHOT_DIR)"

screenshot-search: build ## Run the scripted search session (text, next hit, CJK, regex, Tools scope, sort, cleared) and capture 7 screenshots; types and clicks nothing
	scripts/screenshot-search.sh "$(BIN)" "$(SHOT_DIR)"

screenshot-detail: build ## Tour the Conversation tab (turn cards, steps, search hits, special cards, the 70MB session, history prompts, a code block, ] twice, Metadata) and capture up to 12 screenshots; types and clicks nothing
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

icon: ## Render assets/icon/*.svg into assets/icon/AppIcon.icns (the .iconset stays in tmp/icon/)
	uv run scripts/make-icon.py

# —— Release: .app bundle, Developer ID signing, notarization, Sparkle appcast (kb/docs/release.md) ——
RELEASE_APP := $(TARGET_DIR)/release/Agent Session Manager.app
# Apple signing / notarization credentials (see that folder's AGENTS.md); CI passes the same names as env
APPLE_SECRETS := $(HOME)/Sync/apple-developer/secrets.env
LOAD_SECRETS := if [ -f "$(APPLE_SECRETS)" ]; then set -a; source "$(APPLE_SECRETS)"; set +a; fi
SPARKLE_KEY_BACKUP := $(HOME)/Sync/apple-developer/sparkle/agent-session-manager_ed25519.key
# CI passes the tag's version and the run number; local builds are 0.0.0 (0) unless given
VERSION ?= 0.0.0
BUILD_NUMBER ?= 0

build-release: ## Release .app (cargo build, bundle, sign, verify); VERSION / BUILD_NUMBER / FEED_URL
	cargo build --release -p $(APP)
	scripts/release/bundle-app.sh "$(TARGET_DIR)/release/$(APP)" "$(RELEASE_APP)" $(VERSION) $(BUILD_NUMBER)
	scripts/release/sign-app.sh "$(RELEASE_APP)"
	scripts/release/verify-app.sh "$(RELEASE_APP)"

notarize-app: ## Notarize the release .app and staple the ticket into it
	@$(LOAD_SECRETS); scripts/release/notarize.sh "$(RELEASE_APP)"

dmg: ## Pack the notarized .app into dist/release/AgentSessionManager-<version>.dmg
	scripts/release/make-dmg.sh "$(RELEASE_APP)" dist/release

notarize: ## Sign, notarize and staple the dmg in dist/release/, then assess it as Gatekeeper would
	@$(LOAD_SECRETS); for f in dist/release/*.dmg; do scripts/release/notarize.sh "$$f"; done

appcast: ## Sign the final dmg and write dist/release/appcast.xml (key: SPARKLE_PRIVATE_KEY_FILE or the backup)
	@if [ -z "$$SPARKLE_PRIVATE_KEY_FILE" ] && [ -f "$(SPARKLE_KEY_BACKUP)" ]; then export SPARKLE_PRIVATE_KEY_FILE="$(SPARKLE_KEY_BACKUP)"; fi; \
		for f in dist/release/*.dmg; do scripts/release/render-appcast.sh "$(RELEASE_APP)" "$$f" dist/release/appcast.xml; done

test-release: ## Behaviour tests for the release scripts (no keychain, network or signing)
	scripts/release/test-release.sh

release-local: build-release ## The whole release chain locally into dist/release/; uploads nothing
	rm -rf dist/release
	$(MAKE) notarize-app dmg notarize appcast

clean: ## Remove this project's build artifacts only (the target dir is shared across projects)
	cargo clean -p $(APP) -p $(CORE)

.PHONY: help setup build run run-autoplay dev dv dev-autoplay release check test clippy fmt screenshot screenshot-clicks screenshot-search screenshot-detail bin asm transcript-reader build-transcript-reader parity icon clean \
	build-release notarize-app dmg notarize appcast test-release release-local
