#!/bin/sh
# Fails if a tracked text file contains Japanese or other CJK text.
#
# A line that carries the marker `i18n:allow-cjk` is exempt. Use it only where
# CJK text is the subject of the code, such as a test of display width.
#
# Requires: git with PCRE support (git grep -P)
# Usage: sh tools/check-english.sh [path...]
set -eu
cd "$(dirname "$0")/.."

# Hiragana, katakana, CJK punctuation and symbols (U+3000-U+30FF), CJK ideographs
# (U+3400-U+4DBF, U+4E00-U+9FFF), full-width forms (U+FF00-U+FFEF) and the
# reference mark U+203B.
pattern='[\x{3000}-\x{30FF}\x{3400}-\x{4DBF}\x{4E00}-\x{9FFF}\x{FF00}-\x{FFEF}\x{203B}]'

hits=$(git grep -I -n -P "$pattern" -- "${@:-.}" ':!*.lock' ':!package-lock.json' \
  | grep -v 'i18n:allow-cjk' || true)

if [ -n "$hits" ]; then
    printf '%s\n' "$hits" >&2
    count=$(printf '%s\n' "$hits" | wc -l | tr -d ' ')
    echo "check-english: $count lines contain CJK text" >&2
    exit 1
fi
echo "check-english: OK"
