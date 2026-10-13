// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
//! Command line definition.

use clap::{Args, Parser, Subcommand, ValueEnum};
use rustfus_core::ops::{Fs, Scheme, Target};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "rustfus",
    version,
    about = "Create bootable USB drives. Run without arguments to open the GUI."
)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// List the drives Rustfus can write to
    List {
        /// Also list USB hard drives
        #[arg(long)]
        all: bool,
    },
    /// Show what Rustfus detects in an image and the settings it would pick
    Info { image: PathBuf },
    /// Print the MD5, SHA-1, SHA-256 and SHA-512 of an image
    Hash { image: PathBuf },
    /// Write an image to a drive, or just format it
    Write(WriteArgs),
    /// Save a drive to a .img or fixed .vhd file
    Save {
        /// Drive to read, as shown by `rustfus list`
        #[arg(short, long)]
        device: String,
        /// Output file; a .vhd extension writes a fixed VHD
        output: PathBuf,
        /// Allow USB hard drives
        #[arg(long)]
        all: bool,
    },
    /// Download an official Windows ISO (or the UEFI Shell), like Rufus' Fido
    Download(DownloadArgs),
}

#[derive(Args)]
pub struct WriteArgs {
    /// Drive to write, as shown by `rustfus list`
    #[arg(short, long)]
    pub device: String,
    /// ISO or disk image (.img, .vhd, .vhdx, .ffu, compressed...); omit to format only
    pub image: Option<PathBuf>,
    /// Make the drive boot FreeDOS instead of writing an image
    #[arg(long, conflicts_with = "image")]
    pub freedos: bool,
    /// Write the image raw (DD mode) even if it is an ISO
    #[arg(long, requires = "image")]
    pub dd: bool,
    /// Partition scheme [default: picked from the image]
    #[arg(long, value_enum)]
    pub scheme: Option<SchemeArg>,
    /// Target system [default: picked from the image]
    #[arg(long, value_enum)]
    pub target: Option<TargetArg>,
    /// File system [default: picked from the image]
    #[arg(long, value_enum)]
    pub fs: Option<FsArg>,
    /// Cluster size in bytes [default: file system default]
    #[arg(long, default_value_t = 0)]
    pub cluster: u32,
    /// Volume label [default: the image's or the drive's]
    #[arg(short, long)]
    pub label: Option<String>,
    /// Zero the whole drive instead of a quick format
    #[arg(long)]
    pub full_format: bool,
    /// Don't write autorun.inf with the label and icon
    #[arg(long)]
    pub no_extended_label: bool,
    /// Check for bad blocks first: 0 = 1 pass, 1 = 2 passes, 2-4 = 4 passes for SLC/MLC/TLC NAND, 5 = random pattern
    #[arg(long, value_name = "MODE", value_parser = clap::value_parser!(u8).range(0..6))]
    pub bad_blocks: Option<u8>,
    /// Don't read back and compare after a DD write
    #[arg(long)]
    pub no_verify: bool,
    /// Add fixes for old BIOSes (partition at sector 63)
    #[arg(long)]
    pub old_bios: bool,
    /// Add UEFI media validation (uefi-md5sum)
    #[arg(long)]
    pub uefi_validation: bool,
    /// Persistent partition size in MB for Ubuntu/Debian live ISOs (at least 256)
    #[arg(long, value_name = "MB", default_value_t = 0)]
    pub persistence: u64,
    /// Download GRUB/Syslinux files matching the image from rufus.ie when needed
    #[arg(long)]
    pub allow_downloads: bool,
    /// Windows setup customizations, comma separated
    #[arg(long, value_enum, value_delimiter = ',')]
    pub wue: Vec<WueArg>,
    /// Create a local Windows account with this name
    #[arg(long, value_name = "NAME")]
    pub local_account: Option<String>,
    /// Windows To Go with this install.wim edition index (Windows only; see `rustfus info`)
    #[arg(long, value_name = "INDEX")]
    pub wtg: Option<u32>,
    /// Allow USB hard drives
    #[arg(long)]
    pub all: bool,
    /// Don't ask before erasing the drive
    #[arg(short, long)]
    pub yes: bool,
}

#[derive(Args)]
pub struct DownloadArgs {
    /// Version, e.g. "Windows 11" [default: first listed]
    #[arg(long)]
    pub win: Option<String>,
    /// Release [default: latest]
    #[arg(long)]
    pub rel: Option<String>,
    /// Edition [default: first listed]
    #[arg(long)]
    pub ed: Option<String>,
    /// Language, e.g. "English" or "Spanish" [default: English]
    #[arg(long)]
    pub lang: Option<String>,
    /// Architecture, e.g. x64 or ARM64 [default: this machine's]
    #[arg(long)]
    pub arch: Option<String>,
    /// List the choices for the first option not given, then exit
    #[arg(long)]
    pub list: bool,
    /// Only print the download URL
    #[arg(long, conflicts_with = "output")]
    pub url_only: bool,
    /// Where to save the ISO [default: its name, in the current directory]
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum SchemeArg {
    Mbr,
    Gpt,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum TargetArg {
    BiosUefi,
    Uefi,
    Bios,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum FsArg {
    Fat,
    Fat32,
    Ntfs,
    Udf,
    Exfat,
    Refs,
    Ext2,
    Ext3,
    Ext4,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
pub enum WueArg {
    /// Remove the TPM, Secure Boot and RAM requirements
    BypassHwChecks,
    /// Don't require an online Microsoft account
    NoOnlineAccount,
    /// Use this computer's regional settings
    Regional,
    /// Disable data collection questions
    NoDataCollection,
    /// Disable automatic BitLocker encryption
    NoBitlocker,
    /// Windows To Go: hide the internal disks
    WtgNoInternal,
}

impl From<SchemeArg> for Scheme {
    fn from(s: SchemeArg) -> Self {
        match s {
            SchemeArg::Mbr => Scheme::Mbr,
            SchemeArg::Gpt => Scheme::Gpt,
        }
    }
}

impl From<TargetArg> for Target {
    fn from(t: TargetArg) -> Self {
        match t {
            TargetArg::BiosUefi => Target::BiosUefi,
            TargetArg::Uefi => Target::Uefi,
            TargetArg::Bios => Target::Bios,
        }
    }
}

impl From<FsArg> for Fs {
    fn from(f: FsArg) -> Self {
        // Same order as `Fs::ALL`.
        Fs::ALL[f as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn fs_args_match_fs_order() {
        for f in FsArg::value_variants() {
            let name = f.to_possible_value().unwrap().get_name().to_string();
            assert_eq!(Fs::from(*f).name().to_lowercase(), name);
        }
    }
}
