#!/bin/bash
# Behaviour tests for screenshot-resident.sh's verdicts: a stand-in app plays each failure the
# script must catch, with stand-ins for winid, screencapture, cliclick and the other screen tools
# first on PATH. Nothing here starts a GUI, captures the screen or clicks. Usage:
# test-screenshot-resident.sh (or `make test-screenshot-resident`).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fakes="$work/fakes"
mkdir -p "$fakes"

# The app: prints the autoplay's lines for each gate command; FAKE_SCENARIO picks what goes wrong.
cat > "$fakes/app" <<'EOF'
#!/bin/bash
set -u
state=$FAKE_STATE
line() { echo "autoplay-resident: $*"; }
after='$1 | Today  $1.11 | In 689k · Out 16k | Scanned 14:45'
title='$0'
[ "$FAKE_SCENARIO" = bad-title ] && title='$3'
touch "$state/window"
line "shown · $title | Today  \$0.09 | In 53k · Out 1k | Scanned 14:44 · status bar Today \$0.09"
n=1
while :; do
  f=
  for p in "$ASM_RESIDENT_AUTOPLAY/$n-"*; do [ -e "$p" ] && f=$p; done
  if [ -z "$f" ]; then sleep 0.05; continue; fi
  command=${f##*/}
  command=${command#*-}
  n=$((n + 1))
  case "$FAKE_SCENARIO:$command" in
    notice-crash:close) echo "window: first-close notice"; exit 134 ;;
    notice-quit:close) echo "window: first-close notice"; echo "window: first-close notice · 立即退出"; exit 0 ;;
    not-hidden:close) line "close · hidden=false" ;;
    still-visible:close) line "close · hidden=true" ;;
    *:close) rm -f "$state/window"; echo "window: hidden"; line "close · hidden=true" ;;
    menu-closes:menu) echo "menu bar: menu open"; line "menu · open at 1000,30"; echo "menu bar: menu closed (open 0.02 s)" ;;
    *:menu) touch "$state/menu"; echo "menu bar: menu open"; line "menu · open at 1000,30" ;;
    *:close-menu) rm -f "$state/menu"; echo "menu bar: menu closed (open 1.00 s)"; line "close-menu · closed=true" ;;
    crash-scan:scan) exit 139 ;;
    no-scan:scan) line "scan · no scan · \$0 | Today  \$0.09 | In 53k · Out 1k | Scanned 14:44" ;;
    same-today:scan) line "scan · \$0 | Today  \$0.09 | In 53k · Out 1k | Scanned 14:45" ;;
    *:scan) [ -e "$state/menu" ] && rm "$state/menu" && echo "menu bar: menu closed (open 1.00 s)"
            echo "menu bar: ScanNow"; line "scan · $after" ;;
    not-active:open) touch "$state/window"; line "open · shown=true active=false table-focused=true · $after" ;;
    *:open) touch "$state/window"; echo "menu bar: OpenSessions"; line "open · shown=true active=true table-focused=true · $after" ;;
    quit-23:quit) line "quit · through the menu item"; exit 23 ;;
    *:quit) line "quit · through the menu item"; exit 0 ;;
  esac
done
EOF

# winid: the window while it is shown, the menu while it is open; under any point the menu bar
cat > "$fakes/winid" <<'EOF'
#!/bin/bash
case "$1" in
  --screen) echo "1440 900" ;;
  --menu) [ -e "$FAKE_STATE/menu" ] && echo "77 1000 30 300 200" || exit 1 ;;
  --at) echo "1 Window Server" ;;
  -) [ -e "$FAKE_STATE/window" ] && echo 42 || exit 1 ;;
  *) exit 1 ;;
esac
EOF
cat > "$fakes/screencapture" <<'EOF'
#!/bin/bash
for a; do out=$a; done
touch "$out"
EOF
cat > "$fakes/cliclick" <<'EOF'
#!/bin/bash
touch "$FAKE_STATE/clicked"
EOF
cat > "$fakes/lsappinfo" <<'EOF'
#!/bin/bash
case "$1" in
  front) echo "ASN:0x0-0x1:" ;;
  info) echo '"LSDisplayName"="Stand-in"' ;;
esac
EOF
cat > "$fakes/defaults" <<'EOF'
#!/bin/bash
echo bottom
EOF
for tool in wake-screen caffeinate open; do printf '#!/bin/bash\n' > "$fakes/$tool"; done
chmod +x "$fakes"/*

failures=0
# check <scenario> <pass|fail> <text the output must have> [VAR=value...]
check() {
  local name=$1 want=$2 text=$3 dir="$work/$1" out rc=0
  shift 3
  mkdir -p "$dir/state" "$dir/claude"
  out=$(env -i HOME="$HOME" PATH="$fakes:/usr/bin:/bin:/usr/sbin:/sbin" \
    ASM_DB="$dir/index.sqlite" ASM_CLAUDE_DIR="$dir/claude" SHOT_WINID="$fakes/winid" \
    FAKE_STATE="$dir/state" FAKE_SCENARIO="$name" RESIDENT_SETTLE=0 "$@" \
    "$here/screenshot-resident.sh" "$fakes/app" "$dir/out" 2>&1) || rc=$?
  local verdict=ok
  case "$want" in
    pass) [ "$rc" = 0 ] || verdict="exit $rc, wanted 0" ;;
    fail) [ "$rc" != 0 ] || verdict="exit 0, wanted a failure" ;;
  esac
  if [ "$verdict" = ok ] && ! grep -qF -- "$text" <<< "$out"; then verdict="no '$text' in the output"; fi
  if [ "$verdict" = ok ] && [ -e "$dir/state/clicked" ]; then verdict="it clicked"; fi
  if [ "$verdict" = ok ]; then
    echo "ok   $name"
  else
    echo "FAIL $name: $verdict" >&2
    sed 's/^/     | /' <<< "$out" >&2
    failures=$((failures + 1))
  fi
}

check ok pass "all checks passed"
check notice-quit pass "the app quit from the first-close notice (立即退出), exit status 0" RESIDENT_NOTICE=1
check quit-23 fail "quit: exit status 23"
check notice-crash fail "the app exited during the close (exit status 134)" RESIDENT_NOTICE=1
check crash-scan fail "the app exited during 'scan' (exit status 139)"
check not-hidden fail "close: the window did not hide"
check still-visible fail "close: the window is still on screen"
check menu-closes fail "the menu closed by itself before the capture (front app: Stand-in)"
check no-scan fail "scan: no scan finished"
check same-today fail "Today stayed 'Today  \$0.09' after RESIDENT_BEFORE_SCAN" RESIDENT_BEFORE_SCAN=:
check not-active fail "open: the window is not in front"
check bad-title fail "the title \$3 is not the status bar's Today"
# the frontmost window under Scan Now is not the app's menu: no click
check click-elsewhere fail "is '1 Window Server', not the app's menu; not clicking" RESIDENT_CLICK=1

if [ "$failures" -gt 0 ]; then
  echo "$failures failed" >&2
  exit 1
fi
echo "all screenshot-resident.sh checks behave"
