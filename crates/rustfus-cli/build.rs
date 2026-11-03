// SPDX-License-Identifier: GPL-3.0-or-later
// Windows executables get the Rustfus icon. GPUI owns the single embedded
// compatibility manifest; elevation is requested at runtime with ShellExecuteExW.
fn main() {
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../res/rustfus.ico");
        res.compile().expect("compiling Windows resources");
    }
}
