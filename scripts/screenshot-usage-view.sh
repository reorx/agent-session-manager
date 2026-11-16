#!/usr/bin/env bash
# Usage view tour (kb/plans/2026-11-16-usage-stats/usage-view.md) on a COPY of the index and the
# usage ledger: launch the app with ASM_USAGE_VIEW_AUTOPLAY and capture its window after each step
# of `AppRoot::usage_view_autoplay`. Captures in <out-dir>:
#
#   01-today.png        Today by the hour: the line at now, the dashed boxes of the day before
#   02-today-hover.png  the tooltip of today's most expensive hour (shown without the mouse)
#   03-yesterday.png    Yesterday by the hour
#   04-d30.png          30 days: a bar a day, the 7-day average, `so far` over today
#   05-d30-in.png       the chart in In
#   06-d30-hit.png      the chart in Hit rate: a line, the axis not from 0
#   07-d30-hover.png    the tooltip of the most expensive day
#   08-drill.png        that day by the hour, its chip next to the range buttons
#   09-back.png         back in the 30 days (Escape's path)
#   10-project.png      the most expensive project picked: its row lit, the numbers its own
#   11-month.png        This month, the project still picked
#   12-narrow.png       the window 900 px wide: the panels one above the other
#   13-sessions.png     the session table, the project filter still on
#   14-jump-project.png the project's ↗ (kb/plans/2026-11-16-usage-stats/usage-drilldown.md): the
#                       table with that project, Updated any time, the `Active …` tag, only the
#                       sessions with a call in This month
#   15-jump-session.png the most expensive session the table has, as a click on it in Usage: it is
#                       selected, in the detail panel, the keys on the table
#   16-empty.png        a range without calls: $0.00, empty axes, "No calls in this range"
#   17-no-usage.png     a second launch on an empty data directory: "No usage recorded yet"
#
# The app prints its state with each step (`autoplay-usage-view: <step> · view=… range=…
# chart=hourly/24 hover=… empty=… calls=… project=… width=… today-matches=… active=… updated=…
# rows=… selected=… detail=… focus=… usage-sessions=…`) and holds it until
# the script has captured it (`<step>.shot` in the gate directory). The script checks the state:
# today by the hour with its cost equal to the status bar's Today, 30 bars a day, the metric
# switched, a tooltip pinned, the drill by the hour and back to 30 days, the project picked and
# kept in the session table, the narrow width, the jumps (the Active filter on and Updated any
# time, the rows those of the notice's count; the revealed session selected with the keys on the
# table), the empty states. The rows of the project jump go to the log as `jump-rows: <ids>`, to
# compare with the ledger (see gpui-kit-guide.md). A failed check prints FAILED and
# the run goes on; it exits 1 at the end. A step the app skipped (no calls to hover or drill into)
# is not captured and fails too. Nothing is typed or clicked, and the app never comes to the
# front by itself.
#
# Env:
#   ASM_DB              required, and not in the real data directory: a dev build must not open
#                       the real index and usage ledger. The ledger is the `usage-ledger.sqlite`
#                       next to it: put a copy there for real numbers (see gpui-kit-guide.md)
#   ASM_CLAUDE_DIR      the transcripts; may be the real ~/.claude (only read) or a copy
#   ASM_STATE           defaults to a fresh file in <out-dir>, so the tour starts from defaults
#   ASM_USAGE_VIEW_STEPS  comma-separated steps to run instead of all (their names above)
#   USAGE_VIEW_FRESH=0  skip the second launch (15-no-usage)
#
# Usage: screenshot-usage-view.sh <binary> <out-dir>.
set -euo pipefail

BIN=$1
OUT=$2
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
# shellcheck source=shot-lib.sh
. "$HERE/shot-lib.sh"

require_db_copy "a dev build must not open the real index and usage ledger"
mkdir -p "$OUT"
build_winid
wake-screen

STAMP=$(date +%H%M%S)
STEPS="today today-hover yesterday d30 d30-in d30-hit d30-hover drill back project month narrow sessions jump-project jump-session empty"
if [ -n "${ASM_USAGE_VIEW_STEPS:-}" ]; then
  STEPS=$(echo "$ASM_USAGE_VIEW_STEPS" | tr ',' ' ')
fi

FAILED=
fail_check() {
  echo "FAILED: $*" >&2
  FAILED="$FAILED
  - $*"
}

PID=
cleanup() {
  [ -z "$PID" ] && return 0
  kill "$PID" 2>/dev/null || true
  wait "$PID" 2>/dev/null || true
}
trap cleanup EXIT

# launch <log> <gate>: the app with the autoplay on, its window id in WID
launch() {
  local log=$1 gate=$2
  ASM_USAGE_VIEW_AUTOPLAY="$gate" "$BIN" > "$log" 2>&1 &
  PID=$!
  # keep the display awake while the app runs (captures of a sleeping screen fail)
  caffeinate -d -w "$PID" &
  WID=
  for _ in $(seq 1 100); do
    WID=$("$WINID" - --pid "$PID" 2>/dev/null) && [ -n "$WID" ] && break
    WID=
    kill -0 "$PID" 2>/dev/null || break
    sleep 0.2
  done
  [ -n "$WID" ] || { echo "FAILED: the app window never showed up; see $log" >&2; exit 1; }
}

# step <log> <gate> <n> <step>: waits for the step's line (the first one waits out the start-up
# scan), captures the window as <n>-<step>.png, lets the app go on, and leaves the line in LINE.
step() {
  local log=$1 gate=$2 n=$3 name=$4 tries=600
  [ "$n" = 01 ] && tries=3000
  LINE=
  for _ in $(seq 1 "$tries"); do
    LINE=$(grep -m1 -E "^autoplay-usage-view: $name( |$)" "$log" || true)
    [ -n "$LINE" ] && break
    if ! kill -0 "$PID" 2>/dev/null; then
      echo "FAILED: the app is gone before '$name'; see $log" >&2
      exit 1
    fi
    sleep 0.1
  done
  if [ -z "$LINE" ]; then
    echo "FAILED: step '$name' never settled; see $log" >&2
    exit 1
  fi
  echo "$LINE"
  case "$LINE" in
    *" skipped · "*)
      fail_check "$name: ${LINE#*skipped · }"
      return 0
      ;;
  esac
  sleep 0.3
  capture_window "$WID" "$OUT/$n-$name.png" || fail_check "$name: the capture failed"
  touch "$gate/$name.shot"
}

# has <step> <key=value>: the step's line carries it
has() {
  case " $LINE " in
    *" $2 "*) ;;
    *) fail_check "$1: expected $2" ;;
  esac
}
# lacks <step> <key=value>
lacks() {
  case " $LINE " in
    *" $2 "*) fail_check "$1: did not expect $2" ;;
    *) ;;
  esac
}

check() {
  local name=$1
  case "$LINE" in *" skipped · "*) return 0 ;; esac
  case "$name" in
    today)
      has today view=usage
      has today range=Today
      echo "$LINE" | grep -qE " chart=hourly/(23|24|25) " || fail_check "today: expected 24 hours (23 or 25 on a DST day)"
      has today today-matches=yes
      ;;
    today-hover | d30-hover) lacks "$name" hover=- ; has "$name" metric=cost ;;
    yesterday) has yesterday "range=Yesterday" ;;
    d30) has d30 "chart=daily/30" ; has d30 metric=cost ; has d30 hover=- ;;
    d30-in) has d30-in metric=in ;;
    d30-hit) has d30-hit metric=hit ;;
    drill)
      echo "$LINE" | grep -qE " chart=hourly/(23|24|25) " || fail_check "drill: expected the day by the hour"
      lacks drill "range=Last 30 days"
      ;;
    back) has back "chart=daily/30" ; echo "$LINE" | grep -q "range=Last 30 days " || fail_check "back: expected the 30 days again" ;;
    project) lacks project project=- ;;
    month) echo "$LINE" | grep -q "range=This month " || fail_check "month: expected This month" ; lacks month project=- ;;
    narrow) has narrow width=900 ;;
    sessions) has sessions view=sessions ; lacks sessions project=- ; has sessions active=- ;;
    jump-project)
      has jump-project view=sessions ; lacks jump-project project=- ; lacks jump-project active=-
      has jump-project updated=any ; lacks jump-project rows=0 ; has jump-project focus=table
      # the notice counts what Usage has and the table does not list (acceptance 10)
      local rows usage unlisted
      rows=$(echo "$LINE" | sed -nE 's/.* rows=([0-9]+) .*/\1/p')
      usage=$(echo "$LINE" | sed -nE 's/.* usage-sessions=([0-9]+)( .*|$)/\1/p')
      unlisted=$(grep -E "^notice: 这段时间还有 [0-9]+ 个" "$LOG" | tail -1 | sed -nE 's/^notice: 这段时间还有 ([0-9]+) 个.*/\1/p' || true)
      [ $((rows + ${unlisted:-0})) = "$usage" ] || fail_check "jump-project: $rows rows + ${unlisted:-0} unlisted is not Usage's $usage sessions"
      ;;
    jump-session)
      has jump-session view=sessions ; lacks jump-session selected=- ; has jump-session detail=yes ; has jump-session focus=table
      local sel
      sel=$(echo "$LINE" | sed -nE 's/.* selected=([^ ]+) .*/\1/p')
      grep -q "^reveal: $sel$" "$LOG" || fail_check "jump-session: $sel is not the session revealed"
      ;;
    empty) has empty view=usage ; has empty empty=no-calls ; has empty calls=0 ; has empty project=- ;;
  esac
}

GATE=$(mktemp -d "$OUT/gate-$STAMP.XXXX")
LOG="$OUT/app-$STAMP.log"
export ASM_STATE="${ASM_STATE:-$OUT/state-$STAMP.json}"
launch "$LOG" "$GATE"
n=0
for name in $STEPS; do
  n=$((n + 1))
  step "$LOG" "$GATE" "$(printf %02d "$n")" "$name"
  check "$name"
done
for _ in $(seq 1 50); do
  grep -q "^autoplay-usage-view: done" "$LOG" && break
  sleep 0.1
done
cleanup
PID=

if [ "${USAGE_VIEW_FRESH:-1}" != 0 ] && [ -z "${ASM_USAGE_VIEW_STEPS:-}" ]; then
  # an empty data directory: the ledger holds nothing yet
  FRESH=$(mktemp -d "$OUT/fresh-$STAMP.XXXX")
  mkdir -p "$FRESH/claude"
  FRESH_LOG="$OUT/app-$STAMP-fresh.log"
  ASM_DB="$FRESH/index.sqlite" ASM_CLAUDE_DIR="$FRESH/claude" ASM_STATE="$FRESH/state.json" ASM_USAGE_VIEW_STEPS=today launch "$FRESH_LOG" "$FRESH"
  n=$((n + 1))
  step "$FRESH_LOG" "$FRESH" "$(printf %02d "$n")" today
  mv "$OUT/$(printf %02d "$n")-today.png" "$OUT/$(printf %02d "$n")-no-usage.png" 2>/dev/null || true
  has no-usage empty=no-usage
  cleanup
  PID=
fi

echo "screenshots written to $OUT"
grep -E "^(autoplay-usage-view|usage view|view):" "$LOG" || true
if [ -n "$FAILED" ]; then
  echo "FAILED checks:$FAILED" >&2
  exit 1
fi
echo "all checks passed"
