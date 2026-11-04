#!/usr/bin/env python3
"""Build embedded helpers exclusively below Cargo's OUT_DIR, without root."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
ENV_KEYS = ("CC", "AR", "RANLIB", "CFLAGS", "CPPFLAGS", "LDFLAGS", "MAKE",
            "PKG_CONFIG", "MACOSX_DEPLOYMENT_TARGET", "SDKROOT", "PATH")


def run(args, cwd, env, capture=False):
    print("+ " + shlex.join(map(str, args)), flush=True)
    return subprocess.run(list(map(str, args)), cwd=cwd, env=env, check=True,
                          text=True, stdout=subprocess.PIPE if capture else None).stdout


def digest_sources(paths):
    digest = hashlib.sha256()
    for base in paths:
        for path in sorted(base.rglob("*")):
            if path.is_file() and not any(p in (".git", "__pycache__") for p in path.parts):
                digest.update(str(path.relative_to(ROOT)).encode())
                digest.update(path.read_bytes())
    return digest.hexdigest()


def build(out, host, target):
    if "windows" in target:
        return
    mac = target.endswith("apple-darwin")
    if not mac and ("linux" not in target or host != target):
        raise RuntimeError("only native Linux and macOS x86_64/aarch64 targets are supported")
    if mac and not host.endswith("apple-darwin"):
        raise RuntimeError("macOS helpers require an Apple SDK on a macOS host")
    pins = json.loads((ROOT / "packaging/native/pins.json").read_text())
    sources = [ROOT / "vendor" / name for name in pins]
    for source in sources:
        if not (source / "configure.ac").is_file():
            raise RuntimeError("missing submodule; run git submodule update --init --recursive")
        if (source / ".git").exists():
            actual = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
            if actual != pins[source.name]["commit"]:
                raise RuntimeError(f"{source.name}: expected {pins[source.name]['commit']}, found {actual}")
    glue = ROOT / "packaging/native/ntfs-import.c"
    if not glue.is_file():
        raise RuntimeError(f"missing app-owned helper source: {glue}")
    work = out / "native"
    if ROOT == out or ROOT in out.parents and "vendor" in out.relative_to(ROOT).parts:
        raise RuntimeError("OUT_DIR cannot be the repository or a vendor source directory")
    env = os.environ.copy()
    env["GIT_CEILING_DIRECTORIES"] = str(out)
    cc = shlex.split(env.get("CC", "cc"))
    make = shlex.split(env.get("MAKE", "make"))
    pkg = shlex.split(env.get("PKG_CONFIG", "pkg-config"))
    for tool in ["autoreconf", "autoconf", "automake", "aclocal", *[cc[0], make[0], pkg[0]],
                 env.get("AR", "ar"), env.get("RANLIB", "ranlib")]:
        if not shutil.which(tool):
            raise RuntimeError(f"missing build tool {tool}; see docs/native-tools.md")
    if not (shutil.which("glibtoolize") or shutil.which("libtoolize")):
        raise RuntimeError("missing GNU libtool (glibtoolize/libtoolize)")
    if mac:
        arch = {"aarch64": "arm64", "x86_64": "x86_64"}.get(target.split("-")[0])
        if not arch:
            raise RuntimeError(f"unsupported macOS target {target}")
        for key in ("CFLAGS", "LDFLAGS"):
            env[key] = (env.get(key, "-O2" if key == "CFLAGS" else "") + f" -arch {arch}").strip()
        env.setdefault("MACOSX_DEPLOYMENT_TARGET", "11.0")
    env.setdefault("CFLAGS", "-O2")
    fingerprint = hashlib.sha256(json.dumps({
        "sources": digest_sources([*sources, ROOT / "packaging/native"]),
        "host": host, "target": target,
        "environment": {key: env.get(key, "") for key in ENV_KEYS},
    }, sort_keys=True).encode()).hexdigest()
    stamp = work / "stamp"
    names = ("mkntfs", "wimlib-imagex", "ntfs-import")
    if stamp.is_file() and stamp.read_text() == fingerprint and all((work / "bin" / n).is_file() for n in names):
        print("Native helpers are up to date.")
        return
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    prefix = work / "install"
    binary = work / "bin"
    binary.mkdir()
    jobs = os.environ.get("NUM_JOBS", str(os.cpu_count() or 1))
    for source in sources:
        staged = work / source.name
        shutil.copytree(source, staged, ignore=shutil.ignore_patterns(".git", "__pycache__"))
        if source.name == "ntfs-3g":
            # Upstream references this optional macro even with crypto disabled.
            (staged / "m4/rustfus-no-crypto.m4").write_text(
                "AC_DEFUN([AM_PATH_LIBGCRYPT], [AC_MSG_ERROR([Rustfus disables crypto])])\n")
        run(["autoreconf", "--install", "--force"], staged, env)
        flags = [f"--prefix={prefix}", "--disable-shared", "--enable-static"]
        if mac and host != target:
            flags += [f"--host={arch}-apple-darwin"]
        if source.name == "ntfs-3g":
            flags += ["--disable-ntfs-3g", "--disable-crypto", "--disable-nfconv",
                      "--disable-mount-helper", "--disable-ldconfig", "--without-uuid",
                      "--disable-library"]
            # Homebrew gettext can expose libintl.h even when its library is not
            # available for the target.  NTFS-3G only needs libc's setlocale;
            # accepting that optional header redirects the call to
            # libintl_setlocale and breaks the static mkntfs link.
            configure_env = env.copy()
            configure_env["ac_cv_header_libintl_h"] = "no"
            run([staged / "configure", *flags], staged, configure_env)
            run([*make, f"-j{jobs}", "-C", "libntfs-3g"], staged, env)
            run([*make, f"-j{jobs}", "-C", "ntfsprogs", "mkntfs"], staged, env)
            shutil.copy2(staged / "ntfsprogs/mkntfs", binary / "mkntfs")
        else:
            # 1.14.5 has an internal XML parser, no libxml2 option or dependency.
            flags += ["--without-fuse", "--without-ntfs-3g"]
            run([staged / "configure", *flags], staged, env)
            run([*make, f"-j{jobs}", "wimlib-imagex"], staged, env)
            shutil.copy2(staged / "wimlib-imagex", binary / "wimlib-imagex")
    env["PKG_CONFIG_LIBDIR"] = str(work / "ntfs-3g/libntfs-3g")
    env.pop("PKG_CONFIG_PATH", None)
    cflags = shlex.split(run([*pkg, f"--define-variable=includedir={work / 'ntfs-3g/include'}",
                             "--cflags", "libntfs-3g"], work, env, True))
    cflags += ["-DHAVE_CONFIG_H", "-I" + str(work / "ntfs-3g/include/ntfs-3g"),
               "-I" + str(work / "ntfs-3g")]
    libs = [str(work / "ntfs-3g/libntfs-3g/.libs/libntfs-3g.a")]
    run([*cc, *shlex.split(env.get("CPPFLAGS", "")), *shlex.split(env["CFLAGS"]),
         *cflags, glue, "-o", binary / "ntfs-import", *shlex.split(env.get("LDFLAGS", "")), *libs], work, env)
    for name in names:
        helper = binary / name
        deps = run(["otool", "-L", helper] if mac else ["ldd", helper], work, env, True)
        print(deps)
        if mac:
            bad = [line for line in deps.splitlines()[1:] if line.strip() and not line.strip().startswith(("/usr/lib/", "/System/Library/"))]
        else:
            allowed = ("linux-vdso", "ld-linux", "libc.so", "libm.so", "libpthread.so", "libdl.so", "librt.so", "libgcc_s.so")
            bad = [line for line in deps.splitlines() if line.strip() and not any(token in line for token in allowed)]
        if bad:
            raise RuntimeError(f"{name} links non-system runtime libraries: {bad}")
    stamp.write_text(fingerprint)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out-dir", required=True, type=Path)
    parser.add_argument("--host", required=True)
    parser.add_argument("--target", required=True)
    args = parser.parse_args()
    try:
        build(args.out_dir.resolve(), args.host, args.target)
    except (RuntimeError, subprocess.CalledProcessError, OSError) as error:
        sys.exit(f"Native helper build failed: {error}")


if __name__ == "__main__":
    main()
