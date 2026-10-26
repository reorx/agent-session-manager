#!/usr/bin/env bash
# Launch the app and drive it with real clicks (cliclick) to capture the interactive states:
# dropdown open, kind filter, time range, project picked, preview mode, row selected, Metadata tab,
# then the Custom range: Updated menu, picker shown, picker open, a picker shortcut applied,
# then the Conversation tab's turn cards: Show steps, a tool row opened, a tick hovered and clicked,
# then the row context menu: on a row that is not the selected one, and on a history-only row.
# Runs on the real index (or $ASM_DB) with a throwaway view state (defaults on every run).
# Usage: screenshot-clicks.sh <binary> <out-dir>. Coordinates are window-relative points.
#
# Toolbar buttons sit left to right (Project, Scope, Updated, [date picker], Kind) and each one's label
# moves every button to its right. So a button is only clicked while the labels to its left are known:
# Kind and Updated go before a project is picked (project names vary in length), and the Custom range
# steps first put the project back to "All projects".
#
# The turn-card steps need a session whose first card always looks the same: the app is launched a
# second time with only the selection preset to 5f6eea99 (the 70 MB session, unchanged since 09-14),
# and every click lands in the detail panel. Without that session the second part is skipped; on
# another machine pick a frozen session of your own and measure its card again.
#
# The context menus need a third launch with defaults (the top row selected, Kind "All"): the menu is
# opened on the 3rd row, then Kind is narrowed to History so that a history-only row is the first one.
set -euo pipefail

BIN=$1
OUT=$2
CARDS_SESSION=5f6eea99-2f43-4938-9035-b9db89173039
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
WINID="$ROOT/tmp/winid"

mkdir -p "$OUT" "$ROOT/tmp"
if [ ! -x "$WINID" ] || [ "$HERE/winid.swift" -nt "$WINID" ]; then
  swiftc -O -o "$WINID" "$HERE/winid.swift"
fi
wake-screen

export ASM_STATE="$OUT/state.json"
: > "$OUT/app.log"
PID=
trap 'kill $PID 2>/dev/null; wait $PID 2>/dev/null || true' EXIT
# launch [selected session id]: defaults, or only the selection preset
launch() {
  rm -f "$ASM_STATE"
  [ -z "${1:-}" ] || printf '{"selected_id": "%s"}\n' "$1" > "$ASM_STATE"
  "$BIN" >> "$OUT/app.log" 2>&1 &
  PID=$!
  # keep the display from sleeping while the app runs: a sleeping or locked screen makes window
  # captures fail halfway (wake-screen only wakes it once, at the start)
  caffeinate -d -w "$PID" &
  sleep 2.5
  read -r WID WX WY _ _ < <("$WINID" "$(basename "$BIN")" --bounds)
}
quit() { kill "$PID" 2>/dev/null; wait "$PID" 2>/dev/null || true; sleep 0.5; }
# hover first: GPUI decides what was clicked from the hover state of the last frame, so a click right
# after the pointer jumps can land on nothing (the item only lights up)
click() { local p="$((WX + $1)),$((WY + $2))"; cliclick "m:$p" w:200 "c:$p"; sleep 0.6; }
hover() { cliclick "m:$((WX + $1)),$((WY + $2))"; sleep 0.6; }
rclick() { local p="$((WX + $1)),$((WY + $2))"; cliclick "m:$p" w:300 "rc:$p"; sleep 0.8; }
shot() { screencapture -x -o -l "$WID" "$OUT/$1.png"; }

launch
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
click 300 155; shot 10-row-selected         # select the 2nd table row (the picked project may have only a few)
click 903 132; shot 11-metadata-tab         # switch to Metadata tab

# Custom range. The picker goes right of the Updated button and pushes Kind right, so these run last.
click 54 49;   click 80 79                  # Project back to "All projects" (1st item): Updated is at its usual x again
click 274 49;  shot 12-updated-menu-custom  # Updated menu: presets, This month, Last month, separator, Custom range…
click 300 279; shot 13-custom-range         # pick "Custom range…" (after the separator): a date picker shows up right of the button
click 430 49;  shot 14-date-picker-open     # open it: two months, with the 4 shortcuts on the left
click 387 109; shot 15-last-month           # "Last month" (2nd shortcut): last month's sessions only, popover closed

# Turn cards (Conversation tab) of CARDS_SESSION, see the header comment; detail panel only.
quit
launch "$CARDS_SESSION"
if grep -q "^detail: ${CARDS_SESSION:0:8} " "$OUT/app.log"; then
  click 700 14;  shot 16-cards-session      # title bar (activates the window); TURN 1 of 2 folded, 120 ticks in two rows
  hover 1160 363; shot 17-tick-hover        # mouse on a tick of TURN 1: the stats line describes that step
  click 1380 345; shot 18-show-steps        # TURN 1 "Show steps": its steps listed between the ticks and the reply
  click 1100 463; shot 19-tool-row-open     # its first tool row (Bash): input and output boxes
  click 1380 345                            # "Hide steps": TURN 1 folded again, same layout as in 16
  click 1160 363; shot 20-tick-click        # the same tick: the turn opens, scrolls to that step and opens it
else
  echo "session $CARDS_SESSION not opened: turn-card steps skipped"
fi

# Row context menus, see the header comment.
quit
launch
click 700 14                                # title bar (activates the window)
rclick 300 191; shot 21-row-menu            # right-click the 3rd row while the 1st stays selected: the menu is about the 3rd
click 700 14                                # title bar: closes the menu
click 383 49;  click 383 79                 # Kind: untick CLI (1st item); the menu closes after each pick
click 383 49;  click 383 105                # untick SDK (2nd)
click 383 49;  click 383 131                # untick BG (3rd): only history-only rows are left
rclick 300 119; shot 22-history-row-menu    # right-click the 1st row: Favorite, Copy Resume, Reveal and Export disabled

echo "screenshots written to $OUT"
cat "$OUT/app.log"
