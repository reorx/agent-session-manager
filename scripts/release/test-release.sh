#!/bin/bash
# Behaviour tests for the release scripts. Touches no keychain, network or real signing identity.
set -euo pipefail
cd "$(dirname "$0")/../.."
source scripts/release/config.sh
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

# --- bundle-app.sh: the .app is assembled from the cargo binary, versions come from the tag
mkdir -p "$work/sparkle/Sparkle.framework/Versions/B"
printf 'framework' > "$work/sparkle/Sparkle.framework/Versions/B/Sparkle"
export SPARKLE_DIR="$work/sparkle"
app="$work/out/$APP_NAME.app"
plist="$app/Contents/Info.plist"
scripts/release/bundle-app.sh /usr/bin/true "$app" 1.2.3 42 >/dev/null
expect() {
    local got
    got=$(/usr/libexec/PlistBuddy -c "Print :$1" "$plist")
    [[ "$got" == "$2" ]] || fail "$1 = $got, want $2"
}
expect CFBundleShortVersionString 1.2.3
expect CFBundleVersion 42
expect CFBundleExecutable "$EXECUTABLE"
expect CFBundleIdentifier "$BUNDLE_ID"
expect CFBundleName "$APP_NAME"
expect CFBundlePackageType APPL
expect CFBundleIconFile AppIcon
expect LSMinimumSystemVersion "$MIN_MACOS_VERSION"
expect NSHighResolutionCapable true
expect SUFeedURL "$FEED_URL_DEFAULT"
expect SUPublicEDKey "$SPARKLE_PUBLIC_KEY"
expect SUEnableAutomaticChecks true
expect SUScheduledCheckInterval 86400
expect SUAutomaticallyUpdate false
expect SUAllowsAutomaticUpdates false
[[ -x "$app/Contents/MacOS/$EXECUTABLE" ]] || fail 'executable missing'
cmp -s assets/icon/AppIcon.icns "$app/Contents/Resources/AppIcon.icns" || fail 'icon missing'
[[ -f "$app/Contents/Frameworks/Sparkle.framework/Versions/B/Sparkle" ]] || fail 'Sparkle.framework missing'
echo 'PASS: bundle-app writes the tag version, build number, bundle id, icon, Sparkle feed and public key'

FEED_URL=http://127.0.0.1:8765/appcast.xml scripts/release/bundle-app.sh /usr/bin/true "$app" 1.2.3 42 >/dev/null
expect SUFeedURL http://127.0.0.1:8765/appcast.xml
echo 'PASS: FEED_URL points a local test build at another feed'

for bad in '1.2 42' 'v1.2.3 42' '1.2.3 x'; do
    # shellcheck disable=SC2086
    if scripts/release/bundle-app.sh /usr/bin/true "$work/bad.app" $bad >/dev/null 2>&1; then fail "accepted $bad"; fi
done
echo 'PASS: bundle-app rejects malformed versions and build numbers'
scripts/release/bundle-app.sh /usr/bin/true "$app" 1.2.3 42 >/dev/null

# --- render-appcast.sh: one item for the final dmg, signed and self-verified
mkdir -p "$work/bin"
cat > "$work/bin/sign_update" <<'SH'
#!/bin/bash
set -euo pipefail
printf '%s\n' "$*" >> "$SIGN_LOG"
if [[ " $* " == *' --verify '* ]]; then exit 0; fi
printf 'test-signature\n'
SH
chmod +x "$work/bin/sign_update"
export SIGN_UPDATE="$work/bin/sign_update" SIGN_LOG="$work/sign.log"
dmg="$work/$DMG_NAME-1.2.3.dmg"
printf 'final stapled dmg bytes' > "$dmg"
xml="$work/appcast.xml"
scripts/release/render-appcast.sh "$app" "$dmg" "$xml" 2>/dev/null
xmllint --noout "$xml"
expect_xml() {
    [[ "$(xmllint --xpath "$1" "$xml")" == "$2" ]] || fail "$1 = $(xmllint --xpath "$1" "$xml"), want $2"
}
expect_xml 'count(/rss/channel/item)' 1
expect_xml 'string(/rss/channel/item/*[local-name()="version"])' 42
expect_xml 'string(/rss/channel/item/*[local-name()="shortVersionString"])' 1.2.3
expect_xml 'string(/rss/channel/item/*[local-name()="minimumSystemVersion"])' "$MIN_MACOS_VERSION"
expect_xml 'string(/rss/channel/item/enclosure/@url)' "https://github.com/$REPO/releases/download/v1.2.3/$DMG_NAME-1.2.3.dmg"
expect_xml 'string(/rss/channel/item/enclosure/@length)' "$(stat -f%z "$dmg")"
expect_xml 'string(/rss/channel/item/enclosure/@*[local-name()="edSignature"])' test-signature
expect_xml 'boolean(/rss/channel/item/description)' true
grep -q -- '--verify' "$SIGN_LOG" || fail 'appcast signature was not verified'
echo 'PASS: appcast uses the bundle version, final dmg size and versioned URL, and verifies its signature'

mv "$dmg" "$work/$DMG_NAME-9.9.9.dmg"
if scripts/release/render-appcast.sh "$app" "$work/$DMG_NAME-9.9.9.dmg" "$xml" >/dev/null 2>&1; then
    fail 'accepted a dmg whose name does not match the bundle version'
fi
echo 'PASS: appcast rejects a dmg named for another version'

# --- render-cask.sh
scripts/release/render-cask.sh 1.2.3 "$(printf '%064d' 0)" > "$work/cask.rb"
grep -qx '  auto_updates true' "$work/cask.rb" || fail 'cask lacks auto_updates'
grep -qF "url \"https://github.com/$REPO/releases/download/v#{version}/$DMG_NAME-#{version}.dmg\"" "$work/cask.rb" || fail 'cask url'
grep -qF "app \"$APP_NAME.app\"" "$work/cask.rb" || fail 'cask app stanza'
grep -qF '"~/Library/Application Support/dev.reorx.AgentSessionManager"' "$work/cask.rb" || fail 'cask zap misses the index dir'
echo 'PASS: cask points at the versioned dmg, installs the app and declares auto_updates'

# --- verify-app.sh: every nested code object must be Developer ID, runtime, timestamped; arm64 required
mkdir -p "$app/Contents/Frameworks/Helper.app/Contents/MacOS"
cp /usr/bin/true "$app/Contents/Frameworks/Helper.app/Contents/MacOS/Helper"
cat > "$work/bin/codesign" <<'SH'
#!/bin/bash
if [[ " $* " == *' --verify '* ]]; then exit 0; fi
if [[ " $* " == *' --entitlements '* ]]; then exit 0; fi
if [[ "$*" == *Helper* ]]; then
    case "${BAD_NESTED:-}" in
        identity) echo 'Authority=Apple Development: Wrong' ;;
        *) echo 'Authority=Developer ID Application: Test' ;;
    esac
    [[ "${BAD_NESTED:-}" == runtime ]] || echo 'CodeDirectory flags=0x10000(runtime)'
    [[ "${BAD_NESTED:-}" == timestamp ]] || echo 'Timestamp=Oct 18, 2026'
else
    echo 'Authority=Developer ID Application: Test'
    echo 'CodeDirectory flags=0x10000(runtime)'
    echo 'Timestamp=Oct 18, 2026'
fi
SH
cat > "$work/bin/lipo" <<'SH'
#!/bin/bash
echo "${FAKE_ARCHS:-arm64}"
SH
chmod +x "$work/bin/codesign" "$work/bin/lipo"
PATH="$work/bin:$PATH"
for bad in identity runtime timestamp; do
    if BAD_NESTED="$bad" scripts/release/verify-app.sh "$app" > "$work/verify.log" 2>&1; then
        fail "accepted nested code with bad $bad"
    fi
done
if FAKE_ARCHS=x86_64 scripts/release/verify-app.sh "$app" > "$work/verify.log" 2>&1; then
    fail 'accepted an executable without arm64'
fi
scripts/release/verify-app.sh "$app" > "$work/verify.log" 2>&1 || { cat "$work/verify.log" >&2; fail 'rejected a good bundle'; }
echo 'PASS: verify-app rejects bad nested identity / runtime / timestamp and a missing arm64 slice'

# --- sign-app.sh: inner code before outer bundles, entitlements kept, runtime and timestamp added
cat > "$work/bin/codesign" <<'SH'
#!/bin/bash
set -euo pipefail
if [[ " $* " == *' --verify '* ]]; then exit 0; fi
[[ " $* " == *' --options runtime '* && " $* " == *' --timestamp '* && " $* " == *' --preserve-metadata=entitlements '* ]]
printf '%s\n' "${!#}" >> "$SIGN_ORDER"
SH
export SIGN_ORDER="$work/order"
scripts/release/sign-app.sh "$app"
[[ "$(tail -1 "$SIGN_ORDER")" == "$app" ]] || fail 'outer app was not signed last'
helper_line=$(grep -nFx "$app/Contents/Frameworks/Helper.app/Contents/MacOS/Helper" "$SIGN_ORDER" | cut -d: -f1)
bundle_line=$(grep -nFx "$app/Contents/Frameworks/Helper.app" "$SIGN_ORDER" | cut -d: -f1)
framework_line=$(grep -nFx "$app/Contents/Frameworks/Sparkle.framework" "$SIGN_ORDER" | cut -d: -f1)
[[ "$helper_line" -lt "$bundle_line" && -n "$framework_line" ]] || fail 'nested code signed out of order'
echo 'PASS: sign-app re-signs nested code inside out, keeping entitlements'
