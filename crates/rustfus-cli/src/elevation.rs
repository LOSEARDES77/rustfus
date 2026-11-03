// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
//! Windows elevation without a second embedded manifest resource.

#[cfg(windows)]
use std::{ffi::OsStr, os::windows::ffi::OsStrExt};

// CommandLineToArgvW/MSVC escaping, not shell escaping: runas executes the binary
// directly, so metacharacters must remain literal arguments.
#[cfg(any(windows, test))]
fn quote(argument: &[u16]) -> Vec<u16> {
    let mut output = vec![b'"' as u16];
    let mut slashes = 0;
    for &unit in argument {
        if unit == b'\\' as u16 {
            slashes += 1;
        } else {
            output.extend(std::iter::repeat_n(
                b'\\' as u16,
                if unit == b'"' as u16 {
                    slashes * 2 + 1
                } else {
                    slashes
                },
            ));
            output.push(unit);
            slashes = 0;
        }
    }
    output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    output.push(b'"' as u16);
    output
}

#[cfg(windows)]
fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

/// None means this process is already elevated. A GUI relaunch lets its parent
/// exit; CLI callers wait and receive the elevated command's full Windows status.
/// The caller must use process::exit(code as i32), since stable ExitCode is u8-only.
#[cfg(windows)]
pub(crate) fn ensure(gui: bool) -> Result<Option<u32>, String> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::{ERROR_CANCELLED, WAIT_OBJECT_0},
        System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject},
        UI::{
            Shell::{
                IsUserAnAdmin, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
                ShellExecuteExW,
            },
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    };
    // SAFETY: IsUserAnAdmin has no pointer arguments or additional preconditions.
    if unsafe { IsUserAnAdmin() } != 0 {
        return Ok(None);
    }
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let directory = std::env::current_dir().map_err(|e| e.to_string())?;
    let file = wide(executable.as_os_str());
    let directory = wide(directory.as_os_str());
    let verb = wide(OsStr::new("runas"));
    let mut arguments = Vec::new();
    for argument in std::env::args_os().skip(1) {
        if !arguments.is_empty() {
            arguments.push(b' ' as u16);
        }
        arguments.extend(quote(&argument.encode_wide().collect::<Vec<_>>()));
    }
    arguments.push(0);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        lpParameters: arguments.as_ptr(),
        lpDirectory: directory.as_ptr(),
        nShow: SW_SHOWNORMAL,
        ..Default::default()
    };
    // SAFETY: All UTF-16 buffers and info remain live for this synchronous call.
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        let error = std::io::Error::last_os_error();
        return Err(if error.raw_os_error() == Some(ERROR_CANCELLED as i32) {
            "Administrator authorization was cancelled".into()
        } else {
            format!("Could not request administrator authorization: {error}")
        });
    }
    if info.hProcess.is_null() {
        return Err("Windows did not return an elevated process handle".into());
    }
    // SAFETY: SEE_MASK_NOCLOSEPROCESS transfers ownership of this valid handle.
    let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess) };
    if gui {
        return Ok(Some(0));
    }
    // SAFETY: process keeps the handle alive throughout the wait.
    if unsafe { WaitForSingleObject(process.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
        return Err(format!(
            "Could not wait for the elevated process: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut code = 0u32;
    // SAFETY: process is valid and code points to writable storage.
    if unsafe { GetExitCodeProcess(process.as_raw_handle(), &mut code) } == 0 {
        return Err(format!(
            "Could not read the elevated process exit code: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(Some(code))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn encoded(value: &str) -> String {
        String::from_utf16(&quote(&value.encode_utf16().collect::<Vec<_>>())).unwrap()
    }
    #[test]
    fn quotes_empty_spaces_quotes_and_trailing_backslashes() {
        assert_eq!(encoded(""), "\"\"");
        assert_eq!(
            encoded("C:\\My Images\\Windows.iso"),
            "\"C:\\My Images\\Windows.iso\""
        );
        assert_eq!(encoded("a\"b"), "\"a\\\"b\"");
        assert_eq!(encoded("C:\\dir\\"), "\"C:\\dir\\\\\"");
        assert_eq!(encoded("& ; $ 日本語"), "\"& ; $ 日本語\"");
    }

    #[test]
    fn doubles_backslashes_only_before_quotes_and_the_closing_quote() {
        assert_eq!(encoded(r#"a\"b"#), r#""a\\\"b""#);
        assert_eq!(encoded(r"a\\b"), r#""a\\b""#);
        assert_eq!(encoded("\t\n"), "\"\t\n\"");
    }

    #[test]
    fn preserves_unpaired_utf16_surrogates() {
        assert_eq!(
            quote(&[0xd800, 0xdc00, 0xdfff]),
            [34, 0xd800, 0xdc00, 0xdfff, 34]
        );
    }
}
