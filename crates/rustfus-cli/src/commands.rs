// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
//! Command line handlers, mirroring what the GUI does for the same choices.

use crate::args::{Cmd, DownloadArgs, WriteArgs, WueArg};
use rustfus_core::drives::{self, Drive, human};
use rustfus_core::ops::{self, Boot, Fs, Job, Msg, Report, Scheme, Target};
use rustfus_core::update::{self, Version};
use rustfus_core::{fido, image, iso, net, wtg, wue};
use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Sender, channel};

const MB: u64 = 1 << 20;

pub fn run(cmd: Cmd) -> Result<(), String> {
    match cmd {
        Cmd::List { all } => list(all),
        Cmd::Info { image } => info(&image),
        Cmd::Hash { image } => hash(&image),
        Cmd::Write(a) => write(a),
        Cmd::Save {
            device,
            output,
            all,
        } => {
            let d = find_drive(&device, all)?;
            run_job(move |tx, c| ops::save_drive(&d, &output, tx, c))
        }
        Cmd::Download(a) => download(a),
        Cmd::CheckUpdate { betas } => check_update(betas),
    }
}

fn check_update(include_betas: bool) -> Result<(), String> {
    let current = Version::current();
    match update::check(include_betas)? {
        Some(r) => println!(
            "Rustfus {} is available (you have {current}): {}",
            r.version, r.url
        ),
        None => println!("Rustfus {current} is up to date"),
    }
    Ok(())
}

fn list(all: bool) -> Result<(), String> {
    let drives = drives::list(all)?;
    if drives.is_empty() {
        eprintln!("No drives found{}", if all { "" } else { " (try --all)" });
    }
    for d in drives {
        println!("{}\t{}", d.path, d.label());
    }
    Ok(())
}

/// Only drives Rustfus would list, so a typo can't wipe a system disk.
fn find_drive(device: &str, all: bool) -> Result<Drive, String> {
    drives::list(all)?
        .into_iter()
        .find(|d| d.path == device)
        .ok_or_else(|| {
            format!(
                "{device} is not a drive Rustfus can write to; see `rustfus list`{}",
                if all {
                    ""
                } else {
                    " (or --all for USB hard drives)"
                }
            )
        })
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

fn info(path: &Path) -> Result<(), String> {
    let r = ops::analyze(path)?;
    println!("Image:        {}", path.display());
    if !r.is_iso {
        println!("Type:         disk image, written in DD mode");
        println!("Hybrid MBR:   {}", yes_no(r.hybrid));
        return Ok(());
    }
    let (scheme, target) = r.default_layout();
    println!("Type:         ISO, label {:?}", r.label);
    println!("Files:        {}", human(r.size));
    println!(
        "Boot:         BIOS {}, UEFI {} ({})",
        yes_no(r.bios),
        yes_no(r.efi),
        r.arch
    );
    println!("Windows:      {}", yes_no(r.windows));
    if r.grub2 {
        println!("GRUB 2:       {}", r.grub2_version);
    }
    println!("Hybrid:       {} (--dd writes it raw)", yes_no(r.hybrid));
    println!("Persistence:  {}", yes_no(r.persistence_capable()));
    println!(
        "Defaults:     --scheme {} --target {} --fs {}",
        format!("{scheme:?}").to_lowercase(),
        match target {
            Target::BiosUefi => "bios-uefi",
            Target::Uefi => "uefi",
            Target::Bios => "bios",
        },
        r.default_fs().name().to_lowercase()
    );
    if r.windows
        && let Ok(eds) = iso::Iso::open(path)
            .map_err(|e| e.to_string())
            .and_then(|i| wtg::editions(&i))
    {
        println!("Editions (--wtg):");
        for (i, name) in eds {
            println!("  {i}\t{name}");
        }
    }
    Ok(())
}

fn hash(path: &Path) -> Result<(), String> {
    let never = AtomicBool::new(false);
    let h = image::hashes(
        path,
        &|p| progress(&format!("Hashing: {:.1}%", p * 100.0)),
        &never,
    )
    .map_err(|e| e.to_string())?;
    eprint!("\r{:70}\r", "");
    for (name, v) in ["MD5", "SHA1", "SHA256", "SHA512"].iter().zip(h) {
        println!("{name:7}{v}");
    }
    Ok(())
}

fn progress(s: &str) {
    eprint!("\r{s:70}");
    std::io::stderr().flush().ok();
}

/// Runs a job on a thread, printing its progress and log to stderr.
fn run_job<F>(f: F) -> Result<(), String>
where
    F: FnOnce(&Sender<Msg>, &AtomicBool) -> Result<String, String> + Send + 'static,
{
    let (tx, rx) = channel();
    // ponytail: no Ctrl-C handler, so cancel is never set; killing the process stops the job.
    std::thread::spawn(move || {
        let r = f(&tx, &AtomicBool::new(false));
        tx.send(Msg::Done(r)).ok();
    });
    for msg in rx {
        match msg {
            Msg::Progress(_, s) => progress(&s),
            Msg::Log(s) => eprintln!("\r{s:70}"),
            Msg::Done(r) => {
                eprint!("\r{:70}\r", "");
                println!("{}", r?);
                return Ok(());
            }
        }
    }
    Err("job ended without a result".into())
}

fn write(a: WriteArgs) -> Result<(), String> {
    let drive = find_drive(&a.device, a.all)?;
    let report = match &a.image {
        Some(p) => ops::analyze(p).map_err(|e| format!("{}: {e}", p.display()))?,
        None => Report::default(),
    };
    let boot = match (&a.image, a.freedos) {
        (Some(p), _) => Boot::Image(p.clone()),
        (None, true) => Boot::FreeDos,
        (None, false) => Boot::NonBootable,
    };
    let dd = a.image.is_some() && (a.dd || !report.is_iso);
    let iso_mode = a.image.is_some() && !dd;

    let (scheme, target) = if iso_mode {
        report.default_layout()
    } else if a.freedos {
        (Scheme::Mbr, Target::Bios)
    } else {
        (Scheme::Mbr, Target::BiosUefi)
    };
    let scheme = a.scheme.unwrap_or(scheme);
    let target = match (a.target, a.scheme) {
        (Some(t), _) => t,
        // Like the GUI: picking a scheme resets the target to its default.
        (None, Some(_)) if scheme == Scheme::Gpt => Target::Uefi,
        (None, Some(_)) => Target::BiosUefi,
        (None, None) => target,
    };
    if scheme == Scheme::Gpt && target != Target::Uefi {
        return Err("GPT drives can only target UEFI".into());
    }
    let fs = a.fs.unwrap_or(if iso_mode {
        report.default_fs()
    } else {
        Fs::Fat32
    });
    if !fs.supported() {
        return Err(format!("{} cannot be created on this OS", fs.name()));
    }
    if a.freedos && !fs.is_fat() {
        return Err("FreeDOS needs FAT or FAT32".into());
    }
    if !fs.clusters().contains(&a.cluster) {
        return Err(format!(
            "{} cluster sizes: {:?} (0 = default)",
            fs.name(),
            fs.clusters()
        ));
    }
    if a.persistence > 0 && !(iso_mode && report.persistence_capable()) {
        return Err("--persistence needs an Ubuntu (casper) or Debian live ISO".into());
    }
    if a.persistence > 0 && a.persistence < 256 {
        return Err("--persistence must be at least 256 MB".into());
    }
    if a.uefi_validation && !(iso_mode && report.efi) {
        return Err("--uefi-validation needs a UEFI-bootable ISO".into());
    }
    if a.wtg.is_some() && !(iso_mode && report.windows) {
        return Err("--wtg needs a Windows ISO".into());
    }
    if (!a.wue.is_empty() || a.local_account.is_some()) && !(iso_mode && report.windows) {
        return Err("--wue and --local-account need a Windows ISO".into());
    }
    if iso_mode && ops::needs_grub_download(&report, scheme, target) && !a.allow_downloads {
        eprintln!(
            "WARNING: the image uses GRUB {}; add --allow-downloads to fetch a matching core.img or BIOS boot may fail",
            report.grub2_version
        );
    }
    let label = a.label.unwrap_or_else(|| {
        if iso_mode {
            report.label.clone()
        } else {
            drive.name.clone()
        }
    });
    let has = |w| a.wue.contains(&w);
    let job = Job {
        boot,
        dd,
        scheme,
        target,
        fs,
        cluster: a.cluster,
        label,
        quick: !a.full_format,
        ext_label: !a.no_extended_label,
        bad_blocks: a.bad_blocks,
        verify: !a.no_verify,
        old_bios: a.old_bios,
        uefi_validation: a.uefi_validation,
        persistence: a.persistence * MB,
        wue: wue::Options {
            bypass_hw_checks: has(WueArg::BypassHwChecks),
            no_online_account: has(WueArg::NoOnlineAccount),
            local_account: a.local_account.filter(|u| !u.trim().is_empty()),
            regional: has(WueArg::Regional),
            no_data_collection: has(WueArg::NoDataCollection),
            no_bitlocker: has(WueArg::NoBitlocker),
            wtg_no_internal: has(WueArg::WtgNoInternal) && a.wtg.is_some(),
        },
        allow_downloads: a.allow_downloads,
        wtg: a.wtg,
        drive,
    };
    eprintln!(
        "{} -> {}: {}, {:?}, {}, {}",
        match &job.boot {
            Boot::Image(p) => p.display().to_string(),
            Boot::FreeDos => "FreeDOS".into(),
            Boot::NonBootable => "format".into(),
        },
        job.drive.label(),
        if dd { "DD mode" } else { "ISO mode" },
        job.scheme,
        job.target.name(),
        job.fs.name()
    );
    if !a.yes && !confirm(&job.drive)? {
        return Err("Cancelled".into());
    }
    run_job(move |tx, c| ops::run_job(&job, tx, c))
}

fn confirm(d: &Drive) -> Result<bool, String> {
    eprint!(
        "WARNING: ALL DATA ON {} ({}) WILL BE DESTROYED. Continue? [y/N] ",
        d.label(),
        d.path
    );
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "YES"))
}

/// Default when an option is omitted: Fido's choices (English, this machine's arch, else the first).
fn default_pick(field: usize, opts: &[String]) -> String {
    let want = match field {
        3 => "English",
        4 => match std::env::consts::ARCH {
            "aarch64" => "ARM64",
            "x86" => "x86",
            _ => "x64",
        },
        _ => "",
    };
    opts.iter()
        .find(|o| o.as_str() == want)
        .unwrap_or(&opts[0])
        .clone()
}

fn download(a: DownloadArgs) -> Result<(), String> {
    let given = [a.win, a.rel, a.ed, a.lang, a.arch];
    let mut chosen = Vec::new();
    for (i, given) in given.into_iter().enumerate() {
        let field = fido::FIELDS[i];
        let opts = fido::list(&chosen)?;
        if opts.is_empty() {
            return Err(format!("No {field} choices"));
        }
        let pick = match given {
            // Exact match first, else a case-insensitive prefix ("26H2", "spanish").
            Some(v) => opts
                .iter()
                .find(|o| o.eq_ignore_ascii_case(&v))
                .or_else(|| {
                    opts.iter()
                        .find(|o| o.to_lowercase().starts_with(&v.to_lowercase()))
                })
                .cloned()
                .ok_or_else(|| format!("No {field} {v:?}; choices: {}", opts.join(", ")))?,
            None if a.list => {
                println!("{field}:");
                for o in opts {
                    println!("  {o}");
                }
                return Ok(());
            }
            None => default_pick(i, &opts),
        };
        eprintln!("{field}: {pick}");
        chosen.push(pick);
    }
    if a.list {
        eprintln!("All options given; nothing left to list");
        return Ok(());
    }
    let url = fido::url(&chosen)?;
    if a.url_only {
        println!("{url}");
        return Ok(());
    }
    let out = a.output.unwrap_or_else(|| fido::file_name(&url).into());
    eprintln!("Saving to {}", out.display());
    run_job(move |tx, c| {
        let progress = |p: f32, got| {
            tx.send(Msg::Progress(
                p,
                format!("Downloading: {} ({:.1}%)", human(got), p * 100.0),
            ))
            .ok();
        };
        net::download(&url, &out, &progress, c)?;
        Ok(format!("Downloaded {}", out.display()))
    })
}

#[cfg(test)]
#[path = "../tests/unit/commands.rs"]
mod tests;
