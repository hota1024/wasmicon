#!/bin/sh
# 2 つのボードのシリアル出力からトレースを取り出して突き合わせる。
# docs/handoff.md §5 Phase 6 の (2)。
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
#
# **トレース行が 1 行も取れなければ失敗にする。** 空同士は文字列としては
# 一致してしまうが、それは「一致した」ではなく「何も比較していない」。
# ここは完了条件の判定に使うので、黙って成功を返すのが最悪の壊れ方になる。
# 実際に起きうる: 取り込みが run の前で切れていた、渡すファイルを間違えた、
# 端末ソフトが行頭にタイムスタンプやタグを付けていた。
set -eu

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# CR を落として `>` か `<` で始まる行だけ残す。
normalize() {
    tr -d '\r' < "$1" | grep -E '^[<>]' || true
}

# 正規化して行数を数える。0 行なら失敗。
prepare() {
    src=$1
    dst=$2
    label=$3
    normalize "$src" > "$dst"
    n=$(wc -l < "$dst" | tr -d ' ')
    if [ "$n" -eq 0 ]; then
        echo "$label ($src) からトレース行を 1 行も取り出せない。" >&2
        echo "  abi-spec §9 の '> ' / '< ' で始まる行が必要。取り込みが途中で" >&2
        echo "  切れていないか、行頭にタイムスタンプが付いていないか確認すること。" >&2
        exit 1
    fi
    echo "$n"
}

self_test() {
    cat > "$work/a.log" <<'SAMPLE'
wasmicon rp2040
[wasm] blink start
> wasmicon:hal/log@0.1.0/log(2, "blink start")
<
> wasmicon:hal/gpio@0.1.0/[static]pin.open(role:led, 3)
< 0 [1]
SAMPLE
    # 同じトレースだがバナーとログ行が違う（ボードが違えば当然そうなる）。
    printf 'wasmicon esp32s3\r\n[wasm] blink start\r\n' > "$work/b.log"
    printf '> wasmicon:hal/log@0.1.0/log(2, "blink start")\r\n<\r\n' >> "$work/b.log"
    printf '> wasmicon:hal/gpio@0.1.0/[static]pin.open(role:led, 3)\r\n< 0 [1]\r\n' >> "$work/b.log"

    normalize "$work/a.log" > "$work/a.trace"
    normalize "$work/b.log" > "$work/b.trace"
    if ! diff -u "$work/a.trace" "$work/b.trace" > /dev/null; then
        echo "self-test 失敗: バナーと CR の違いを吸収できていない" >&2
        exit 1
    fi

    # 中身が違えば検出できること。
    sed 's/role:led/role:lcd-cs/' "$work/a.log" > "$work/c.log"
    normalize "$work/c.log" > "$work/c.trace"
    if diff -u "$work/a.trace" "$work/c.trace" > /dev/null; then
        echo "self-test 失敗: 実際の差分を検出できていない" >&2
        exit 1
    fi

    # トレース行が無いファイル同士を「一致」と言わないこと。
    printf 'wasmicon rp2040\r\n[wasm] blink start\r\n' > "$work/empty1.log"
    printf 'wasmicon esp32s3\r\n[wasm] blink start\r\n' > "$work/empty2.log"
    if sh "$0" "$work/empty1.log" "$work/empty2.log" > /dev/null 2>&1; then
        echo "self-test 失敗: 空トレース同士を一致と報告している" >&2
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

# 正規化した結果は作業ディレクトリに置く。入力の隣に書くと、読み取り専用の
# 取り込み先や process substitution で書けずに落ち、肝心の差分が出ない。
na=$(prepare "$1" "$work/a.trace" "1 つ目")
nb=$(prepare "$2" "$work/b.trace" "2 つ目")

if diff -q "$work/a.trace" "$work/b.trace" > /dev/null; then
    echo "一致: $na 行（$1 と $2）"
    exit 0
fi

echo "不一致: $1 は $na 行、$2 は $nb 行" >&2
diff -u "$work/a.trace" "$work/b.trace" | head -40
exit 1
