#!/bin/sh
set -eu
exec python3 "$(dirname "$0")/scarlet-plugins.py" --arch aarch64 --output "$1"
