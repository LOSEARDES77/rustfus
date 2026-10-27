// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Application state and logic; the UI lives in `sections` and `dialogs`.

mod dialogs;
mod sections;

use crate::widgets::{add_script_fonts, rufus_style};
use eframe::egui::{self};
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

#[derive(Clone, Copy, PartialEq)]
enum BootSel {
    Image,
    FreeDos,
    NonBootable,
}

enum Dialog {
    None,
    Download,
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
    UpdatePolicy,
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
    scan_rx: Receiver<Vec<Drive>>,
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
    show_settings: bool,
    dark: bool,
    log: Vec<String>,

    update: update::Settings,
    update_rx: Option<Receiver<Result<Option<Release>, String>>>,
    /// Started with "Check Now": a newer release always gets its dialog.
    update_manual: bool,
    /// Outcome of the last check, shown in the settings window.
    update_note: String,

    busy: Option<(Receiver<Msg>, Arc<AtomicBool>)>,
    hash_rx: Option<Receiver<(f32, Option<[String; 4]>)>>,
    progress: f32,
    status: String,
    /// Outcome of the last job, shown until the next device scan.
    done: Option<bool>,
    started: Option<Instant>,
    elapsed: Duration,
    height: f32,
}

/// Background drive scan every 2 s so plugging a stick updates the list.
fn spawn_scanner(
    ctx: egui::Context,
    list_all: Arc<AtomicBool>,
    rescan: Arc<AtomicBool>,
) -> Receiver<Vec<Drive>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut last = None;
        loop {
            if let Ok(d) = drives::list(list_all.load(Ordering::Relaxed))
                && last.as_ref() != Some(&d)
            {
                last = Some(d.clone());
                if tx.send(d).is_err() {
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
        rufus_style(&cc.egui_ctx, false);
        add_script_fonts(&cc.egui_ctx);
        loc::set_language(&loc::default_for_system());
        let list_all = Arc::new(AtomicBool::new(false));
        let rescan = Arc::new(AtomicBool::new(false));
        let saved_policy = update::Settings::load();
        let mut app = App {
            drives: Vec::new(),
            selected: None,
            scan_rx: spawn_scanner(cc.egui_ctx.clone(), list_all.clone(), rescan.clone()),
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
            dialog: if saved_policy.is_some() {
                Dialog::None
            } else {
                Dialog::UpdatePolicy
            },
            show_log: false,
            show_about: false,
            show_settings: false,
            dark: false,
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
            height: 0.0,
        };
        if saved_policy.is_some_and(|s| s.is_due(update::now())) {
            app.check_updates(&cc.egui_ctx, /* manual= */ false);
        }
        app
    }

    fn drive(&self) -> Option<&Drive> {
        self.selected.and_then(|i| self.drives.get(i))
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
        while let Ok(d) = self.scan_rx.try_recv() {
            if self.busy.is_none() {
                self.set_drives(d);
            }
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
                    self.rescan.store(true, Ordering::Relaxed);
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

        egui::Panel::bottom("statusbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                let status = match (self.busy.is_some(), self.done) {
                    (true, _) => self.status.clone(),
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
                    ui.separator();
                });
            });
        });
        let bottom = egui::CentralPanel::default_margins()
            .show(ui, |ui| {
                ui.add_enabled_ui(self.busy.is_none(), |ui| {
                    self.drive_section(ui);
                    self.format_section(ui);
                });
                self.status_section(ui);
                ui.cursor().min.y
            })
            .inner;
        // Like Rufus, grow or shrink the window with the advanced sections.
        let want = bottom + 44.0;
        if (want - self.height).abs() > 1.0 {
            self.height = want;
            // A fixed-size window only resizes when its size limits move too.
            let size = egui::vec2(470.0, want);
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(size));
            ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(size));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        }
        self.dialogs(&ctx);
    }
}

/// Rufus' strings with Rustfus' name, for the ones that name the program.
fn rustfus_name(s: &str) -> String {
    s.replace("Rufus", "Rustfus")
}

pub fn run() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([470.0, 610.0])
        .with_resizable(false);
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
