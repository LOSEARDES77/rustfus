// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! UI-independent operations, polled by the native application.

use rustfus_core::drives::Drive;
use rustfus_core::loc::{t, tf};
use rustfus_core::ops::{Boot, Fs, Job, Msg, Report, Scheme, Target};
use rustfus_core::update::{self, Release};
use rustfus_core::{drives, iso, loc, net, ops, wtg, wue};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MB: u64 = 1 << 20;

pub(crate) type ScanResult = Result<Vec<Drive>, String>;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Log(Vec<String>);

impl Log {
    pub(crate) fn push(&mut self, value: String) {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            % 86_400;
        let timestamp = format!(
            "[{:02}:{:02}:{:02}]",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        );
        // split preserves empty messages and trailing blank rows, unlike lines().
        self.0.extend(value.split('\n').map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            format!("{timestamp} {line}")
        }));
    }

    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }
}

impl std::ops::Deref for Log {
    type Target = [String];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

fn finish_scan(
    scanning: &mut bool,
    scan_error: &mut Option<String>,
    result: ScanResult,
) -> Option<Vec<Drive>> {
    *scanning = false;
    match result {
        Ok(drives) => {
            *scan_error = None;
            Some(drives)
        }
        Err(error) => {
            *scan_error = Some(error);
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BootSel {
    Image,
    FreeDos,
    NonBootable,
}

#[derive(Clone, Debug)]
pub(crate) enum Dialog {
    None,
    Download,
    /// Rufus' "Download required" for a Syslinux 5+ loader Rustfus doesn't
    /// embed: the image's version ("6.03/2014-10-06").
    SyslinuxDownload(String),
    /// Rufus' "Replace menu.c32?": index into the image's obsolete c32 files.
    ReplaceC32(usize),
    Edition,
    Wue,
    Hybrid {
        dd: bool,
    },
    Confirm,
    NewVersion(Release),
    Error(String),
}

pub(crate) struct Model {
    pub(crate) drives: Vec<Drive>,
    pub(crate) selected: Option<usize>,
    pub(crate) scan_rx: Receiver<ScanResult>,
    pub(crate) scanning: bool,
    pub(crate) scan_error: Option<String>,
    pub(crate) pending_drives: Option<Vec<Drive>>,
    pub(crate) list_all: Arc<AtomicBool>,
    pub(crate) rescan: Arc<AtomicBool>,
    pub(crate) scanner_stop: Arc<AtomicBool>,
    pub(crate) confirmation_drive: Option<Drive>,

    pub(crate) boot: BootSel,
    pub(crate) image: Option<PathBuf>,
    pub(crate) report: Report,
    pub(crate) scheme: Scheme,
    pub(crate) target: Target,
    pub(crate) fs: Fs,
    pub(crate) cluster: u32,
    pub(crate) label: String,
    pub(crate) persistence_mb: u64,

    pub(crate) adv_drive: bool,
    pub(crate) adv_format: bool,
    pub(crate) old_bios: bool,
    pub(crate) uefi_validation: bool,
    pub(crate) quick: bool,
    pub(crate) ext_label: bool,
    pub(crate) bad_blocks: bool,
    pub(crate) passes: u8,
    pub(crate) verify: bool,
    pub(crate) wue: wue::Options,
    pub(crate) wue_user: String,
    pub(crate) wue_use_account: bool,
    pub(crate) dd: bool,
    pub(crate) allow_downloads: bool,
    pub(crate) replace_c32: Vec<String>,
    pub(crate) wtg: bool,
    pub(crate) editions: Vec<(u32, String)>,
    pub(crate) edition: u32,
    pub(crate) download_mode: bool,
    pub(crate) pending_image: Option<PathBuf>,

    pub(crate) dialog: Dialog,
    pub(crate) show_log: bool,
    pub(crate) log: Log,

    pub(crate) update: update::Settings,
    pub(crate) update_rx: Option<Receiver<Result<Option<Release>, String>>>,
    /// Started with "Check Now": a newer release always gets its dialog.
    pub(crate) update_manual: bool,
    /// Outcome of the last check, shown in the toolbar popover.
    pub(crate) update_note: String,

    pub(crate) busy: Option<(Receiver<Msg>, Arc<AtomicBool>)>,
    pub(crate) progress: f32,
    pub(crate) status: String,
    pub(crate) current_file: String,
    /// Outcome of the last job, shown until the next device scan.
    pub(crate) done: Option<bool>,
    pub(crate) started: Option<Instant>,
    pub(crate) elapsed: Duration,
}

impl Model {
    pub(crate) fn progress_label(&self) -> String {
        if self.busy.is_some() {
            if self.status.is_empty() {
                "Starting…".into()
            } else {
                self.status.clone()
            }
        } else if let Some(done) = self.done {
            if done { t("MSG_210") } else { t("MSG_212") }
        } else if self.scanning {
            "Scanning for devices…".into()
        } else if self.scan_error.is_some() {
            "Device scan failed".into()
        } else if self.drives.is_empty() {
            "No devices found".into()
        } else {
            t("MSG_210")
        }
    }

    pub(crate) fn progress_value(&self) -> f32 {
        if self.busy.is_some() {
            if self.progress.is_finite() {
                self.progress.clamp(0., 1.) * 100.
            } else {
                0.
            }
        } else if self.done == Some(true) {
            100.
        } else {
            0.
        }
    }

    pub(crate) fn can_start(&self) -> bool {
        self.scan_ready()
            && self.busy.is_none()
            && self.drive().is_some()
            && (self.boot != BootSel::Image || self.image.is_some())
    }

    pub(crate) fn cancel_dialog(&mut self) {
        self.confirmation_drive = None;
        self.dialog = Dialog::None;
    }

    pub(crate) fn cancel_job(&self) {
        if let Some((_, cancel)) = &self.busy {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    pub(crate) fn advance(&mut self, stage: u8) {
        if self.can_start() {
            self.dialog = self.next_dialog(stage);
        } else {
            self.cancel_dialog();
        }
    }

    pub(crate) fn approve(&mut self, yes: bool) {
        match self.dialog.clone() {
            Dialog::Download => {
                self.allow_downloads |= yes;
                self.advance(1);
            }
            Dialog::SyslinuxDownload(_) if yes => {
                self.allow_downloads = true;
                self.advance(2);
            }
            Dialog::ReplaceC32(i) => {
                let paths = self.obsolete_c32();
                let path = paths.get(i).cloned();
                let more = i + 1 < paths.len();
                if yes && let Some(path) = path {
                    self.replace_c32.push(path);
                }
                if more {
                    self.dialog = Dialog::ReplaceC32(i + 1);
                } else {
                    self.advance(2);
                }
            }
            Dialog::Edition if yes => self.advance(3),
            Dialog::Wue if yes => self.advance(4),
            Dialog::Hybrid { dd } if yes => {
                self.dd = dd;
                self.advance(4);
            }
            Dialog::Confirm if yes => self.start_job(),
            _ => self.cancel_dialog(),
        }
    }

    pub(crate) fn save_drive(&mut self, path: PathBuf) {
        if !self.scan_ready() || self.busy.is_some() {
            return;
        }
        if let Some(drive) = self.drive().cloned() {
            self.spawn(move |tx, cancel| ops::save_drive(&drive, &path, tx, cancel));
        }
    }

    pub(crate) fn download_iso(&mut self, url: String, path: PathBuf) {
        if self.busy.is_some() {
            return;
        }
        self.cancel_dialog();
        self.log.push(format!("Downloading {url}"));
        self.pending_image = Some(path.clone());
        self.spawn(move |tx, cancel| {
            let progress = |p, got| {
                tx.send(Msg::Progress(p, tf("MSG_241", &[&drives::human(got)])))
                    .ok();
            };
            net::download(&url, &path, &progress, cancel)?;
            Ok(format!("Downloaded {}", path.display()))
        });
    }

    pub(crate) fn new() -> Self {
        loc::set_language(&loc::default_for_system());
        let list_all = Arc::new(AtomicBool::new(false));
        let rescan = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let saved = update::Settings::load();
        let scan_rx = spawn_scanner(list_all.clone(), rescan.clone(), stop.clone());
        let mut model = Self::with_inputs(scan_rx, list_all, rescan, saved.clone());
        model.scanner_stop = stop;
        if saved.is_some_and(|s| s.is_due(update::now())) {
            model.check_updates(false);
        }
        model
    }

    pub(crate) fn with_inputs(
        scan_rx: Receiver<ScanResult>,
        list_all: Arc<AtomicBool>,
        rescan: Arc<AtomicBool>,
        saved_policy: Option<update::Settings>,
    ) -> Self {
        let mut log = Log::default();
        log.push(format!("Rustfus {}", env!("CARGO_PKG_VERSION")));
        Self {
            drives: Vec::new(),
            selected: None,
            scan_rx,
            scanning: true,
            scan_error: None,
            pending_drives: None,
            list_all,
            rescan,
            scanner_stop: Arc::new(AtomicBool::new(false)),
            confirmation_drive: None,
            boot: BootSel::Image,
            image: None,
            report: Report::default(),
            scheme: Scheme::Mbr,
            target: Target::BiosUefi,
            fs: Fs::Fat32,
            cluster: 0,
            label: String::new(),
            persistence_mb: 0,
            adv_drive: false,
            adv_format: false,
            old_bios: false,
            uefi_validation: false,
            quick: true,
            ext_label: true,
            bad_blocks: false,
            passes: 0,
            verify: true,
            wue: wue::Options {
                bypass_hw_checks: true,
                no_online_account: true,
                no_data_collection: true,
                ..Default::default()
            },
            wue_user: std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_default(),
            wue_use_account: false,
            dd: false,
            allow_downloads: false,
            replace_c32: Vec::new(),
            wtg: false,
            editions: Vec::new(),
            edition: 0,
            download_mode: false,
            pending_image: None,
            dialog: Dialog::None,
            show_log: false,
            log,
            update: saved_policy.clone().unwrap_or_default(),
            update_rx: None,
            update_manual: false,
            update_note: String::new(),
            busy: None,
            progress: 0.0,
            status: String::new(),
            current_file: String::new(),
            done: None,
            started: None,
            elapsed: Duration::ZERO,
        }
    }

    pub(crate) fn drive(&self) -> Option<&Drive> {
        self.selected.and_then(|i| self.drives.get(i))
    }

    pub(crate) fn retry_scan(&mut self) {
        self.scanning = true;
        self.rescan.store(true, Ordering::Relaxed);
    }

    pub(crate) fn scan_ready(&self) -> bool {
        !self.scanning && self.scan_error.is_none()
    }

    pub(crate) fn set_drives(&mut self, d: Vec<Drive>) {
        let keep = self.drive().map(|d| d.path.clone());
        self.selected = keep
            .as_ref()
            .and_then(|p| d.iter().position(|x| &x.path == p))
            .or((!d.is_empty()).then_some(0));
        if self.drive() != self.selected.and_then(|i| d.get(i)) {
            self.done = None;
        }
        self.drives = d;
        if !self.image_mode() && self.label.is_empty() {
            self.label = self.drive().map(|d| d.name.clone()).unwrap_or_default();
        }
    }

    pub(crate) fn select_image(&mut self, path: PathBuf) {
        if self.busy.is_some() {
            return;
        }
        self.cancel_dialog();
        match ops::analyze(&path) {
            Ok(r) => {
                self.log
                    .push(format!("Scanned image {}: {r:?}", path.display()));
                self.dd = !r.is_iso;
                if r.is_iso {
                    self.label = r.label.clone();
                    self.fs = r.default_fs();
                    (self.scheme, self.target) = r.default_layout();
                }
                self.cluster = 0;
                self.persistence_mb = 0;
                self.uefi_validation = false;
                self.wtg = false;
                self.report = r;
                self.image = Some(path);
            }
            Err(err) => {
                self.dialog = Dialog::Error(format!(
                    "{}: {err}",
                    tf("MSG_322", &[&path.display().to_string()])
                ))
            }
        }
    }

    pub(crate) fn boot_mode(&self) -> Boot {
        match (self.boot, &self.image) {
            (BootSel::Image, Some(p)) => Boot::Image(p.clone()),
            (BootSel::FreeDos, _) => Boot::FreeDos,
            _ => Boot::NonBootable,
        }
    }

    pub(crate) fn image_mode(&self) -> bool {
        self.boot == BootSel::Image && self.image.is_some()
    }

    pub(crate) fn iso_mode(&self) -> bool {
        self.image_mode() && self.report.is_iso && !self.dd
    }

    fn windows_options(&self) -> wue::Options {
        if !self.iso_mode() || !self.report.windows {
            return wue::Options::default();
        }
        wue::Options {
            bypass_hw_checks: self.report.windows_11 && self.wue.bypass_hw_checks,
            no_online_account: self.report.windows_11 && self.wue.no_online_account,
            local_account: self
                .wue_use_account
                .then(|| self.wue_user.trim().to_string())
                .filter(|name| !name.is_empty()),
            wtg_no_internal: self.wtg && self.wue.wtg_no_internal,
            ..self.wue.clone()
        }
    }

    /// Rufus' START sequence: boot file downloads, WTG edition, Windows
    /// customization or ISOHybrid choice, then the data-loss warning.
    /// `stage` is how far the user already got.
    pub(crate) fn next_dialog(&mut self, stage: u8) -> Dialog {
        let r = &self.report;
        let iso = self.image_mode() && r.is_iso;
        if stage < 1 && iso && ops::needs_grub_download(r, self.scheme, self.target) {
            return Dialog::Download;
        }
        // Syslinux 5+ (download) and < 5 (obsolete c32) are exclusive, so they share a stage.
        if stage < 2
            && iso
            && let Some(sl) = ops::syslinux_download(r, self.fs, self.scheme, self.target)
        {
            return Dialog::SyslinuxDownload(format!("{}{}", sl.version, sl.ext));
        }
        if stage < 2 && iso && !self.obsolete_c32().is_empty() {
            return Dialog::ReplaceC32(0);
        }
        if stage < 3 && iso && r.windows && self.wtg {
            return Dialog::Edition;
        }
        if stage < 4 && self.iso_mode() && r.windows {
            return Dialog::Wue;
        }
        if stage < 4 && iso && r.hybrid {
            self.dd = false;
            return Dialog::Hybrid { dd: false };
        }
        Dialog::Confirm
    }

    pub(crate) fn obsolete_c32(&self) -> &[String] {
        ops::obsolete_c32(&self.report, self.fs, self.scheme, self.target)
    }

    pub(crate) fn on_start(&mut self) {
        if !self.can_start() {
            return;
        }
        self.allow_downloads = false;
        self.replace_c32.clear();
        self.confirmation_drive = self.drive().cloned();
        self.dialog = self.next_dialog(0);
    }

    pub(crate) fn spawn<F>(&mut self, f: F)
    where
        F: FnOnce(&Sender<Msg>, &AtomicBool) -> Result<String, String> + Send + 'static,
    {
        if self.busy.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        std::thread::spawn(move || {
            let r = f(&tx, &c);
            tx.send(Msg::Done(r)).ok();
        });
        self.busy = Some((rx, cancel));
        self.started = Some(Instant::now());
        self.progress = 0.0;
        self.done = None;
        self.status.clear();
        self.current_file.clear();
    }

    pub(crate) fn start_job(&mut self) {
        // A scan can fail while the confirmation dialog is open.
        self.poll();
        if !self.can_start()
            || !matches!(self.dialog, Dialog::Confirm)
            || self.confirmation_drive.as_ref() != self.drive()
        {
            return;
        }
        let Some(drive) = self.drive().cloned() else {
            return;
        };
        self.dialog = Dialog::None;
        self.confirmation_drive = None;
        let job = Job {
            drive,
            boot: self.boot_mode(),
            dd: self.dd,
            scheme: self.scheme,
            target: self.target,
            fs: self.fs,
            cluster: self.cluster,
            label: self.label.clone(),
            quick: self.quick,
            ext_label: self.ext_label,
            bad_blocks: self.bad_blocks.then_some(self.passes),
            verify: self.verify,
            old_bios: self.old_bios,
            uefi_validation: self.uefi_validation && self.iso_mode(),
            persistence: if self.iso_mode() && self.report.persistence_capable() {
                self.persistence_mb * MB
            } else {
                0
            },
            wue: self.windows_options(),
            allow_downloads: self.allow_downloads,
            replace_c32: self.replace_c32.clone(),
            wtg: (self.wtg && self.iso_mode() && self.report.windows).then_some(self.edition),
        };
        self.log.push(format!(
            "Starting: {:?} dd={} {:?} {} {} cluster={} label={:?}",
            job.boot,
            job.dd,
            job.scheme,
            job.target.name(),
            job.fs.name(),
            job.cluster,
            job.label
        ));
        #[cfg(target_os = "macos")]
        self.spawn(move |tx, c| rustfus_core::macos_auth::run_job(&job, tx, c));
        #[cfg(not(target_os = "macos"))]
        self.spawn(move |tx, c| ops::run_job(&job, tx, c));
    }

    pub(crate) fn check_updates(&mut self, manual: bool) {
        if self.update_rx.is_some() {
            self.update_manual |= manual;
            return;
        }
        let (tx, rx) = channel();
        let include_betas = self.update.include_betas;
        std::thread::spawn(move || {
            tx.send(update::check(include_betas)).ok();
        });
        self.update_rx = Some(rx);
        self.update_manual = manual;
        self.update_note = rustfus_name(&t("MSG_243"));
        self.log.push(self.update_note.clone());
    }

    pub(crate) fn on_update_checked(&mut self, result: Result<Option<Release>, String>) {
        match result {
            Ok(found) => {
                self.update.last_check = update::now();
                if let Err(err) = self.update.save() {
                    self.log
                        .push(format!("Could not save the update settings: {err}"));
                }
                self.update_note = rustfus_name(&t(if found.is_some() {
                    "MSG_246"
                } else {
                    "MSG_247"
                }));
                self.log.push(self.update_note.clone());
                // An automatic check must not interrupt START or a running job.
                if let Some(r) = found
                    && (self.update_manual
                        || (matches!(self.dialog, Dialog::None) && self.busy.is_none()))
                {
                    self.dialog = Dialog::NewVersion(r);
                }
            }
            Err(err) => {
                self.update_note = t("MSG_244");
                self.log.push(format!("{}: {err}", self.update_note));
            }
        }
    }

    pub(crate) fn poll(&mut self) {
        if let Some(rx) = &self.update_rx
            && let Ok(r) = rx.try_recv()
        {
            self.update_rx = None;
            self.on_update_checked(r);
        }
        while let Ok(result) = self.scan_rx.try_recv() {
            if let Some(drives) = finish_scan(&mut self.scanning, &mut self.scan_error, result) {
                self.pending_drives = Some(drives);
            }
        }
        if self.busy.is_none()
            && let Some(drives) = self.pending_drives.take()
        {
            self.set_drives(drives);
        }
        let Some((rx, _)) = &self.busy else { return };
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Progress(p, s) => (self.progress, self.status) = (p, s),
                Msg::CurrentFile(s) => self.current_file = s,
                Msg::Log(s) => self.log.push(s),
                Msg::Done(r) => {
                    self.busy = None;
                    self.elapsed = self.started.take().map(|t| t.elapsed()).unwrap_or_default();
                    match r {
                        Ok(s) => {
                            self.log.push(s);
                            self.done = Some(true);
                            if let Some(p) = self.pending_image.take() {
                                self.select_image(p);
                            }
                        }
                        Err(err) => {
                            self.pending_image = None;
                            self.log.push(format!("ERROR: {err}"));
                            self.done = Some(false);
                            self.dialog = Dialog::Error(err);
                        }
                    }
                    self.busy = None;
                    self.retry_scan();
                    // Apply a scan received during the job even if the next scan is unchanged.
                    if let Some(drives) = self.pending_drives.take() {
                        let done = self.done;
                        self.set_drives(drives);
                        self.done = done;
                    }
                    return;
                }
            }
        }
    }

    pub(crate) fn load_editions(&mut self) {
        let r = self.image.as_ref().ok_or(t("MSG_086")).and_then(|p| {
            let iso = iso::Iso::open(p).map_err(|e| e.to_string())?;
            wtg::editions(&iso)
        });
        match r {
            Ok(e) if !e.is_empty() => {
                self.edition = e[0].0;
                self.editions = e;
            }
            Ok(_) => self.dialog = Dialog::Error("No Windows editions found in this image".into()),
            Err(err) => {
                self.wtg = false;
                self.dialog = Dialog::Error(format!("{}: {err}", t("MSG_118")));
            }
        }
    }
}

fn rustfus_name(s: &str) -> String {
    s.replace("Rufus", "Rustfus")
}

impl Drop for Model {
    fn drop(&mut self) {
        self.scanner_stop.store(true, Ordering::Relaxed);
        self.cancel_job();
    }
}

fn spawn_scanner(
    list_all: Arc<AtomicBool>,
    rescan: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) -> Receiver<ScanResult> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut last = None;
        while !stop.load(Ordering::Relaxed) {
            // A retry must emit even an unchanged result.
            if rescan.swap(false, Ordering::Relaxed) {
                last = None;
            }
            let result = drives::list(list_all.load(Ordering::Relaxed));
            if stop.load(Ordering::Relaxed) {
                return;
            }
            if last.as_ref() != Some(&result) {
                last = Some(result.clone());
                if tx.send(result).is_err() {
                    return;
                }
            }
            for _ in 0..20 {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                if rescan.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Model, Sender<ScanResult>) {
        let (tx, rx) = channel();
        (
            Model::with_inputs(
                rx,
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicBool::new(false)),
                None,
            ),
            tx,
        )
    }

    fn drive() -> Drive {
        Drive {
            path: "test-device".into(),
            name: "Test USB".into(),
            size: 8_000_000_000,
            serial: Some("test-serial".into()),
        }
    }

    #[test]
    fn log_timestamps_preserve_every_row() {
        let mut log = Log::default();
        log.push("ERROR: <disk> & café\n\nsecond\r\n".into());
        log.push(String::new());
        for (row, message) in log
            .iter()
            .zip(["ERROR: <disk> & café", "", "second", "", ""])
        {
            assert_eq!(&row[11..], message);
            assert_eq!(&row[3..4], ":");
            assert!(row[1..3].parse::<u8>().unwrap() < 24);
            assert!(row[4..6].parse::<u8>().unwrap() < 60);
            assert!(row[7..9].parse::<u8>().unwrap() < 60);
        }
        assert_eq!(log.len(), 5);
        log.clear();
        assert!(log.is_empty());
    }

    #[test]
    fn scan_empty_failure_recovery_and_retry() {
        let (mut model, tx) = fixture();
        tx.send(Ok(vec![])).unwrap();
        model.poll();
        assert!(model.scan_ready());
        assert!(model.drive().is_none());
        tx.send(Ok(vec![drive()])).unwrap();
        model.poll();
        tx.send(Err("Permission denied".into())).unwrap();
        model.poll();
        assert!(!model.scan_ready());
        assert_eq!(model.drives, vec![drive()]);
        model.retry_scan();
        assert!(model.scanning && model.rescan.load(Ordering::Relaxed));
        tx.send(Ok(vec![drive()])).unwrap();
        model.poll();
        assert!(model.scan_ready());
        tx.send(Ok(vec![])).unwrap();
        model.poll();
        assert!(model.selected.is_none());
    }

    #[test]
    fn job_error_clears_pending_image_and_retains_result() {
        let (mut model, scan) = fixture();
        let (tx, rx) = channel();
        model.busy = Some((rx, Arc::new(AtomicBool::new(false))));
        model.pending_image = Some("not-a-real-image".into());
        scan.send(Ok(vec![drive()])).unwrap();
        tx.send(Msg::Done(Err("Synthetic failure".into()))).unwrap();
        model.poll();
        assert!(matches!(model.dialog, Dialog::Error(ref e) if e == "Synthetic failure"));
        assert_eq!(model.done, Some(false));
        assert!(model.pending_image.is_none() && model.busy.is_none());
        assert_eq!(model.drives, vec![drive()]);
        assert!(model.scanning);
    }

    #[test]
    fn start_requires_ready_drive_image_and_confirmation_identity() {
        let (mut model, tx) = fixture();
        model.on_start();
        assert!(matches!(model.dialog, Dialog::None));
        tx.send(Ok(vec![])).unwrap();
        model.poll();
        model.on_start();
        assert!(matches!(model.dialog, Dialog::None));
        tx.send(Ok(vec![drive()])).unwrap();
        model.poll();
        model.on_start();
        assert!(matches!(model.dialog, Dialog::None));
        model.boot = BootSel::NonBootable;
        model.start_job();
        assert!(model.busy.is_none());
        model.on_start();
        assert!(matches!(model.dialog, Dialog::Confirm));
        model.cancel_dialog();
        model.approve(true);
        assert!(model.busy.is_none());
        model.on_start();
        tx.send(Err("Device scan failed".into())).unwrap();
        model.start_job();
        assert!(model.busy.is_none());
        tx.send(Ok(vec![drive()])).unwrap();
        model.poll();
        model.on_start();
        model.drives[0].serial = Some("replacement-device".into());
        model.start_job();
        assert!(model.busy.is_none());
    }

    #[test]
    fn progress_bar_tracks_idle_writing_completion_and_failure() {
        let (mut model, scan) = fixture();
        scan.send(Ok(vec![])).unwrap();
        model.poll();
        assert_eq!(model.progress_label(), "No devices found");
        assert_eq!(model.progress_value(), 0.);

        scan.send(Ok(vec![drive()])).unwrap();
        model.poll();
        assert_eq!(model.progress_label(), t("MSG_210"));
        assert_eq!(model.progress_value(), 0.);

        let (tx, rx) = channel();
        model.busy = Some((rx, Arc::new(AtomicBool::new(false))));
        assert_eq!(model.progress_label(), "Starting…");
        tx.send(Msg::CurrentFile("sources/install.wim (4.4 GB)".into()))
            .unwrap();
        tx.send(Msg::Progress(0.86, "Copying ISO files: 86.0%".into()))
            .unwrap();
        model.poll();
        assert_eq!(model.current_file, "sources/install.wim (4.4 GB)");
        assert_eq!(model.progress_label(), "Copying ISO files: 86.0%");
        assert_eq!(model.progress_value(), 86.);

        tx.send(Msg::Done(Ok("Finished".into()))).unwrap();
        model.poll();
        assert_eq!(model.progress_label(), t("MSG_210"));
        assert_eq!(model.progress_value(), 100.);
        scan.send(Ok(vec![drive()])).unwrap();
        model.poll();
        assert_eq!(model.progress_value(), 100.);

        let (tx, rx) = channel();
        model.busy = Some((rx, Arc::new(AtomicBool::new(false))));
        tx.send(Msg::Done(Err("Synthetic failure".into()))).unwrap();
        model.poll();
        assert_eq!(model.progress_label(), t("MSG_212"));
        assert_eq!(model.progress_value(), 0.);
    }

    #[test]
    fn progress_clamps_invalid_values_and_resets_on_drive_change() {
        let (mut model, _scan) = fixture();
        let (_tx, rx) = channel();
        model.busy = Some((rx, Arc::new(AtomicBool::new(false))));
        for (value, expected) in [(f32::NAN, 0.), (f32::INFINITY, 0.), (-0.5, 0.), (2., 100.)] {
            model.progress = value;
            assert_eq!(model.progress_value(), expected);
        }
        model.busy = None;
        model.set_drives(vec![drive()]);
        model.done = Some(true);
        let mut replacement = drive();
        replacement.serial = Some("replacement".into());
        model.set_drives(vec![replacement]);
        assert_eq!(model.done, None);
        assert_eq!(model.progress_value(), 0.);
    }

    #[test]
    fn drop_signals_scanner_and_active_job() {
        let (mut model, _tx) = fixture();
        let stop = model.scanner_stop.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let (_, rx) = channel();
        model.busy = Some((rx, cancel.clone()));
        drop(model);
        assert!(stop.load(Ordering::Relaxed));
        assert!(cancel.load(Ordering::Relaxed));
    }
}
