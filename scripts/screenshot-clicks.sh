#!/usr/bin/env bash
# Launch the app and drive it with real clicks (cliclick) to capture the interactive states:
# dropdown open, project picked, time range, preview mode, row selected, chip toggled, Metadata tab.
# Runs on the real index with a throwaway view state (defaults on every run).
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

export ASM_STATE="$OUT/state.json"
rm -f "$ASM_STATE"
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
click 300 152; shot 5-updated-30-days       # pick "30 days"
click 1353 49; shot 6-preview-menu          # open Preview dropdown (right-anchored)
click 1300 152; shot 7-preview-last-message # pick "Last message"
click 300 227; shot 8-row-selected          # select the 3rd table row
click 960 166; shot 9-assistant-chip-off    # toggle the Assistant chip
click 903 132; shot 10-metadata-tab         # switch to Metadata tab
# thinking / tool rows come back with M4 (transcripts read directly); re-add their clicks then

echo "screenshots written to $OUT"
cat "$OUT/app.log"
