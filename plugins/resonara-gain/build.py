#!/usr/bin/env python3
"""Build and audit a freestanding Linux or native-Scarlet CLAP shared object."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess

ROOT = Path(__file__).resolve().parent


def audit(path, machine):
    data = path.read_bytes()
    if len(data) < 64 or data[:7] != b"\x7fELF\x02\x01\x01" or struct.unpack_from("<HH", data, 16) != (3, machine):
        raise RuntimeError("expected a little-endian ELF64 ET_DYN for the selected architecture")
    dynamic = subprocess.check_output(["readelf", "-Wd", str(path)], text=True)
    symbols = subprocess.check_output(["readelf", "-W", "--dyn-syms", str(path)], text=True)
    program = subprocess.check_output(["readelf", "-Wl", str(path)], text=True)
    relocations = subprocess.check_output(["readelf", "-Wr", str(path)], text=True)
    forbidden = ("NEEDED", "RPATH", "RUNPATH", "VERNEED", "VERSYM", "VERDEF", "TEXTREL")
    if any(f"({tag})" in dynamic for tag in forbidden):
        raise RuntimeError("unsupported dynamic tag:\n" + dynamic)
    if re.search(r"\bTLS\b", program) or re.search(r"\b(?:TLS|IFUNC)\b", symbols):
        raise RuntimeError("TLS/IFUNC are forbidden")
    for line in symbols.splitlines():
        fields = line.split()
        if len(fields) >= 8 and fields[6] == "UND":
            raise RuntimeError("undefined import: " + line)
    if not re.search(r"\bOBJECT\b.*\bclap_entry$", symbols, re.M):
        raise RuntimeError("clap_entry is not an exported data symbol")
    # No imports, so every dynamic relocation must be a local absolute/relative one.
    allowed = {
        62: {"R_X86_64_RELATIVE", "R_X86_64_64", "R_X86_64_GLOB_DAT"},
        183: {"R_AARCH64_RELATIVE", "R_AARCH64_ABS64", "R_AARCH64_GLOB_DAT"},
        243: {"R_RISCV_RELATIVE", "R_RISCV_64"},
    }[machine]
    kinds = set(re.findall(r"\bR_[A-Z0-9_]+\b", relocations))
    if not kinds.issubset(allowed):
        raise RuntimeError("unsupported relocations: " + str(sorted(kinds - allowed)))
    return {"machine": machine, "undefined_imports": [], "needed_libraries": [],
            "tls": False, "relocation_types": sorted(kinds), "size_bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--arch", choices=("linux", "aarch64", "riscv64"), required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--toolchain", type=Path, help="toolchain prefix containing bin/{rustc,cargo,rustdoc}")
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if args.toolchain:
        tool_bin = args.toolchain.resolve() / "bin"
        rustc, cargo, rustdoc = (str(tool_bin / name) for name in ("rustc", "cargo", "rustdoc"))
    else:
        rustc, cargo, rustdoc = (shutil.which(name) for name in ("rustc", "cargo", "rustdoc"))
        if not all((rustc, cargo, rustdoc)):
            parser.error("missing Rust tools; supply --toolchain or set PATH")
        tool_bin = Path(rustc).parent
    env = os.environ.copy()
    env.update(RUSTC=rustc, RUSTDOC=rustdoc, CARGO_TARGET_DIR=str(output / "cargo"))
    env["PATH"] = str(tool_bin) + os.pathsep + env.get("PATH", "")
    command = [cargo, "build", "--release", "--locked", "--manifest-path", str(ROOT / "Cargo.toml")]
    original = "host Linux"
    if args.arch == "linux":
        machine = 62
        artifact = output / "cargo/release/libresonara_gain.so"
        destination = output / "resonara-gain.clap"
    else:
        machine = 183 if args.arch == "aarch64" else 243
        original = "aarch64-unknown-scarlet" if args.arch == "aarch64" else "riscv64gc-unknown-scarlet"
        sysroot = Path(subprocess.check_output([rustc, "--print", "sysroot"], text=True).strip())
        if not (sysroot / "lib/rustlib/src/rust/library/core/Cargo.toml").is_file():
            parser.error("Scarlet compiler requires real rust-src for isolated -Zbuild-std")
        spec = json.loads(subprocess.check_output([rustc, "--print", "target-spec-json", "-Zunstable-options", "--target", original], text=True))
        # Preserve native ABI/ISA; never alter the installed target or sysroot.
        spec.update({"dynamic-linking": True, "relocation-model": "pic", "dll-prefix": "lib", "dll-suffix": ".so"})
        spec.setdefault("pre-link-args", {}).setdefault("gnu-lld", []).extend([
            "-z", "max-page-size=4096", "-z", "now", "-z", "defs", "-Bsymbolic",
            "--hash-style=both", "-soname", "resonara-gain.clap",
        ])
        spec.setdefault("metadata", {}).update(description=f"Isolated Scarlet {args.arch} CLAP cdylib", std=False)
        target = output / f"scarlet-clap-{args.arch}.json"
        target.write_text(json.dumps(spec, indent=2) + "\n")
        command += ["--target", str(target), "-Zbuild-std=core,compiler_builtins"]
        artifact = output / "cargo" / target.stem / "release/libresonara_gain.so"
        destination = output / "staging/system/plugins/resonara-gain.clap"
    if args.offline:
        command.append("--offline")
    subprocess.run(command, env=env, check=True)
    data = bytearray(artifact.read_bytes())
    if args.arch != "linux":
        data[7] = 83  # ELFOSABI_SCARLET, as in tools/loader-smoke/build-rust-dso.py.
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(data)
    report = audit(destination, machine)
    notices = []
    for name in ("LICENSE", "vendor/clap-sys/LICENSE-MIT", "vendor/clap-sys/LICENSE-CLAP"):
        notices.append(f"=== {name} ===\n\n" + (ROOT / name).read_text())
    license_path = destination.with_name("resonara-gain.LICENSE.txt")
    license_path.write_text("\n".join(notices))
    report["license_notices"] = str(license_path)
    report.update(artifact=str(destination), original_target=original, build_command=command,
                  rustc=subprocess.check_output([rustc, "--version", "--verbose"], text=True).strip(),
                  runtime_validation="ELF audited; execute through a CLAP host to validate runtime")
    (output / "build.json").write_text(json.dumps(report, indent=2) + "\n")
    print(destination)


if __name__ == "__main__":
    main()
