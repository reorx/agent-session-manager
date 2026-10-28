#!/bin/bash
# Install instructions at the top of the GitHub Release body (the workflow passes --notes-file; the generated
# change list follows).
set -euo pipefail

source "$(dirname "$0")/config.sh"
cat <<MD
## Install

\`\`\`sh
brew install --cask reorx/tap/$CASK_TOKEN
\`\`\`

If Homebrew 7 says it cannot load the cask, run \`brew trust --tap reorx/tap\` first. Or download the dmg below and drag $APP_NAME into Applications.

Requires macOS $MIN_MACOS_VERSION or later on Apple Silicon. Signed with Developer ID and notarized by Apple.

## Updates

The app checks for updates once a day; you can also choose **$APP_NAME → Check for Updates…**. It always asks before installing and relaunching.
MD
