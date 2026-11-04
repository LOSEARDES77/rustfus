#!/usr/bin/env python3
"""Native build configuration tests; subprocesses are mocked, not compiled."""

import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("native_build", Path(__file__).with_name("build.py"))
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)


class NativeBuild(unittest.TestCase):
    def test_ntfs_excludes_gettext_without_changing_other_build_environments(self):
        for host, target in [
            ("x86_64-apple-darwin", "x86_64-apple-darwin"),
            ("aarch64-apple-darwin", "aarch64-apple-darwin"),
            ("aarch64-apple-darwin", "x86_64-apple-darwin"),
            ("x86_64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"),
        ]:
            with self.subTest(host=host, target=target), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                packaging = root / "packaging/native"
                packaging.mkdir(parents=True)
                (packaging / "ntfs-import.c").write_text("")
                pins = {name: {"commit": "fixture"} for name in ("ntfs-3g", "wimlib")}
                (packaging / "pins.json").write_text(json.dumps(pins))
                for name, helper in [("ntfs-3g", "ntfsprogs/mkntfs"), ("wimlib", "wimlib-imagex")]:
                    source = root / "vendor" / name
                    (source / "m4").mkdir(parents=True)
                    (source / "configure.ac").write_text("")
                    binary = source / helper
                    binary.parent.mkdir(parents=True, exist_ok=True)
                    binary.write_text("mock executable")

                configured = {}
                make_environments = []

                def run(args, cwd, env, capture=False):
                    if Path(args[0]).name == "configure":
                        configured[cwd.name] = env.copy()
                    if args[0] == "make":
                        make_environments.append(env.copy())
                    if "-o" in args:
                        Path(args[args.index("-o") + 1]).write_text("mock executable")
                    return "" if capture else None

                with patch.object(native, "ROOT", root), \
                     patch.object(native, "run", side_effect=run), \
                     patch.object(native.shutil, "which", return_value="/mock/tool"), \
                     patch.dict(os.environ, {"ac_cv_header_libintl_h": "yes", "NUM_JOBS": "1"}, clear=True):
                    native.build(root / "out", host, target)
                    self.assertEqual(os.environ["ac_cv_header_libintl_h"], "yes")

                self.assertEqual(configured["ntfs-3g"]["ac_cv_header_libintl_h"], "no")
                self.assertEqual(configured["wimlib"]["ac_cv_header_libintl_h"], "yes")
                self.assertTrue(make_environments)
                self.assertTrue(all(env["ac_cv_header_libintl_h"] == "yes" for env in make_environments))


if __name__ == "__main__":
    unittest.main()
