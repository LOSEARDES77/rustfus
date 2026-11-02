#!/usr/bin/env python3
"""Install Rustfus's macOS icons into an unsigned .app bundle."""

import argparse
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ICON_SOURCE = ROOT / "res/macos/rustfus.icon"
FALLBACK = ROOT / "res/macos/rustfus.icns"


def install(app, mode="auto", actool=None):
    info_path = app / "Contents/Info.plist"
    with info_path.open("rb") as stream:
        info = plistlib.load(stream)
    resources = app / "Contents/Resources"
    resources.mkdir(parents=True, exist_ok=True)
    if (resources / "Assets.car").exists():
        raise ValueError("Resources/Assets.car already exists; refusing to overwrite another catalog")

    if mode != "fallback" and actool is None:
        found = subprocess.run(["xcrun", "--find", "actool"], capture_output=True, text=True)
        if found.returncode == 0:
            actool = found.stdout.strip()
    if mode == "required" and not actool:
        raise ValueError("Xcode 26+ actool is required; select full Xcode with DEVELOPER_DIR or xcode-select")

    compiled_info = {}
    with tempfile.TemporaryDirectory(prefix="rustfus-icons-") as temporary:
        output = Path(temporary)
        if mode != "fallback" and actool:
            version = subprocess.run([str(actool), "--version"], check=True, capture_output=True)
            metadata = plistlib.loads(version.stdout)
            number = metadata["com.apple.actool.version"]["short-bundle-version"]
            major = re.match(r"\d+", number)
            if not major or int(major.group()) < 26:
                if mode == "required":
                    raise ValueError(f"Xcode 26+ actool is required, found {number}")
                print(f"macOS icons: actool {number} predates Icon Composer; using ICNS fallback", file=sys.stderr)
                actool = None
            else:
                print(f"macOS icons: compiling Icon Composer source with actool {number}", flush=True)
                partial = output / "icon-info.plist"
                # Keep the app's deployment target; the layered rendition is for macOS 26+.
                subprocess.run([
                    str(actool), str(ICON_SOURCE), "--compile", str(output),
                    "--output-format", "human-readable-text", "--notices", "--warnings",
                    "--output-partial-info-plist", str(partial), "--app-icon", "rustfus",
                    "--include-all-app-icons", "--enable-on-demand-resources", "NO",
                    "--development-region", "en", "--target-device", "mac",
                    "--minimum-deployment-target", str(info.get("LSMinimumSystemVersion", "11.0")),
                    "--platform", "macosx",
                ], check=True)
                catalog = output / "Assets.car"
                if not catalog.is_file() or catalog.stat().st_size == 0:
                    raise ValueError("actool succeeded without a nonempty Assets.car")
                with partial.open("rb") as stream:
                    generated = plistlib.load(stream)
                # Do not let generated metadata raise the app's minimum OS or replace its identity.
                compiled_info = {key: value for key, value in generated.items()
                                 if key in ("CFBundleIconName", "CFBundleIcons", "CFBundleIcons~ipad")}
                compiled_info["CFBundleIconName"] = "rustfus"
                shutil.copyfile(catalog, resources / "Assets.car")

        # actool may also emit a flattened ICNS; preserve the owner's supplied fallback instead.
        shutil.copyfile(FALLBACK, resources / "rustfus.icns")
        for key in ("CFBundleIconName", "CFBundleIcons", "CFBundleIcons~ipad"):
            info.pop(key, None)
        info.update(compiled_info)
        info["CFBundleIconFile"] = "rustfus.icns"
        info["NSRemovableVolumesUsageDescription"] = (
            "Rustfus needs access to removable volumes to create bootable media."
        )
        with info_path.open("wb") as stream:
            plistlib.dump(info, stream, sort_keys=False)

    if compiled_info:
        print("macOS icons: installed Assets.car plus supplied ICNS fallback")
    else:
        print("macOS icons: ICNS fallback only (no compiled Liquid Glass icon)", file=sys.stderr)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("app", type=Path, help="unsigned .app with an existing Contents/Info.plist")
    parser.add_argument("--mode", choices=("auto", "required", "fallback"), default="auto",
                        help="auto falls back only when actool is missing or older than 26; compilation errors fail")
    parser.add_argument("--actool", type=Path, help="explicit actool executable (otherwise use xcrun)")
    arguments = parser.parse_args()
    try:
        install(arguments.app, arguments.mode, arguments.actool)
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError, plistlib.InvalidFileException) as error:
        parser.exit(1, f"macOS icon packaging failed: {error}\n")


if __name__ == "__main__":
    main()
