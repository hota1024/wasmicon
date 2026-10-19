#!/bin/sh
# C のゲストを .wasm にビルドする。ゲストのビルドフラグの C 側の実体。
#
# 使い方: sh bindings/c/build.sh <出力.wasm> <ソース.c>...
# clang は WASMICON_CLANG で差し替えられる（既定: clang）。wasm32 ターゲットと
# wasm-ld が要る（wasi-sdk、または LLVM の clang + lld）。
#
# 値の意味は Rust の apps/.cargo/config.toml と同じ（docs/app-workflow.md §4.4）:
# - 対応機能セット（abi-spec §6.1）。mvp から必要なものだけを足す。
#   clang の既定 CPU は reference-types などを有効にするので使わない
# - 線形メモリ 1 ページ（64 KiB）。RP2040 の 2 ページ上限に収める
# - シャドースタック 8 KiB。wasm-ld の既定（64 KiB）は 1 ページを食い尽くす
# - libc は使わない（-nostdlib）。_start / _initialize は使わない（--no-entry）
set -eu

if [ $# -lt 2 ]; then
    echo "使い方: sh $0 <出力.wasm> <ソース.c>..." >&2
    exit 2
fi

out=$1
shift
dir=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$(dirname "$out")"

exec "${WASMICON_CLANG:-clang}" \
    --target=wasm32-unknown-unknown \
    -std=c11 -O2 -Wall -Wextra -Werror \
    -ffreestanding -nostdlib \
    -mcpu=mvp -msign-ext -mnontrapping-fptoint -mbulk-memory -mmutable-globals -mmultivalue \
    -I "$dir" \
    -Wl,--no-entry \
    -Wl,--initial-memory=65536 \
    -Wl,-z,stack-size=8192 \
    -Wl,--strip-all \
    -o "$out" "$@"
