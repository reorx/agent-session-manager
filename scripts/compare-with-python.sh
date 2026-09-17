#!/usr/bin/env bash
# Compare `asm` with the reference script on the real ~/.claude (ANSI stripped, byte-for-byte diff).
# Usage: scripts/compare-with-python.sh [ASM_BIN] [OUT_DIR]
set -u

ASM=${1:-$(cargo metadata --format-version 1 --no-deps 2>/dev/null | python3 -c 'import sys, json; print(json.load(sys.stdin)["target_directory"])')/release/asm}
OUT=${2:-tmp/$(date +%Y-%m-%d)-python-parity}
PY=~/Code/scripts/claude_session_search.py
export ASM_DB=${ASM_DB:-$OUT/index.sqlite}
mkdir -p "$OUT"

strip() { sed 's/\x1b\[[0-9;]*m//g'; }

pass=0; fail=0
compare() {
  local name=$1; shift
  local py_args=("$@")
  python3 "$PY" "${py_args[@]}" 2>"$OUT/$name.py.err" | strip > "$OUT/$name.py.txt"
  "$ASM" "${asm_args[@]}" 2>"$OUT/$name.asm.err" | strip > "$OUT/$name.asm.txt"
  if diff -u "$OUT/$name.py.txt" "$OUT/$name.asm.txt" > "$OUT/$name.diff"; then
    echo "PASS $name ($(wc -l < "$OUT/$name.py.txt") lines)"; pass=$((pass + 1)); rm -f "$OUT/$name.diff"
  else
    echo "FAIL $name (see $OUT/$name.diff, $(grep -c '^[-+][^-+]' "$OUT/$name.diff") changed lines)"; fail=$((fail + 1))
  fi
}

echo "== index"
"$ASM" index --rebuild

echo "== list"
for proj in Code-scripts agent-session-manager vocalflow tenderbuddy dotfiles; do
  asm_args=(list "$proj" -n 1000); compare "list-$proj" -L "$proj" -n 1000
done
asm_args=(list ~/Code/scripts -n 1000 --full); compare "list-path-full" -L ~/Code/scripts -n 1000 --full
asm_args=(list -d 30 code -n 5000); compare "list-code-30d" -L code -d 30 -n 5000

echo "== session"
for sid in $(ls ~/.claude/projects/-Users-reorx-Code-scripts/*.jsonl | head -3 | xargs -n1 basename | cut -c1-8); do
  asm_args=(session "$sid"); compare "session-$sid" -s "$sid"
done

echo "== search"
asm_args=(search grafana -d 30 -n 10000);                        compare "search-grafana" grafana -d 30 -n 10000
asm_args=(search 重构 -d 30 -n 10000);                            compare "search-cjk" 重构 -d 30 -n 10000
asm_args=(search 'docker compose' -d 60 -l tool_use:Bash -n 10000); compare "search-bash" 'docker compose' -d 60 -l tool_use:Bash -n 10000
asm_args=(search Traceback -d 30 -l tool_result -C 200 -n 10000);   compare "search-traceback" Traceback -d 30 -l tool_result -C 200 -n 10000
asm_args=(search worktree -d 14 -r user -r assistant -n 10000);    compare "search-roles" worktree -d 14 -r user -r assistant -n 10000
asm_args=(search '[Pasted text' -d 30 -n 10000);                   compare "search-literal" '[Pasted text' -d 30 -n 10000
asm_args=(search 'envops (show|copy|set|list-keys)' -d 60 -n 10000); compare "search-regex" 'envops (show|copy|set|list-keys)' -d 60 -n 10000
asm_args=(search TODO --case-sensitive -d 14 -n 10000);            compare "search-case" TODO --case-sensitive -d 14 -n 10000
asm_args=(search resume -r user -d 0 -n 50);                       compare "search-limit" resume -r user -d 0 -n 50
asm_args=(search 'the user' -l thinking -d 30 -n 10000);           compare "search-thinking" 'the user' -l thinking -d 30 -n 10000
asm_args=(search curl -p tenderbuddy -d 60 -n 10000);              compare "search-project" curl -p tenderbuddy -d 60 -n 10000

echo
echo "$pass passed, $fail failed; outputs in $OUT"
[ "$fail" -eq 0 ]
