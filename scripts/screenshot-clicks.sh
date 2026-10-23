#!/usr/bin/env bash
# Launch the app and drive it with real clicks (cliclick) to capture the interactive states:
# dropdown open, kind filter, time range, project picked, preview mode, row selected, chip toggled, Metadata tab,
# then the Custom range: Updated menu, picker shown, picker open, a picker shortcut applied.
# Runs on the real index with a throwaway view state (defaults on every run).
# Usage: screenshot-clicks.sh <binary> <out-dir>. Coordinates are window-relative points.
#
# Toolbar buttons sit left to right (Project, Scope, Updated, [date picker], Kind) and each one's label
# moves every button to its right. So a button is only clicked while the labels to its left are known:
# Kind and Updated go before a project is picked (project names vary in length), and the Custom range
# steps first put the project back to "All projects".
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
# hover first: GPUI decides what was clicked from the hover state of the last frame, so a click right
# after the pointer jumps can land on nothing (the item only lights up)
click() { local p="$((WX + $1)),$((WY + $2))"; cliclick "m:$p" w:200 "c:$p"; sleep 0.6; }
shot() { screencapture -x -o -l "$WID" "$OUT/$1.png"; }

shot 1-initial
click 700 14                                # title bar: a window launched from a script is not active yet, and its first click only activates it
click 383 49;  shot 2-kind-menu             # open Kind dropdown (Project All, Scope Both, Updated Any time to its left)
click 383 162; shot 3-kind-no-history       # untick "History" (4th item); the menu closes like Scope's
click 383 49;  click 383 162                # tick it again: the remaining steps run on the default view
click 274 49;  shot 4-updated-menu          # open Updated dropdown (Project still "All")
click 300 152; shot 5-updated-30-days       # pick "30 days" (4th item)
click 54 49;   shot 6-project-menu          # open Project dropdown
click 80 128;  shot 7-project-picked        # pick the 2nd project in the list
click 1353 49; shot 8-preview-menu          # open Preview dropdown (right-anchored)
click 1300 152; shot 9-preview-last-message # pick "Last message"
click 300 227; shot 10-row-selected         # select the 4th table row
click 960 166; shot 11-assistant-chip-off   # toggle the Assistant chip
click 903 132; shot 12-metadata-tab         # switch to Metadata tab
# thinking / tool rows come back with M4 (transcripts read directly); re-add their clicks then

# Custom range. The picker goes right of the Updated button and pushes Kind right, so these run last.
click 54 49;   click 80 79                  # Project back to "All projects" (1st item): Updated is at its usual x again
click 274 49;  shot 13-updated-menu-custom  # Updated menu: presets, This month, Last month, separator, Custom range…
click 300 279; shot 14-custom-range         # pick "Custom range…" (after the separator): a date picker shows up right of the button
click 430 49;  shot 15-date-picker-open     # open it: two months, with the 4 shortcuts on the left
click 387 109; shot 16-last-month           # "Last month" (2nd shortcut): last month's sessions only, popover closed

echo "screenshots written to $OUT"
cat "$OUT/app.log"
