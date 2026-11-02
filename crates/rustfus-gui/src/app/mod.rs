// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Application state and logic; the UI lives in `sections` and `dialogs`.

mod dialogs;
mod sections;

use crate::widgets::sync_style;
use eframe::egui::{self};
use functora_egui::ShadcnThemeExt;
use rustfus_core::drives::Drive;
use rustfus_core::loc::{t, tf};
use rustfus_core::ops::{Boot, Fs, Job, Msg, Report, Scheme, Target};
use rustfus_core::update::{self, Release};
use rustfus_core::{drives, fido, image, iso, loc, ops, wtg, wue};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

const MB: u64 = 1 << 20;

type ScanResult = Result<Vec<Drive>, String>;

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

#[derive(Clone, Copy, PartialEq)]
enum BootSel {
    Image,
    FreeDos,
    NonBootable,
}

enum Dialog {
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
    Hash(Option<[String; 4]>, f32),
    Fido(FidoState),
    NewVersion(Release),
    Error(String),
}

/// Rufus' DOWNLOAD dialog: one Fido query per step (version, release, ...).
struct FidoState {
    chosen: Vec<String>,
    options: Vec<String>,
    pick: usize,
    rx: Option<Receiver<Result<Vec<String>, String>>>,
    error: Option<String>,
}

impl FidoState {
    fn query(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let chosen = self.chosen.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = if chosen.len() == fido::FIELDS.len() {
                fido::url(&chosen).map(|u| vec![u])
            } else {
                fido::list(&chosen)
            };
            tx.send(r).ok();
            ctx.request_repaint();
        });
        self.rx = Some(rx);
        self.error = None;
    }
}

struct App {
    drives: Vec<Drive>,
    selected: Option<usize>,
    scan_rx: Receiver<ScanResult>,
    scanning: bool,
    scan_error: Option<String>,
    pending_drives: Option<Vec<Drive>>,
    list_all: Arc<AtomicBool>,
    rescan: Arc<AtomicBool>,

    boot: BootSel,
    image: Option<PathBuf>,
    report: Report,
    scheme: Scheme,
    target: Target,
    fs: Fs,
    cluster: u32,
    label: String,
    persistence_mb: u64,

    adv_drive: bool,
    adv_format: bool,
    old_bios: bool,
    uefi_validation: bool,
    quick: bool,
    ext_label: bool,
    bad_blocks: bool,
    passes: u8,
    verify: bool,
    wue: wue::Options,
    wue_user: String,
    wue_use_account: bool,
    dd: bool,
    allow_downloads: bool,
    replace_c32: Vec<String>,
    wtg: bool,
    editions: Vec<(u32, String)>,
    edition: u32,
    download_mode: bool,
    pending_image: Option<PathBuf>,

    dialog: Dialog,
    show_log: bool,
    show_about: bool,
    theme_preference: egui::ThemePreference,
    log: Vec<String>,

    update: update::Settings,
    update_rx: Option<Receiver<Result<Option<Release>, String>>>,
    /// Started with "Check Now": a newer release always gets its dialog.
    update_manual: bool,
    /// Outcome of the last check, shown in the toolbar popover.
    update_note: String,

    busy: Option<(Receiver<Msg>, Arc<AtomicBool>)>,
    hash_rx: Option<Receiver<(f32, Option<[String; 4]>)>>,
    progress: f32,
    status: String,
    /// Outcome of the last job, shown until the next device scan.
    done: Option<bool>,
    started: Option<Instant>,
    elapsed: Duration,
}

/// Background drive scan every 2 s so plugging a stick updates the list.
fn spawn_scanner(
    ctx: egui::Context,
    list_all: Arc<AtomicBool>,
    rescan: Arc<AtomicBool>,
) -> Receiver<ScanResult> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut last = None;
        loop {
            let result = drives::list(list_all.load(Ordering::Relaxed));
            if last.as_ref() != Some(&result) {
                last = Some(result.clone());
                if tx.send(result).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
            for _ in 0..20 {
                if rescan.swap(false, Ordering::Relaxed) {
                    last = None;
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });
    rx
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        cc.egui_ctx.set_theme(egui::ThemePreference::System);
        sync_style(&cc.egui_ctx);
        loc::set_language(&loc::default_for_system());
        let list_all = Arc::new(AtomicBool::new(false));
        let rescan = Arc::new(AtomicBool::new(false));
        let saved_policy = update::Settings::load();
        let scan_rx = spawn_scanner(cc.egui_ctx.clone(), list_all.clone(), rescan.clone());
        let mut app = Self::with_inputs(scan_rx, list_all, rescan, saved_policy.clone());
        if saved_policy.is_some_and(|s| s.is_due(update::now())) {
            app.check_updates(&cc.egui_ctx, /* manual= */ false);
        }
        app
    }

    fn with_inputs(
        scan_rx: Receiver<ScanResult>,
        list_all: Arc<AtomicBool>,
        rescan: Arc<AtomicBool>,
        saved_policy: Option<update::Settings>,
    ) -> Self {
        App {
            drives: Vec::new(),
            selected: None,
            scan_rx,
            scanning: true,
            scan_error: None,
            pending_drives: None,
            list_all,
            rescan,
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
            show_about: false,
            theme_preference: egui::ThemePreference::System,
            log: vec![format!("Rustfus {}", env!("CARGO_PKG_VERSION"))],
            update: saved_policy.clone().unwrap_or_default(),
            update_rx: None,
            update_manual: false,
            update_note: String::new(),
            busy: None,
            hash_rx: None,
            progress: 0.0,
            status: String::new(),
            done: None,
            started: None,
            elapsed: Duration::ZERO,
        }
    }

    fn drive(&self) -> Option<&Drive> {
        self.selected.and_then(|i| self.drives.get(i))
    }

    pub(super) fn retry_scan(&mut self) {
        self.scanning = true;
        self.rescan.store(true, Ordering::Relaxed);
    }

    fn scan_ready(&self) -> bool {
        !self.scanning && self.scan_error.is_none()
    }

    fn set_drives(&mut self, d: Vec<Drive>) {
        let keep = self.drive().map(|d| d.path.clone());
        self.selected = keep
            .and_then(|p| d.iter().position(|x| x.path == p))
            .or((!d.is_empty()).then_some(0));
        self.done = None;
        self.drives = d;
        if !self.image_mode() && self.label.is_empty() {
            self.label = self.drive().map(|d| d.name.clone()).unwrap_or_default();
        }
    }

    fn select_image(&mut self, path: PathBuf) {
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

    fn boot_mode(&self) -> Boot {
        match (self.boot, &self.image) {
            (BootSel::Image, Some(p)) => Boot::Image(p.clone()),
            (BootSel::FreeDos, _) => Boot::FreeDos,
            _ => Boot::NonBootable,
        }
    }

    fn image_mode(&self) -> bool {
        self.boot == BootSel::Image && self.image.is_some()
    }

    fn iso_mode(&self) -> bool {
        self.image_mode() && self.report.is_iso && !self.dd
    }

    /// Rufus' START sequence: boot file downloads, WTG edition, Windows
    /// customization or ISOHybrid choice, then the data-loss warning.
    /// `stage` is how far the user already got.
    fn next_dialog(&mut self, stage: u8) -> Dialog {
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
        if stage < 4 && iso && r.windows {
            return Dialog::Wue;
        }
        if stage < 4 && iso && r.hybrid {
            self.dd = false;
            return Dialog::Hybrid { dd: false };
        }
        Dialog::Confirm
    }

    fn obsolete_c32(&self) -> &[String] {
        ops::obsolete_c32(&self.report, self.fs, self.scheme, self.target)
    }

    fn on_start(&mut self) {
        if !self.scan_ready() {
            return;
        }
        self.allow_downloads = false;
        self.replace_c32.clear();
        self.dialog = self.next_dialog(0);
    }

    fn spawn<F>(&mut self, ctx: &egui::Context, f: F)
    where
        F: FnOnce(&Sender<Msg>, &AtomicBool) -> Result<String, String> + Send + 'static,
    {
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = f(&tx, &c);
            tx.send(Msg::Done(r)).ok();
            ctx.request_repaint();
        });
        self.busy = Some((rx, cancel));
        self.started = Some(Instant::now());
        self.progress = 0.0;
    }

    fn start_job(&mut self, ctx: &egui::Context) {
        // A scan can fail while the confirmation dialog is open.
        if !self.scan_ready() {
            return;
        }
        let Some(drive) = self.drive().cloned() else {
            return;
        };
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
            wue: wue::Options {
                local_account: self
                    .wue_use_account
                    .then(|| self.wue_user.clone())
                    .filter(|u| !u.trim().is_empty()),
                wtg_no_internal: self.wue.wtg_no_internal && self.wtg,
                ..self.wue.clone()
            },
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
        self.spawn(ctx, move |tx, c| ops::run_job(&job, tx, c));
    }

    fn check_updates(&mut self, ctx: &egui::Context, manual: bool) {
        let (tx, rx) = channel();
        let include_betas = self.update.include_betas;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            tx.send(update::check(include_betas)).ok();
            ctx.request_repaint();
        });
        self.update_rx = Some(rx);
        self.update_manual = manual;
        self.update_note = rustfus_name(&t("MSG_243"));
        self.log.push(self.update_note.clone());
    }

    fn on_update_checked(&mut self, result: Result<Option<Release>, String>) {
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

    fn poll(&mut self) {
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
        if !matches!(self.dialog, Dialog::Hash(..)) {
            self.hash_rx = None;
        }
        if let Some(rx) = &self.hash_rx {
            while let Ok((p, h)) = rx.try_recv() {
                let done = h.is_some();
                self.dialog = Dialog::Hash(h, p);
                if done {
                    self.hash_rx = None;
                    break;
                }
            }
        }
        let Some((rx, _)) = &self.busy else { return };
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Progress(p, s) => (self.progress, self.status) = (p, s),
                Msg::Log(s) => self.log.push(s),
                Msg::Done(r) => {
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

    fn load_editions(&mut self) {
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

    fn compute_hashes(&mut self, ctx: &egui::Context) {
        let Some(path) = self.image.clone() else {
            return;
        };
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let never = AtomicBool::new(false);
            let progress = |p| {
                tx.send((p, None)).ok();
                ctx.request_repaint();
            };
            let h = image::hashes(&path, &progress, &never).unwrap_or_else(|e| {
                [
                    tf("MSG_043", &[&e.to_string()]),
                    String::new(),
                    String::new(),
                    String::new(),
                ]
            });
            tx.send((1.0, Some(h))).ok();
            ctx.request_repaint();
        });
        self.hash_rx = Some(rx);
        self.dialog = Dialog::Hash(None, 0.0);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        let ctx = ui.ctx().clone();
        sync_style(&ctx);
        if self.busy.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        if let Some(p) = ctx.input(|i| {
            i.raw
                .dropped_files
                .first()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
        }) && self.busy.is_none()
        {
            self.boot = BootSel::Image;
            self.select_image(p);
        }

        let mut root = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(ctx.content_rect())
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        let ui = &mut root;
        let surface = ctx.shadcn_theme().background;
        egui::Panel::bottom("statusbar")
            .frame(
                egui::Frame::NONE
                    .fill(surface)
                    .inner_margin(egui::Margin::symmetric(12, 4)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let status = match (self.busy.is_some(), self.done) {
                        (true, _) => self.status.clone(),
                        (false, _) if self.scanning => "Scanning for devices…".into(),
                        (false, _) if self.scan_error.is_some() => "Device scan failed".into(),
                        (false, Some(true)) => "DONE".into(),
                        (false, Some(false)) => t("MSG_212"),
                        (false, None) => tf(
                            if self.drives.len() == 1 {
                                "MSG_208"
                            } else {
                                "MSG_209"
                            },
                            &[&self.drives.len().to_string()],
                        ),
                    };
                    ui.label(status);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let t = self
                            .started
                            .map(|t| t.elapsed())
                            .unwrap_or(self.elapsed)
                            .as_secs();
                        ui.monospace(format!("{:02}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60));
                        ui.add(functora_egui::Separator::vertical());
                    });
                });
            });
        egui::Panel::bottom("actions")
            .frame(
                egui::Frame::NONE
                    .fill(surface)
                    .inner_margin(egui::Margin::symmetric(12, 10)),
            )
            .show(ui, |ui| self.footer(ui));
        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(surface)
                    .inner_margin(egui::Margin::symmetric(12, 12)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("main-form-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let width = ui.available_width();
                        ui.set_width(width);
                        ui.add_space(4.0);
                        ui.add_enabled_ui(self.busy.is_none(), |ui| {
                            self.drive_section(ui);
                            self.format_section(ui);
                        });
                        self.status_section(ui);
                        ui.add_space(4.0);
                    });
            });
        self.dialogs(&ctx);
    }
}

/// Rufus' strings with Rustfus' name, for the ones that name the program.
fn rustfus_name(s: &str) -> String {
    s.replace("Rufus", "Rustfus")
}

pub fn run() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([520.0, 700.0])
        .with_min_inner_size([470.0, 440.0])
        .with_resizable(true);
    if let Ok(icon) =
        eframe::icon_data::from_png_bytes(include_bytes!("../../../../res/rustfus.png"))
    {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Rustfus",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::action_label;
    use egui_kittest::{
        Harness,
        kittest::{By, NodeT, Queryable},
    };

    fn form_app() -> (App, Sender<ScanResult>) {
        let (tx, rx) = channel();
        let mut app = App::with_inputs(
            rx,
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
            Some(update::Settings::default()),
        );
        app.scanning = false;
        app.boot = BootSel::NonBootable;
        app.set_drives(vec![drive()]);
        (app, tx)
    }

    fn form_harness(app: App) -> Harness<'static, App> {
        form_harness_at(app, egui::vec2(470.0, 900.0))
    }

    fn form_harness_at(app: App, size: egui::Vec2) -> Harness<'static, App> {
        Harness::builder()
            .with_size(size)
            .with_step_dt(0.2)
            .with_max_steps(8)
            .build_ui_state(
                |ui, app| {
                    ui.ctx().set_theme(app.theme_preference);
                    sync_style(ui.ctx());
                    app.poll();
                    let ctx = ui.ctx().clone();
                    let viewport = ctx.content_rect();
                    // The kittest UI wrapper has an 8-point margin; the native root does not.
                    let mut root = ui.new_child(egui::UiBuilder::new().max_rect(viewport));
                    let surface = ctx.shadcn_theme().background;
                    let status = egui::Panel::bottom("statusbar")
                        .frame(
                            egui::Frame::NONE
                                .fill(surface)
                                .inner_margin(egui::Margin::symmetric(12, 4)),
                        )
                        .show(&mut root, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(&app.status);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.monospace("00:00:00");
                                        ui.add(functora_egui::Separator::vertical());
                                    },
                                );
                            });
                        });
                    let actions = egui::Panel::bottom("actions")
                        .frame(
                            egui::Frame::NONE
                                .fill(surface)
                                .inner_margin(egui::Margin::symmetric(12, 10)),
                        )
                        .show(&mut root, |ui| app.footer(ui));
                    let central = egui::CentralPanel::default()
                        .frame(
                            egui::Frame::NONE
                                .fill(surface)
                                .inner_margin(egui::Margin::symmetric(12, 12)),
                        )
                        .show(&mut root, |ui| {
                            let form = egui::ScrollArea::vertical()
                                .id_salt("main-form-scroll")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.add_space(4.0);
                                    ui.add_enabled_ui(app.busy.is_none(), |ui| {
                                        app.drive_section(ui);
                                        app.format_section(ui);
                                    });
                                    app.status_section(ui);
                                    ui.add_space(4.0);
                                });
                            ctx.data_mut(|data| {
                                data.insert_temp(
                                    egui::Id::new("test-form-layout"),
                                    (form.inner_rect, form.content_size),
                                );
                            });
                        });
                    ctx.data_mut(|data| {
                        data.insert_temp(
                            egui::Id::new("test-panel-rects"),
                            [
                                status.response.rect,
                                actions.response.rect,
                                central.response.rect,
                            ],
                        );
                    });
                    app.dialogs(&ctx);
                },
                app,
            )
    }

    fn assert_form_fits(harness: &Harness<'static, App>) {
        let (rect, content) = harness.ctx.data(|data| {
            data.get_temp::<(egui::Rect, egui::Vec2)>(egui::Id::new("test-form-layout"))
                .unwrap()
        });
        assert!(harness.ctx.content_rect().contains_rect(rect));
        assert!(
            content.x <= rect.width() + 1.0,
            "horizontal form overflow: content {content:?}, viewport {rect:?}"
        );
    }

    fn action_text(key: &str) -> String {
        let text = t(key);
        if text.chars().any(char::is_uppercase) && !text.chars().any(char::is_lowercase) {
            let mut chars = text.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_string() + &chars.as_str().to_lowercase()
            })
        } else {
            text
        }
    }

    #[test]
    fn whole_form_fits_light_and_dark_with_advanced_options() {
        for dark in [false, true] {
            for advanced in [false, true] {
                let (mut app, _scan_tx) = form_app();
                app.theme_preference = if dark {
                    egui::ThemePreference::Dark
                } else {
                    egui::ThemePreference::Light
                };
                app.adv_drive = advanced;
                app.adv_format = advanced;
                let mut harness = form_harness(app);
                harness.run();
                assert_form_fits(&harness);
                for key in [
                    "IDS_DEVICE_TXT",
                    "IDS_BOOT_SELECTION_TXT",
                    "IDS_PARTITION_TYPE_TXT",
                    "IDS_TARGET_SYSTEM_TXT",
                    "IDS_LABEL_TXT",
                    "IDS_FILE_SYSTEM_TXT",
                    "IDS_CLUSTER_SIZE_TXT",
                ] {
                    let label = t(key);
                    let node = harness.get_by_label(&label);
                    assert!(
                        node.rect().left() >= 0.0 && node.rect().right() <= 470.0,
                        "{key} horizontal overflow"
                    );
                }
                if advanced {
                    for key in [
                        "IDC_LIST_USB_HDD",
                        "IDC_OLD_BIOS_FIXES",
                        "IDC_QUICK_FORMAT",
                        "IDC_EXTENDED_LABEL",
                        "IDC_BAD_BLOCKS",
                    ] {
                        let label = t(key);
                        harness.get_by_label(&label);
                    }
                    harness.get_by_label("Verify write (DD mode)");
                }
                assert!(
                    !harness
                        .get_by_label(&action_text("IDC_START"))
                        .accesskit_node()
                        .is_disabled()
                );
                assert!(harness.state().busy.is_none());
                assert!(harness.state().update_rx.is_none());
            }
        }
    }

    #[test]
    fn language_popover_opens_only_on_request_and_escape_preserves_language() {
        let (app, _scan_tx) = form_app();
        let mut harness = form_harness_at(app, egui::vec2(470.0, 700.0));
        let original = loc::current();
        let language = loc::languages().into_iter().next().unwrap();
        assert!(harness.query_by_label(&language.name).is_none());
        harness.get_by_label(&t("MSG_273")).click();
        harness.run();
        harness.run();
        harness.get_by_label(&language.name);
        assert_eq!(loc::current(), original);
        harness.key_press(egui::Key::Escape);
        harness.run();
        assert!(harness.query_by_label(&language.name).is_none());
        assert_eq!(loc::current(), original);
        assert!(harness.state().busy.is_none());
    }

    #[test]
    fn utility_menus_are_bounded_and_escape_does_not_start_work() {
        let original_language = loc::current();
        let languages = loc::languages();
        for width in [470.0, 520.0] {
            for preference in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
                for language_menu in [false, true] {
                    let (mut app, _scan_tx) = form_app();
                    app.theme_preference = preference;
                    let mut harness = form_harness_at(app, egui::vec2(width, 440.0));
                    harness.run_steps(3);
                    let (trigger, requested, choices) = if language_menu {
                        (
                            t("MSG_273"),
                            280.0,
                            languages
                                .iter()
                                .take(3)
                                .map(|l| l.name.clone())
                                .collect::<Vec<_>>(),
                        )
                    } else {
                        (
                            "Theme".into(),
                            160.0,
                            vec!["System".into(), "Light".into(), "Dark".into()],
                        )
                    };
                    for choice in &choices {
                        assert!(harness.query_by_label(choice).is_none());
                    }
                    let node = harness.get_by_label(&trigger);
                    let accesskit_id = u128::from(node.accesskit_node().id());
                    let id =
                        unsafe { egui::Id::from_high_entropy_bits((accesskit_id >> 64) as u64) };
                    node.click();
                    harness.run_steps(3);
                    let bounds = harness
                        .ctx
                        .data(|d| d.get_temp::<egui::Rect>(id.with("menu-bounds")))
                        .unwrap();
                    assert!(bounds.width() <= requested + 14.0, "{trigger}: {bounds:?}");
                    assert!(
                        harness.ctx.content_rect().contains_rect(bounds),
                        "{trigger}: {bounds:?}"
                    );
                    if language_menu {
                        assert!(bounds.height() <= 294.0, "{bounds:?}");
                    }
                    for choice in &choices {
                        let row = harness.get_by_label(choice).rect();
                        assert!((row.height() - 28.0).abs() <= 0.5, "{choice}: {row:?}");
                        assert!(bounds.contains_rect(row));
                    }
                    harness.key_press(egui::Key::Escape);
                    harness.run_steps(3);
                    for choice in &choices {
                        assert!(harness.query_by_label(choice).is_none());
                    }
                    assert_eq!(harness.state().theme_preference, preference);
                    assert_eq!(loc::current(), original_language);
                    assert!(harness.state().busy.is_none());
                    assert!(harness.state().update_rx.is_none());
                    assert!(harness.state().hash_rx.is_none());
                }
            }
        }
    }

    fn rendered_label_count(shape: &egui::epaint::Shape, label: &str) -> usize {
        match shape {
            egui::epaint::Shape::Text(text) => usize::from(text.galley.text() == label),
            egui::epaint::Shape::Vec(shapes) => {
                shapes.iter().map(|s| rendered_label_count(s, label)).sum()
            }
            _ => 0,
        }
    }

    #[test]
    fn footer_icon_tooltips_render_each_label_once_after_hover() {
        for preference in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
            for key in ["MSG_273", "MSG_302", "MSG_303"] {
                let (mut app, _scan_tx) = form_app();
                app.theme_preference = preference;
                let mut harness = form_harness_at(app, egui::vec2(470.0, 440.0));
                harness.run_steps(3);
                let label = action_label(&t(key));
                assert_eq!(
                    harness
                        .output()
                        .shapes
                        .iter()
                        .map(|s| rendered_label_count(&s.shape, &label))
                        .sum::<usize>(),
                    0
                );
                harness.get_by_label(&label).hover();
                harness.run_steps(10);
                assert_eq!(
                    harness
                        .output()
                        .shapes
                        .iter()
                        .map(|s| rendered_label_count(&s.shape, &label))
                        .sum::<usize>(),
                    1,
                    "{label}"
                );
                assert!(harness.state().busy.is_none());
                assert!(harness.state().update_rx.is_none());
            }
        }
    }

    #[test]
    fn boot_image_controls_fit_the_padded_form_without_opening_files() {
        for width in [470.0, 520.0] {
            for preference in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
                let (mut app, _scan_tx) = form_app();
                app.boot = BootSel::Image;
                app.theme_preference = preference;
                let mut harness = form_harness_at(app, egui::vec2(width, 440.0));
                harness.run_steps(3);
                assert_form_fits(&harness);
                let padded = harness.ctx.content_rect().shrink2(egui::vec2(12.0, 12.0));
                for label in [
                    t("IDS_BOOT_SELECTION_TXT"),
                    t("MSG_314"),
                    action_text("IDC_SELECT"),
                    "Image source".into(),
                ] {
                    let rect = harness.get_by_label(&label).rect();
                    assert!(
                        padded.contains_rect(rect),
                        "{label}: {rect:?} outside {padded:?}"
                    );
                }
                assert!(harness.state().image.is_none());
                assert!(harness.state().hash_rx.is_none());
                assert!(harness.state().busy.is_none());
                assert!(harness.state().update_rx.is_none());
            }
        }
    }

    #[test]
    fn footer_icons_and_actions_remain_inline_at_supported_widths() {
        for width in [470.0, 520.0] {
            for busy in [false, true] {
                let (mut app, _scan_tx) = form_app();
                let (job_tx, job_rx) = channel();
                let cancel = Arc::new(AtomicBool::new(false));
                if busy {
                    app.busy = Some((job_rx, cancel.clone()));
                }
                let mut harness = form_harness_at(app, egui::vec2(width, 440.0));
                harness.run();
                let action = action_text(if busy { "MSG_007" } else { "IDC_START" });
                let action_rect = harness.get_by_label(&action).rect();
                let close_label = action_text("IDCANCEL");
                let close = harness.get_by_label(&close_label);
                assert_eq!(close.accesskit_node().is_disabled(), busy);
                let viewport = harness.ctx.content_rect();
                for label in [
                    t("MSG_273"),
                    t("MSG_302"),
                    "Theme".into(),
                    "Check for updates".into(),
                    t("MSG_303"),
                    action.clone(),
                    action_text("IDCANCEL"),
                ] {
                    let rect = harness.get_by_label(&label).rect();
                    assert!(
                        viewport.contains_rect(rect),
                        "{label}: {rect:?} outside {viewport:?}, busy={busy}"
                    );
                    assert!(
                        (rect.center().y - action_rect.center().y).abs() <= 1.0,
                        "{label}: {rect:?} is not inline with {action_rect:?}"
                    );
                }
                if busy {
                    harness.get_by_label(&action).click();
                    harness.run();
                    assert!(cancel.load(Ordering::Relaxed));
                    assert!(harness.state().busy.is_some());
                    assert!(harness.state().started.is_none());
                }
                drop(job_tx);
            }
        }
    }

    fn painted_rects(shape: &egui::epaint::Shape, rects: &mut Vec<(egui::Rect, egui::Color32)>) {
        match shape {
            egui::epaint::Shape::Rect(rect) => rects.push((rect.rect, rect.fill)),
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    painted_rects(shape, rects);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn proportional_fonts_prefer_inter() {
        let mut harness = Harness::new_ui(|ui| sync_style(ui.ctx()));
        harness.run();
        harness.ctx.fonts(|fonts| {
            assert_eq!(
                fonts.definitions().families[&egui::FontFamily::Proportional]
                    .first()
                    .map(String::as_str),
                Some("Inter Variable")
            );
            assert!(fonts.definitions().font_data.contains_key("Inter Variable"));
        });
    }

    #[test]
    fn root_and_footer_panels_have_opaque_matching_theme_surfaces() {
        for preference in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
            let (mut app, _scan_tx) = form_app();
            app.theme_preference = preference;
            let mut harness = form_harness_at(app, egui::vec2(520.0, 700.0));
            harness.run();
            let surface = harness.ctx.shadcn_theme().background;
            assert_eq!(surface.a(), 255);
            if preference == egui::ThemePreference::Light {
                assert_eq!(surface, egui::Color32::WHITE);
            }
            assert_eq!(harness.ctx.global_style().visuals.panel_fill, surface);
            let panels = harness.ctx.data(|data| {
                data.get_temp::<[egui::Rect; 3]>(egui::Id::new("test-panel-rects"))
                    .unwrap()
            });
            let mut rects = Vec::new();
            for shape in &harness.output().shapes {
                painted_rects(&shape.shape, &mut rects);
            }
            for panel in panels {
                assert!(
                    rects.iter().any(|(rect, fill)| {
                        rect.contains_rect(panel.shrink(1.0)) && *fill == surface
                    }),
                    "panel {panel:?} has no opaque theme fill {surface:?}; painted: {rects:?}"
                );
            }
        }
    }

    #[test]
    fn scanning_and_scan_error_disable_start_without_starting_a_job() {
        for scanning in [true, false] {
            let (mut app, _scan_tx) = form_app();
            app.scanning = scanning;
            app.scan_error = (!scanning).then(|| "Synthetic scan failure".into());
            let mut harness = form_harness(app);
            harness.run();
            let start = action_text("IDC_START");
            assert!(harness.get_by_label(&start).accesskit_node().is_disabled());
            harness.get_by_label(&start).click();
            harness.run();
            harness.state_mut().on_start();
            assert!(matches!(harness.state().dialog, Dialog::None));
            assert!(harness.state().busy.is_none());
            assert!(harness.state().started.is_none());
            assert!(!harness.state().rescan.load(Ordering::Relaxed));
        }
    }

    #[test]
    fn retry_scan_only_requests_the_synthetic_scanner() {
        let (mut app, scan_tx) = form_app();
        app.scan_error = Some("Synthetic scan failure".into());
        let mut harness = form_harness(app);
        harness.get_by_label("Retry scan").click();
        harness.run();
        assert!(harness.state().scanning);
        assert!(harness.state().rescan.load(Ordering::Relaxed));
        assert!(harness.state().busy.is_none());
        scan_tx.send(Ok(vec![drive()])).unwrap();
        harness.run();
        assert!(!harness.state().scanning);
        assert!(harness.state().scan_error.is_none());
        assert_eq!(harness.state().drives, vec![drive()]);
    }

    #[test]
    fn escape_cancels_confirmation_even_when_approval_has_focus() {
        let (app, _scan_tx) = form_app();
        let mut harness = form_harness(app);
        harness.get_by_label(&action_text("IDC_START")).click();
        harness.run();
        assert!(matches!(harness.state().dialog, Dialog::Confirm));
        let approve = action_text("IDOK");
        harness
            .get(
                By::new()
                    .role(egui::accesskit::Role::Button)
                    .label(&approve),
            )
            .focus();
        harness.run();
        assert!(harness.get_by_label(&approve).is_focused());
        // Never activate approval: this fixture deliberately has a selected drive.
        harness.key_press(egui::Key::Escape);
        harness.run();
        assert!(matches!(harness.state().dialog, Dialog::None));
        assert!(harness.state().busy.is_none());
        assert!(harness.state().started.is_none());
    }

    #[test]
    fn late_checksum_messages_do_not_reopen_a_dismissed_dialog() {
        for completed in [false, true] {
            let (mut app, _scan_tx) = form_app();
            let (tx, rx) = channel();
            app.hash_rx = Some(rx);
            app.dialog = Dialog::Hash(None, 0.0);
            tx.send((0.25, None)).unwrap();
            app.poll();
            assert!(matches!(app.dialog, Dialog::Hash(None, p) if p == 0.25));
            app.dialog = Dialog::None;
            let hashes = completed.then(|| std::array::from_fn(|i| format!("checksum-{i}")));
            tx.send((0.75, hashes)).unwrap();
            app.poll();
            assert!(matches!(app.dialog, Dialog::None));
            assert!(app.hash_rx.is_none());
            assert!(tx.send((1.0, None)).is_err());
            app.poll();
            assert!(matches!(app.dialog, Dialog::None));
            assert!(app.busy.is_none());
        }
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
    fn first_empty_scan_is_a_success_not_an_error() {
        let mut scanning = true;
        let mut error = None;
        let drives = finish_scan(&mut scanning, &mut error, Ok(vec![]));
        assert_eq!(drives, Some(vec![]));
        assert!(!scanning);
        assert_eq!(error, None);
    }

    #[test]
    fn first_failed_scan_finishes_scanning_and_exposes_error() {
        let mut scanning = true;
        let mut error = None;
        assert_eq!(
            finish_scan(&mut scanning, &mut error, Err("Permission denied".into())),
            None
        );
        assert!(!scanning);
        assert_eq!(error.as_deref(), Some("Permission denied"));
    }

    #[test]
    fn transient_failure_preserves_drives_until_successful_empty_scan() {
        let mut scanning = true;
        let mut error = None;
        let mut drives = finish_scan(&mut scanning, &mut error, Ok(vec![drive()])).unwrap();
        let result = finish_scan(&mut scanning, &mut error, Err("Scan failed".into()));
        if let Some(updated) = result {
            drives = updated;
        }
        assert_eq!(drives, vec![drive()]);
        assert!(error.is_some());

        // Recovery must be delivered even when it matches the last successful list.
        let recovered = finish_scan(&mut scanning, &mut error, Ok(drives.clone()));
        assert_eq!(recovered, Some(vec![drive()]));
        assert_eq!(error, None);

        drives = finish_scan(&mut scanning, &mut error, Ok(vec![])).unwrap();
        assert!(drives.is_empty());
        assert!(!scanning);
        assert_eq!(error, None);
    }
}
