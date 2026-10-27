#!/bin/sh
# Cross-checks the signature derivation of wasmicon-gen against the reference
# implementation tools/wit2sig.py.
#
# Requires: wasm-tools, python3, cargo
# Usage: sh tools/check-sigs.sh
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# wit-device/ has no deps/ copy of the HAL (docs/device-layer.md §3), so the
# HAL goes into deps/ of a scratch copy for wasm-tools to resolve it.
mkdir -p "$tmp/tree/deps/hal"
cp "$root"/wit/*.wit "$tmp/tree/deps/hal/"
cp "$root"/wit-device/*.wit "$tmp/tree/"
wasm-tools component wit --json "$tmp/tree" > "$tmp/hal.json"

# Normalize the wit2sig.py output to module<TAB>name<TAB>sig.
python3 "$root/tools/wit2sig.py" "$tmp/hal.json" | awk '
  /^--- / { mod = $2; next }
  NF >= 2 { print mod "\t" $1 "\t" $2 }
' | sort > "$tmp/ref.txt"

(cd "$root" && cargo run -q -p wasmicon-gen -- --sigs) | sort > "$tmp/gen.txt"

if diff -u "$tmp/ref.txt" "$tmp/gen.txt"; then
    echo "match: $(wc -l < "$tmp/gen.txt" | tr -d ' ') imports"
else
    echo "mismatch: wit2sig.py and wasmicon-gen derive different signatures" >&2
    exit 1
fi
