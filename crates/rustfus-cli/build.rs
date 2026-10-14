// SPDX-License-Identifier: GPL-3.0-or-later
// Windows executables get the Rustfus icon and a UAC manifest. Only when
// building on Windows with MSVC, where `rc.exe` is available.
fn main() {
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../res/rustfus.ico")
            .set_manifest_file("../../packaging/rustfus.manifest");
        res.compile().expect("compiling Windows resources");
    }
}
