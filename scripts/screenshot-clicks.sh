#!/usr/bin/env bash
# Launch the app and drive it with real clicks (cliclick) to capture the interactive states:
# dropdown open, project picked, Metadata tab, expanded thinking / tool rows, chip toggled.
# Usage: screenshot-clicks.sh <binary> <out-dir>. Coordinates are window-relative points.
set -euo pipefail

BIN=$1
OUT=$2
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
WINID="$ROOT/tmp/winid"

mkdir -p "$OUT" "$ROOT/tmp"
if [ ! -x "$WINID" ] || [ "$HERE/winid.swift" -nt "$WINID" ]; then
  swiftc -O -o "$WINID" "$HERE/winid.swift"
fi
wake-screen

"$BIN" > "$OUT/app.log" 2>&1 &
PID=$!
trap 'kill $PID 2>/dev/null; wait $PID 2>/dev/null || true' EXIT
sleep 2.5

read -r WID WX WY _ _ < <("$WINID" "$(basename "$BIN")" --bounds)
click() { cliclick "c:$((WX + $1)),$((WY + $2))"; sleep 0.6; }
shot() { screencapture -x -o -l "$WID" "$OUT/$1.png"; }

shot 1-initial
click 54 49;   shot 2-project-menu          # open Project dropdown
click 80 128;  shot 3-project-picked        # pick the 2nd project in the list
click 274 49;  shot 4-updated-menu          # open Updated dropdown
click 300 80;  shot 5-updated-today         # pick "Today"
click 1353 49; shot 6-preview-menu          # open Preview dropdown (right-anchored)
click 1300 152; shot 7-preview-last-message # pick "Last message"
click 300 227; shot 8-row-selected          # select the 3rd table row
click 1080 234; shot 9-thinking-expanded    # expand first thinking row
click 1080 297; shot 10-tool-expanded       # expand first tool row (position may have shifted)
click 1032 166; shot 11-tools-chip-off      # toggle the Tools chip
click 903 132; shot 12-metadata-tab         # switch to Metadata tab

echo "screenshots written to $OUT"
cat "$OUT/app.log"
