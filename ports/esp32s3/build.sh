#!/bin/sh
# Runs cargo for the ESP32-S3 port with the espup environment loaded.
#
# The Xtensa linker (xtensa-esp32s3-elf-gcc) is installed by espup but is not
# on PATH, so ~/export-esp.sh is sourced before cargo runs
# (docs/decisions.md P-10).
#
# Requires: espup (`espup install` writes ~/export-esp.sh and the esp toolchain)
# Usage: sh ports/esp32s3/build.sh <cargo subcommand and arguments>
#        sh ports/esp32s3/build.sh clippy --release -- -D warnings
#        sh ports/esp32s3/build.sh build --release
set -eu

if [ -f "$HOME/export-esp.sh" ]; then
    # shellcheck disable=SC1091
    . "$HOME/export-esp.sh"
else
    echo "~/export-esp.sh not found (run espup install first)" >&2
    exit 1
fi

cd "$(dirname "$0")"
exec cargo "$@"
