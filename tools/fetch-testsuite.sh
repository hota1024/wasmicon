#!/bin/sh
# Fetches the WebAssembly spec testsuite into third_party/ (gitignored).
# The runtime core's spec tests (runtime/tests/spec.rs) run against it and
# silently skip when it is missing.
#
# The SHA is pinned. Upstream master keeps adding post-MVP features to the
# core .wast files, so following it would silently change what "all pass"
# means.
#
# Requires: git
# Usage: sh tools/fetch-testsuite.sh
set -eu

REV=34f1f402e9d5240ccc9a3969e3183087dd258082

root=$(cd "$(dirname "$0")/.." && pwd)
dest="$root/third_party/testsuite"

if [ ! -d "$dest/.git" ]; then
    echo "fetching: $dest"
    mkdir -p "$root/third_party"
    git clone --filter=blob:none --no-checkout \
        https://github.com/WebAssembly/testsuite.git "$dest"
fi

git -C "$dest" fetch --depth 1 origin "$REV"
git -C "$dest" checkout --detach "$REV"
echo "testsuite $REV / $(ls "$dest"/*.wast | wc -l | tr -d ' ') .wast files"
