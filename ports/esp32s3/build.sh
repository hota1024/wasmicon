#!/bin/sh
# ESP32-S3 のビルド。
#
# Xtensa のリンカ（xtensa-esp32s3-elf-gcc）は espup が入れるが PATH には
# 入っていないので、~/export-esp.sh を読んでから cargo を呼ぶ。
set -eu

if [ -f "$HOME/export-esp.sh" ]; then
    # shellcheck disable=SC1091
    . "$HOME/export-esp.sh"
else
    echo "~/export-esp.sh が無い。espup install を実行すること" >&2
    exit 1
fi

cd "$(dirname "$0")"
exec cargo "$@"
