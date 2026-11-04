#!/usr/bin/env python3
"""Archive committed app and recursive submodule sources for GPL releases."""
import argparse
import io
import json
from pathlib import Path
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[2]


def archive(output):
    pins = json.loads((ROOT / "packaging/native/pins.json").read_text())
    for name, pin in pins.items():
        actual = subprocess.check_output(["git", "-C", str(ROOT / "vendor" / name), "rev-parse", "HEAD"], text=True).strip()
        if actual != pin["commit"]:
            raise RuntimeError(f"{name}: source commit does not match pins.json")
    submodules = subprocess.check_output(["git", "submodule", "status", "--recursive"], cwd=ROOT, text=True)
    projects = [(ROOT, "rustfus/")]
    for line in submodules.splitlines():
        if not line.startswith(" "):
            raise RuntimeError(f"submodule is absent or mismatched: {line}")
        path = line.strip().split()[1]
        projects.append((ROOT / path, f"rustfus/{path}/"))
    output.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(output, "w:gz") as dest:
        for source, prefix in projects:
            data = subprocess.check_output(["git", "archive", "--format=tar", f"--prefix={prefix}", "HEAD"], cwd=source)
            with tarfile.open(fileobj=io.BytesIO(data)) as src:
                for member in src:
                    dest.addfile(member, src.extractfile(member) if member.isfile() else None)
    print(output)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    archive(parser.parse_args().output.resolve())
