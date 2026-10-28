#!/bin/bash
# 输出 bundle 内的 Mach-O 和代码 bundle（NUL 分隔、从内到外、不跟随符号链接）。
set -euo pipefail
app=${1:?usage: list-code.sh APP}
find "$app" -depth \( -type f -o -type d \) -print0 | while IFS= read -r -d '' path; do
    if [[ -d "$path" ]]; then
        case "$path" in *.app|*.framework|*.xpc|*.appex) printf '%s\0' "$path" ;; esac
    elif [[ "$(file -b "$path")" == Mach-O* ]]; then
        printf '%s\0' "$path"
    fi
done
