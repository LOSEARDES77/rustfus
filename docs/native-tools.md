# Building embedded native tools

`cargo build -p rustfus-core` builds three helper executables for macOS and Linux:
`mkntfs`, `wimlib-imagex`, and Rustfus's `ntfs-import`. Cargo invokes
`packaging/native/build.py` automatically. No separate install step, root access,
FUSE driver, kernel extension, Homebrew runtime package or helper sidecar is needed.
Windows builds skip this infrastructure and use their Windows implementation.

Validation on 2026-11-03: macOS arm64 workspace tests and Linux arm64 core tests
and Clippy passed. Disposable regular-file tests covered NTFS import/readback,
Unicode paths, host filename-alias rejection, a file larger than 4 GiB, and WIM
capture/split/verification. macOS x86_64 helpers cross-compiled and their load
commands showed only system runtime linkage; execution on this arm64 host was
unavailable without Rosetta. Physical USB creation, firmware boot, and a full
release/source-archive reconstruction were not validated by those tests.

## Runtime behavior

The executable embeds the three helper binaries. Each operation extracts them
into a private directory (0700) as non-writable executable files (0500), invokes
absolute paths with a cleared environment, and removes them when finished. No
PATH-based replacement or download is used. Cancellation kills and waits for a
running helper; failed helpers report their exit status and bounded diagnostics.

NTFS jobs prepare a private regular filesystem image with mkntfs, import the
existing guarded ISO-copy output through libntfs-3g, and verify every staged file
before opening the USB. The import helper rejects devices, symlinks, hardlinked
source files, special files, unsafe names and excessive directory depth. The
whole partition image is copied with checked I/O and a final media flush.
Temporary space must cover the partition image plus staged files and headroom.
This is not a mounted quick-copy backend. Linux Syslinux-on-NTFS jobs retain the
external mounted backend because their loader needs physical sector mapping.

FAT32 Windows jobs split only `sources/install.wim`, using bundled wimlib and
validating part headers, GUIDs, sequence, size and destination collisions before
disk erasure. macOS retains the OS-managed GPT/FAT32 copy path. Linux uses the
shared splitter with the existing FAT writer. Oversized ESD and other files
require NTFS. No Windows To Go capability is implied by bundling these tools.

Initialize the pinned source before building:

```sh
git submodule update --init --recursive
```

Build dependencies:

* macOS: Xcode command-line tools, Python 3, GNU make, autoconf, automake,
  GNU libtool and pkg-config. With Homebrew:
  `brew install autoconf automake libtool pkg-config`.
* Debian/Ubuntu: `sudo apt-get install build-essential python3 autoconf automake libtool pkg-config`.
* Other Linux distributions: install equivalent compiler, libc development,
  Python 3 and autotools packages. Linux cross-compilation is not supported here.

Only build-time tools come from the package manager. The build checks executable
load commands and rejects dependencies outside macOS system libraries, or outside
Linux libc/compiler support libraries. NTFS and WIM libraries are linked from
static archives. Linux libc itself remains dynamically linked, so release builders
must retain the documented oldest supported glibc baseline.

## Sources and configuration

`packaging/native/pins.json` records full upstream commits:

* ntfs-3g 2026.9.28: `7f0f841fc52cf719106c5c93bafe465004e36816`.
* wimlib v1.14.5: `cd5e231c348c255ae5088873b5a66ee0eb96fa07`.

NTFS builds disable shared libraries, the FUSE driver, crypto, Unicode
normalization conversion, mount helper installation, ldconfig and optional UUID
library detection. Library installation is disabled as well: upstream's install
hook attempts to move shared objects even for a static-only build. Only the
static library and `mkntfs` tool are built. A local m4 definition makes the
optional libgcrypt macro fail if crypto is accidentally enabled; no libgcrypt
development package is needed to regenerate configure with crypto disabled.
`ntfs-import.c` is linked against the same archive with pkg-config's include flags.

WIM builds use `--disable-shared --enable-static --without-fuse --without-ntfs-3g`.
Version 1.14.5 uses its internal XML parser and has no libxml2 dependency or
`--without-libxml2` configure option. We do not pass an unrecognized option.

The script copies both pristine source trees into `$OUT_DIR/native` before running
`autoreconf`; no configure output, objects or generated headers touch submodules.
A source/flags/target fingerprint reuses completed outputs. `cargo clean` removes
them. Changes to source trees, glue, script, pins or documented build flags trigger
a rebuild. A failed build has no completed stamp and is retried from a clean stage.

## Build contract

On macOS/Linux the core build script exports absolute paths for Rust's
`include_bytes!(env!("..."))`:

* `RUSTFUS_MKNTFS`
* `RUSTFUS_WIMLIB`
* `RUSTFUS_NTFS_IMPORT`

They point to real executables, not libtool wrappers. They are not defined for
Windows and must only be referenced in platform-gated Rust code.

`PYTHON` overrides the Python executable. `CC`, `AR`, `RANLIB`, `MAKE`,
`PKG_CONFIG`, `CFLAGS`, `CPPFLAGS`, `LDFLAGS`, `SDKROOT`, and
`MACOSX_DEPLOYMENT_TARGET` configure the native build; `NUM_JOBS` comes from Cargo.
CC/MAKE/PKG_CONFIG support shell-style argument splitting, not shell evaluation.
On macOS the script appends `-arch arm64` or `-arch x86_64` to compilation and link
flags based on Cargo's TARGET, including cross-architecture builds on an Apple
host. The default deployment target is 11.0. Build each Rust target separately,
then combine the application with `lipo`; each Rust slice embeds its own matching
helpers. Do not lipo helper byte arrays themselves.

For a standalone diagnostic build, use an output directory under Cargo's output
area, for example:

```sh
python3 packaging/native/build.py --out-dir "$PWD/target/native-check" \
  --host aarch64-apple-darwin --target aarch64-apple-darwin
```

## Corresponding source distribution

Release checkout is recursive. `packaging/native/source-archive.py OUTPUT.tar.gz`
archives the committed app tree and all recursive submodules using `git archive`.
This includes the glue, build scripts, pins, notices and upstream license files.
It refuses missing or mismatched submodules. It archives committed content, not
working-tree edits; run it from the release commit. The release source asset also
retains the archives listed in `res/SOURCES.txt` for older embedded resources.
