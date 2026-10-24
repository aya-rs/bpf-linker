"""Regression coverage for Rust toolchain and LLVM archive selection."""

import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch


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

    def test_failed_archive_build_restores_checkout(self) -> None:
        with TemporaryDirectory() as directory:
            module = Path(directory) / "MODULE.bazel"
            module.write_bytes(b"original module\r\n")
            lock = Path(directory) / "MODULE.bazel.lock"
            lock.write_bytes(b"original lock\n")

            def fail(args: list[str], *, check: bool) -> None:
                self.assertTrue(check, "archive builds must propagate Bazel failures")
                self.assertEqual(module.read_text(), "selected module\n")
                lock.write_text("updated lock\n")
                raise subprocess.CalledProcessError(1, args)

            with (
                patch.object(rustc_llvm, "MODULE", module),
                patch.object(subprocess, "run", side_effect=fail),
                self.assertRaises(subprocess.CalledProcessError),
            ):
                rustc_llvm.build_archives("selected module\n", ["//:llvm-archives"])
            self.assertEqual(module.read_bytes(), b"original module\r\n")
            self.assertEqual(lock.read_bytes(), b"original lock\n")


if __name__ == "__main__":
    unittest.main()
