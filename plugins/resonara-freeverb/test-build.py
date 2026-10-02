"""Regression for Scarlet's loader flags, independent of host readelf."""
import importlib.util
from pathlib import Path
import struct
import unittest
spec = importlib.util.spec_from_file_location("builder", Path(__file__).with_name("build.py"))
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)

class DynamicFlags(unittest.TestCase):
    def image(self, tag, value):
        data = bytearray(152)
        struct.pack_into("<Q", data, 32, 64)
        struct.pack_into("<HH", data, 54, 56, 1)
        struct.pack_into("<IIQQQQQQ", data, 64, 2, 0, 120, 0, 0, 32, 32, 8)
        struct.pack_into("<qQ", data, 120, tag, value)
        return data
    def test_symbolic_was_buildable_but_cannot_be_loaded(self):
        with self.assertRaisesRegex(RuntimeError, "BIND_NOW"):
            builder.validate_native_dynamic_flags(self.image(30, 2 | 8))
    def test_eager_binding_and_loader_supported_modes(self):
        for tag, value in [(30, 8), (30, 0), (0x6ffffffb, 1 | 8 | 0x08000000)]:
            builder.validate_native_dynamic_flags(self.image(tag, value))
    def test_unknown_modes_and_truncated_headers_rejected(self):
        with self.assertRaises(RuntimeError):
            builder.validate_native_dynamic_flags(self.image(0x6ffffffb, 0x10))
        with self.assertRaises(RuntimeError):
            builder.validate_native_dynamic_flags(self.image(30, 8)[:110])

if __name__ == "__main__":
    unittest.main()
