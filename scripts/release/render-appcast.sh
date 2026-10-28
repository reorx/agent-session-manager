#!/bin/bash
# Sign the final, stapled dmg and write a one-item Sparkle appcast, verifying the signature with the same key.
# Without SPARKLE_PRIVATE_KEY_FILE, sign_update reads the key from the keychain account CASK_TOKEN.
# Usage: render-appcast.sh <App.app> <dmg> <output.xml>
#   DOWNLOAD_URL overrides the enclosure URL, for local update tests against a feed on 127.0.0.1
set -euo pipefail

source "$(dirname "$0")/config.sh"
app=${1:?usage: render-appcast.sh APP DMG OUTPUT}
dmg=${2:?usage: render-appcast.sh APP DMG OUTPUT}
output=${3:?usage: render-appcast.sh APP DMG OUTPUT}
plist="$app/Contents/Info.plist"
version=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$plist")
build=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$plist")
minimum=$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' "$plist")
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ && "$build" =~ ^[0-9]+$ && "$minimum" =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?$ ]] \
    || { echo 'malformed version, build number or minimum system version' >&2; exit 1; }
[[ "$(basename "$dmg")" == "$DMG_NAME-$version.dmg" ]] || { echo "dmg name does not match app version $version: $dmg" >&2; exit 1; }
url=${DOWNLOAD_URL:-https://github.com/$REPO/releases/download/v$version/$DMG_NAME-$version.dmg}

sign_update=${SIGN_UPDATE:-$("$(dirname "$0")/sparkle-tools.sh")/sign_update}
key_args=(--account "$CASK_TOKEN")
if [[ -n "${SPARKLE_PRIVATE_KEY_FILE:-}" ]]; then key_args=(--ed-key-file "$SPARKLE_PRIVATE_KEY_FILE"); fi
signature=$("$sign_update" "${key_args[@]}" -p "$dmg")
length=$(stat -f%z "$dmg")
"$sign_update" "${key_args[@]}" --verify "$dmg" "$signature" >&2

cat > "$output" <<XML
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>$APP_NAME</title>
    <link>https://github.com/$REPO</link>
    <description>$APP_NAME releases</description>
    <item>
      <title>$APP_NAME $version</title>
      <pubDate>$(LC_ALL=C date -u '+%a, %d %b %Y %H:%M:%S +0000')</pubDate>
      <sparkle:version>$build</sparkle:version>
      <sparkle:shortVersionString>$version</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>$minimum</sparkle:minimumSystemVersion>
      <description><![CDATA[
        <h2>$APP_NAME $version</h2>
        <p>For macOS $minimum or later on Apple Silicon. Signed with Developer ID and notarized by Apple.</p>
        <p><a href="https://github.com/$REPO/releases/tag/v$version">Release notes</a></p>
      ]]></description>
      <enclosure url="$url"
        sparkle:edSignature="$signature" length="$length" type="application/octet-stream" />
    </item>
  </channel>
</rss>
XML
xmllint --noout "$output"
echo "✓ appcast: $version ($build), $length bytes → $output" >&2
