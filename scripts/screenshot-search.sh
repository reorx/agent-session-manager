#!/usr/bin/env bash
# Launch the app with ASM_SEARCH_AUTOPLAY and capture its window after every step of the scripted
# search session: text search, next hit, two-character CJK, regex, Tools scope, sort by Matches,
# cleared. The app drives itself and the window is captured by id, so nothing is typed or
# clicked: it does not matter which window is in front, and the clipboard is left alone.
# Runs on the real index with a throwaway view state. Usage: screenshot-search.sh <binary> <out-dir>.
# ASM_AUTOPLAY_QUERY / ASM_AUTOPLAY_CJK / ASM_AUTOPLAY_REGEX replace the default queries.
set -euo pipefail

BIN=$1
OUT=$2
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
WINID="$ROOT/tmp/winid"
STEPS=(text-search next-hit cjk-two-chars regex regex-tools sorted-by-matches cleared)

mkdir -p "$OUT" "$ROOT/tmp"
if [ ! -x "$WINID" ] || [ "$HERE/winid.swift" -nt "$WINID" ]; then
  swiftc -O -o "$WINID" "$HERE/winid.swift"
fi
wake-screen

export ASM_STATE="$OUT/state.json"
rm -f "$ASM_STATE" "$OUT"/*.png
ASM_SEARCH_AUTOPLAY=1 "$BIN" > "$OUT/app.log" 2>&1 &
PID=$!
trap 'kill $PID 2>/dev/null; wait $PID 2>/dev/null || true' EXIT
# keep the display from sleeping while the app runs: a sleeping or locked screen makes window
# captures fail halfway (wake-screen only wakes it once, at the start)
caffeinate -d -w "$PID" &
WID=
for _ in $(seq 1 50); do
  WID=$("$WINID" "$(basename "$BIN")" 2>/dev/null) && [ -n "$WID" ] && break
  sleep 0.2
done
[ -n "$WID" ] || { echo "the app window never showed up; see $OUT/app.log" >&2; exit 1; }

# the app prints "autoplay-search: <step>" once a step has settled, then holds the view for 1.5s
n=0
for step in "${STEPS[@]}"; do
  n=$((n + 1))
  for _ in $(seq 1 150); do
    grep -q "^autoplay-search: $step\$" "$OUT/app.log" && break
    sleep 0.1
  done
  if ! grep -q "^autoplay-search: $step\$" "$OUT/app.log"; then
    echo "step '$step' never settled; see $OUT/app.log" >&2
    exit 1
  fi
  screencapture -x -o -l "$WID" "$OUT/$n-$step.png"
done

echo "screenshots written to $OUT"
cat "$OUT/app.log"
