#!/bin/sh
set -eu
exec python3 "$(dirname "$0")/scarlet-plugins.py" --arch riscv64 --output "$1"
