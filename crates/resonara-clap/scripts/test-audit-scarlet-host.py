#!/usr/bin/env python3
"""Regression tests for the mandatory native executable allowlist audit."""
import importlib.util
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("host_audit", Path(__file__).with_name("audit-scarlet-host.py"))
AUDIT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(AUDIT)


def symbol_table(full=False):
    header = "Symbol table '.symtab' contains 5 entries:\n" if full else "Symbol table '.dynsym' contains 5 entries:\n"
    return header + "\n".join(f"{n}: 0000000000000000 0 NOTYPE GLOBAL DEFAULT UND {name}" for n, name in enumerate(sorted(AUDIT.IMPORTS), 1))


class AuditTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / "host"
        data = bytearray(136)
        data[:8] = b"\x7fELF\x02\x01\x01\x53"
        struct.pack_into("<HH", data, 16, 3, 183)
        struct.pack_into("<Q", data, 32, 64)
        struct.pack_into("<HH", data, 54, 56, 1)
        struct.pack_into("<IIQQQQQQ", data, 64, 3, 4, 120, 0, 0, 16, 16, 1)
        data[120:] = b"/bin/scarlet-ld\0"
        self.path.write_bytes(data)
        self.outputs = {
            "--dynamic": "(FLAGS) BIND_NOW\n(FLAGS_1) Flags: NOW PIE\n",
            "--dyn-syms": symbol_table(),
            "--symbols": symbol_table(True),
            "--relocs": "\n".join(f"0000 0000 R_AARCH64_JUMP_SLOT 0000 {name} + 0" for name in sorted(AUDIT.IMPORTS)),
        }
        self.mock = patch.object(AUDIT, "readelf", side_effect=lambda path, option: self.outputs[option])
        self.mock.start()
        self.addCleanup(self.mock.stop)

    def test_valid_exact_loader_contract(self):
        result = AUDIT.audit(self.path, "aarch64")
        self.assertEqual(result["undefined_imports"], sorted(AUDIT.IMPORTS))
        self.assertEqual(result["needed_libraries"], [])

    def test_extra_runtime_dependency_rejected(self):
        self.outputs["--dynamic"] += "(NEEDED) Shared library: [link_seed.so]\n"
        with self.assertRaisesRegex(ValueError, "NEEDED"):
            AUDIT.audit(self.path, "aarch64")

    def test_ignored_undefined_outside_dynsym_rejected(self):
        self.outputs["--symbols"] += "\n9: 0000 0 NOTYPE GLOBAL DEFAULT UND unexpected_api"
        with self.assertRaisesRegex(ValueError, "all unresolved"):
            AUDIT.audit(self.path, "aarch64")

    def test_missing_dynamic_import_rejected(self):
        self.outputs["--dyn-syms"] = self.outputs["--dyn-syms"].replace("dlopen", "wrong_api")
        with self.assertRaisesRegex(ValueError, "dynamic unresolved"):
            AUDIT.audit(self.path, "aarch64")

    def test_unsupported_relocation_rejected(self):
        self.outputs["--relocs"] += "\n0000 0000 R_AARCH64_IRELATIVE 0000 + 0"
        with self.assertRaisesRegex(ValueError, "unsupported relocation"):
            AUDIT.audit(self.path, "aarch64")

    def test_missing_import_relocation_rejected(self):
        self.outputs["--relocs"] = "\n".join(line for line in self.outputs["--relocs"].splitlines() if "dlsym" not in line)
        with self.assertRaisesRegex(ValueError, "relocation for dlsym"):
            AUDIT.audit(self.path, "aarch64")

    def test_wrong_interpreter_rejected(self):
        data = self.path.read_bytes().replace(b"/bin/scarlet-ld", b"/bin/another-ld")
        self.path.write_bytes(data)
        with self.assertRaisesRegex(ValueError, "interpreter"):
            AUDIT.audit(self.path, "aarch64")

    def test_wrong_machine_rejected(self):
        with self.assertRaisesRegex(ValueError, "riscv64"):
            AUDIT.audit(self.path, "riscv64")

    def test_lazy_binding_rejected(self):
        self.outputs["--dynamic"] = "(FLAGS_1) Flags: PIE\n"
        with self.assertRaisesRegex(ValueError, "eager binding"):
            AUDIT.audit(self.path, "aarch64")


if __name__ == "__main__":
    unittest.main()
