#!/bin/sh
# Checks the section citations that comments and strings make to the docs.
#
#   1. Every `§` outside docs/ and *.md is part of a citation that names its
#      document: `docs/abi-spec.md §4.1`, `docs/TODO.md §5-2`,
#      `docs/abi-spec.md Appendix D`, `docs/decisions.md D-07`.
#   2. Every cited document exists and contains the cited section or ID.
#   3. Code never cites docs/handoff.md (a historical record) and carries no
#      inline TODO / FIXME / XXX markers (remaining work lives in docs/TODO.md).
#
# Requires: git with PCRE support (git grep -P), perl
# Usage: sh tools/check-citations.sh
set -eu
cd "$(dirname "$0")/.."

self='tools/check-citations.sh'
tmp=$(mktemp)
trap 'rm -f "$tmp" "$tmp.cites"' EXIT
fail=0

doc='(docs/[A-Za-z-]+\.md|apps(/[a-z0-9-]+)?/README\.md)'
sec='§[0-9]+(\.[0-9]+)*(-[0-9]+)?'
id='[DAP]-[0-9]{2}'
cite="$doc (($sec(, $sec)*)|Appendix [A-Z]|$id(, $id)*)"

# 1. A § that is not part of an accepted citation.
git grep -I -n -P '§' -- . ':!docs/' ':!*.md' ":!$self" \
    | perl -pe "s{$cite}{}g" | grep '§' > "$tmp" || true
if [ -s "$tmp" ]; then
    echo "check-citations: citations that do not name their document:" >&2
    cat "$tmp" >&2
    fail=1
fi

# 2. Every cited (document, section) resolves.
git grep -I -h -o -P "$cite" -- . ":!$self" ':!docs/handoff.md' | sort -u > "$tmp.cites" || true
while IFS= read -r line; do
    path=${line%% *}
    rest=${line#* }
    if [ ! -f "$path" ]; then
        echo "check-citations: missing document: $line" >&2
        fail=1
        continue
    fi
    case $rest in
        Appendix\ *)
            letter=${rest#Appendix }
            grep -qE "^#{1,6} (Appendix|付録) $letter([. ]|$)" "$path" \
                || { echo "check-citations: no Appendix $letter in $path" >&2; fail=1; }
            ;;
        *)
            for tok in $(printf '%s' "$rest" | tr ',' ' '); do
                case $tok in
                    §*-*)
                        grep -qF "$tok" "$path" \
                            || { echo "check-citations: no $tok in $path" >&2; fail=1; }
                        ;;
                    §*)
                        n=$(printf '%s' "${tok#§}" | sed 's/\./\\./g')
                        grep -qE "^#{1,6} $n([. ]|$)" "$path" \
                            || { echo "check-citations: no heading $tok in $path" >&2; fail=1; }
                        ;;
                    *)
                        grep -qE "(^|[^A-Za-z])$tok([^0-9]|$)" "$path" \
                            || { echo "check-citations: no $tok in $path" >&2; fail=1; }
                        ;;
                esac
            done
            ;;
    esac
done < "$tmp.cites"

# 3. No handoff citations and no inline work markers in code.
git grep -I -n -i 'handoff' -- . ':!docs/' ':!*.md' ":!$self" > "$tmp" || true
if [ -s "$tmp" ]; then
    echo "check-citations: code cites docs/handoff.md (historical record):" >&2
    cat "$tmp" >&2
    fail=1
fi
git grep -I -n -E '(TODO|FIXME|XXX)[:(]' -- . ':!*.md' ":!$self" > "$tmp" || true
if [ -s "$tmp" ]; then
    echo "check-citations: inline work markers (track work in docs/TODO.md):" >&2
    cat "$tmp" >&2
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    exit 1
fi
echo "check-citations: OK"
