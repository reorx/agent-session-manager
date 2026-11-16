#!/usr/bin/env bash
# Menu bar acceptance on a COPY of the data (kb/plans/2026-11-16-usage-stats/menubar-resident.md):
# launch the app with ASM_RESIDENT_AUTOPLAY and walk it through closing into the menu bar, the
# menu bar item's menu, Scan Now while the window is hidden, Open Sessions and Quit. The app
# drives itself (commands dropped into a gate directory, see `AppRoot::resident_autoplay`); the
# window and the menu are captured by window id, the menu bar and the Dock as screen strips.
# Captures in <out-dir>:
#
#   1-shown-{window,menubar,dock}.png    after the start-up scan: icon and today's cost (acceptance 1)
#   2-hidden-{menubar,dock}.png          after the close: no window, no Dock icon, the process lives on (3)
#   3-menu.png                           the menu, opened at a fixed place (2)
#   4-scanned-{menubar,dock}.png         after Scan Now with the window hidden (4, 9)
#   5-menu.png                           the menu again, with the new numbers
#   6-open-{window,menubar,dock}.png     after Open Sessions: the window and the Dock icon are back (5)
#
# and finally Quit through the menu. The log lines
# `autoplay-resident: <step> · $N | Today … | In … · Out … | Scanned …` carry the item's title and
# the menu's three lines at each step, the `shown` one also the status bar's Today.
#
# Every step is checked: the title is the status bar's Today rounded to dollars, the close hid
# the window (gone from the screen, the process alive), a scan finished (and changed Today after
# RESIDENT_BEFORE_SCAN), each menu was on screen when captured, Open Sessions brought the window
# back in front with the table focused, Quit ended the process with exit status 0. A failed check
# is printed at once and the run goes on for the rest of the evidence, then exits 1 listing
# them; a failure that makes the rest meaningless (no window, the close did not hide it, the app
# died) exits 1 at once. Exit 0 only when every check passed.
#
# Env:
#   RESIDENT_BEFORE_SCAN  shell command run while the window is hidden, before Scan Now (e.g.
#                         appending a call made now to a copied transcript, for acceptance 4)
#   RESIDENT_HOLD         seconds to keep the window hidden after that (default 0), printing the
#                         time of every `auto-scan: timer` meanwhile (one every 600 s from launch,
#                         unless App Nap holds the timer back: a gap over 660 s fails the run);
#                         captures 2b-held-{menubar,dock}.png
#   ASM_RESIDENT_QUERY    searched for before the first capture: 1-shown and 6-open should show the
#                         same query, results and selection
#   RESIDENT_CLICK=1      click Scan Now and Open Sessions in the open menu with cliclick instead
#                         of choosing them programmatically: the click is the user's say-so that
#                         lets macOS bring the app to the front. Clicks only when the frontmost
#                         window under the point is the app's own menu (`winid --at`); otherwise
#                         fails that check and chooses the item programmatically
#   RESIDENT_FRONT_APP    an app (e.g. Finder) brought to the front with `open -a` before the second
#                         menu and Open Sessions: does the window still come to the front?
#   RESIDENT_REOPEN       path of the .app whose binary <binary> is: instead of Open Sessions, open
#                         it again with `open` (Finder's reopen). Refused unless LaunchServices
#                         knows the running instance; a second instance (it would have none of the
#                         env, so the real data) is killed at once and fails the run
#   RESIDENT_NOTICE=1     start without the first-close notice shown: the close shows it and
#                         waits (up to 10 minutes) for you to answer it in the window (7); 立即退出
#                         ends the run there, with exit 0 only if the app says so and exits 0
#   ASM_RESIDENT_MENU_AT  x,y (points from the top-left of the main screen) for the menu
#   RESIDENT_SETTLE       seconds to let the screen settle before a capture (default 0.5)
#
# Without RESIDENT_CLICK, Open Sessions is chosen without a click, so macOS may refuse to bring
# the app to the front while some other app is (a game did): `active=false` fails the run.
# A menu opened this way by an app in the background may close at once while another app is busy
# in front; the menu checks then fail with the front app's name.
# A menu bar manager (Thaw) may keep the item off screen: the *-menubar.png strips then do not
# show it, and the menu is opened at a fixed place instead of under the item. Bringing the app to
# the front happens twice (launch, Open Sessions); keys typed meanwhile go to the app's window.
# The window is found by process id, so <binary> may be the one inside the .app.
# Refuses to run unless ASM_DB and ASM_CLAUDE_DIR point at copies. Usage:
# screenshot-resident.sh <binary> <out-dir>. Its behaviour tests: test-screenshot-resident.sh.
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

SETTLE=${RESIDENT_SETTLE:-0.5}
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

settle() { sleep "$SETTLE"; }
count() { grep -c "$1" "$LOG" || true; }
front_app() { lsappinfo info -only name "$(lsappinfo front)" 2>/dev/null | sed -n 's/.*"LSDisplayName"="\(.*\)".*/\1/p'; }
app_window() { "$WINID" - --pid "$PID" 2>/dev/null; }

# Checks that failed: the run goes on, and exits 1 at the end (`finish`).
FAILED=
fail_check() {
  echo "FAILED: $*" >&2
  FAILED="$FAILED
  - $*"
}
# A failure that makes the rest of the run meaningless.
die() {
  echo "FAILED: $*" >&2
  echo "see $LOG" >&2
  exit 1
}
# The app's exit status, once it is gone (APP_RC; `wait` answers only once, so not in a subshell).
reap() {
  APP_RC=0
  wait "$PID" 2>/dev/null || APP_RC=$?
}
finish() {
  echo "screenshots written to $OUT"
  grep -E "^(autoplay-resident|menu bar|window|auto-scan|scan|today):" "$LOG" || true
  if [ -n "$FAILED" ]; then
    echo "FAILED checks:$FAILED" >&2
    exit 1
  fi
  echo "all checks passed"
  exit 0
}

WID=
for _ in $(seq 1 50); do
  WID=$(app_window) && [ -n "$WID" ] && break
  WID=
  kill -0 "$PID" 2>/dev/null || break
  sleep 0.2
done
[ -n "$WID" ] || die "the app window never showed up"

# waits until `autoplay-resident: <step> ` was printed more than <before> times and prints that
# line; 2 when the app is gone first, 1 on timeout
wait_line() {
  local step=$1 before=$2 limit=$3
  for _ in $(seq 1 $((limit * 10))); do
    if [ "$(count "^autoplay-resident: $step ")" -gt "$before" ]; then
      grep "^autoplay-resident: $step " "$LOG" | tail -1
      return 0
    fi
    kill -0 "$PID" 2>/dev/null || return 2
    sleep 0.1
  done
  return 1
}

N=0
LINE=
# step <command> [timeout s]: drops the next command into the gate, waits for its line (LINE).
# Returns 2 when the app is gone before printing it (only the close may end that way, from the
# first-close notice); any other failure ends the run.
step() {
  local before rc=0
  before=$(count "^autoplay-resident: $1 ")
  N=$((N + 1))
  touch "$GATE/$N-$1"
  LINE=$(wait_line "$1" "$before" "${2:-120}") || rc=$?
  case $rc in
    0) echo "$LINE" ;;
    2) [ "$1" = close ] && return 2
       reap
       die "the app exited during '$1' (exit status $APP_RC)" ;;
    *) die "step '$1' never finished" ;;
  esac
}

capture() {
  capture_window "$1" "$OUT/$2.png" || fail_check "$2: the capture failed"
}
strips() {
  capture_rect "0,0,$SCREEN_W,40" "$OUT/$1-menubar.png" || fail_check "$1-menubar: the capture failed"
  case "$DOCK" in
    left) capture_rect "0,0,120,$SCREEN_H" "$OUT/$1-dock.png" ;;
    right) capture_rect "$((SCREEN_W - 120)),0,120,$SCREEN_H" "$OUT/$1-dock.png" ;;
    *) capture_rect "0,$((SCREEN_H - 120)),$SCREEN_W,120" "$OUT/$1-dock.png" ;;
  esac || fail_check "$1-dock: the capture failed"
}

# the fields of a step line: 1 is the title (after `· `), 2 the Today line
field() { awk -F ' [|] ' -v n="$2" '{ sub(/^[^·]*· /, "", $1); print $n }' <<< "$1"; }

# The title is the status bar's Today rounded to dollars, nothing without a ledger (acceptance 1).
check_title() {
  local title today want
  title=$(field "$1" 1)
  title=${title// /}
  today=$(sed -n 's/.*· status bar Today \([^ ]*\).*/\1/p' <<< "$1")
  case "$today" in
    '$'*) want=$(awk -v v="${today//[\$,]/}" 'BEGIN { printf "%d", int(v + 0.5) }') ;;
    *) want= ;;
  esac
  title=${title//[\$,]/}
  if [ "$title" != "$want" ]; then fail_check "shown: the title \$$title is not the status bar's Today $today rounded"; fi
}

# The menu opened by the last `menu` step: its top-left corner is in that line (`open at X,Y`).
MENU_X= MENU_Y=
menu_open() { [ "$(count '^menu bar: menu open')" -gt "$(count '^menu bar: menu closed')" ]; }
# menu_step <capture>: opens the menu and captures its window. False when there is no menu to
# work with (the check failed).
menu_step() {
  local at found id x y w h
  step menu
  at=$(sed -n 's/.*open at \([0-9]*\),\([0-9]*\).*/\1 \2/p' <<< "$LINE")
  if [ -z "$at" ]; then fail_check "$1: the menu did not open ($LINE)"; return 1; fi
  read -r MENU_X MENU_Y <<< "$at"
  settle
  if ! menu_open; then
    fail_check "$1: the menu closed by itself before the capture (front app: $(front_app))"
    return 1
  fi
  if ! found=$("$WINID" --menu --pid "$PID" 2>/dev/null); then
    fail_check "$1: the menu says it is open but no menu window of the app is on screen (front app: $(front_app)); $1-rect.png is the place it should be"
    capture_rect "$((MENU_X - 10)),$((MENU_Y - 10)),420,240" "$OUT/$1-rect.png" || true
    return 1
  fi
  read -r id x y w h <<< "$found"
  capture_window "$id" "$OUT/$1.png" || capture_rect "$x,$y,$w,$h" "$OUT/$1.png" || fail_check "$1: the capture failed"
  if ! menu_open; then fail_check "$1: the menu closed during the capture: $1.png may not show it"; return 1; fi
  return 0
}

# --- launch -------------------------------------------------------------------------------
wait_line shown 0 120 > /dev/null || die "the app never printed 'shown' (exit or timeout)"
LINE=$(grep "^autoplay-resident: shown " "$LOG" | tail -1)
echo "$LINE"
SHOWN_TODAY=$(field "$LINE" 2)
check_title "$LINE"
capture "$WID" 1-shown-window
strips 1-shown

# --- close --------------------------------------------------------------------------------
rc=0
step close 600 || rc=$?
if [ "$rc" = 2 ]; then
  reap
  if grep -q '^window: first-close notice · 立即退出' "$LOG" && [ "$APP_RC" = 0 ]; then
    echo "the app quit from the first-close notice (立即退出), exit status 0"
    finish
  fi
  die "the app exited during the close (exit status $APP_RC) without 立即退出 being chosen"
fi
case "$LINE" in *"hidden=true"*) ;; *) die "close: the window did not hide ($LINE)" ;; esac
settle
settle
strips 2-hidden
if app_window > /dev/null; then
  fail_check "close: the window is still on screen"
else
  echo "window on screen: no"
fi
if kill -0 "$PID" 2>/dev/null; then
  echo "process: $(ps -p "$PID" -o pid=,comm=)"
else
  reap
  die "the app is gone after the close (exit status $APP_RC)"
fi

# --- hidden: new usage, the timer ---------------------------------------------------------
if [ -n "${RESIDENT_BEFORE_SCAN:-}" ]; then
  echo "running: $RESIDENT_BEFORE_SCAN"
  eval "$RESIDENT_BEFORE_SCAN"
fi
if [ "${RESIDENT_HOLD:-0}" -gt 0 ]; then
  echo "$(date +%T) holding the window hidden for $RESIDENT_HOLD s"
  seen=$(count '^auto-scan: timer')
  last=$SECONDS
  end=$((SECONDS + RESIDENT_HOLD))
  while [ $SECONDS -lt $end ]; do
    now=$(count '^auto-scan: timer')
    if [ "$now" -gt "$seen" ]; then
      echo "$(date +%T) auto-scan: timer (#$now, $((SECONDS - last)) s after the last one or the hold's start) · $(grep '^menu bar: \$' "$LOG" | tail -1)"
      seen=$now
      last=$SECONDS
    elif [ $((SECONDS - last)) -gt 660 ]; then
      fail_check "hold: no automatic scan for 660 s while hidden (App Nap?)"
      last=$SECONDS
    fi
    sleep 1
  done
  strips 2b-held
fi

# click_item <label> <dy>: clicks the menu item <dy> points below the menu's top-left corner
# (offsets measured on the menu's captures), only when the frontmost window under the point is
# the app's own menu: never into another app. False when it did not click.
click_item() {
  local cx=$((MENU_X + 90)) cy=$((MENU_Y + $2)) under
  under=$("$WINID" --at "$cx,$cy" 2>/dev/null || echo "nothing")
  if [ "${under%% *}" = "$PID" ] && menu_open; then
    echo "clicking $1 at $cx,$cy (under it: $under)"
    cliclick -e 150 "m:$cx,$cy" w:200 "c:$cx,$cy"
    return 0
  fi
  fail_check "click $1: the window under $cx,$cy is '$under', not the app's menu; not clicking"
  return 1
}
# clicked <command>: the click chose that item (`menu bar: <command>` since the click)
clicked() { [ "$(count "^menu bar: $1\$")" -gt "$2" ] || fail_check "click: the click did not choose $1"; }

# --- the menu, Scan Now -------------------------------------------------------------------
before=$(count '^menu bar: ScanNow$')
if menu_step 3-menu && [ "${RESIDENT_CLICK:-0}" = 1 ] && click_item "Scan Now" 124; then
  step wait-scan
  clicked ScanNow "$before"
else
  step scan
fi
case "$LINE" in *"no scan"*) fail_check "scan: no scan finished ($LINE)" ;; esac
if [ -n "${RESIDENT_BEFORE_SCAN:-}" ] && [ "$(field "$LINE" 2)" = "$SHOWN_TODAY" ]; then
  fail_check "scan: Today stayed '$SHOWN_TODAY' after RESIDENT_BEFORE_SCAN"
fi
settle
strips 4-scanned

# --- another app in front, the menu again, Open Sessions ----------------------------------
if [ -n "${RESIDENT_FRONT_APP:-}" ]; then
  open -a "$RESIDENT_FRONT_APP"
  for _ in $(seq 1 50); do [ "$(front_app)" = "$RESIDENT_FRONT_APP" ] && break; sleep 0.1; done
  if [ "$(front_app)" = "$RESIDENT_FRONT_APP" ]; then
    echo "front app: $RESIDENT_FRONT_APP"
  else
    fail_check "front: $RESIDENT_FRONT_APP did not come to the front (front app: $(front_app))"
  fi
fi

how="Open Sessions chosen without a click"
before=$(count '^menu bar: OpenSessions$')
if menu_step 5-menu; then
  if [ "${RESIDENT_CLICK:-0}" = 1 ] && [ -z "${RESIDENT_REOPEN:-}" ] && click_item "Open Sessions" 100; then
    how="a click on Open Sessions"
  else
    step close-menu
  fi
fi
if [ -n "${RESIDENT_REOPEN:-}" ]; then
  case "$BIN" in "$RESIDENT_REOPEN"/Contents/MacOS/*) ;; *) die "reopen: $BIN is not inside $RESIDENT_REOPEN" ;; esac
  [ -n "$(lsappinfo find pid="$PID")" ] || die "reopen: LaunchServices does not know pid $PID, so open would start a second instance"
  how="the app opened again (open $RESIDENT_REOPEN)"
  open "$RESIDENT_REOPEN"
  sleep 1
  for other in $(pgrep -f "$RESIDENT_REOPEN/Contents/MacOS/" || true); do
    if [ "$other" != "$PID" ]; then
      kill "$other" 2>/dev/null || true
      die "reopen: open started a second instance (pid $other, killed)"
    fi
  done
fi
if [ "$how" = "Open Sessions chosen without a click" ]; then step open; else step wait-open; fi
case "$how" in "a click"*) clicked OpenSessions "$before" ;; esac
case "$how" in "the app opened again"*) grep -q '^window: bring forward' "$LOG" || fail_check "reopen: the app never heard of it (no 'window: bring forward')" ;; esac
case "$LINE" in *"shown=true"*) ;; *) fail_check "open: the window did not come back ($LINE)" ;; esac
case "$LINE" in *"table-focused=true"*) ;; *) fail_check "open: the table does not have the focus ($LINE)" ;; esac
case "$LINE" in
  *"active=true"*) echo "in front after $how" ;;
  *) fail_check "open: the window is not in front after $how (front app: $(front_app))" ;;
esac
settle
capture "$WID" 6-open-window
strips 6-open

# --- Quit ---------------------------------------------------------------------------------
step quit 5
for _ in $(seq 1 50); do kill -0 "$PID" 2>/dev/null || break; sleep 0.1; done
if kill -0 "$PID" 2>/dev/null; then die "still running 5 s after Quit (pid $PID)"; fi
reap
if [ "$APP_RC" = 0 ]; then echo "quit: exit status 0"; else fail_check "quit: exit status $APP_RC"; fi
finish
