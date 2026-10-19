#!/bin/sh
# Measures the code size of the runtime core for a microcontroller target.
#
# The default target is thumbv6m-none-eabi (RP2040). Pass another one as the
# argument:
#   sh tools/measure-size.sh thumbv8m.main-none-eabihf   # RP2350
#
# LTO is turned off for the measurement: with LTO on, the rlib holds LLVM
# bitcode and has no section sizes. Real firmware shrinks further at link
# time, so this number is an upper bound.
#
# Requires: cargo with the target installed, llvm-size (rustup component add llvm-tools)
# Usage: sh tools/measure-size.sh [target]
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
target=${1:-thumbv6m-none-eabi}
size=$(ls "$HOME"/.rustup/toolchains/stable-*/lib/rustlib/*/bin/llvm-size 2>/dev/null | head -1)
if [ -z "$size" ]; then
    echo "llvm-size not found; run: rustup component add llvm-tools" >&2
    exit 1
fi

cd "$root"
cargo build --release -p wasmicon-core --target "$target" --config 'profile.release.lto=false' >/dev/null

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
(cd "$tmp" && ar x "$root/target/$target/release/libwasmicon_core.rlib")

"$size" -A "$tmp"/*.rcgu.o 2>/dev/null |
    awk '$1 ~ /^\.(text|rodata)/ {s+=$2} END {printf "wasmicon-core (%s, no LTO): %d bytes (%.1f KiB)\n", "'"$target"'", s, s/1024}'
