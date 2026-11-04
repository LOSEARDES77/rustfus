use std::{env, path::PathBuf, process::Command};

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    for path in ["packaging/native", "vendor/ntfs-3g", "vendor/wimlib"] {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    for key in [
        "PYTHON",
        "CC",
        "AR",
        "RANLIB",
        "CFLAGS",
        "CPPFLAGS",
        "LDFLAGS",
        "MAKE",
        "PKG_CONFIG",
        "MACOSX_DEPLOYMENT_TARGET",
        "SDKROOT",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    if os != "macos" && os != "linux" {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let status = Command::new(env::var_os("PYTHON").unwrap_or_else(|| "python3".into()))
        .arg(root.join("packaging/native/build.py"))
        .arg("--out-dir")
        .arg(&out)
        .arg("--host")
        .arg(env::var("HOST").unwrap())
        .arg("--target")
        .arg(env::var("TARGET").unwrap())
        .status()
        .expect("native helpers require Python 3; see docs/native-tools.md");
    assert!(
        status.success(),
        "native helper build failed; see docs/native-tools.md"
    );
    for (key, name) in [
        ("RUSTFUS_MKNTFS", "mkntfs"),
        ("RUSTFUS_WIMLIB", "wimlib-imagex"),
        ("RUSTFUS_NTFS_IMPORT", "ntfs-import"),
    ] {
        let path = out.join("native/bin").join(name);
        assert!(path.is_file(), "missing native helper: {}", path.display());
        println!("cargo:rustc-env={key}={}", path.display());
    }
}
