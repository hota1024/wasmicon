#!/bin/sh
# ランタイムコアのコードサイズを RP2040 のターゲットで測る（HANDOFF §5 Phase 2）。
#
# LTO を切って測る。rlib のままだと LTO 有効時は LLVM ビットコードになり
# セクションサイズが取れないため。実際のファームウェアではリンク時に
# さらに削れるので、ここで出る値は上限。
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
target=thumbv6m-none-eabi
size=$(ls "$HOME"/.rustup/toolchains/stable-*/lib/rustlib/*/bin/llvm-size 2>/dev/null | head -1)
if [ -z "$size" ]; then
    echo "llvm-size が無い。rustup component add llvm-tools" >&2
    exit 1
fi

cd "$root"
cargo build --release -p wasmicon-core --target "$target" --config 'profile.release.lto=false' >/dev/null

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
(cd "$tmp" && ar x "$root/target/$target/release/libwasmicon_core.rlib")

"$size" -A "$tmp"/*.rcgu.o 2>/dev/null |
    awk '$1 ~ /^\.(text|rodata)/ {s+=$2} END {printf "wasmicon-core (%s, LTO なし): %d bytes (%.1f KiB)\n", "'"$target"'", s, s/1024}'
