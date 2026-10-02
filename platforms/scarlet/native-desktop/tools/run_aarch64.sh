#!/usr/bin/env bash
# Reuse the pinned official runner with this separate project's native images.
set -euo pipefail
native_project="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export SCARLET_QEMU_PROJECT_DIR="${SCARLET_QEMU_PROJECT_DIR:-$native_project}"
export SCARLET_QEMU_BOOT_IMAGE="${SCARLET_QEMU_BOOT_IMAGE:-$SCARLET_QEMU_PROJECT_DIR/.scarlet/images/limine-aarch64-resonara-native.img}"
export SCARLET_QEMU_ROOTFS_IMAGE="${SCARLET_QEMU_ROOTFS_IMAGE:-$SCARLET_QEMU_PROJECT_DIR/.scarlet/images/rootfs-aarch64-resonara-native.ext2}"
exec bash "$native_project/../aarch64-limine-full/tools/run_aarch64.sh" "$@"
