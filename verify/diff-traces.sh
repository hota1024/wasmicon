#!/bin/sh
# 2 つのボードのシリアル出力からトレースを取り出して突き合わせる。
# HANDOFF §5 Phase 6 の (2)。
#
# 使い方:
#   sh verify/diff-traces.sh pico.log esp32s3.log
#   sh verify/diff-traces.sh --self-test
#
# シリアルには起動時のバナーやゲストの log 出力（`[wasm] ...`）も混ざる。
# トレースは abi-spec §9 の `> ...` / `< ...` の行だけなので、それを抜き出す。
# `[wasm]` の行を捨ててよいのは、同じ内容が log の host call としてトレースに
# 出ているため（捨てないと片方のボードのバナーだけで差分が出る）。
#
# abi-spec §9 により time はトレースに出ず、役割名で引いた GPIO 番号は
# `role:led` に正規化済みなので、ここでの追加の正規化は要らない。
set -eu

normalize() {
    # CR を落として `>` か `<` で始まる行だけ残す。
    tr -d '\r' < "$1" | grep -E '^[<>]' || true
}

self_test() {
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT

    cat > "$tmp/a.log" <<'SAMPLE'
wasmicon rp2040
[wasm] blink start
> wasmicon:hal/log@0.1.0/log(2, "blink start")
<
> wasmicon:hal/gpio@0.1.0/[static]pin.open(role:led, 3)
< 0 [1]
SAMPLE
    # 同じトレースだがバナーとログ行が違う（ボードが違えば当然そうなる）。
    printf 'wasmicon esp32s3\r\n[wasm] blink start\r\n' > "$tmp/b.log"
    printf '> wasmicon:hal/log@0.1.0/log(2, "blink start")\r\n<\r\n' >> "$tmp/b.log"
    printf '> wasmicon:hal/gpio@0.1.0/[static]pin.open(role:led, 3)\r\n< 0 [1]\r\n' >> "$tmp/b.log"

    normalize "$tmp/a.log" > "$tmp/a.trace"
    normalize "$tmp/b.log" > "$tmp/b.trace"
    if ! diff -u "$tmp/a.trace" "$tmp/b.trace" > /dev/null; then
        echo "self-test 失敗: バナーと CR の違いを吸収できていない" >&2
        exit 1
    fi

    # 中身が違えば検出できること。
    sed 's/role:led/role:lcd-cs/' "$tmp/a.log" > "$tmp/c.log"
    normalize "$tmp/c.log" > "$tmp/c.trace"
    if diff -u "$tmp/a.trace" "$tmp/c.trace" > /dev/null; then
        echo "self-test 失敗: 実際の差分を検出できていない" >&2
        exit 1
    fi

    echo "self-test OK"
    exit 0
}

if [ "${1:-}" = "--self-test" ]; then
    self_test
fi

if [ $# -ne 2 ]; then
    echo "使い方: sh verify/diff-traces.sh <a.log> <b.log>" >&2
    echo "        sh verify/diff-traces.sh --self-test" >&2
    exit 2
fi

a=$(normalize "$1")
b=$(normalize "$2")
na=$(printf '%s' "$a" | grep -c '' || true)
nb=$(printf '%s' "$b" | grep -c '' || true)

if [ "$a" = "$b" ]; then
    echo "一致: $na 行（$1 と $2）"
    exit 0
fi

echo "不一致: $1 は $na 行、$2 は $nb 行" >&2
printf '%s\n' "$a" > "$1.trace"
printf '%s\n' "$b" > "$2.trace"
diff -u "$1.trace" "$2.trace" | head -40
exit 1
