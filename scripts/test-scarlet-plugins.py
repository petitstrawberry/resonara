"""Packaging regressions; fixture builders do not validate native ELF execution."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import tomllib
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("scarlet_plugins", ROOT / "scripts/scarlet-plugins.py")
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


class PackagingTests(unittest.TestCase):
    def test_bundle_includes_app_and_architecture_specific_plugin_stage(self):
        layers = tomllib.loads((ROOT / "platforms/scarlet/bundle.toml").read_text())["layers"]
        self.assertEqual(layers[0]["bin"], "resonara")
        self.assertEqual(layers[0]["to"], "/bin/resonara")
        self.assertEqual(layers[1]["to"], "/usr/lib/clap")
        self.assertNotIn("output", layers[1])
        for arch in ("aarch64", "riscv64", "riscv64gc"):
            script = ROOT / "platforms/scarlet" / layers[1]["source"].replace("{arch}", arch)
            self.assertTrue(script.is_file())

    def check_stage(self, failure=None):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            destination = root / "rootfs/usr/lib/clap"
            destination.mkdir(parents=True)
            existing = destination / "resonara-gain.clap"
            existing.write_bytes(b"previous gain")
            unrelated = destination / "other.clap"
            unrelated.write_bytes(b"other")
            calls = []

            def run(command, check):
                self.assertTrue(check)
                plugin = Path(command[1]).parent.name
                calls.append(plugin)
                self.assertEqual(existing.read_bytes(), b"previous gain")
                if plugin == "resonara-freeverb" and failure == "audit":
                    raise subprocess.CalledProcessError(1, command)
                output = Path(command[command.index("--output") + 1]) / "staging/usr/lib/clap"
                output.mkdir(parents=True)
                (output / f"{plugin}.clap").write_bytes(plugin.encode())
                if not (plugin == "resonara-freeverb" and failure == "license"):
                    (output / f"{plugin}.LICENSE.txt").write_text("notices")

            with patch.object(builder.subprocess, "run", run):
                if failure:
                    with self.assertRaises((subprocess.CalledProcessError, RuntimeError)):
                        builder.stage("aarch64", destination, root / "build", Path("/toolchain"))
                    self.assertEqual(existing.read_bytes(), b"previous gain")
                    self.assertFalse((destination / "resonara-freeverb.clap").exists())
                else:
                    builder.stage("aarch64", destination, root / "build", Path("/toolchain"))
                    self.assertEqual(existing.read_bytes(), b"resonara-gain")
                    self.assertEqual((destination / "resonara-freeverb.clap").read_bytes(), b"resonara-freeverb")
                    self.assertEqual((destination / "resonara-freeverb.LICENSE.txt").read_text(), "notices")
            self.assertEqual(calls, ["resonara-gain", "resonara-freeverb"])
            self.assertEqual(unrelated.read_bytes(), b"other")
            self.assertFalse((root / "rootfs/system").exists())

    def test_all_plugins_and_notices_are_staged(self):
        self.check_stage()

    def test_second_audit_failure_preserves_installed_plugins(self):
        self.check_stage("audit")

    def test_missing_license_preserves_installed_plugins(self):
        self.check_stage("license")


if __name__ == "__main__":
    unittest.main()
