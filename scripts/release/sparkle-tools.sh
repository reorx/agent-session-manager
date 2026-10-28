#!/bin/bash
# 与 project.yml 固定同一版本；校验官方发布包，stdout 只输出工具目录。
set -euo pipefail
version=2.10.0
checksum=c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c
root="$(cd "$(dirname "$0")/../.." && pwd)/.build/SparkleTools/$version"
if [[ ! -x "$root/bin/sign_update" ]]; then
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    curl -fL --retry 3 "https://github.com/sparkle-project/Sparkle/releases/download/$version/Sparkle-$version.tar.xz" -o "$work/Sparkle.tar.xz"
    printf '%s  %s\n' "$checksum" "$work/Sparkle.tar.xz" | shasum -a 256 -c - >&2
    mkdir -p "$root"
    tar -xf "$work/Sparkle.tar.xz" -C "$root"
fi
printf '%s\n' "$root/bin"
