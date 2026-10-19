#!/bin/sh
# Builds a C guest into a .wasm file. The C side of the guest build flags.
#
# Requires: clang with the wasm32 target and wasm-ld (wasi-sdk, or LLVM clang + lld).
#   WASMICON_CLANG selects another clang (default: clang).
# Usage: sh bindings/c/build.sh <output.wasm> <source.c>...
#
# The values mean the same as in apps/.cargo/config.toml for Rust (docs/app-workflow.md §4.4):
# - supported feature set (docs/abi-spec.md §6.1): start from mvp and add only what is needed.
#   clang's default CPU enables reference-types and more, so it is not used
# - one page (64 KiB) of linear memory, within the two-page limit of RP2040
# - 8 KiB shadow stack; the wasm-ld default (64 KiB) would eat the whole page
# - no libc (-nostdlib); _start / _initialize are not used (--no-entry)
set -eu

if [ $# -lt 2 ]; then
    echo "Usage: sh $0 <output.wasm> <source.c>..." >&2
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
