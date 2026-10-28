#!/bin/bash
# Assemble the release .app around the cargo binary: Info.plist (version from the tag), icon, Sparkle.framework.
# Signing happens afterwards in sign-app.sh. The app loads Sparkle at runtime (crates/app/src/updater.rs),
# so only this bundle has an updater; the bare debug binary has none.
# Usage: bundle-app.sh <binary> <path/to/App.app> <version> <build-number>
#   FEED_URL     overrides SUFeedURL, for local update tests against a feed on 127.0.0.1
#   SPARKLE_DIR  directory holding Sparkle.framework (default: the release tarball from sparkle-tools.sh)
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
source "$here/config.sh"
usage='usage: bundle-app.sh BINARY APP VERSION BUILD'
bin=${1:?$usage}
app=${2:?$usage}
version=${3:?$usage}
build=${4:?$usage}
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "version must be MAJOR.MINOR.PATCH: $version" >&2; exit 1; }
[[ "$build" =~ ^[0-9]+$ ]] || { echo "build number must be an integer: $build" >&2; exit 1; }
[[ -f "$bin" ]] || { echo "binary not found: $bin" >&2; exit 1; }
sparkle_dir=${SPARKLE_DIR:-$(dirname "$("$here/sparkle-tools.sh")")}
feed_url=${FEED_URL:-$FEED_URL_DEFAULT}

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$app/Contents/Frameworks"
install -m 755 "$bin" "$app/Contents/MacOS/$EXECUTABLE"
cp "$root/assets/icon/AppIcon.icns" "$app/Contents/Resources/AppIcon.icns"
# ditto keeps the framework's Versions/Current symlinks
ditto "$sparkle_dir/Sparkle.framework" "$app/Contents/Frameworks/Sparkle.framework"

# Sparkle: check once a day, always ask before installing (no silent updates)
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key><string>en</string>
    <key>CFBundleDisplayName</key><string>$APP_NAME</string>
    <key>CFBundleExecutable</key><string>$EXECUTABLE</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleName</key><string>$APP_NAME</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$build</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
    <key>LSMinimumSystemVersion</key><string>$MIN_MACOS_VERSION</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSHumanReadableCopyright</key><string>MIT License</string>
    <key>SUFeedURL</key><string>$feed_url</string>
    <key>SUPublicEDKey</key><string>$SPARKLE_PUBLIC_KEY</string>
    <key>SUEnableAutomaticChecks</key><true/>
    <key>SUScheduledCheckInterval</key><integer>86400</integer>
    <key>SUAutomaticallyUpdate</key><false/>
    <key>SUAllowsAutomaticUpdates</key><false/>
</dict>
</plist>
PLIST
plutil -lint "$app/Contents/Info.plist" >/dev/null
echo "✓ $app ($version, build $build, feed $feed_url)"
