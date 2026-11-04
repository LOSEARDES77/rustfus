// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
//! Stage FAT32-compatible Windows setup files before opening the destination disk.

use super::Ctx;
use crate::{
    iso::{Entry, Iso},
    native_tools::{Tool, Tools},
};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

const FAT_MAX_FILE: u64 = (4 << 30) - 1;
pub(super) type SplitFiles = Vec<(String, PathBuf)>;

pub(super) struct StagedWim {
    _directory: tempfile::TempDir,
    pub original: String,
    pub files: SplitFiles,
}

fn validate_fat_manifest<'a>(
    entries: impl IntoIterator<Item = (&'a str, bool, u64)>,
    capacity: u64,
) -> Result<(), String> {
    let mut paths = HashMap::new();
    let mut required = 256 * super::MB;
    for (path, directory, size) in entries {
        if path.is_empty()
            || path.contains(['\\', '\0'])
            || !path.split('/').all(|part| {
                !part.is_empty()
                    && part != "."
                    && part != ".."
                    && !part.ends_with([' ', '.'])
                    && !part
                        .chars()
                        .any(|c| c.is_control() || "<>:\"|?*".contains(c))
            })
        {
            return Err(format!(
                "Unsafe FAT32 destination path: {path}; no disk was erased"
            ));
        }
        if paths.insert(path.to_lowercase(), directory).is_some() {
            return Err(format!(
                "Duplicate FAT32 destination path: {path}; no disk was erased"
            ));
        }
        if !directory && size > FAT_MAX_FILE {
            return Err("A staged file exceeds FAT32 limits; no disk was erased".into());
        }
        let allocated = if directory {
            65536
        } else {
            size.checked_add(65535).ok_or("FAT32 allocation overflow")? / 65536 * 65536
        };
        required = required
            .checked_add(allocated)
            .ok_or("FAT32 allocation overflow")?;
    }
    for path in paths.keys() {
        let mut parent = path.as_str();
        while let Some((prefix, _)) = parent.rsplit_once('/') {
            if paths.get(prefix) == Some(&false) {
                return Err(format!(
                    "FAT32 file/directory conflict: {prefix}; no disk was erased"
                ));
            }
            parent = prefix;
        }
    }
    if required > capacity {
        return Err(
            "The prepared Windows files do not fit the FAT32 partition; no disk was erased".into(),
        );
    }
    Ok(())
}

pub(super) fn validate_fat_capacity(
    iso: &Iso,
    staged: &StagedWim,
    capacity: u64,
) -> Result<(), String> {
    let entries = iso.entries().map_err(super::e)?;
    let mut manifest: Vec<_> = entries
        .iter()
        .filter(|entry| !entry.path.eq_ignore_ascii_case(&staged.original))
        .map(|entry| (entry.path.as_str(), entry.is_dir, entry.size))
        .collect();
    for (path, source) in &staged.files {
        manifest.push((
            path.as_str(),
            false,
            fs::metadata(source).map_err(super::e)?.len(),
        ));
    }
    validate_fat_manifest(manifest, capacity)
}

pub(super) fn stage_wim(iso: &Iso, entry: &Entry, ctx: &Ctx) -> Result<StagedWim, String> {
    ctx.check()?;
    if !entry.path.eq_ignore_ascii_case("sources/install.wim")
        || entry.is_dir
        || entry.symlink().is_some()
    {
        return Err("Only sources/install.wim can be split for FAT32; no disk was erased".into());
    }
    let tools = Tools::new()?;
    let directory = tempfile::Builder::new()
        .prefix("rustfus-wim-")
        .tempdir()
        .map_err(super::e)?;
    let source = directory.path().join("input.wim");
    let mut reader = iso.reader(entry).map_err(super::e)?;
    let mut output = File::create(&source).map_err(super::e)?;
    let mut buffer = vec![0; 4 << 20];
    let mut done = 0u64;
    loop {
        ctx.check()?;
        let read = reader.read(&mut buffer).map_err(super::e)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read]).map_err(super::e)?;
        done += read as u64;
        ctx.progress(
            done as f32 / entry.size.max(1) as f32,
            "Staging install.wim before erasing USB…",
        );
    }
    if done != entry.size {
        return Err("Unexpected end of install.wim; no disk was erased".into());
    }
    output.sync_all().map_err(super::e)?;
    drop(output);
    ctx.progress(0., "Splitting install.wim for FAT32 before erasing USB…");
    split(&tools, &source, directory.path(), "3800", ctx.cancel)
        .map_err(|error| format!("Could not split install.wim; no disk was erased. {error}"))?;
    ctx.check()?;
    let parent = entry
        .path
        .rsplit_once('/')
        .ok_or("Invalid install.wim path")?
        .0;
    let files = collect_parts(directory.path(), parent)?;
    validate_split_paths(
        iso.entries()
            .map_err(super::e)?
            .iter()
            .map(|entry| entry.path.as_str()),
        &files,
    )?;
    Ok(StagedWim {
        _directory: directory,
        original: entry.path.clone(),
        files,
    })
}

fn split(
    tools: &Tools,
    source: &Path,
    directory: &Path,
    size_mb: &str,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let output = directory.join("install.swm");
    tools.run(
        Tool::Wimlib,
        &[
            OsStr::new("split"),
            source.as_os_str(),
            output.as_os_str(),
            OsStr::new(size_mb),
        ],
        cancel,
    )
}

fn part_number(name: &str) -> Option<u16> {
    if name == "install.swm" {
        return Some(1);
    }
    let number = name.strip_prefix("install")?.strip_suffix(".swm")?;
    let part = number.parse::<u16>().ok()?;
    (part >= 2 && number == part.to_string()).then_some(part)
}

fn collect_parts(directory: &Path, parent: &str) -> Result<SplitFiles, String> {
    let mut parts = Vec::new();
    for item in fs::read_dir(directory).map_err(super::e)? {
        let path = item.map_err(super::e)?.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Invalid split WIM filename")?;
        if name == "input.wim" {
            continue;
        }
        let number =
            part_number(name).ok_or_else(|| format!("Unexpected split WIM output: {name}"))?;
        let metadata = fs::symlink_metadata(&path).map_err(super::e)?;
        if !metadata.is_file() || metadata.len() < 208 || metadata.len() > FAT_MAX_FILE {
            return Err(
                "A split WIM part exceeds FAT32 limits or is invalid; no disk was erased".into(),
            );
        }
        let mut header = [0u8; 208];
        File::open(&path)
            .and_then(|mut file| file.read_exact(&mut header))
            .map_err(super::e)?;
        let header_size = u32::from_le_bytes(header[8..12].try_into().unwrap()) as u64;
        let header_part = u16::from_le_bytes(header[40..42].try_into().unwrap());
        let total = u16::from_le_bytes(header[42..44].try_into().unwrap());
        if &header[..8] != b"MSWIM\0\0\0"
            || header_size < 208
            || header_size > metadata.len()
            || header_part != number
            || total < number
        {
            return Err(format!(
                "Invalid split WIM header: {name}; no disk was erased"
            ));
        }
        let guid: [u8; 16] = header[24..40].try_into().unwrap();
        parts.push((number, total, guid, format!("{parent}/{name}"), path));
    }
    parts.sort_by_key(|part| part.0);
    let first = parts
        .first()
        .ok_or("wimlib produced no install.swm; no disk was erased")?;
    let total = first.1;
    let guid = first.2;
    if parts.len() != usize::from(total)
        || parts.iter().enumerate().any(|(index, part)| {
            usize::from(part.0) != index + 1 || part.1 != total || part.2 != guid
        })
    {
        return Err("Split WIM parts are missing, out of sequence, or from different images; no disk was erased".into());
    }
    Ok(parts
        .into_iter()
        .map(|(_, _, _, destination, path)| (destination, path))
        .collect())
}

pub(super) fn validate_split_paths<'a>(
    existing: impl IntoIterator<Item = &'a str>,
    files: &SplitFiles,
) -> Result<(), String> {
    let existing: HashSet<_> = existing.into_iter().map(str::to_ascii_lowercase).collect();
    let mut seen = HashSet::new();
    for (path, _) in files {
        let lower = path.to_ascii_lowercase();
        let safe = !path.is_empty()
            && !path.contains(['\\', '\0'])
            && path
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".." && !part.contains(':'));
        if !safe
            || existing.contains(&lower)
            || existing
                .iter()
                .any(|path| path.starts_with(&format!("{lower}/")))
            || !seen.insert(lower)
        {
            return Err(format!(
                "Split WIM output conflicts with ISO content: {path}. No disk was erased."
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fat_capacity_and_manifest_fail_before_target_access() {
        let entries = [
            ("sources", true, 0),
            ("sources/install.swm", false, 100 * super::super::MB),
        ];
        assert!(validate_fat_manifest(entries, 512 * super::super::MB).is_ok());
        assert!(validate_fat_manifest(entries, 128 * super::super::MB).is_err());
        for entries in [
            vec![("A", false, 1), ("a", false, 1)],
            vec![("A", false, 1), ("a/child", false, 1)],
            vec![("../escape", false, 1)],
        ] {
            assert!(validate_fat_manifest(entries, 1024 * super::super::MB).is_err());
        }
    }

    fn fixture(directory: &Path, name: &str, part: u16, total: u16, guid: u8) {
        let mut header = [0u8; 208];
        header[..8].copy_from_slice(b"MSWIM\0\0\0");
        header[8..12].copy_from_slice(&208u32.to_le_bytes());
        header[24..40].fill(guid);
        header[40..42].copy_from_slice(&part.to_le_bytes());
        header[42..44].copy_from_slice(&total.to_le_bytes());
        fs::write(directory.join(name), header).unwrap();
    }

    #[test]
    fn parts_follow_numeric_order_not_filename_order() {
        let directory = tempfile::tempdir().unwrap();
        for part in 1..=12 {
            let name = if part == 1 {
                "install.swm".into()
            } else {
                format!("install{part}.swm")
            };
            fixture(directory.path(), &name, part, 12, 1);
        }
        let files = collect_parts(directory.path(), "sources").unwrap();
        assert_eq!(files[1].0, "sources/install2.swm");
        assert_eq!(files[9].0, "sources/install10.swm");
    }

    #[test]
    fn rejects_missing_parts_wrong_headers_and_mixed_images() {
        let directory = tempfile::tempdir().unwrap();
        fixture(directory.path(), "install.swm", 1, 2, 1);
        assert!(collect_parts(directory.path(), "sources").is_err());
        fixture(directory.path(), "install2.swm", 1, 2, 1);
        assert!(collect_parts(directory.path(), "sources").is_err());
        fixture(directory.path(), "install2.swm", 2, 2, 2);
        assert!(collect_parts(directory.path(), "sources").is_err());
        fixture(directory.path(), "install2.swm", 2, 2, 1);
        assert!(collect_parts(directory.path(), "sources").is_ok());
        fs::write(directory.path().join("install2.swm"), b"MSWIM\0\0\0").unwrap();
        assert!(collect_parts(directory.path(), "sources").is_err());
    }

    #[test]
    fn rejects_noncanonical_names_and_symlinks() {
        assert_eq!(part_number("install1.swm"), None);
        assert_eq!(part_number("install02.swm"), None);
        assert_eq!(part_number("install0.swm"), None);
        let directory = tempfile::tempdir().unwrap();
        fixture(directory.path(), "other", 1, 1, 1);
        std::os::unix::fs::symlink(
            directory.path().join("other"),
            directory.path().join("install.swm"),
        )
        .unwrap();
        assert!(collect_parts(directory.path(), "sources").is_err());
    }

    #[test]
    fn rejects_destination_collisions_and_unsafe_paths() {
        let files = vec![("sources/install.swm".into(), PathBuf::from("part"))];
        assert!(validate_split_paths(["sources/install.wim"], &files).is_ok());
        assert!(validate_split_paths(["SOURCES/INSTALL.SWM"], &files).is_err());
        assert!(validate_split_paths(["sources/install.swm/child"], &files).is_err());
        assert!(validate_split_paths([], &vec![files[0].clone(), files[0].clone()]).is_err());
        assert!(
            validate_split_paths([], &vec![("../install.swm".into(), PathBuf::from("part"))])
                .is_err()
        );
    }

    #[test]
    fn bundled_capture_and_split_small_wim_without_devices() {
        let tools = Tools::new().expect("bundled wimlib must be available");
        assert!(tools.path(Tool::Wimlib).is_absolute());
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("capture");
        fs::create_dir(&source).unwrap();
        // Separate streams force multiple parts even though a WIM stream cannot be divided.
        for file in 0..4u32 {
            let bytes: Vec<u8> = (0..(700 * 1024u32))
                .map(|index| (index.wrapping_mul(31).wrapping_add(file * 17) % 251) as u8)
                .collect();
            fs::write(source.join(format!("file{file}.bin")), bytes).unwrap();
        }
        let output = directory.path().join("parts");
        fs::create_dir(&output).unwrap();
        let wim = output.join("input.wim");
        let cancel = AtomicBool::new(false);
        tools
            .run(
                Tool::Wimlib,
                &[
                    OsStr::new("capture"),
                    source.as_os_str(),
                    wim.as_os_str(),
                    OsStr::new("--compress=none"),
                    OsStr::new("--no-acls"),
                ],
                &cancel,
            )
            .unwrap();
        split(&tools, &wim, &output, "1", &cancel).unwrap();
        let files = collect_parts(&output, "sources").unwrap();
        assert!(files.len() > 1);
        for (_, path) in &files {
            assert!(fs::metadata(path).unwrap().len() <= FAT_MAX_FILE);
        }
        tools
            .run(
                Tool::Wimlib,
                &[
                    OsStr::new("verify"),
                    files[0].1.as_os_str(),
                    OsStr::new(&format!("--ref={}/*.swm", output.display())),
                ],
                &cancel,
            )
            .unwrap();
        assert!(validate_split_paths(["sources/INSTALL2.SWM"], &files).is_err());
    }
}
