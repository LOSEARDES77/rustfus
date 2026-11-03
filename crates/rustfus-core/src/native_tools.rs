// SPDX-License-Identifier: GPL-3.0-or-later
//! Embedded userspace tools, extracted only into a private per-operation directory.
use std::{
    ffi::OsStr,
    fs,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[derive(Clone, Copy)]
pub(crate) enum Tool {
    Mkntfs,
    Wimlib,
    NtfsImport,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Self::Mkntfs => "mkntfs",
            Self::Wimlib => "wimlib-imagex",
            Self::NtfsImport => "ntfs-import",
        }
    }
    fn bytes(self) -> &'static [u8] {
        match self {
            Self::Mkntfs => include_bytes!(env!("RUSTFUS_MKNTFS")),
            Self::Wimlib => include_bytes!(env!("RUSTFUS_WIMLIB")),
            Self::NtfsImport => include_bytes!(env!("RUSTFUS_NTFS_IMPORT")),
        }
    }
}

pub(crate) struct Tools {
    directory: tempfile::TempDir,
}
impl Tools {
    pub(crate) fn new() -> Result<Self, String> {
        let directory = tempfile::Builder::new()
            .prefix("rustfus-tools-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        for tool in [Tool::Mkntfs, Tool::Wimlib, Tool::NtfsImport] {
            let path = directory.path().join(tool.name());
            fs::write(&path, tool.bytes())
                .map_err(|e| format!("Extracting bundled {}: {e}", tool.name()))?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o500))
                .map_err(|e| e.to_string())?;
        }
        Ok(Self { directory })
    }
    pub(crate) fn path(&self, tool: Tool) -> std::path::PathBuf {
        self.directory.path().join(tool.name())
    }
    pub(crate) fn run(
        &self,
        tool: Tool,
        args: &[&OsStr],
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        let mut diagnostics = tempfile::tempfile().map_err(|e| e.to_string())?;
        let mut child = Command::new(self.path(tool))
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("LC_ALL", "C")
            .env("TMPDIR", self.directory.path())
            .stdin(Stdio::null())
            .stdout(diagnostics.try_clone().map_err(|e| e.to_string())?)
            .stderr(diagnostics.try_clone().map_err(|e| e.to_string())?)
            .spawn()
            .map_err(|e| format!("Starting bundled {}: {e}", tool.name()))?;
        let status = loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Cancelled".into());
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error.to_string());
                }
            }
        };
        if status.success() {
            return Ok(());
        }
        diagnostics
            .seek(SeekFrom::Start(0))
            .map_err(|e| e.to_string())?;
        let mut message = String::new();
        diagnostics
            .take(16384)
            .read_to_string(&mut message)
            .map_err(|e| e.to_string())?;
        Err(format!(
            "Bundled {} failed ({status}): {}",
            tool.name(),
            message.trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_tools_run_without_path_or_shared_libraries() {
        let tools = Tools::new().unwrap();
        for tool in [Tool::Mkntfs, Tool::Wimlib] {
            tools
                .run(tool, &[OsStr::new("--version")], &AtomicBool::new(false))
                .unwrap();
        }
        assert!(
            tools
                .run(
                    Tool::Wimlib,
                    &[OsStr::new("--version")],
                    &AtomicBool::new(true)
                )
                .is_err()
        );
    }
}
