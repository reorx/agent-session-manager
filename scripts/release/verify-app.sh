#!/bin/bash
# Check the release bundle before notarizing; Apple rejects, or users' Macs block, anything that fails here:
# Developer ID signature, hardened runtime, secure timestamp, no get-task-allow, on every nested code object;
# the main executable contains every arch in config.sh's ARCHS.
# Usage: verify-app.sh <path/to/App.app>    REQUIRE_ARCHS= (empty) skips the arch check
set -euo pipefail

source "$(dirname "$0")/config.sh"
app=${1:?usage: verify-app.sh APP}
fail() { echo "✗ $*" >&2; exit 1; }

codesign --verify --deep --strict "$app" || fail "codesign --verify failed"

while IFS= read -r -d '' code; do
    codesign --verify --strict "$code" || fail "invalid nested signature: $code"
    info=$(codesign -dvv "$code" 2>&1)
    grep -q '^Authority=Developer ID Application' <<<"$info" || fail "not signed with Developer ID Application: $code"
    grep -qE '^CodeDirectory .*flags=.*runtime' <<<"$info" || fail "hardened runtime off: $code"
    grep -q '^Timestamp=' <<<"$info" || fail "no secure timestamp: $code"
    entitlements=$(codesign -d --entitlements - --xml "$code" 2>/dev/null || true)
    if grep -q 'get-task-allow' <<<"$entitlements"; then
        fail "has get-task-allow: $code"
    fi
    echo "✓ Developer ID; runtime; timestamp: $code"
done < <("$(dirname "$0")/list-code.sh" "$app")

exe="$app/Contents/MacOS/$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$app/Contents/Info.plist")"
archs=$(lipo -archs "$exe")
for arch in ${REQUIRE_ARCHS-$ARCHS}; do
    [[ " $archs " == *" $arch "* ]] || fail "executable lacks $arch: $archs"
done

echo "✓ $(grep -m1 '^Authority=Developer ID Application' <<<"$info"); runtime; timestamp; archs: $archs"
