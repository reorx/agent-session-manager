#!/usr/bin/env bash
# Launch the app with ASM_SPIKE_AUTOPLAY and capture its window three times
# (initial, after auto-scroll, after auto-sort). Usage: screenshot.sh <binary> <out-dir>
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

# Screenshots of a sleeping display are black; wake-screen exits non-zero if it cannot wake it.
wake-screen

ASM_SPIKE_AUTOPLAY=1 "$BIN" > "$OUT/app.log" 2>&1 &
PID=$!
trap 'kill $PID 2>/dev/null; wait $PID 2>/dev/null || true' EXIT

sleep 2.5
WID=$("$WINID" "$(basename "$BIN")")
screencapture -x -o -l "$WID" "$OUT/1-initial.png"
sleep 3
screencapture -x -o -l "$WID" "$OUT/2-scrolled.png"
sleep 3
screencapture -x -o -l "$WID" "$OUT/3-sorted.png"

echo "screenshots written to $OUT"
cat "$OUT/app.log"
