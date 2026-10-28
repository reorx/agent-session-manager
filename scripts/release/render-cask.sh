#!/bin/bash
# Render reorx/homebrew-tap's Casks/<CASK_TOKEN>.rb for a published dmg, to stdout.
# Rendering only: no network, no git. The workflow's homebrew job downloads, checks, commits and pushes.
# Usage: render-cask.sh <version> <sha256>
set -euo pipefail

source "$(dirname "$0")/config.sh"
version=${1:?usage: render-cask.sh VERSION SHA256}
sha256=${2:?usage: render-cask.sh VERSION SHA256}
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "invalid version: $version" >&2; exit 1; }
[[ "$sha256" =~ ^[a-f0-9]{64}$ ]] || { echo "invalid sha256: $sha256" >&2; exit 1; }

# zap lists only what the app writes: the index, view state, config and default favorites folder
# (crates/core/src/claude/paths.rs), plus Sparkle's preferences, download cache and HTTP storage.
# A favorites folder moved elsewhere by the user is left alone.
cat <<RUBY
cask "$CASK_TOKEN" do
  version "$version"
  sha256 "$sha256"

  url "https://github.com/$REPO/releases/download/v#{version}/$DMG_NAME-#{version}.dmg"
  name "$APP_NAME"
  desc "$DESC"
  homepage "https://github.com/$REPO"

  livecheck do
    url :url
    strategy :github_latest
  end

  auto_updates true
  depends_on arch: :arm64
  depends_on macos: :$MIN_MACOS

  app "$APP_NAME.app"

  uninstall quit: "$BUNDLE_ID"

  zap trash: [
    "~/Library/Application Support/dev.reorx.AgentSessionManager",
    "~/Library/Caches/$BUNDLE_ID",
    "~/Library/HTTPStorages/$BUNDLE_ID",
    "~/Library/Preferences/$BUNDLE_ID.plist",
    "~/Library/Saved Application State/$BUNDLE_ID.savedState",
  ]
end
RUBY
