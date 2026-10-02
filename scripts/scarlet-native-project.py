#!/usr/bin/env python3
"""Prepare a separate native-desktop project without modifying the full project.

Called by the image wrapper while it holds this project's advisory lock. Existing
source edits are refused; SDK-managed lock files are seeded once and preserved.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

REPO = Path(__file__).resolve().parent.parent
PROFILE = REPO / "platforms/scarlet/native-desktop"
NAME = "aarch64-limine-resonara-native"
MARKER = ".resonara-native-project.json"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def source_files(root):
    for directory, dirs, files in os.walk(root):
        dirs[:] = [name for name in dirs if name not in {"target", ".scarlet", ".git"}]
        for name in files:
            path = Path(directory) / name
            if path.is_symlink():
                raise ValueError(f"Refusing a symlinked project source: {path}")
            yield path, path.relative_to(root)


def prepare(scarlet):
    full = scarlet / "projects/aarch64-limine-full"
    project = scarlet / "projects" / NAME
    if project.is_symlink():
        raise ValueError(f"Preserving a symlinked project directory: {project}")
    required = [
        full / "bsp/Cargo.toml", full / "bsp/Cargo.lock",
        full / "bsp/.cargo/config.toml", full / "bsp/src/main.rs",
        scarlet / "kernel/targets/aarch64-unknown-none-elf.json",
        scarlet / "bundles/desktop/bundle.toml",
        scarlet / "bundles/base/bundle.toml",
        scarlet / "bundles/cli-utils/bundle.toml",
    ]
    for path in required:
        if not path.is_file():
            raise ValueError(f"Required pinned Scarlet source is missing: {path}")
    files = {str(relative): (path.read_bytes(), path.stat().st_mode & 0o777)
             for path, relative in source_files(PROFILE)}
    for path, relative in source_files(full / "bsp"):
        files[f"bsp/{relative}"] = (path.read_bytes(), path.stat().st_mode & 0o777)
    if (full / "scarlet.lock").is_file():
        files["scarlet.lock"] = ((full / "scarlet.lock").read_bytes(), 0o644)
    mutable_locks = {"scarlet.lock", "bsp/Cargo.lock"}
    marker = project / MARKER
    if marker.is_symlink():
        raise ValueError(f"Refusing a symlinked provenance marker: {marker}")
    if marker.exists():
        saved = json.loads(marker.read_text())
        if saved.get("generator") != "resonara-native-desktop-v1":
            raise ValueError(f"Preserving an unrecognized existing project: {project}")
    elif project.exists() and any(path.name != ".scarlet" for path in project.iterdir()):
        raise ValueError(f"Preserving an existing unmanaged project: {project}")
    # Check everything before writing. Never reset a user's manifest, BSP or runner.
    for relative, (data, _) in files.items():
        destination = project / relative
        cursor = destination
        while cursor != project:
            if cursor.is_symlink():
                raise ValueError(f"Preserving a symlinked generated path: {cursor}")
            cursor = cursor.parent
        if destination.exists() and relative not in mutable_locks and destination.read_bytes() != data:
            raise ValueError(f"Preserving edited project source (or changed Scarlet source): {destination}")
    for relative, (data, mode) in files.items():
        destination = project / relative
        if not destination.exists():
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
            destination.chmod(mode)
    if not marker.exists():
        commit = subprocess.run(["git", "-C", str(scarlet), "rev-parse", "HEAD"],
                                text=True, capture_output=True, check=False)
        marker.write_text(json.dumps({
            "generator": "resonara-native-desktop-v1",
            "scarlet_commit": commit.stdout.strip() if commit.returncode == 0 else None,
            "desktop_bundle": "../../bundles/desktop/bundle.toml",
            "desktop_bundle_sha256": sha((scarlet / "bundles/desktop/bundle.toml").read_bytes()),
            "generated_source_sha256": {relative: sha(data) for relative, (data, _) in files.items()},
            "rootfs_min_size_mib": 2048,
            "excluded_top_level_bundles": ["full-debian", "linux", "experimental", "rust-toolchain"],
        }, indent=2) + "\n")
    print(f"Native desktop project: {project}")
    print("Exact desktop bundle retained; Debian/Wine, experimental apps and guest Rust-toolchain layers are not included.")
    print("Rootfs minimum: 2048 MiB, with SDK automatic growth. Budget at least 2114 MiB for the GPT plus staging/cache/ext2 allocations.")
    return project


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scarlet", type=Path)
    args = parser.parse_args()
    try:
        prepare(args.scarlet.resolve())
    except (OSError, ValueError) as error:
        parser.exit(1, f"Native desktop project preparation failed: {error}\n")
