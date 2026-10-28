# Product constants for the release scripts (sourced, not run). The workflow's env repeats DMG_NAME and CASK_TOKEN.
APP_NAME="Agent Session Manager"   # .app file name, CFBundleName, dmg volume name, cask `name`
DMG_NAME=AgentSessionManager       # release asset prefix: GitHub turns spaces in asset names into dots
EXECUTABLE=agent-session-manager   # cargo binary, Contents/MacOS/<this>
BUNDLE_ID=com.reorx.agent-session-manager
REPO=reorx/agent-session-manager
CASK_TOKEN=agent-session-manager
DESC="Browse, search and resume Claude Code sessions"
MIN_MACOS_VERSION=14.0             # LSMinimumSystemVersion
MIN_MACOS=sonoma                   # the same floor as a Homebrew symbol
ARCHS=arm64                        # Apple Silicon only
FEED_URL_DEFAULT="https://github.com/$REPO/releases/latest/download/appcast.xml"
# Pairs with the private key in ~/Sync/apple-developer/sparkle/agent-session-manager_ed25519.key
# (CI secret SPARKLE_ED_PRIVATE_KEY). Never regenerate: installed copies only accept updates signed by it.
SPARKLE_PUBLIC_KEY="Y5kCJX4VD30F0FqbN+q1wIXluZtuan65JYdZOSKHAJs="
