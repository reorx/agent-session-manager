#!/bin/bash
# xcodebuild build 不会像 Archive 导出那样重签嵌套辅助程序，必须由内到外重签。
set -euo pipefail
app=${1:?usage: sign-app.sh APP}
while IFS= read -r -d '' code; do
    codesign --force --sign "${SIGN_IDENTITY:-Developer ID Application}" \
        --options runtime --timestamp --preserve-metadata=entitlements "$code"
done < <("$(dirname "$0")/list-code.sh" "$app")
codesign --verify --deep --strict "$app"
