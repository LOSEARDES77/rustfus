// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! The main window: drive properties, format options and status sections.

use super::{App, BootSel, Dialog, FidoState, MB};
use crate::widgets::{
    bad_block_mode, button, checkbox, cluster_name, combo, icon_button, input, progress, section,
    select, separator, toggle,
};
use eframe::egui;
use rustfus_core::loc::{t, tf};
use rustfus_core::ops::{Fs, Scheme, Target};
use rustfus_core::{image, loc, ops};
use shadcn_egui::components::{button::ButtonVariant, popover::Popover, slider::Slider};
use std::sync::atomic::Ordering;

const PERSISTENCE_STEP_MB: u64 = 256;

fn persistence_slots(max_mb: u64) -> u64 {
    max_mb.div_ceil(PERSISTENCE_STEP_MB)
}

fn persistence_mb_from_slot(slot: f32, max_mb: u64) -> u64 {
    let slots = persistence_slots(max_mb);
    let slot = (slot.round() as u64).min(slots);
    slot.saturating_mul(PERSISTENCE_STEP_MB).min(max_mb)
}

impl App {
    pub(super) fn drive_section(&mut self, ui: &mut egui::Ui) {
        section(ui, &t("IDS_DRIVE_PROPERTIES_TXT"));
        let device_label = ui.label(t("IDS_DEVICE_TXT"));
        if self.scanning {
            ui.label("Scanning for devices…");
        } else if let Some(error) = &self.scan_error {
            ui.label(format!("Device scan failed: {error}"));
            if ui.add(button("Retry scan")).clicked() {
                self.retry_scan();
            }
            if !self.drives.is_empty() {
                ui.label("The previous device list is shown below; rescan before continuing.");
            }
        } else if self.drives.is_empty() {
            ui.label("No devices found. Connect a USB drive and rescan.");
            if ui.add(button("Rescan devices")).clicked() {
                self.retry_scan();
            }
        }
        ui.horizontal(|ui| {
            let choices: Vec<String> = self.drives.iter().map(|d| d.label()).collect();
            let mut selected = self.selected.unwrap_or(usize::MAX);
            let width = (ui.available_width() - 36.0 - ui.spacing().item_spacing.x).max(1.0);
            let response = ui
                .allocate_ui(egui::vec2(width, 32.0), |ui| {
                    ui.set_width(width);
                    ui.add_enabled_ui(self.scan_ready(), |ui| {
                        select(ui, "device", &mut selected, &choices, "Select a device")
                            .labelled_by(device_label.id)
                    })
                    .inner
                })
                .inner;
            if let Some(drive) = self.drive() {
                response.clone().on_hover_text(&drive.path);
            }
            if response.changed() {
                self.selected = (selected < self.drives.len()).then_some(selected);
            }
            let save = ui.add_enabled(
                self.scan_ready() && self.drive().is_some(),
                icon_button(t("MSG_304"), "save"),
            );
            if save.on_hover_text(t("MSG_304")).clicked()
                && let Some(out) = rfd::FileDialog::new()
                    .add_filter(t("MSG_343"), &["vhd"])
                    .add_filter(t("MSG_095"), &["img"])
                    .set_file_name("drive.vhd")
                    .save_file()
                && let Some(d) = self.drive().cloned()
            {
                self.log
                    .push(format!("Saving {} to {}", d.path, out.display()));
                let ctx = ui.ctx().clone();
                self.spawn(&ctx, move |tx, c| ops::save_drive(&d, &out, tx, c));
            }
        });

        let boot_label = ui.label(t("IDS_BOOT_SELECTION_TXT"));
        ui.horizontal(|ui| {
            let name = match &self.image {
                Some(p) => p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                None => tf("MSG_281", &[&t("MSG_280")]),
            };
            let before = self.boot;
            let w = (ui.available_width() - 36.0 - ui.spacing().item_spacing.x).max(1.0);
            ui.allocate_ui(egui::vec2(w, ui.spacing().interact_size.y), |ui| {
                ui.set_max_width(w);
                let choices = vec![name, "FreeDOS".into(), t("MSG_279")];
                let mut selected = match self.boot {
                    BootSel::Image => 0,
                    BootSel::FreeDos => 1,
                    BootSel::NonBootable => 2,
                };
                if select(ui, "boot", &mut selected, &choices, &t("MSG_280"))
                    .labelled_by(boot_label.id)
                    .changed()
                {
                    self.boot = match selected {
                        1 => BootSel::FreeDos,
                        2 => BootSel::NonBootable,
                        _ => BootSel::Image,
                    };
                }
            });
            if before != self.boot {
                match self.boot {
                    BootSel::FreeDos => {
                        (self.fs, self.scheme, self.target) = (Fs::Fat32, Scheme::Mbr, Target::Bios)
                    }
                    BootSel::NonBootable => {
                        (self.scheme, self.target) = (Scheme::Mbr, Target::BiosUefi)
                    }
                    BootSel::Image => {}
                }
                self.dd = self.image_mode() && !self.report.is_iso;
                if !self.image_mode() {
                    self.label = self.drive().map(|d| d.name.clone()).unwrap_or_default();
                }
            }
            let hash = ui.add_enabled(self.image_mode(), icon_button(t("MSG_314"), "hash"));
            if hash.on_hover_text(t("MSG_314")).clicked() {
                self.compute_hashes(ui.ctx());
            }
        });
        ui.horizontal_wrapped(|ui| {
            // Rufus' split button: SELECT, or DOWNLOAD (Fido) from the arrow menu.
            ui.add_enabled_ui(self.boot == BootSel::Image, |ui| {
                let (select_text, download) = (t("IDC_SELECT"), t("MSG_040"));
                let text = if self.download_mode {
                    &download
                } else {
                    &select_text
                };
                let main = ui.add(button(text));
                let menu = ui.add(icon_button("Image source", "chevron-down").has_popup(true));
                Popover::new(&menu).show(|ui| {
                    if ui.add(button(&select_text)).clicked() {
                        self.download_mode = false;
                        egui::Popup::close_all(ui.ctx());
                    }
                    if ui.add(button(&download)).clicked() {
                        self.download_mode = true;
                        egui::Popup::close_all(ui.ctx());
                    }
                });
                if main.clicked() {
                    if self.download_mode {
                        let mut st = FidoState {
                            chosen: Vec::new(),
                            options: Vec::new(),
                            pick: 0,
                            rx: None,
                            error: None,
                        };
                        st.query(ui.ctx());
                        self.dialog = Dialog::Fido(st);
                    } else if let Some(p) = rfd::FileDialog::new()
                        .add_filter(
                            "Disk images", // no Rufus string
                            &[
                                "iso", "img", "bin", "raw", "dd", "vhd", "vhdx", "ffu", "gz",
                                "bz2", "xz", "lzma", "zst", "zip",
                            ],
                        )
                        .add_filter(t("MSG_107"), &["*"])
                        .pick_file()
                    {
                        self.select_image(p);
                    }
                }
            });
        });

        if self.iso_mode() && self.report.windows {
            let image_label = ui.label(t("IDS_IMAGE_OPTION_TXT"));
            let before = self.wtg;
            let mut choices = vec![(false, t("MSG_117"))];
            if cfg!(windows) {
                choices.push((true, t("MSG_118")));
            }
            combo(ui, "imgopt", &mut self.wtg, &choices, image_label.id);
            if !cfg!(windows) {
                ui.label("Windows To Go needs DISM and bcdboot, which only exist on Windows.");
            }
            if self.wtg && !before {
                self.fs = Fs::Ntfs;
                self.load_editions();
            }
        }
        if self.iso_mode() && self.report.persistence_capable() {
            ui.label(t("MSG_123"));
            let max = self
                .drive()
                .map(|d| {
                    d.size
                        .saturating_sub(self.report.size.saturating_add(64 * MB))
                        / MB
                })
                .unwrap_or(0);
            self.persistence_mb = if self.persistence_mb == 0 {
                0
            } else {
                self.persistence_mb.max(256.min(max)).min(max)
            };
            let mut slot = persistence_slots(self.persistence_mb) as f32;
            if ui
                .add(
                    Slider::new(&mut slot, 0.0..=persistence_slots(max) as f32)
                        .step(1.0)
                        .enabled(max > 0)
                        .label(t("MSG_123")),
                )
                .changed()
            {
                self.persistence_mb = persistence_mb_from_slot(slot, max);
            }
            ui.label(format!("{} MB", self.persistence_mb));
        }

        let formatting = !self.dd;
        ui.add_enabled_ui(formatting, |ui| {
            ui.columns(2, |c| {
                let scheme_label = c[0].label(t("IDS_PARTITION_TYPE_TXT"));
                let schemes = [
                    (Scheme::Mbr, "MBR".to_string()),
                    (Scheme::Gpt, "GPT".to_string()),
                ];
                if combo(
                    &mut c[0],
                    "scheme",
                    &mut self.scheme,
                    &schemes,
                    scheme_label.id,
                ) {
                    self.target = if self.scheme == Scheme::Gpt {
                        Target::Uefi
                    } else {
                        Target::BiosUefi
                    };
                }
                let target_label = c[1].label(t("IDS_TARGET_SYSTEM_TXT"));
                let targets: Vec<(Target, String)> = match self.scheme {
                    Scheme::Gpt => vec![(Target::Uefi, Target::Uefi.name())],
                    Scheme::Mbr => [Target::BiosUefi, Target::Bios, Target::Uefi]
                        .iter()
                        .map(|t| (*t, t.name()))
                        .collect(),
                };
                combo(
                    &mut c[1],
                    "target",
                    &mut self.target,
                    &targets,
                    target_label.id,
                );
            });
        });

        toggle(ui, &mut self.adv_drive, &t("MSG_119"));
        if self.adv_drive {
            let mut all = self.list_all.load(Ordering::Relaxed);
            if checkbox(ui, &mut all, t("IDC_LIST_USB_HDD")).changed() {
                self.list_all.store(all, Ordering::Relaxed);
                self.rescan.store(true, Ordering::Relaxed);
            }
            ui.add_enabled_ui(formatting, |ui| {
                checkbox(ui, &mut self.old_bios, t("IDC_OLD_BIOS_FIXES"));
            });
            let can_validate = self.iso_mode() && self.report.efi;
            ui.add_enabled_ui(can_validate, |ui| {
                checkbox(
                    ui,
                    &mut self.uefi_validation,
                    t("IDC_UEFI_MEDIA_VALIDATION"),
                );
            });
            ui.add_enabled_ui(self.dd, |ui| {
                checkbox(ui, &mut self.verify, "Verify write (DD mode)");
            });
        }
    }

    pub(super) fn format_section(&mut self, ui: &mut egui::Ui) {
        section(ui, &t("IDS_FORMAT_OPTIONS_TXT"));
        let formatting = !self.dd;
        ui.add_enabled_ui(formatting, |ui| {
            let volume_label = ui.label(t("IDS_LABEL_TXT"));
            input(ui, &mut self.label).labelled_by(volume_label.id);
            let big = self.drive().is_some_and(|d| d.size > 4 << 30);
            let default_fs = if self.report.large_file && self.image_mode() {
                Fs::Ntfs
            } else {
                Fs::Fat32
            };
            let fss: Vec<(Fs, String)> = Fs::ALL
                .iter()
                .filter(|f| f.supported() && !(**f == Fs::Fat && big))
                .filter(|f| self.boot != BootSel::FreeDos || f.is_fat())
                .map(|f| {
                    (
                        *f,
                        if *f == default_fs {
                            tf("MSG_030", &[f.name()])
                        } else {
                            f.name().into()
                        },
                    )
                })
                .collect();
            ui.columns(2, |c| {
                let fs_label = c[0].label(t("IDS_FILE_SYSTEM_TXT"));
                if combo(&mut c[0], "fs", &mut self.fs, &fss, fs_label.id) {
                    self.cluster = 0;
                }
                let cluster_label = c[1].label(t("IDS_CLUSTER_SIZE_TXT"));
                let cl: Vec<(u32, String)> = self
                    .fs
                    .clusters()
                    .into_iter()
                    .map(|v| (v, cluster_name(v)))
                    .collect();
                combo(
                    &mut c[1],
                    "cluster",
                    &mut self.cluster,
                    &cl,
                    cluster_label.id,
                );
            });
        });
        toggle(ui, &mut self.adv_format, &t("MSG_120"));
        if self.adv_format {
            ui.add_enabled_ui(formatting, |ui| {
                checkbox(ui, &mut self.quick, t("IDC_QUICK_FORMAT"));
            });
            ui.add_enabled_ui(formatting, |ui| {
                checkbox(ui, &mut self.ext_label, t("IDC_EXTENDED_LABEL"));
            });
            ui.columns(2, |c| {
                let passes_label = checkbox(&mut c[0], &mut self.bad_blocks, t("IDC_BAD_BLOCKS"));
                c[1].add_enabled_ui(self.bad_blocks, |ui| {
                    let passes: Vec<(u8, String)> = (0..image::BAD_BLOCK_MODES.len() as u8)
                        .map(|i| (i, bad_block_mode(i)))
                        .collect();
                    combo(ui, "passes", &mut self.passes, &passes, passes_label.id);
                });
            });
        }
    }

    pub(super) fn status_section(&mut self, ui: &mut egui::Ui) {
        section(ui, &t("IDS_STATUS_TXT"));
        ui.add_space(4.0);
        let (fraction, status) = if self.busy.is_some() {
            (self.progress, self.status.clone())
        } else if self.scanning {
            (0.0, "Scanning for devices…".into())
        } else if self.scan_error.is_some() {
            (0.0, "Device scan failed".into())
        } else if let Some(success) = self.done {
            if success {
                (1.0, "DONE".into())
            } else {
                (self.progress, t("MSG_212"))
            }
        } else if self.drives.is_empty() {
            (0.0, "No devices found".into())
        } else {
            (0.0, t("MSG_210"))
        };
        progress(ui, fraction, status);
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            let languages = loc::languages();
            let labels: Vec<String> = languages.iter().map(|l| l.name.clone()).collect();
            let mut current = languages
                .iter()
                .position(|l| l.code == loc::current())
                .unwrap_or(usize::MAX);
            let lang = ui
                .add(icon_button(t("MSG_273"), "languages"))
                .on_hover_text(t("MSG_273"));
            let language_label = ui.label(t("MSG_273"));
            let width = ui.available_width();
            ui.allocate_ui(egui::vec2(width, 32.0), |ui| {
                ui.set_width(width);
                let response = select(ui, "language", &mut current, &labels, &t("MSG_273"))
                    .labelled_by(language_label.id);
                if lang.clicked() {
                    response.request_focus();
                    egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&response));
                }
                if response.changed()
                    && let Some(language) = languages.get(current)
                {
                    loc::set_language(&language.code);
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add(icon_button(t("MSG_302"), "info"))
                .on_hover_text(t("MSG_302"))
                .clicked()
            {
                self.show_about = true;
            }
            if ui
                .add(icon_button(t("MSG_301"), "settings"))
                .on_hover_text(t("MSG_301"))
                .clicked()
            {
                self.show_settings = true;
            }
            if ui
                .add(icon_button(t("MSG_303"), "logs"))
                .on_hover_text(t("MSG_303"))
                .clicked()
            {
                self.show_log = !self.show_log;
            }
        });
        separator(ui);
        ui.horizontal_wrapped(|ui| {
            let ready = self.busy.is_none()
                && self.scan_ready()
                && self.drive().is_some()
                && (self.boot != BootSel::Image || self.image.is_some());
            if ui
                .add_enabled(
                    ready,
                    button(t("IDC_START")).variant(ButtonVariant::Default),
                )
                .on_hover_text(t("MSG_171"))
                .clicked()
            {
                self.on_start();
            }
            if let Some((_, cancel)) = &self.busy {
                if ui.add(button(t("MSG_007"))).clicked() {
                    cancel.store(true, Ordering::Relaxed);
                }
            } else if ui.add(button(t("IDCANCEL"))).clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{persistence_mb_from_slot, persistence_slots};

    #[test]
    fn persistence_steps_include_off_minimum_and_exact_capacity() {
        for max in [0, 1, 255, 256, 257, 1024, 1100, 16_000_000] {
            let slots = persistence_slots(max);
            assert_eq!(persistence_mb_from_slot(0.0, max), 0);
            assert_eq!(persistence_mb_from_slot(1.0, max), 256.min(max));
            assert_eq!(persistence_mb_from_slot(slots as f32, max), max);
            let minimum_slot = persistence_slots(256.min(max)) as f32;
            assert_eq!(persistence_mb_from_slot(minimum_slot - 1.0, max), 0);
        }
    }

    #[test]
    fn persistence_steps_clamp_out_of_range_values() {
        for slot in [-100.0, f32::NEG_INFINITY, f32::NAN] {
            assert_eq!(persistence_mb_from_slot(slot, 1100), 0);
        }
        for slot in [100.0, f32::INFINITY, f32::MAX] {
            assert_eq!(persistence_mb_from_slot(slot, 1100), 1100);
        }
        assert_eq!(persistence_mb_from_slot(f32::MAX, u64::MAX), u64::MAX);
    }
}
