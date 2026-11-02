// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
#![cfg_attr(windows, windows_subsystem = "windows")]

mod args;
mod commands;

use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    #[cfg(target_os = "macos")]
    {
        let mut arguments = std::env::args_os().skip(1);
        if arguments.next().as_deref()
            == Some(std::ffi::OsStr::new(rustfus_core::macos_auth::WORKER_ARG))
        {
            let result = (|| {
                let socket = arguments.next().ok_or("Missing worker connection")?;
                let token = arguments.next().ok_or("Missing worker session")?;
                let uid = arguments.next().ok_or("Missing worker user")?;
                if arguments.next().is_some() {
                    return Err("Unexpected worker arguments".into());
                }
                let token = token.to_str().ok_or("Invalid worker session")?;
                let uid = uid
                    .to_str()
                    .ok_or("Invalid worker user")?
                    .parse::<u32>()
                    .map_err(|e| e.to_string())?;
                rustfus_core::macos_auth::worker(std::path::Path::new(&socket), token, uid)
            })();
            return match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::FAILURE
                }
            };
        }
    }
    // No arguments: the GUI, as launched from the menu entry.
    if std::env::args_os().len() <= 1 {
        return match rustfus_gui::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    #[cfg(windows)]
    attach_console();
    match commands::run(args::Cli::parse().cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The GUI subsystem starts without a console; print to the one we were run from.
#[cfg(windows)]
fn attach_console() {
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    unsafe extern "system" {
        fn AttachConsole(pid: u32) -> i32;
    }
    // SAFETY: AttachConsole takes a plain integer and has no memory-safety
    // preconditions; it fails harmlessly when there is no parent console.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
