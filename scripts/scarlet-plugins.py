#!/usr/bin/env python3
"""Build/audit all standard native CLAP effects before installing any of them."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
PLUGINS = ("resonara-gain", "resonara-freeverb")


def stage(arch, destination, build_root, toolchain):
    files = []
    for plugin in PLUGINS:
        output = build_root / plugin
        subprocess.run([
            sys.executable, str(ROOT / "plugins" / plugin / "build.py"),
            "--arch", arch, "--toolchain", str(toolchain), "--output", str(output),
        ], check=True)
        for suffix in ("clap", "LICENSE.txt"):
            artifact = output / "staging/usr/lib/clap" / f"{plugin}.{suffix}"
            if not artifact.is_file():
                raise RuntimeError(f"Audited plugin output is missing: {artifact}")
            files.append(artifact)
    # Failed builds/audits leave the destination untouched. Preserve unrelated
    # installed plugins and replace our own files only after all outputs exist.
    destination.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".resonara-plugins-", dir=destination) as temporary:
        prepared = []
        for artifact in files:
            path = Path(temporary) / artifact.name
            shutil.copyfile(artifact, path)
            path.chmod(0o644)
            prepared.append(path)
        for path in prepared:
            os.replace(path, destination / path.name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--arch", choices=("aarch64", "riscv64"), required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    toolchain = Path(subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip())
    # Independent from the application's Cargo target directory and build lock.
    build_root = ROOT / "target/scarlet-plugins" / args.arch
    stage(args.arch, args.output.resolve(), build_root, toolchain)


if __name__ == "__main__":
    main()
