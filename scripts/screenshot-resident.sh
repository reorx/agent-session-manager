#!/usr/bin/env bash
# Menu bar acceptance on a COPY of the data (kb/plans/2026-11-16-usage-stats/menubar-resident.md):
# launch the app with ASM_RESIDENT_AUTOPLAY and walk it through closing into the menu bar, the
# menu bar item's menu, Scan Now while the window is hidden, Open Sessions and Quit. The app
# drives itself (commands dropped into a gate directory, see `AppRoot::resident_autoplay`); the
# window is captured by id, the menu bar and the Dock as screen strips. Captures in <out-dir>:
#
#   1-shown-{window,menubar,dock}.png    after the start-up scan: icon and today's cost (acceptance 1)
#   2-hidden-{menubar,dock}.png          after the close: no window, no Dock icon, the process lives on (3)
#   3-menu.png                           the menu, opened at a fixed place (2)
#   4-scanned-menubar.png                after Scan Now with the window hidden (4, 9)
#   5-menu.png                           the menu again, with the new numbers
#   6-open-{window,menubar,dock}.png     after Open Sessions: the window and the Dock icon are back (5)
#
# and finally Quit through the menu: the process must be gone (8). The log lines
# `autoplay-resident: <step> · $N | Today … | In … · Out … | Scanned …` carry the item's title and
# the menu's three lines at each step, the `shown` one also the status bar's Today.
#
# Env:
#   RESIDENT_BEFORE_SCAN  shell command run while the window is hidden, before Scan Now (e.g.
#                         appending a call made now to a copied transcript, for acceptance 4)
#   RESIDENT_HOLD         seconds to keep the window hidden after that (default 0), printing the
#                         time of every `auto-scan: timer` meanwhile (one every 600 s from launch,
#                         unless App Nap holds the timer back); captures 2b-held-{menubar,dock}.png
#   ASM_RESIDENT_QUERY    searched for before the first capture: 1-shown and 6-open should show the
#                         same query, results and selection
#   RESIDENT_CLICK=1      click Scan Now in the open menu with cliclick instead of choosing it
#                         programmatically; clicks only while the app's log says the menu is open
#   RESIDENT_NOTICE=1     start without the first-close notice shown: the close shows it and
#                         waits (up to 10 minutes) for you to answer it in the window (7); 立即退出
#                         ends the run there
#   ASM_RESIDENT_MENU_AT  x,y (points from the top-left of the main screen) for the menu
#
# A menu bar manager (Thaw) may keep the item off screen: the *-menubar.png strips then do not
# show it, and the menu is opened at a fixed place instead of under the item. Bringing the app to
# the front happens twice (launch, Open Sessions); keys typed meanwhile go to the app's window.
# Refuses to run unless ASM_DB and ASM_CLAUDE_DIR point at copies. Usage:
# screenshot-resident.sh <binary> <out-dir>.
set -euo pipefail

BIN=$1
OUT=$2
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
# shellcheck source=shot-lib.sh
. "$HERE/shot-lib.sh"

require_copies "a dev build must not open the real index and usage ledger"
mkdir -p "$OUT"
build_winid
wake-screen

read -r SCREEN_W SCREEN_H < <("$WINID" --screen)
DOCK=$(defaults read com.apple.dock orientation 2>/dev/null || echo bottom)
STAMP=$(date +%H%M%S)
export ASM_STATE="${ASM_STATE:-$OUT/state-$STAMP.json}"
if [ "${RESIDENT_NOTICE:-0}" != 1 ] && [ ! -e "$ASM_STATE" ]; then
  # as on every launch after the first close: the window closes without the notice
  echo '{"resident_notice_shown": true}' > "$ASM_STATE"
fi
GATE=$(mktemp -d "$OUT/gate-$STAMP.XXXX")
LOG="$OUT/app-$STAMP.log"
ASM_RESIDENT_AUTOPLAY="$GATE" "$BIN" > "$LOG" 2>&1 &
PID=$!
trap 'kill $PID 2>/dev/null || true; wait $PID 2>/dev/null || true' EXIT
# keep the display awake while the app runs (captures of a sleeping screen fail)
caffeinate -d -w "$PID" &
WID=
for _ in $(seq 1 50); do
  WID=$("$WINID" "$(basename "$BIN")" --pid "$PID" 2>/dev/null) && [ -n "$WID" ] && break
  sleep 0.2
done
[ -n "$WID" ] || { echo "the app window never showed up; see $LOG" >&2; exit 1; }

count() { grep -c "$1" "$LOG" || true; }

# waits until `autoplay-resident: <step> ` was printed more than <before> times, prints the line
wait_line() {
  local step=$1 before=$2 limit=$3
  for _ in $(seq 1 $((limit * 10))); do
    if [ "$(count "^autoplay-resident: $step ")" -gt "$before" ]; then
      grep "^autoplay-resident: $step " "$LOG" | tail -1
      return 0
    fi
    kill -0 "$PID" 2>/dev/null || break
    sleep 0.1
  done
  if ! kill -0 "$PID" 2>/dev/null && [ "$step" = close ] && grep -q "^window: first-close notice" "$LOG"; then
    echo "the app quit from the first-close notice (立即退出)"
    exit 0
  fi
  echo "step '$step' never finished; see $LOG" >&2
  exit 1
}

N=0
# step <command> [timeout s]: drops the next command into the gate, waits for its line
step() {
  local before
  before=$(count "^autoplay-resident: $1 ")
  N=$((N + 1))
  touch "$GATE/$N-$1"
  wait_line "$1" "$before" "${2:-120}"
}

strips() {
  capture_rect "0,0,$SCREEN_W,40" "$OUT/$1-menubar.png"
  case "$DOCK" in
    left) capture_rect "0,0,120,$SCREEN_H" "$OUT/$1-dock.png" ;;
    right) capture_rect "$((SCREEN_W - 120)),0,120,$SCREEN_H" "$OUT/$1-dock.png" ;;
    *) capture_rect "0,$((SCREEN_H - 120)),$SCREEN_W,120" "$OUT/$1-dock.png" ;;
  esac
}

# the menu opened by the last `menu` step: its top-left corner is in that line (`open at X,Y`)
MENU_X= MENU_Y=
menu_at() {
  local at
  at=$(grep "^autoplay-resident: menu " "$LOG" | tail -1 | sed -n 's/.*open at \([0-9]*\),\([0-9]*\).*/\1 \2/p')
  [ -n "$at" ] || { echo "the menu did not open; see $LOG" >&2; exit 1; }
  read -r MENU_X MENU_Y <<< "$at"
}

menu_open() { [ "$(count '^menu bar: menu open')" -gt "$(count '^menu bar: menu closed')" ]; }

wait_line shown 0 120
capture_window "$WID" "$OUT/1-shown-window.png"
strips 1-shown

step close 600
sleep 1
strips 2-hidden
if "$WINID" "$(basename "$BIN")" --pid "$PID" > /dev/null 2>&1; then echo "window on screen: YES (should be hidden)"; else echo "window on screen: no"; fi
echo "process: $(ps -p "$PID" -o pid=,comm= || echo gone)"

if [ -n "${RESIDENT_BEFORE_SCAN:-}" ]; then
  echo "running: $RESIDENT_BEFORE_SCAN"
  eval "$RESIDENT_BEFORE_SCAN"
fi
if [ "${RESIDENT_HOLD:-0}" -gt 0 ]; then
  echo "$(date +%T) holding the window hidden for $RESIDENT_HOLD s"
  seen=$(count '^auto-scan: timer')
  end=$((SECONDS + RESIDENT_HOLD))
  while [ $SECONDS -lt $end ]; do
    now=$(count '^auto-scan: timer')
    if [ "$now" -gt "$seen" ]; then
      echo "$(date +%T) auto-scan: timer (#$now) · $(grep '^menu bar: \$' "$LOG" | tail -1)"
      seen=$now
    fi
    sleep 1
  done
  strips 2b-held
fi

step menu
menu_at
sleep 0.5
capture_rect "$((MENU_X - 10)),$((MENU_Y - 10)),420,240" "$OUT/3-menu.png"
if [ "${RESIDENT_CLICK:-0}" = 1 ]; then
  # Scan Now is the fifth line (offsets measured in usage-p2-probe); never click a closed menu,
  # the click would land in whatever window is behind it
  if menu_open; then
    cliclick -e 150 "m:$((MENU_X + 90)),$((MENU_Y + 124))" w:200 "c:$((MENU_X + 90)),$((MENU_Y + 124))"
    step wait-scan
  else
    echo "the menu closed before the click; choosing Scan Now programmatically" >&2
    step scan
  fi
else
  step scan
fi
sleep 0.5
strips 4-scanned

step menu
menu_at
sleep 0.5
capture_rect "$((MENU_X - 10)),$((MENU_Y - 10)),420,240" "$OUT/5-menu.png"
step close-menu

step open
sleep 0.5
capture_window "$WID" "$OUT/6-open-window.png"
strips 6-open

step quit 5
for _ in $(seq 1 50); do kill -0 "$PID" 2>/dev/null || break; sleep 0.1; done
if kill -0 "$PID" 2>/dev/null; then echo "still running after Quit (pid $PID)" >&2; exit 1; fi
wait "$PID" 2>/dev/null && echo "quit: exit status 0" || echo "quit: exit status $?"

echo "screenshots written to $OUT"
grep -E "^(autoplay-resident|menu bar|window|auto-scan|scan|today):" "$LOG" || true
