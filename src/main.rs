// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
#![cfg_attr(windows, windows_subsystem = "windows")]

mod blockdev;
mod bootrec;
mod drives;
mod ext;
mod fat;
mod fido;
mod fsos;
mod image;
mod iso;
mod loc;
mod net;
mod ops;
mod part;
mod syslinux;
mod vhdx;
mod wtg;
mod wue;

use drives::Drive;
use loc::{t, tf};
use eframe::egui::{self, Color32, RichText};
use ops::{Boot, Fs, Job, Msg, Report, Scheme, Target};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

const RUFUS_GREEN: Color32 = Color32::from_rgb(0x06, 0xB0, 0x25);
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
    Edition,
    Wue,
    Hybrid { dd: bool },
    Confirm,
    Hash(Option<[String; 4]>, f32),
    Fido(FidoState),
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
            let r = if chosen.len() == fido::FIELDS.len() { fido::url(&chosen).map(|u| vec![u]) } else { fido::list(&chosen) };
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
fn spawn_scanner(ctx: egui::Context, list_all: Arc<AtomicBool>, rescan: Arc<AtomicBool>) -> Receiver<Vec<Drive>> {
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

/// Rufus section heading: bold title followed by a horizontal rule.
/// Rufus look: taller, bordered controls and roomier rows.
fn rufus_style(ctx: &egui::Context, dark: bool) {
    ctx.set_visuals(if dark { egui::Visuals::dark() } else { egui::Visuals::light() });
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing.y = 5.0;
        s.spacing.interact_size.y = 22.0;
        s.spacing.button_padding = egui::vec2(8.0, 4.0);
        let border = egui::Stroke::new(1.0, if dark { Color32::from_gray(90) } else { Color32::from_gray(170) });
        s.visuals.widgets.inactive.bg_stroke = border;
        s.visuals.widgets.noninteractive.bg_stroke = border;
    });
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(20.0).strong());
        let rect = ui.available_rect_before_wrap();
        let stroke = egui::Stroke::new(2.0, ui.visuals().strong_text_color());
        ui.painter().hline(rect.left() + 8.0..=rect.right(), rect.center().y + 2.0, stroke);
    });
}

fn toggle(ui: &mut egui::Ui, open: &mut bool, what: &str) {
    let text = if *open { format!("⏶  {}", tf("MSG_122", &[what])) } else { format!("⏷  {}", tf("MSG_121", &[what])) };
    if ui.add(egui::Label::new(text).sense(egui::Sense::click())).clicked() {
        *open = !*open;
    }
}

/// Rufus' bad blocks combo labels (MSG_034/035/087); index = `image::BAD_BLOCK_MODES` entry.
fn bad_block_mode(i: u8) -> String {
    match i {
        0 => tf("MSG_034", &["1"]),
        1 => tf("MSG_035", &["2", ""]).trim_end().into(),
        2..=4 => tf("MSG_035", &["4", &tf("MSG_087", &[["SLC", "MLC", "TLC"][i as usize - 2]])]),
        _ => image::BAD_BLOCK_MODES[i as usize].0.into(),
    }
}

fn cluster_name(c: u32) -> String {
    match c {
        0 => t("MSG_029"),
        c if c >= 1 << 20 => format!("{} {}", c >> 20, t("MSG_028")),
        c if c >= 1024 => format!("{} {}", c >> 10, t("MSG_027")),
        c => format!("{c} {}", t("MSG_026")),
    }
}

/// Full-width combo box over `(value, text)` items; true when changed.
fn combo<T: PartialEq + Copy>(ui: &mut egui::Ui, id: &str, value: &mut T, items: &[(T, String)]) -> bool {
    let before = *value;
    let text = items.iter().find(|(v, _)| v == value).map(|(_, s)| s.clone()).unwrap_or_default();
    egui::ComboBox::from_id_salt(id).truncate().width(ui.available_width()).selected_text(text).show_ui(ui, |ui| {
        for (v, s) in items {
            ui.selectable_value(value, *v, s);
        }
    });
    before != *value
}

fn ok_cancel(ui: &mut egui::Ui) -> Option<bool> {
    let mut r = None;
    ui.horizontal(|ui| {
        if ui.button(t("IDOK")).clicked() {
            r = Some(true);
        }
        if ui.button(t("MSG_007")).clicked() {
            r = Some(false);
        }
    });
    r
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        rufus_style(&cc.egui_ctx, false);
        loc::set_language(&loc::default_for_system());
        let list_all = Arc::new(AtomicBool::new(false));
        let rescan = Arc::new(AtomicBool::new(false));
        App {
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
            wue: wue::Options { bypass_hw_checks: true, no_online_account: true, no_data_collection: true, ..Default::default() },
            wue_user: std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default(),
            wue_use_account: false,
            dd: false,
            allow_downloads: false,
            wtg: false,
            editions: Vec::new(),
            edition: 0,
            download_mode: false,
            pending_image: None,
            dialog: Dialog::None,
            show_log: false,
            show_about: false,
            show_settings: false,
            dark: false,
            log: vec![format!("Rustfus {}", env!("CARGO_PKG_VERSION"))],
            busy: None,
            hash_rx: None,
            progress: 0.0,
            status: String::new(),
            done: None,
            started: None,
            elapsed: Duration::ZERO,
            height: 0.0,
        }
    }

    fn drive(&self) -> Option<&Drive> {
        self.selected.and_then(|i| self.drives.get(i))
    }

    fn set_drives(&mut self, d: Vec<Drive>) {
        let keep = self.drive().map(|d| d.path.clone());
        self.selected = keep.and_then(|p| d.iter().position(|x| x.path == p)).or((!d.is_empty()).then_some(0));
        self.done = None;
        self.drives = d;
        if !self.image_mode() && self.label.is_empty() {
            self.label = self.drive().map(|d| d.name.clone()).unwrap_or_default();
        }
    }

    fn select_image(&mut self, path: PathBuf) {
        match ops::analyze(&path) {
            Ok(r) => {
                self.log.push(format!("Scanned image {}: {r:?}", path.display()));
                self.dd = !r.is_iso;
                if r.is_iso {
                    self.label = r.label.clone();
                    self.fs = if r.large_file { Fs::Ntfs } else { Fs::Fat32 };
                    (self.scheme, self.target) = if r.efi && (r.windows || !r.bios) {
                        (Scheme::Gpt, Target::Uefi)
                    } else if r.efi {
                        (Scheme::Mbr, Target::BiosUefi)
                    } else {
                        (Scheme::Mbr, Target::Bios)
                    };
                }
                self.cluster = 0;
                self.persistence_mb = 0;
                self.uefi_validation = false;
                self.wtg = false;
                self.report = r;
                self.image = Some(path);
            }
            Err(err) => self.dialog = Dialog::Error(format!("{}: {err}", tf("MSG_322", &[&path.display().to_string()]))),
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

    /// Rufus' START sequence: boot file download, WTG edition, Windows
    /// customization or ISOHybrid choice, then the data-loss warning.
    /// `stage` is how far the user already got.
    fn next_dialog(&mut self, stage: u8) -> Dialog {
        let r = &self.report;
        let iso = self.image_mode() && r.is_iso;
        if stage < 1 && iso && ops::needs_grub_download(r, self.scheme, self.target) {
            return Dialog::Download;
        }
        if stage < 2 && iso && r.windows && self.wtg {
            return Dialog::Edition;
        }
        if stage < 3 && iso && r.windows {
            return Dialog::Wue;
        }
        if stage < 3 && iso && r.hybrid {
            self.dd = false;
            return Dialog::Hybrid { dd: false };
        }
        Dialog::Confirm
    }

    fn on_start(&mut self) {
        self.allow_downloads = false;
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
        let Some(drive) = self.drive().cloned() else { return };
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
            persistence: if self.iso_mode() && self.report.persistence_capable() { self.persistence_mb * MB } else { 0 },
            wue: wue::Options {
                local_account: self.wue_use_account.then(|| self.wue_user.clone()).filter(|u| !u.trim().is_empty()),
                wtg_no_internal: self.wue.wtg_no_internal && self.wtg,
                ..self.wue.clone()
            },
            allow_downloads: self.allow_downloads,
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

    fn poll(&mut self) {
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
        let Some(path) = self.image.clone() else { return };
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let never = AtomicBool::new(false);
            let progress = |p| {
                tx.send((p, None)).ok();
                ctx.request_repaint();
            };
            let h = image::hashes(&path, &progress, &never)
                .unwrap_or_else(|e| [tf("MSG_043", &[&e.to_string()]), String::new(), String::new(), String::new()]);
            tx.send((1.0, Some(h))).ok();
            ctx.request_repaint();
        });
        self.hash_rx = Some(rx);
        self.dialog = Dialog::Hash(None, 0.0);
    }

    fn drive_section(&mut self, ui: &mut egui::Ui) {
        section(ui, &t("IDS_DRIVE_PROPERTIES_TXT"));
        ui.label(t("IDS_DEVICE_TXT"));
        ui.horizontal(|ui| {
            let text = self.drive().map(|d| d.label()).unwrap_or_default();
            egui::ComboBox::from_id_salt("device").truncate().width(ui.available_width() - 28.0).selected_text(text).show_ui(ui, |ui| {
                for (i, d) in self.drives.iter().enumerate() {
                    ui.selectable_value(&mut self.selected, Some(i), d.label()).on_hover_text(&d.path);
                }
            });
            let save = ui.add_enabled(self.drive().is_some(), egui::Button::new("💾").frame(false));
            if save.on_hover_text(t("MSG_304")).clicked()
                && let Some(out) = rfd::FileDialog::new()
                    .add_filter(t("MSG_343"), &["vhd"])
                    .add_filter(t("MSG_095"), &["img"])
                    .set_file_name("drive.vhd")
                    .save_file()
                && let Some(d) = self.drive().cloned()
            {
                self.log.push(format!("Saving {} to {}", d.path, out.display()));
                let ctx = ui.ctx().clone();
                self.spawn(&ctx, move |tx, c| ops::save_drive(&d, &out, tx, c));
            }
        });

        ui.label(t("IDS_BOOT_SELECTION_TXT"));
        ui.horizontal(|ui| {
            let name = match (self.boot, &self.image) {
                (BootSel::Image, Some(p)) => p.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                (BootSel::Image, None) => tf("MSG_281", &[&t("MSG_280")]),
                (BootSel::FreeDos, _) => "FreeDOS".into(),
                (BootSel::NonBootable, _) => t("MSG_279"),
            };
            let before = self.boot;
            // Fixed width so long translated image names truncate instead of pushing SELECT off-screen.
            let w = ui.available_width() - 165.0;
            ui.allocate_ui(egui::vec2(w, ui.spacing().interact_size.y), |ui| {
                ui.set_max_width(w);
                egui::ComboBox::from_id_salt("boot").truncate().width(w).selected_text(name).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.boot, BootSel::Image, t("MSG_280"));
                    ui.selectable_value(&mut self.boot, BootSel::FreeDos, "FreeDOS");
                    ui.selectable_value(&mut self.boot, BootSel::NonBootable, t("MSG_279"));
                });
            });
            if before != self.boot {
                match self.boot {
                    BootSel::FreeDos => (self.fs, self.scheme, self.target) = (Fs::Fat32, Scheme::Mbr, Target::Bios),
                    BootSel::NonBootable => (self.scheme, self.target) = (Scheme::Mbr, Target::BiosUefi),
                    BootSel::Image => {}
                }
                self.dd = self.image_mode() && !self.report.is_iso;
                if !self.image_mode() {
                    self.label = self.drive().map(|d| d.name.clone()).unwrap_or_default();
                }
            }
            let check = RichText::new("✔").size(16.0).color(Color32::from_rgb(0x1E, 0x7F, 0xD8));
            let hash = ui.add_enabled(self.image_mode(), egui::Button::new(check).frame(false));
            if hash.on_hover_text(t("MSG_314")).clicked() {
                self.compute_hashes(ui.ctx());
            }
            // Rufus' split button: SELECT, or DOWNLOAD (Fido) from the arrow menu.
            ui.add_enabled_ui(self.boot == BootSel::Image, |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let (select, download) = (t("IDC_SELECT").to_uppercase(), t("MSG_040").to_uppercase());
                let text = if self.download_mode { &download } else { &select };
                let w = (ui.available_width() - 22.0).max(40.0);
                let main = ui.add_sized([w, ui.spacing().interact_size.y], egui::Button::new(text).truncate());
                ui.menu_button("⏷", |ui| {
                    if ui.button(&select).clicked() {
                        self.download_mode = false;
                    }
                    if ui.button(&download).clicked() {
                        self.download_mode = true;
                    }
                });
                if main.clicked() {
                    if self.download_mode {
                        let mut st = FidoState { chosen: Vec::new(), options: Vec::new(), pick: 0, rx: None, error: None };
                        st.query(ui.ctx());
                        self.dialog = Dialog::Fido(st);
                    } else if let Some(p) = rfd::FileDialog::new()
                        .add_filter("Disk images", // no Rufus string
                         &["iso", "img", "bin", "raw", "dd", "vhd", "vhdx", "ffu", "gz", "bz2", "xz", "lzma", "zst", "zip"])
                        .add_filter(t("MSG_107"), &["*"])
                        .pick_file()
                    {
                        self.select_image(p);
                    }
                }
            });
        });

        if self.iso_mode() && self.report.windows {
            ui.label(t("IDS_IMAGE_OPTION_TXT"));
            let before = self.wtg;
            egui::ComboBox::from_id_salt("imgopt").truncate()
                .width(ui.available_width() / 2.0 - 4.0)
                .selected_text(t(if self.wtg { "MSG_118" } else { "MSG_117" }))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.wtg, false, t("MSG_117"));
                    ui.add_enabled_ui(cfg!(windows), |ui| ui.selectable_value(&mut self.wtg, true, t("MSG_118")))
                        .response
                        .on_disabled_hover_text("Windows To Go needs DISM and bcdboot, which only exist on Windows");
                });
            if self.wtg && !before {
                self.fs = Fs::Ntfs;
                self.load_editions();
            }
        }
        if self.iso_mode() && self.report.persistence_capable() {
            ui.label(t("MSG_123"));
            let max = self.drive().map(|d| d.size.saturating_sub(self.report.size + 64 * MB) / MB).unwrap_or(0);
            ui.spacing_mut().slider_width = ui.available_width() - 90.0;
            ui.add(egui::Slider::new(&mut self.persistence_mb, 0..=max).suffix(" MB"));
            if self.persistence_mb > 0 && self.persistence_mb < 256 {
                self.persistence_mb = 256;
            }
        }

        let formatting = !self.dd;
        ui.add_enabled_ui(formatting, |ui| {
            ui.columns(2, |c| {
                c[0].label(t("IDS_PARTITION_TYPE_TXT"));
                let schemes = [(Scheme::Mbr, "MBR".to_string()), (Scheme::Gpt, "GPT".to_string())];
                if combo(&mut c[0], "scheme", &mut self.scheme, &schemes) {
                    self.target = if self.scheme == Scheme::Gpt { Target::Uefi } else { Target::BiosUefi };
                }
                c[1].label(t("IDS_TARGET_SYSTEM_TXT"));
                let targets: Vec<(Target, String)> = match self.scheme {
                    Scheme::Gpt => vec![(Target::Uefi, Target::Uefi.name())],
                    Scheme::Mbr => [Target::BiosUefi, Target::Bios, Target::Uefi].iter().map(|t| (*t, t.name())).collect(),
                };
                combo(&mut c[1], "target", &mut self.target, &targets);
            });
        });

        toggle(ui, &mut self.adv_drive, &t("MSG_119"));
        if self.adv_drive {
            let mut all = self.list_all.load(Ordering::Relaxed);
            if ui.checkbox(&mut all, t("IDC_LIST_USB_HDD")).changed() {
                self.list_all.store(all, Ordering::Relaxed);
                self.rescan.store(true, Ordering::Relaxed);
            }
            ui.add_enabled(formatting, egui::Checkbox::new(&mut self.old_bios, t("IDC_OLD_BIOS_FIXES")));
            let can_validate = self.iso_mode() && self.report.efi;
            ui.add_enabled(can_validate, egui::Checkbox::new(&mut self.uefi_validation, t("IDC_UEFI_MEDIA_VALIDATION")));
            ui.add_enabled(self.dd, egui::Checkbox::new(&mut self.verify, "Verify write (DD mode)"));
        }
    }

    fn format_section(&mut self, ui: &mut egui::Ui) {
        section(ui, &t("IDS_FORMAT_OPTIONS_TXT"));
        let formatting = !self.dd;
        ui.add_enabled_ui(formatting, |ui| {
            ui.label(t("IDS_LABEL_TXT"));
            ui.add(egui::TextEdit::singleline(&mut self.label).desired_width(f32::INFINITY));
            let big = self.drive().is_some_and(|d| d.size > 4 << 30);
            let default_fs = if self.report.large_file && self.image_mode() { Fs::Ntfs } else { Fs::Fat32 };
            let fss: Vec<(Fs, String)> = Fs::ALL
                .iter()
                .filter(|f| f.supported() && !(**f == Fs::Fat && big))
                .filter(|f| self.boot != BootSel::FreeDos || f.is_fat())
                .map(|f| (*f, if *f == default_fs { tf("MSG_030", &[f.name()]) } else { f.name().into() }))
                .collect();
            ui.columns(2, |c| {
                c[0].label(t("IDS_FILE_SYSTEM_TXT"));
                if combo(&mut c[0], "fs", &mut self.fs, &fss) {
                    self.cluster = 0;
                }
                c[1].label(t("IDS_CLUSTER_SIZE_TXT"));
                let cl: Vec<(u32, String)> = self.fs.clusters().into_iter().map(|v| (v, cluster_name(v))).collect();
                combo(&mut c[1], "cluster", &mut self.cluster, &cl);
            });
        });
        toggle(ui, &mut self.adv_format, &t("MSG_120"));
        if self.adv_format {
            ui.add_enabled(formatting, egui::Checkbox::new(&mut self.quick, t("IDC_QUICK_FORMAT")));
            ui.add_enabled(formatting, egui::Checkbox::new(&mut self.ext_label, t("IDC_EXTENDED_LABEL")));
            ui.columns(2, |c| {
                c[0].checkbox(&mut self.bad_blocks, t("IDC_BAD_BLOCKS"));
                c[1].add_enabled_ui(self.bad_blocks, |ui| {
                    let passes: Vec<(u8, String)> = (0..image::BAD_BLOCK_MODES.len() as u8).map(|i| (i, bad_block_mode(i))).collect();
                    combo(ui, "passes", &mut self.passes, &passes);
                });
            });
        }
    }

    fn status_section(&mut self, ui: &mut egui::Ui) {
        section(ui, &t("IDS_STATUS_TXT"));
        ui.add_space(4.0);
        let bar = if self.busy.is_some() {
            egui::ProgressBar::new(self.progress).text(RichText::new(&self.status).strong())
        } else {
            egui::ProgressBar::new(1.0).fill(RUFUS_GREEN).text(RichText::new(t("MSG_210")).color(Color32::WHITE).strong())
        };
        ui.add(bar.desired_height(22.0).corner_radius(0.0));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let icon = |ui: &mut egui::Ui, s: &str, tip: &str| {
                ui.add(egui::Button::new(RichText::new(s).size(18.0)).frame(false)).on_hover_text(tip).clicked()
            };
            let lang = ui.menu_button(RichText::new("🌐").size(18.0), |ui| {
                let cur = loc::current();
                for l in loc::languages() {
                    if ui.radio(l.code == cur, &l.name).clicked() {
                        loc::set_language(&l.code);
                        ui.close();
                    }
                }
            });
            lang.response.on_hover_text(t("MSG_273"));
            if icon(ui, "ℹ", &t("MSG_302")) {
                self.show_about = true;
            }
            if icon(ui, "⚙", &t("MSG_301")) {
                self.show_settings = true;
            }
            if icon(ui, "🗐", &t("MSG_303")) {
                self.show_log = !self.show_log;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let size = egui::vec2(104.0, 26.0);
                if let Some((_, cancel)) = &self.busy {
                    if ui.add(egui::Button::new(t("MSG_007").to_uppercase()).truncate().min_size(size)).clicked() {
                        cancel.store(true, Ordering::Relaxed);
                    }
                } else if ui.add(egui::Button::new(t("IDCANCEL").to_uppercase()).truncate().min_size(size)).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                let ready = self.busy.is_none() && self.drive().is_some() && (self.boot != BootSel::Image || self.image.is_some());
                if ui.add_enabled(ready, egui::Button::new(t("IDC_START").to_uppercase()).truncate().min_size(size)).on_hover_text(t("MSG_171")).clicked() {
                    self.on_start();
                }
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let modal = |id: &str, f: &mut dyn FnMut(&mut egui::Ui)| {
            egui::Modal::new(egui::Id::new(id)).show(ctx, |ui| {
                ui.set_max_width(440.0);
                f(ui)
            });
        };
        let mut next = None;
        let mut download: Option<(String, PathBuf)> = None;
        let mut stage = None;
        let mut start = false;
        match &mut self.dialog {
            Dialog::None => {}
            Dialog::Download => {
                let (v, allow) = (self.report.grub2_version.clone(), &mut self.allow_downloads);
                modal("download", &mut |ui| {
                    ui.heading(t("MSG_115"));
                    ui.label(tf("MSG_116", &[&v, "2.14"]));
                    ui.horizontal(|ui| {
                        if ui.button(t("MSG_008")).clicked() {
                            *allow = true;
                            stage = Some(1);
                        }
                        if ui.button(t("MSG_009")).clicked() {
                            stage = Some(1);
                        }
                        if ui.button(t("MSG_007")).clicked() {
                            next = Some(Dialog::None);
                        }
                    });
                });
            }
            Dialog::Edition => {
                let (eds, sel) = (&self.editions, &mut self.edition);
                modal("edition", &mut |ui| {
                    ui.heading(t("MSG_118"));
                    ui.label("Please select the edition you want to install:");
                    for (i, name) in eds {
                        ui.radio_value(sel, *i, name);
                    }
                    if let Some(ok) = ok_cancel(ui) {
                        if ok { stage = Some(2) } else { next = Some(Dialog::None) }
                    }
                });
            }
            Dialog::Wue => {
                let (o, user, use_acct, wtg) = (&mut self.wue, &mut self.wue_user, &mut self.wue_use_account, self.wtg);
                modal("wue", &mut |ui| {
                    ui.heading(t("MSG_327"));
                    ui.label(t("MSG_328"));
                    ui.checkbox(&mut o.bypass_hw_checks, t("MSG_329"));
                    ui.checkbox(&mut o.no_online_account, t("MSG_330"));
                    ui.horizontal(|ui| {
                        ui.checkbox(use_acct, t("MSG_333"));
                        ui.add_enabled(*use_acct, egui::TextEdit::singleline(user).desired_width(110.0));
                    });
                    ui.checkbox(&mut o.regional, t("MSG_334"));
                    ui.checkbox(&mut o.no_data_collection, t("MSG_331"));
                    ui.checkbox(&mut o.no_bitlocker, t("MSG_335"));
                    if wtg {
                        ui.checkbox(&mut o.wtg_no_internal, t("MSG_332"));
                    }
                    if let Some(ok) = ok_cancel(ui) {
                        next = Some(if ok { Dialog::Confirm } else { Dialog::None });
                    }
                });
            }
            Dialog::Hybrid { dd } => {
                let mut chosen = None;
                modal("hybrid", &mut |ui| {
                    ui.heading(tf("MSG_274", &["ISOHybrid"]));
                    let (iso, dd_s) = (t("MSG_036"), t("MSG_095"));
                    ui.label(tf("MSG_275", &[&iso, &dd_s, &iso, &dd_s]));
                    ui.radio_value(dd, false, tf("MSG_276", &[&iso]));
                    ui.radio_value(dd, true, tf("MSG_277", &[&dd_s]));
                    if let Some(ok) = ok_cancel(ui) {
                        chosen = Some(ok.then_some(*dd));
                    }
                });
                match chosen {
                    Some(Some(dd)) => {
                        self.dd = dd;
                        next = Some(Dialog::Confirm);
                    }
                    Some(None) => next = Some(Dialog::None),
                    None => {}
                }
            }
            Dialog::Confirm => {
                let name = self.selected.and_then(|i| self.drives.get(i)).map(|d| d.label()).unwrap_or_default();
                modal("confirm", &mut |ui| {
                    ui.heading("⚠ WARNING");
                    ui.label(tf("MSG_003", &[&name]));
                    if let Some(ok) = ok_cancel(ui) {
                        start = ok;
                        next = Some(Dialog::None);
                    }
                });
            }
            Dialog::Hash(h, p) => {
                let (h, p) = (h.clone(), *p);
                modal("hash", &mut |ui| {
                    ui.heading("Checksums");
                    match &h {
                        None => {
                            ui.add(egui::ProgressBar::new(p).show_percentage());
                        }
                        Some(h) => {
                            egui::Grid::new("hashes").num_columns(2).show(ui, |ui| {
                                for (name, v) in ["MD5", "SHA1", "SHA256", "SHA512"].iter().zip(h) {
                                    ui.label(*name);
                                    let mut v = v.as_str();
                                    ui.add(egui::TextEdit::singleline(&mut v).desired_width(340.0).font(egui::TextStyle::Monospace));
                                    ui.end_row();
                                }
                            });
                        }
                    }
                    if ui.add_enabled(h.is_some(), egui::Button::new(t("IDOK"))).clicked() {
                        next = Some(Dialog::None);
                    }
                });
            }
            Dialog::Fido(st) => {
                if let Some(rx) = &st.rx
                    && let Ok(r) = rx.try_recv()
                {
                    st.rx = None;
                    match r {
                        Ok(v) if st.chosen.len() == fido::FIELDS.len() => {
                            let url = v[0].clone();
                            let name = url.rsplit('/').next().and_then(|n| n.split('?').next()).unwrap_or("windows.iso").to_string();
                            if let Some(path) = rfd::FileDialog::new().set_file_name(&name).add_filter(t("MSG_036"), &["iso"]).save_file() {
                                download = Some((url, path));
                            }
                            next = Some(Dialog::None);
                        }
                        Ok(v) => {
                            st.options = v;
                            st.pick = 0;
                        }
                        Err(err) => {
                            st.chosen.pop();
                            st.error = Some(err);
                        }
                    }
                }
                let mut query = false;
                modal("fido", &mut |ui| {
                    ui.heading(t("MSG_149"));
                    for (f, v) in fido::FIELDS.iter().zip(&st.chosen) {
                        ui.label(format!("{f}: {v}"));
                    }
                    if let Some(err) = &st.error {
                        ui.colored_label(Color32::RED, err);
                    }
                    if st.rx.is_some() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(t("MSG_142"));
                        });
                    } else if st.chosen.len() < fido::FIELDS.len() && !st.options.is_empty() {
                        ui.label(fido::FIELDS[st.chosen.len()]);
                        let opts = &st.options;
                        egui::ComboBox::from_id_salt("fido_opt").truncate().width(300.0).selected_text(opts[st.pick.min(opts.len() - 1)].clone()).show_ui(ui, |ui| {
                            for (i, o) in opts.iter().enumerate() {
                                ui.selectable_value(&mut st.pick, i, o);
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        let last = st.chosen.len() + 1 == fido::FIELDS.len();
                        let ready = st.rx.is_none() && !st.options.is_empty();
                        if ui.add_enabled(ready, egui::Button::new(t(if last { "MSG_040" } else { "MSG_140" }))).clicked() {
                            st.chosen.push(st.options[st.pick].clone());
                            query = true;
                        }
                        if ui.add_enabled(st.rx.is_none() && !st.chosen.is_empty(), egui::Button::new(t("MSG_141"))).clicked() {
                            st.chosen.pop();
                            query = true;
                        }
                        if ui.button(t("MSG_006")).clicked() {
                            next = Some(Dialog::None);
                        }
                    });
                });
                if query {
                    st.query(ctx);
                }
                if st.rx.is_some() {
                    ctx.request_repaint_after(Duration::from_millis(200));
                }
            }
            Dialog::Error(msg) => {
                let msg = msg.clone();
                modal("error", &mut |ui| {
                    ui.heading(t("MSG_042"));
                    ui.label(&msg);
                    if ui.button(t("IDOK")).clicked() {
                        next = Some(Dialog::None);
                    }
                });
            }
        }
        if let Some(d) = next {
            self.dialog = d;
        }
        if let Some(st) = stage {
            self.dialog = self.next_dialog(st);
        }
        if let Some((url, path)) = download {
            self.log.push(format!("Downloading {url}"));
            self.pending_image = Some(path.clone());
            self.spawn(ctx, move |tx, c| {
                let progress = |p, got| {
                    tx.send(Msg::Progress(p, tf("MSG_241", &[&drives::human(got)]))).ok();
                };
                net::download(&url, &path, &progress, c)?;
                Ok(format!("Downloaded {}", path.display()))
            });
        }
        if start {
            self.start_job(ctx);
        }

        let log = &mut self.log;
        os_window(ctx, "log", &mut self.show_log, &t("IDD_LOG"), [520.0, 300.0], |ui| {
            ui.horizontal(|ui| {
                if ui.button(t("IDC_LOG_CLEAR")).clicked() {
                    log.clear();
                }
                if ui.button(t("IDC_LOG_SAVE")).clicked()
                    && let Some(p) = rfd::FileDialog::new().set_file_name("rustfus.log").save_file()
                {
                    std::fs::write(p, log.join("\n")).ok();
                }
            });
            egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                for l in log.iter() {
                    ui.monospace(l);
                }
            });
        });
        os_window(ctx, "about", &mut self.show_about, "About Rustfus", [440.0, 220.0], |ui| {
            ui.heading(format!("Rustfus {}", env!("CARGO_PKG_VERSION")));
            ui.label("The Reliable USB Formatting Utility, rewritten in Rust.");
            ui.label("Based on Rufus © 2011-2026 Pete Batard. Not affiliated with Rufus.");
            ui.label(
                "Includes ms-sys boot records, Syslinux, GRUB 2, FreeDOS, UEFI:NTFS (ntfs-3g, EfiFs), \
                 uefi-md5sum and the Rufus translations. See THIRD_PARTY_NOTICES.md.",
            );
            ui.hyperlink_to("https://rufus.ie", "https://rufus.ie");
            ui.label("License: GNU General Public License (GPL) version 3 or later.");
        });
        let mut dark = self.dark;
        os_window(ctx, "settings", &mut self.show_settings, &t("IDS_UPDATE_SETTINGS_GRP"), [260.0, 60.0], |ui| {
            ui.checkbox(&mut dark, "Dark mode");
        });
        if dark != self.dark {
            self.dark = dark;
            rufus_style(ctx, dark);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        let ctx = ui.ctx().clone();
        if self.busy.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        if let Some(p) = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()))
            && self.busy.is_none()
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
                    (false, None) => tf(if self.drives.len() == 1 { "MSG_208" } else { "MSG_209" }, &[&self.drives.len().to_string()]),
                };
                ui.label(status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let t = self.started.map(|t| t.elapsed()).unwrap_or(self.elapsed).as_secs();
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

/// Shows a dialog in its own OS window while `*open`; closing the window clears `*open`.
fn os_window(ctx: &egui::Context, id: &str, open: &mut bool, title: &str, size: [f32; 2], mut add: impl FnMut(&mut egui::Ui)) {
    if !*open {
        return;
    }
    let builder = egui::ViewportBuilder::default().with_title(title).with_inner_size(size);
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(id), builder, |ui, _| {
        egui::CentralPanel::default_margins().show(ui, |ui| add(ui));
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            *open = false;
        }
    });
}

fn main() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default().with_inner_size([470.0, 610.0]).with_resizable(false);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../res/rustfus.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("Rustfus", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
