# Helpers shared by the screenshot scripts that run on a COPY of the data
# (screenshot-usage.sh, screenshot-resident.sh). Source it after setting HERE (scripts/) and
# ROOT (the repo); it sets WINID.

WINID="$ROOT/tmp/winid"

# Refuses to go on unless ASM_DB and ASM_CLAUDE_DIR point at copies: a dev build must not open
# the real index and usage ledger. $1 says why the script needs copies.
require_copies() {
  local real_data="$HOME/Library/Application Support/dev.reorx.AgentSessionManager"
  if [ -z "${ASM_DB:-}" ] || [ -z "${ASM_CLAUDE_DIR:-}" ]; then
    echo "set ASM_DB and ASM_CLAUDE_DIR to copies (e.g. under tmp/): $1" >&2
    exit 2
  fi
  case "$ASM_DB" in "$real_data"/*) echo "ASM_DB is in the real data directory: $ASM_DB" >&2; exit 2 ;; esac
  if [ "$(cd "$ASM_CLAUDE_DIR" && pwd -P)" = "$(cd "$HOME/.claude" && pwd -P)" ]; then
    echo "ASM_CLAUDE_DIR is the real ~/.claude: use a copy" >&2
    exit 2
  fi
}

# Compiles scripts/winid.swift into tmp/winid when it is missing or older than the source.
build_winid() {
  mkdir -p "$ROOT/tmp"
  if [ ! -x "$WINID" ] || [ "$HERE/winid.swift" -nt "$WINID" ]; then
    swiftc -O -o "$WINID" "$HERE/winid.swift"
  fi
}

# capture_window <window id> <png>. screencapture now and then fails with "could not create
# image from window" and the same capture a moment later works (2026-11-16): one retry.
capture_window() {
  screencapture -x -o -l "$1" "$2" 2>/dev/null && return 0
  echo "capture of window $1 failed, once more" >&2
  sleep 0.5
  screencapture -x -o -l "$1" "$2"
}

# capture_rect <x,y,w,h> <png>: a screen region (points from the top-left of the main screen),
# with the same one retry ("could not create image from rect", 2026-11-16).
capture_rect() {
  screencapture -x -R"$1" "$2" 2>/dev/null && return 0
  echo "capture of $1 failed, once more" >&2
  sleep 0.5
  screencapture -x -R"$1" "$2"
}
