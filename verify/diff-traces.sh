#!/bin/sh
# Extracts the traces from two boards' serial captures and compares them
# (the two-board determinism check, docs/decisions.md D-10).
#
# Requires: POSIX sh, tr, grep, diff
# Usage:
#   sh verify/diff-traces.sh pico.log esp32s3.log
#   sh verify/diff-traces.sh --self-test
#
# A serial capture also contains the boot banner and the guest's log output
# (`[wasm] ...`). The trace is only the `> ...` / `< ...` lines of
# docs/abi-spec.md §9, so those are extracted. Dropping the `[wasm]` lines is
# safe because the same content appears in the trace as the log host call
# (kept, one board's banner alone would produce a difference).
#
# By docs/abi-spec.md §9, time never appears in the trace and GPIO numbers
# looked up by role are already normalized to `role:led`, so no further
# normalization is needed here.
#
# Zero trace lines is a failure. Two empty traces compare equal as strings,
# but that is "nothing was compared", not "identical". This verdict is used
# as a completion criterion, so a silent success is the worst way to break.
# It does happen: the capture was cut off before run, the wrong file was
# passed, or the terminal program prefixed each line with a timestamp or tag.
set -eu

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Drops CR and NUL and keeps only lines starting with `>` or `<`.
#
# Dropping NUL and passing `grep -a` are both needed. A board's serial port
# emits NUL as line noise at reset or power-on. Left in, `grep` treats the
# file as binary and returns only `Binary file matches`, so a 14,352-line
# trace is reported as "1 line" and the comparison fails
# (docs/verification-report.md §8).
# `LC_ALL=C` is needed too. With a UTF-8 locale, macOS `tr` stops at a byte
# that is invalid UTF-8 with `Illegal byte sequence`. Serial noise produces
# such bytes, and stopping drops every line after it (GNU tr does not stop,
# so the result would pass locally and change in CI). Treating the data as
# bytes gives the same result on both.
normalize() {
    LC_ALL=C tr -d '\r\000' < "$1" | LC_ALL=C grep -a -E '^[<>]' || true
}

# Normalizes and counts the lines. Zero lines is a failure.
prepare() {
    src=$1
    dst=$2
    label=$3
    normalize "$src" > "$dst"
    n=$(wc -l < "$dst" | tr -d ' ')
    if [ "$n" -eq 0 ]; then
        echo "$label ($src): no trace line found." >&2
        echo "  Lines starting with '> ' / '< ' (docs/abi-spec.md §9) are required. Check" >&2
        echo "  that the capture was not cut short and that no timestamp prefixes each line." >&2
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
    # The same trace with a different banner and log line (as it is with a
    # different board).
    printf 'wasmicon esp32s3\r\n[wasm] blink start\r\n' > "$work/b.log"
    printf '> wasmicon:hal/log@0.1.0/log(2, "blink start")\r\n<\r\n' >> "$work/b.log"
    printf '> wasmicon:hal/gpio@0.1.0/[static]pin.open(role:led, 3)\r\n< 0 [1]\r\n' >> "$work/b.log"

    normalize "$work/a.log" > "$work/a.trace"
    normalize "$work/b.log" > "$work/b.trace"
    if ! diff -u "$work/a.trace" "$work/b.trace" > /dev/null; then
        echo "self-test failed: banner and CR differences are not absorbed" >&2
        exit 1
    fi

    # A real difference is detected.
    sed 's/role:led/role:lcd-cs/' "$work/a.log" > "$work/c.log"
    normalize "$work/c.log" > "$work/c.trace"
    if diff -u "$work/a.trace" "$work/c.trace" > /dev/null; then
        echo "self-test failed: a real difference is not detected" >&2
        exit 1
    fi

    # Two files without trace lines are not reported as a match.
    printf 'wasmicon rp2040\r\n[wasm] blink start\r\n' > "$work/empty1.log"
    printf 'wasmicon esp32s3\r\n[wasm] blink start\r\n' > "$work/empty2.log"
    if sh "$0" "$work/empty1.log" "$work/empty2.log" > /dev/null 2>&1; then
        echo "self-test failed: two empty traces are reported as a match" >&2
        exit 1
    fi

    # Every line survives NUL mixed in (docs/verification-report.md §8; see
    # the comment on `normalize` above). A heredoc cannot hold NUL, so printf
    # builds the file.
    #
    # This case protects only the `\000` in `tr`. Dropping it passes NUL to
    # grep, which then treats the file as binary, and this fails. Dropping
    # `grep -a` alone still passes (tr has removed the NUL, so grep does not
    # see binary data). `-a` is insurance against grep implementations that
    # treat other noise as binary; it cannot be detected here, but keep it.
    printf 'wasmicon esp32s3\r\n\000\000[wasm] blink start\r\n' > "$work/nul.log"
    printf '> wasmicon:hal/log@0.1.0/log(2, "blink start")\r\n<\r\n' >> "$work/nul.log"
    printf '\000> wasmicon:hal/gpio@0.1.0/[static]pin.open(role:led, 3)\r\n< 0 [1]\r\n' >> "$work/nul.log"
    normalize "$work/nul.log" > "$work/nul.trace"
    if ! diff -u "$work/a.trace" "$work/nul.trace" > /dev/null; then
        echo "self-test failed: NUL drops lines (normalize needs \\000 in tr -d)" >&2
        exit 1
    fi

    echo "self-test OK"
    exit 0
}

if [ "${1:-}" = "--self-test" ]; then
    self_test
fi

if [ $# -ne 2 ]; then
    echo "Usage: sh verify/diff-traces.sh <a.log> <b.log>" >&2
    echo "       sh verify/diff-traces.sh --self-test" >&2
    exit 2
fi

# The normalized traces go to the work directory. Written next to the input,
# a read-only capture location or a process substitution fails the write and
# the diff that matters never appears.
na=$(prepare "$1" "$work/a.trace" "first")
nb=$(prepare "$2" "$work/b.trace" "second")

if diff -q "$work/a.trace" "$work/b.trace" > /dev/null; then
    echo "match: $na lines ($1 and $2)"
    exit 0
fi

# A mismatch is where the investigation starts, so the evidence is kept. The
# work directory goes away with the trap, so a separate directory is made
# and the files are copied there.
out=$(mktemp -d -t wasmicon-trace-diff)
cp "$work/a.trace" "$out/a.trace"
cp "$work/b.trace" "$out/b.trace"
diff -u "$out/a.trace" "$out/b.trace" > "$out/diff.txt" || true
nd=$(wc -l < "$out/diff.txt" | tr -d ' ')

echo "mismatch: $1 has $na lines, $2 has $nb lines" >&2
head -40 "$out/diff.txt"
if [ "$nd" -gt 40 ]; then
    echo "  (first 40 of $nd diff lines)" >&2
fi
echo "  normalized traces and the full diff: $out" >&2
exit 1
