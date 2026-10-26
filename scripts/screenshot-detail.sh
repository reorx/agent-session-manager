#!/usr/bin/env bash
# Launch the app with ASM_DETAIL_AUTOPLAY and capture its window after every step of the scripted
# tour of the Conversation tab: the busiest session, its longest turn opened, a tool step opened,
# Expand all, a regex search with a hit in a tool output, the next hit, a session with special
# cards (ASM_AUTOPLAY_SPECIAL; skipped without it) and the 70MB session scrolled to its end.
# The app drives itself and the window is captured by id, so nothing is typed or clicked.
# Uses ASM_DB / ASM_STATE like the app (point ASM_DB into tmp/ to leave the real index alone);
# the view state is always a throwaway one. Usage: screenshot-detail.sh <binary> <out-dir>.
# ASM_AUTOPLAY_SESSION / ASM_AUTOPLAY_REGEX / ASM_AUTOPLAY_SPECIAL steer the tour;
# ASM_AUTOPLAY_STEP=cut-output|thinking|mcp makes step-open open that kind of step instead of a
# failed tool call (ASM_AUTOPLAY_CUT_OUTPUT=1 is the same as cut-output).
set -euo pipefail

BIN=$1
OUT=$2
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
WINID="$ROOT/tmp/winid"
STEPS=(conversation steps-open step-open expand-all search-hit next-hit special big-session)

mkdir -p "$OUT" "$ROOT/tmp"
if [ ! -x "$WINID" ] || [ "$HERE/winid.swift" -nt "$WINID" ]; then
  swiftc -O -o "$WINID" "$HERE/winid.swift"
fi
wake-screen

export ASM_STATE="$OUT/state.json"
rm -f "$ASM_STATE" "$OUT"/*.png
ASM_DETAIL_AUTOPLAY=1 "$BIN" > "$OUT/app.log" 2>&1 &
PID=$!
trap 'kill $PID 2>/dev/null; wait $PID 2>/dev/null || true' EXIT
# the tour takes about a minute (longer on a cold index): keep the display from sleeping again,
# a sleeping or locked screen makes window captures fail halfway
caffeinate -d -w "$PID" &
WID=
for _ in $(seq 1 50); do
  WID=$("$WINID" "$(basename "$BIN")" 2>/dev/null) && [ -n "$WID" ] && break
  sleep 0.2
done
[ -n "$WID" ] || { echo "the app window never showed up; see $OUT/app.log" >&2; exit 1; }

# the app prints "autoplay-detail: <step>" once a step has settled (or "<step> skipped"), then
# holds the view for 1.5s; a cold index first has to be scanned, hence the long first wait
n=0
for step in "${STEPS[@]}"; do
  n=$((n + 1))
  for _ in $(seq 1 1200); do
    grep -Eq "^autoplay-detail: $step( skipped)?\$" "$OUT/app.log" && break
    sleep 0.1
  done
  if grep -q "^autoplay-detail: $step skipped\$" "$OUT/app.log"; then
    echo "step '$step' skipped"
    continue
  fi
  if ! grep -q "^autoplay-detail: $step\$" "$OUT/app.log"; then
    echo "step '$step' never settled; see $OUT/app.log" >&2
    exit 1
  fi
  screencapture -x -o -l "$WID" "$OUT/$n-$step.png"
done

echo "screenshots written to $OUT"
grep -E "^(detail|autoplay-detail):" "$OUT/app.log"
