"""Isolated wrapper tests; fake tools do not validate a native build or real VM."""

import fcntl
import json
import os
import pathlib
import shutil
import subprocess
import tempfile
import time
import tomllib

SOURCE = pathlib.Path(__file__).resolve().parent.parent


class Fixture:
    def __init__(self, root, profile="full"):
        self.root = root
        self.app = root / "resonara"
        self.checkout = root / "Scarlet"
        self.profile = profile
        self.full_project = self.checkout / "projects/aarch64-limine-full"
        self.project = self.full_project if profile == "full" else self.checkout / "projects/aarch64-limine-resonara-native"
        image_name = "limine-aarch64-full.img" if profile == "full" else "limine-aarch64-resonara-native.img"
        self.image = self.project / ".scarlet/images" / image_name
        self.overlay = self.project / "scarlet.local.toml"
        self.lock = self.project / ".scarlet/resonara-vm.lock"
        self.tools = root / "fake-tools"
        self.calls_path = root / "calls.jsonl"
        self.nix_path = root / "nix.jsonl"
        (self.app / "scripts").mkdir(parents=True)
        (self.full_project / "tools").mkdir(parents=True)
        (self.checkout / "tools").mkdir()
        self.tools.mkdir()
        for name in ["scarlet-image", "scarlet-run", "scarlet-native-project.py"]:
            shutil.copyfile(SOURCE / "scripts" / name, self.app / "scripts" / name)
        shutil.copytree(SOURCE / "platforms/scarlet/native-desktop", self.app / "platforms/scarlet/native-desktop")
        (self.checkout / "flake.nix").write_text("{}")
        self.manifest = b'schema_version = 2\n[[images.rootfs.layers]]\nkind = "bundle"\npath = "original-full-bundle.toml"\n'
        (self.full_project / "scarlet.toml").write_bytes(self.manifest)
        (self.full_project / "tools/run_aarch64.sh").write_text("# fixture only\n")
        bsp_files = {
            "Cargo.toml": '[package]\nname = "aarch64-limine-full-project"\nversion = "1.0.0"\n',
            "Cargo.lock": "# fixture lock\nversion = 4\n",
            "build.rs": "fn main() {}\n",
            "src/main.rs": "// fixture BSP source\n",
            ".cargo/config.toml": '[build]\ntarget = "../../../kernel/targets/aarch64-unknown-none-elf.json"\n',
            "lds/aarch64_limine.ld": "/* fixture linker script */\n",
        }
        for relative, data in bsp_files.items():
            path = self.full_project / "bsp" / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(data)
        (self.full_project / "scarlet.lock").write_text("# full fixture project lock\n")
        target = self.checkout / "kernel/targets/aarch64-unknown-none-elf.json"
        target.parent.mkdir(parents=True)
        target.write_text("{}")
        for bundle in ["base", "cli-utils", "desktop"]:
            path = self.checkout / "bundles" / bundle / "bundle.toml"
            path.parent.mkdir(parents=True)
            path.write_text(f"# unchanged {bundle} fixture bundle\n")
        (self.checkout / "tools/qemu-display.sh").write_text('''# Fake display helpers for isolated tests.
scarlet_qemu_display() { printf '%s\\n' "${SCARLET_QEMU_DISPLAY:-gtk,gl=on}"; }
scarlet_qemu_gpu() {
    if [ -n "${SCARLET_QEMU_GPU:-}" ]; then printf '%s\\n' "$SCARLET_QEMU_GPU";
    elif [[ "$1" == *,gl=on* ]]; then printf '%s\\n' virtio-gpu-gl-pci;
    else printf '%s\\n' virtio-gpu-pci; fi
}
''')
        (self.app / "Cargo.toml").write_text("[workspace]\n")
        (self.tools / "rustc").write_text('''#!/bin/sh
case "$*" in
    '--print target-list') printf 'aarch64-unknown-scarlet\\n' ;;
    '--print sysroot') printf '%s\\n' "$FIXTURE_SYSROOT" ;;
    *) exit 2 ;;
esac
''')
        audit_dir = self.app / "crates/resonara-clap/scripts"
        audit_dir.mkdir(parents=True)
        (audit_dir / "audit-scarlet-host.py").write_text('''# Fake audit for wrapper ordering/failure tests, not an ELF validator.
import argparse, json, os, pathlib, sys
parser = argparse.ArgumentParser()
parser.add_argument("binary", type=pathlib.Path)
parser.add_argument("--arch", required=True)
parser.add_argument("--json", required=True, type=pathlib.Path)
args = parser.parse_args()
assert args.arch == "aarch64" and args.binary.is_file()
entry = {"args": ["audit-host", *sys.argv[1:]], "rootfs_exists": (args.json.parent / "rootfs").exists()}
with open(os.environ["FIXTURE_LOG"], "a") as handle:
    handle.write(json.dumps(entry) + "\\n")
if os.getenv("FIXTURE_FAIL_HOST_AUDIT"):
    raise SystemExit("mock native host audit rejected unexpected undefined symbol")
args.json.write_text(json.dumps({"fixture_only": True, "binary": str(args.binary), "arch": args.arch}))
''')
        plugin_dir = self.app / "plugins/resonara-gain"
        plugin_dir.mkdir(parents=True)
        (plugin_dir / "build.py").write_text('''# Fake plugin builder for wrapper tests, not a compiler or ELF validator.
import argparse, json, os, pathlib, sys
parser = argparse.ArgumentParser()
parser.add_argument("--arch", required=True)
parser.add_argument("--toolchain", required=True)
parser.add_argument("--output", required=True, type=pathlib.Path)
args = parser.parse_args()
assert args.arch == "aarch64" and args.toolchain == os.environ["FIXTURE_SYSROOT"]
assert (args.output.parent / "native-host-audit.json").is_file()
entry = {"args": ["plugin-build", *sys.argv[1:]], "rootfs_exists": (args.output.parent / "rootfs").exists()}
with open(os.environ["FIXTURE_LOG"], "a") as handle:
    handle.write(json.dumps(entry) + "\\n")
if os.getenv("FIXTURE_FAIL_PLUGIN_BUILD"):
    raise SystemExit("mock native plugin ELF audit rejected output")
staging = args.output / "staging/system/plugins"
staging.mkdir(parents=True, exist_ok=True)
(staging / "resonara-gain.clap").write_bytes(b"MOCK NATIVE CLAP PLUGIN")
(staging / "resonara-gain.LICENSE.txt").write_bytes(b"MOCK LICENSE NOTICES")
(args.output / "build.json").write_text(json.dumps({"fixture_only": True, "audited": True}))
''')
        (self.tools / "cargo").write_text('''#!/usr/bin/env python3
import json, os, pathlib, struct, sys, time
args = sys.argv[1:]
entry = {"args": args}
for key in ["DISPLAY", "GPU", "AUDIO_DRIVER", "HOSTFWD", "NET", "SNAPSHOT"]:
    entry[key] = os.getenv("SCARLET_QEMU_" + key)
with open(os.environ["FIXTURE_LOG"], "a") as handle:
    handle.write(json.dumps(entry) + "\\n")
if args[0] == "build":
    target = pathlib.Path(args[args.index("--target-dir") + 1]) / "aarch64-unknown-scarlet/release/resonara"
    target.parent.mkdir(parents=True, exist_ok=True)
    header = bytearray(64)
    header[:6] = b"\\x7fELF\\x02\\x01"
    struct.pack_into("<H", header, 18, 183)
    target.write_bytes(header)
elif args[:2] == ["scarlet", "image"]:
    project = pathlib.Path(args[args.index("--project") + 1])
    app = pathlib.Path(os.environ["FIXTURE_APP"])
    native = project.name == "aarch64-limine-resonara-native"
    artifacts = app / ("artifacts/scarlet-native" if native else "artifacts/scarlet")
    assert (artifacts / "native-host-audit.json").is_file()
    assert (artifacts / "gain-aarch64/build.json").is_file()
    assert (artifacts / "rootfs/bin/resonara").is_file()
    assert (artifacts / "rootfs/system/plugins/resonara-gain.clap").read_bytes() == b"MOCK NATIVE CLAP PLUGIN"
    assert (artifacts / "rootfs/system/plugins/resonara-gain.LICENSE.txt").read_bytes() == b"MOCK LICENSE NOTICES"
    image = project / ".scarlet/images" / ("limine-aarch64-resonara-native.img" if native else "limine-aarch64-full.img")
    image.parent.mkdir(parents=True, exist_ok=True)
    image.write_text("UNIT TEST FIXTURE ONLY")
elif args[:2] == ["scarlet", "run"] and os.getenv("FIXTURE_HOLD_RUN"):
    marker = pathlib.Path(os.environ["FIXTURE_HOLD_RUN"])
    marker.write_text("mock runner ready")
    release = pathlib.Path(os.environ["FIXTURE_RELEASE_RUN"])
    deadline = time.monotonic() + 15
    while not release.exists():
        if time.monotonic() > deadline:
            raise SystemExit("mock runner release timed out")
        time.sleep(0.01)
''')
        (self.tools / "nix").write_text('''#!/usr/bin/env python3
import json, os, sys
args = sys.argv[1:]
with open(os.environ["FIXTURE_NIX_LOG"], "a") as handle:
    handle.write(json.dumps({"args": args, "cwd": os.getcwd()}) + "\\n")
command = args[args.index("-c") + 1:]
os.execvp(command[0], command)
''')
        for tool in self.tools.iterdir():
            tool.chmod(0o755)
        self.env = os.environ.copy()
        for key in list(self.env):
            if key.startswith(("SCARLET_", "RESONARA_SCARLET_LOCK_")) or key in ["DISPLAY", "WAYLAND_DISPLAY", "CARGO_TARGET_DIR", "BASH_ENV", "RESONARA_SCARLET_VALIDATION_ROOTFS"]:
                self.env.pop(key)
        self.env.update({
            "PATH": str(self.tools) + os.pathsep + self.env["PATH"],
            "FIXTURE_LOG": str(self.calls_path),
            "FIXTURE_NIX_LOG": str(self.nix_path),
            "FIXTURE_SYSROOT": str(root / "fake-sysroot"),
            "FIXTURE_APP": str(self.app),
            "RESONARA_NIX": str(self.tools / "nix"),
            "SCARLET_QEMU_DISPLAY": "gtk,gl=on",
        })

    def command(self, name, *args, inside=True):
        command = ["bash", str(self.app / "scripts" / name)]
        if inside:
            command.append("--inside-nix")
        profile_args = ["--profile", self.profile] if self.profile != "full" else []
        return command + [str(self.checkout), *profile_args, *args]

    def run(self, name, *args, inside=True):
        return subprocess.run(self.command(name, *args, inside=inside), env=self.env, text=True, capture_output=True, timeout=10)

    def calls(self):
        return [json.loads(line) for line in self.calls_path.read_text().splitlines()] if self.calls_path.exists() else []

    def assert_no_build(self, before):
        assert len(self.calls()) == before, "a refused operation invoked Cargo"


def success(result):
    assert result.returncode == 0, result.stderr


with tempfile.TemporaryDirectory(prefix="resonara-wrapper-tests-") as temporary:
    root = pathlib.Path(temporary)
    fixture = Fixture(root / "main")

    # A fresh default run builds while holding the same inherited lock, then boots.
    success(fixture.run("scarlet-run", inside=False))
    calls = fixture.calls()
    assert [call["args"][0] for call in calls] == ["build", "audit-host", "plugin-build", "scarlet", "scarlet"]
    assert calls[3]["args"][:2] == ["scarlet", "image"]
    assert calls[4]["args"][:2] == ["scarlet", "run"]
    assert not calls[1]["rootfs_exists"] and not calls[2]["rootfs_exists"]
    nix_call = json.loads(fixture.nix_path.read_text().splitlines()[-1])
    assert nix_call["cwd"] == str(fixture.checkout)
    assert "--no-write-lock-file" in nix_call["args"]
    assert (fixture.project / "scarlet.toml").read_bytes() == fixture.manifest
    generated = fixture.overlay.read_bytes()
    assert b'[[images.rootfs.layers]]' in generated and b'kind = "copy"' in generated
    assert (fixture.app / "artifacts/scarlet/rootfs/etc/stemd.d/apps/org.resonara.Resonara.desktop").exists()
    plugin_root = fixture.app / "artifacts/scarlet/rootfs/system/plugins"
    assert (plugin_root / "resonara-gain.clap").read_bytes() == b"MOCK NATIVE CLAP PLUGIN"
    assert (plugin_root / "resonara-gain.LICENSE.txt").read_bytes() == b"MOCK LICENSE NOTICES"

    # A rejected host may never reach plugin construction, staging or composition.
    bad_host = Fixture(root / "bad-host")
    bad_host.env["FIXTURE_FAIL_HOST_AUDIT"] = "1"
    refused = bad_host.run("scarlet-image")
    assert refused.returncode and "mock native host audit rejected" in refused.stderr
    assert [call["args"][0] for call in bad_host.calls()] == ["build", "audit-host"]
    assert not (bad_host.app / "artifacts/scarlet/rootfs").exists()
    assert not bad_host.overlay.exists() and not bad_host.image.exists()

    # A rejected plugin likewise prevents any newly staged native application.
    bad_plugin = Fixture(root / "bad-plugin")
    bad_plugin.env["FIXTURE_FAIL_PLUGIN_BUILD"] = "1"
    refused = bad_plugin.run("scarlet-image")
    assert refused.returncode and "mock native plugin ELF audit rejected" in refused.stderr
    assert [call["args"][0] for call in bad_plugin.calls()] == ["build", "audit-host", "plugin-build"]
    assert not (bad_plugin.app / "artifacts/scarlet/rootfs").exists()
    assert not bad_plugin.overlay.exists() and not bad_plugin.image.exists()

    # The explicit native profile copies the BSP and exact desktop bundle recipe
    # into a separate project. It never rewrites the original full project.
    native = Fixture(root / "native", profile="native-desktop")
    full_before = {str(path.relative_to(native.full_project)): path.read_bytes()
                   for path in native.full_project.rglob("*") if path.is_file()}
    desktop_before = (native.checkout / "bundles/desktop/bundle.toml").read_bytes()
    success(native.run("scarlet-run", inside=False))
    assert native.image.exists()
    assert (native.app / "artifacts/scarlet-native/rootfs/system/plugins/resonara-gain.clap").exists()
    manifest = tomllib.loads((native.project / "scarlet.toml").read_text())
    assert manifest["images"]["rootfs"]["min-size-mib"] == 2048
    assert manifest["images"]["rootfs"]["layers"][0]["path"] == "../../bundles/desktop/bundle.toml"
    assert [layer["path"] for layer in manifest["images"]["initramfs"]["layers"]] == ["../../bundles/base/bundle.toml", "../../bundles/cli-utils/bundle.toml"]
    assert manifest["bsp"]["kernel"]["source"]["path"] == "../../kernel"
    assert manifest["runner"]["command"] == "tools/run_aarch64.sh"
    assert (native.project / "bsp/.cargo/config.toml").read_bytes() == (native.full_project / "bsp/.cargo/config.toml").read_bytes()
    assert (native.checkout / "bundles/desktop/bundle.toml").read_bytes() == desktop_before
    assert full_before == {str(path.relative_to(native.full_project)): path.read_bytes()
                           for path in native.full_project.rglob("*") if path.is_file()}
    assert not (native.full_project / "scarlet.local.toml").exists()
    native.image.write_bytes(b"native guest saved audio")
    before = len(native.calls())
    success(native.run("scarlet-run"))
    assert len(native.calls()) == before + 1 and native.image.read_bytes() == b"native guest saved audio"
    refused = native.run("scarlet-image")
    assert refused.returncode and "Preserving the existing guest disk" in refused.stderr
    # SDK lock updates are preserved; source edits are never silently replaced.
    native_lock = native.project / "bsp/Cargo.lock"
    native_lock.write_text("# updated SDK lock, must survive regeneration\n")
    success(native.run("scarlet-image", "--replace-image"))
    assert native_lock.read_text() == "# updated SDK lock, must survive regeneration\n"
    native_manifest = native.project / "scarlet.toml"
    native_manifest.write_text(native_manifest.read_text() + "\n# user project edit\n")
    before = len(native.calls())
    refused = native.run("scarlet-image", "--replace-image")
    assert refused.returncode and "Preserving edited project source" in refused.stderr
    native.assert_no_build(before)
    native_empty = Fixture(root / "native-empty", profile="native-desktop")
    refused = native_empty.run("scarlet-run", "--no-build", inside=False)
    assert refused.returncode and not native_empty.project.exists()
    assert not native_empty.nix_path.exists()

    # Optional validation data is native-only, follows the audited app overlay,
    # and cannot overwrite any executable/plugin or other guest system path.
    validation = Fixture(root / "validation", profile="native-desktop")
    success(validation.run("scarlet-image"))
    base_overlay = validation.overlay.read_text()
    assert len(tomllib.loads(base_overlay)["images"]["rootfs"]["layers"]) == 1
    fixture_root = validation.root / "validation-rootfs"
    fixture_data = fixture_root / "share/resonara-validation"
    fixture_data.mkdir(parents=True)
    (fixture_data / "import-tone.wav").write_bytes(b"UNIT TEST FIXTURE ONLY")
    validation.env["RESONARA_SCARLET_VALIDATION_ROOTFS"] = str(fixture_root)
    success(validation.run("scarlet-image", "--replace-image"))
    layers = tomllib.loads(validation.overlay.read_text())["images"]["rootfs"]["layers"]
    assert len(layers) == 2
    assert layers[0] == tomllib.loads(base_overlay)["images"]["rootfs"]["layers"][0]
    assert (validation.project / layers[1]["source"]).resolve() == fixture_root
    before = len(validation.calls())
    bad_binary = fixture_root / "bin/resonara"
    bad_binary.parent.mkdir()
    bad_binary.write_text("must not overwrite audited executable")
    refused = validation.run("scarlet-image", "--replace-image")
    assert refused.returncode and "only ordinary files/directories under /share/resonara-validation" in refused.stderr
    validation.assert_no_build(before)
    full_validation = Fixture(root / "full-validation")
    full_validation.env["RESONARA_SCARLET_VALIDATION_ROOTFS"] = str(fixture_root)
    refused = full_validation.run("scarlet-image", inside=False)
    assert refused.returncode and "requires --profile native-desktop" in refused.stderr
    assert not full_validation.nix_path.exists()
    full_validation.assert_no_build(0)

    # Existing guest bytes survive ordinary launches, even after application changes.
    fixture.image.write_bytes(b"guest-created project and audio files")
    guest_bytes = fixture.image.read_bytes()
    before = len(fixture.calls())
    success(fixture.run("scarlet-run"))
    assert len(fixture.calls()) == before + 1
    assert fixture.calls()[-1]["args"][:2] == ["scarlet", "run"]
    assert fixture.image.read_bytes() == guest_bytes
    success(fixture.run("scarlet-run", "--no-build"))
    assert fixture.image.read_bytes() == guest_bytes

    # Replacement is a separate, explicit request, and survives the Nix dispatch.
    before = len(fixture.calls())
    refused = fixture.run("scarlet-image", inside=False)
    assert refused.returncode and "Preserving the existing guest disk" in refused.stderr
    fixture.assert_no_build(before)
    assert fixture.image.read_bytes() == guest_bytes
    success(fixture.run("scarlet-image", "--replace-image", inside=False))
    assert fixture.image.read_text() == "UNIT TEST FIXTURE ONLY"
    assert "--replace-image" in json.loads(fixture.nix_path.read_text().splitlines()[-1])["args"]

    # Explicit image replacement never authorizes overwriting an edited overlay.
    edited = generated + b"\n# user customization\n"
    fixture.overlay.write_bytes(edited)
    before = len(fixture.calls())
    refused = fixture.run("scarlet-image", "--replace-image")
    assert refused.returncode and "Preserving" in refused.stderr
    fixture.assert_no_build(before)
    assert fixture.overlay.read_bytes() == edited
    fixture.overlay.write_bytes(b"")
    refused = fixture.run("scarlet-image", "--replace-image")
    assert refused.returncode and "Preserving" in refused.stderr
    assert fixture.overlay.read_bytes() == b"", "empty user overlay was overwritten"
    fixture.overlay.write_bytes(generated)

    # Mock the socket predicate instead of creating a real socket. The execution
    # sandbox intentionally prohibits Unix socket creation in this test process.
    # A Bash function shadows only `test -S` for this fixture's specified QMP path.
    qmp_fixture = Fixture(root / "qmp")
    qmp_fixture.env["SCARLET_QEMU_QMP"] = str(qmp_fixture.root / "active-qmp.sock")
    mock_predicate = qmp_fixture.root / "mock-socket-predicate.sh"
    mock_predicate.write_text('''test() {
    if [[ "${1:-}" == -S && "${2:-}" == "$SCARLET_QEMU_QMP" ]]; then
        return 0
    fi
    builtin test "$@"
}
''')
    qmp_fixture.env["BASH_ENV"] = str(mock_predicate)
    for name in ["scarlet-run", "scarlet-image"]:
        refused = qmp_fixture.run(name, inside=False)
        assert refused.returncode and "QMP socket already exists" in refused.stderr
    assert not (qmp_fixture.project / ".scarlet").exists()
    assert not (qmp_fixture.app / "artifacts").exists()
    assert not qmp_fixture.nix_path.exists()
    qmp_fixture.assert_no_build(0)

    # --no-build with no disk does not construct even a lock or artifact directory.
    empty_fixture = Fixture(root / "empty")
    refused = empty_fixture.run("scarlet-run", "--no-build", inside=False)
    assert refused.returncode and "never constructs" in refused.stderr
    assert not (empty_fixture.project / ".scarlet").exists()
    assert not empty_fixture.nix_path.exists()
    empty_fixture.assert_no_build(0)

    # A held project lock blocks the first build before native Cargo or staging.
    empty_fixture.lock.parent.mkdir()
    with empty_fixture.lock.open("a") as held:
        fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
        for name in ["scarlet-run", "scarlet-image"]:
            refused = empty_fixture.run(name)
            assert refused.returncode and "Project is busy" in refused.stderr
        assert not (empty_fixture.app / "artifacts").exists()
        empty_fixture.assert_no_build(0)

    # A running mock VM keeps the advisory lock for its entire process lifetime.
    marker, release = fixture.root / "run-ready", fixture.root / "run-release"
    fixture.env["FIXTURE_HOLD_RUN"] = str(marker)
    fixture.env["FIXTURE_RELEASE_RUN"] = str(release)
    running = subprocess.Popen(fixture.command("scarlet-run"), env=fixture.env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        deadline = time.monotonic() + 5
        while not marker.exists():
            assert running.poll() is None, running.communicate()[1]
            assert time.monotonic() < deadline, "mock runner did not start"
            time.sleep(0.01)
        before = len(fixture.calls())
        for name, args in [("scarlet-image", ["--replace-image"]), ("scarlet-run", [])]:
            refused = fixture.run(name, *args)
            assert refused.returncode and "Project is busy" in refused.stderr
        fixture.assert_no_build(before)
    finally:
        release.touch()
        stdout, stderr = running.communicate(timeout=10)
        assert running.returncode == 0, stderr
        fixture.env.pop("FIXTURE_HOLD_RUN")
        fixture.env.pop("FIXTURE_RELEASE_RUN")
    success(fixture.run("scarlet-run", "--no-build"))

    # Native rendering safeguards and persistent-disk defaults remain in force.
    fixture.env.pop("SCARLET_QEMU_DISPLAY")
    refused = fixture.run("scarlet-run", "--no-build")
    assert refused.returncode and "No desktop display" in refused.stderr
    fixture.env["SCARLET_QEMU_DISPLAY"] = "vnc=127.0.0.1:2"
    refused = fixture.run("scarlet-run", "--no-build")
    assert refused.returncode and "does not support native" in refused.stderr
    fixture.env["SCARLET_QEMU_DISPLAY"] = "gtk,gl=on"
    fixture.env["SCARLET_QEMU_GPU"] = "virtio-gpu-pci"
    refused = fixture.run("scarlet-run", "--no-build")
    assert refused.returncode and "cannot provide" in refused.stderr
    fixture.env.pop("SCARLET_QEMU_GPU")
    success(fixture.run("scarlet-run", "--no-build"))
    last = fixture.calls()[-1]
    assert "--no-image" in last["args"]
    assert last["DISPLAY"] == "gtk,gl=on"
    assert last["NET"] == "0" and last["SNAPSHOT"] == "0"
    assert last["AUDIO_DRIVER"].startswith("wav,path=")
    assert all("127.0.0.1" in rule for rule in last["HOSTFWD"].split(","))

print("PASS: isolated native-desktop profile/BSP preservation, restricted optional validation overlay, audited-host/plugin staging order, failure-closed audits, native plugin/license staging, fresh/reused image runs, explicit replacement, guest-data preservation, Nix dispatch, overlay preservation, early QMP guard, image/runtime flock, inherited-lock build, GL guards, persistent-disk defaults")
print("These fake-tool tests do not validate Nix, native compilation, image composition, or QEMU boot.")
