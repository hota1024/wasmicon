#!/bin/sh
# 参照実装 tools/wit2sig.py と wasmicon-gen のシグネチャ導出が一致することを確認する。
# HANDOFF §5 Phase 1「まず wit2sig.py の出力と一致することをテストにする」に対応。
#
# 必要: wasm-tools, python3, cargo
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

wasm-tools component wit --json "$root/wit" > "$tmp/hal.json"

# wit2sig.py の出力を module<TAB>name<TAB>sig に正規化する。
python3 "$root/tools/wit2sig.py" "$tmp/hal.json" | awk '
  /^--- / { mod = $2; next }
  NF >= 2 { print mod "\t" $1 "\t" $2 }
' | sort > "$tmp/ref.txt"

(cd "$root" && cargo run -q -p wasmicon-gen -- --sigs) | sort > "$tmp/gen.txt"

if diff -u "$tmp/ref.txt" "$tmp/gen.txt"; then
    echo "一致: $(wc -l < "$tmp/gen.txt" | tr -d ' ') imports"
else
    echo "不一致: wit2sig.py と wasmicon-gen の導出が違う" >&2
    exit 1
fi
