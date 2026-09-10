#!/bin/sh
# WebAssembly spec testsuite を third_party/ に取得する（.gitignore 済み）。
# Phase 2 のランタイムコアの完了条件はこのテストスイートの全通過。
#
# SHA を固定する。upstream の master は post-MVP の機能がコア側の .wast にも
# 入ってくるため、追従すると「全通過」の意味が勝手に変わる。
set -eu

REV=34f1f402e9d5240ccc9a3969e3183087dd258082

root=$(cd "$(dirname "$0")/.." && pwd)
dest="$root/third_party/testsuite"

if [ ! -d "$dest/.git" ]; then
    echo "取得: $dest"
    mkdir -p "$root/third_party"
    git clone --filter=blob:none --no-checkout \
        https://github.com/WebAssembly/testsuite.git "$dest"
fi

git -C "$dest" fetch --depth 1 origin "$REV"
git -C "$dest" checkout --detach "$REV"
echo "testsuite $REV / $(ls "$dest"/*.wast | wc -l | tr -d ' ') 個の .wast"
