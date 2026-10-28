#!/bin/bash
# Pack the .app into a drag-to-install dmg: <dist>/<DMG_NAME>-<version>.dmg with the app and an /Applications link.
# The version comes from the bundle's Info.plist (set from the git tag). Signing and notarizing are in notarize.sh.
# Usage: make-dmg.sh <path/to/App.app> <dist-dir>   on success stdout is only the dmg path
set -euo pipefail

source "$(dirname "$0")/config.sh"
app=${1:?usage: make-dmg.sh APP DIST}
dist=${2:?usage: make-dmg.sh APP DIST}
[[ -d "$app" ]] || { echo "app not found: ${app} (run make build-release first)" >&2; exit 1; }

version=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")

staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
# ditto keeps symlinks and extended attributes; cp -R can break a framework's signature
ditto "$app" "$staging/$APP_NAME.app"
ln -s /Applications "$staging/Applications"

mkdir -p "$dist"
dmg="$dist/$DMG_NAME-$version.dmg"
rm -f "$dmg"
hdiutil create -volname "$APP_NAME" -srcfolder "$staging" -ov -format UDZO "$dmg" >&2
echo "$dmg"
