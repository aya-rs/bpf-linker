"""Regression coverage for beta's LLVM 22-to-23 transition."""

import importlib.util
import sys
import unittest
from pathlib import Path


spec = importlib.util.spec_from_file_location(
    "rustc_llvm", Path(__file__).with_name("rustc-llvm.py")
)
assert spec is not None and spec.loader is not None
rustc_llvm = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = rustc_llvm
spec.loader.exec_module(rustc_llvm)


class ToolchainTest(unittest.TestCase):
    def test_llvm_transition(self) -> None:
        pin = rustc_llvm.LlvmPin("21cf28432798952d942bacc6bcee3a328faa3638", "23.1.0")
        # A beta promotion can change LLVM without changing its channel name.
        for version, excluded in (
            ("22.1.8", "default,llvm-21,llvm-23"),
            ("23.1.0", "llvm-21,llvm-22"),
        ):
            with self.subTest(version=version):
                selection = rustc_llvm.llvm_selection(f"LLVM version: {version}\n", pin)
                self.assertEqual(selection["llvm"], int(version.split(".")[0]))
                self.assertEqual(selection["exclude-features"], excluded)
        with self.assertRaisesRegex(ValueError, "unsupported LLVM 24"):
            rustc_llvm.llvm_selection("LLVM version: 24.0.0\n", pin)


if __name__ == "__main__":
    unittest.main()
