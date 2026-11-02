#!/usr/bin/env python3
"""Packaging contract tests; fake actool output is not a rendering test."""

import importlib.util
import json
import os
import plistlib
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("macos-icons.py")
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("macos_icons", SCRIPT)
icons = importlib.util.module_from_spec(spec)
spec.loader.exec_module(icons)


class MacOSIcons(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="rustfus icon tests ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.app = self.root / "Rustfus.app"
        self.info = self.app / "Contents/Info.plist"
        self.info.parent.mkdir(parents=True)
        self.original = {"CFBundleIdentifier": "io.github.rustfus", "CFBundleVersion": "1.2.3",
                         "LSMinimumSystemVersion": "11.0", "NSHighResolutionCapable": True,
                         "CFBundleIconName": "stale"}
        self.info.write_bytes(plistlib.dumps(self.original))
        self.resources = self.app / "Contents/Resources"

    def run_script(self, *arguments, env=None):
        return subprocess.run([sys.executable, str(SCRIPT), str(self.app), *arguments],
                              capture_output=True, text=True, env=env)

    def fake_actool(self, major=26, fail=False, empty=False):
        tool = self.root / "fake actool"
        tool.write_text(f'''#!{sys.executable}
import json, pathlib, plistlib, sys
args = sys.argv[1:]
if args == ["--version"]:
    sys.stdout.buffer.write(plistlib.dumps({{"com.apple.actool.version": {{"short-bundle-version": "{major}.0"}}}}))
    sys.exit(0)
pathlib.Path(__file__).with_suffix(".args").write_text(json.dumps(args))
if {fail!r}: sys.exit(42)
out = pathlib.Path(args[args.index("--compile") + 1])
if not {empty!r}: (out / "Assets.car").write_bytes(b"mock catalog, not an Apple rendition")
(out / "rustfus.icns").write_bytes(b"unwanted generated fallback")
partial = pathlib.Path(args[args.index("--output-partial-info-plist") + 1])
partial.write_bytes(plistlib.dumps({{"CFBundleIconName": "rustfus", "CFBundleIconFile": "generated.icns", "LSMinimumSystemVersion": "26.0", "CFBundleIdentifier": "wrong"}}))
''')
        tool.chmod(0o755)
        return tool

    def assert_fallback(self):
        self.assertEqual((self.resources / "rustfus.icns").read_bytes(), icons.FALLBACK.read_bytes())
        info = plistlib.loads(self.info.read_bytes())
        self.assertEqual(info["CFBundleIconFile"], "rustfus.icns")
        self.assertEqual(
            info["NSRemovableVolumesUsageDescription"],
            "Rustfus needs access to removable volumes to create bootable media.",
        )
        for key in ("CFBundleIdentifier", "CFBundleVersion", "LSMinimumSystemVersion", "NSHighResolutionCapable"):
            self.assertEqual(info[key], self.original[key])
        return info

    def test_explicit_fallback(self):
        result = self.run_script("--mode", "fallback")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("CFBundleIconName", self.assert_fallback())
        self.assertFalse((self.resources / "Assets.car").exists())

    def test_missing_actool_auto_and_required(self):
        xcrun = self.root / "xcrun"
        xcrun.write_text("#!/bin/sh\nexit 1\n")
        xcrun.chmod(0o755)
        env = dict(os.environ, PATH=str(self.root))
        result = self.run_script(env=env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("CFBundleIconName", self.assert_fallback())
        result = self.run_script("--mode", "required", env=env)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Xcode 26+", result.stderr)

    def test_compile_and_merge(self):
        tool = self.fake_actool()
        result = self.run_script("--mode", "required", "--actool", str(tool))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.assert_fallback()["CFBundleIconName"], "rustfus")
        self.assertTrue((self.resources / "Assets.car").is_file())
        args = json.loads(tool.with_suffix(".args").read_text())
        self.assertEqual(args[0], str(icons.ICON_SOURCE))
        for flag, expected in [("--app-icon", "rustfus"), ("--platform", "macosx"),
                               ("--minimum-deployment-target", "11.0"), ("--target-device", "mac")]:
            self.assertEqual(args[args.index(flag) + 1], expected)
        self.assertFalse((self.resources / "rustfus.icon").exists())

    def test_old_actool(self):
        tool = self.fake_actool(25)
        result = self.run_script("--actool", str(tool))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("CFBundleIconName", self.assert_fallback())
        result = self.run_script("--mode", "required", "--actool", str(tool))
        self.assertNotEqual(result.returncode, 0)

    def test_compile_errors_do_not_silently_fallback(self):
        for options in ({"fail": True}, {"empty": True}):
            tool = self.fake_actool(**options)
            result = self.run_script("--actool", str(tool))
            self.assertNotEqual(result.returncode, 0, result.stdout)
            self.assertFalse((self.resources / "Assets.car").exists())
            self.assertEqual(plistlib.loads(self.info.read_bytes()), self.original)

    def test_existing_catalog_is_not_destroyed(self):
        self.resources.mkdir()
        catalog = self.resources / "Assets.car"
        catalog.write_bytes(b"existing assets")
        result = self.run_script("--mode", "fallback")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(catalog.read_bytes(), b"existing assets")


if __name__ == "__main__":
    unittest.main()
