#!/usr/bin/env bash
# Usage acceptance on a COPY of the data: launch the app with ASM_USAGE_AUTOPLAY and capture its
# window at three points: once the start-up scan and the usage ledger are in (1-ready), after
# Scan for Changes (2-scanned) and after Rebuild Index (3-rebuilt); the status bar's `Today $X`
# is on all three. Before the scan the script runs $USAGE_BEFORE_SCAN (a shell command, e.g.
# moving half of the copied transcripts away). After the rebuild it keeps the app open for
# $USAGE_HOLD seconds (default 0) and captures 4-held: automatic scans go on meanwhile (window
# activation, every 10 minutes), and the log says what they did (`auto-scan:`, `scan (automatic):`,
# `today:`). The app drives itself and the window is captured by id: nothing is typed or clicked.
#
# Refuses to run unless ASM_DB and ASM_CLAUDE_DIR point at copies: the real index and usage
# ledger must not be opened by a dev build. ASM_STATE defaults to a fresh file in <out-dir>.
# Usage: screenshot-usage.sh <binary> <out-dir>.
set -euo pipefail

BIN=$1
OUT=$2
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
# shellcheck source=shot-lib.sh
. "$HERE/shot-lib.sh"

require_copies "this script rebuilds the index"
mkdir -p "$OUT"
build_winid
wake-screen

STAMP=$(date +%H%M%S)
export ASM_STATE="${ASM_STATE:-$OUT/state-$STAMP.json}"
# the app scans when `scan` shows up in here, rebuilds when `rebuild` does
GATE=$(mktemp -d "$OUT/gate-$STAMP.XXXX")
LOG="$OUT/app-$STAMP.log"
ASM_USAGE_AUTOPLAY="$GATE" "$BIN" > "$LOG" 2>&1 &
PID=$!
trap 'kill $PID 2>/dev/null || true; wait $PID 2>/dev/null || true' EXIT
# keep the display awake while the app runs (window captures of a sleeping screen fail)
caffeinate -d -w "$PID" &
WID=
for _ in $(seq 1 50); do
  WID=$("$WINID" "$(basename "$BIN")" --pid "$PID" 2>/dev/null) && [ -n "$WID" ] && break
  sleep 0.2
done
[ -n "$WID" ] || { echo "the app window never showed up; see $LOG" >&2; exit 1; }

# waits for "autoplay-usage: <step> · Today …", then captures the window
capture() {
  local n=$1 step=$2
  for _ in $(seq 1 1200); do
    grep -q "^autoplay-usage: $step " "$LOG" && break
    sleep 0.1
  done
  grep -q "^autoplay-usage: $step " "$LOG" || { echo "step '$step' never settled; see $LOG" >&2; exit 1; }
  capture_window "$WID" "$OUT/$n-$step.png"
  grep "^autoplay-usage: $step " "$LOG"
}

capture 1 ready
if [ -n "${USAGE_BEFORE_SCAN:-}" ]; then
  echo "running: $USAGE_BEFORE_SCAN"
  eval "$USAGE_BEFORE_SCAN"
fi
touch "$GATE/scan"
capture 2 scanned
touch "$GATE/rebuild"
capture 3 rebuilt
if [ "${USAGE_HOLD:-0}" -gt 0 ]; then
  echo "holding the app open for $USAGE_HOLD s (log: $LOG)"
  sleep "$USAGE_HOLD"
  capture_window "$WID" "$OUT/4-held.png"
fi

echo "screenshots written to $OUT"
grep -E "^(autoplay-usage|today|auto-scan|scan|usage day):" "$LOG" || true
