// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! The main window: drive properties, format options and status sections.

use super::{App, BootSel, Dialog, FidoState, MB};
use crate::widgets::{
    action_label, bad_block_mode, button, checkbox, cluster_name, combo, icon_button, input,
    menu_item, popover, progress, section, select, spinner, toggle,
};
use eframe::egui;
use functora_egui::{ButtonGroup, ButtonVariant, DropdownMenu, Slider};
use rustfus_core::loc::{t, tf};
use rustfus_core::ops::{Fs, Scheme, Target};
use rustfus_core::{image, loc, ops};
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
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 36.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                let choices: Vec<String> = self.drives.iter().map(|d| d.label()).collect();
                let mut selected = self.selected.unwrap_or(usize::MAX);
                let width = (ui.available_width() - 32.0 - ui.spacing().item_spacing.x).max(1.0);
                let response = ui
                    .allocate_ui(egui::vec2(width, 36.0), |ui| {
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
                let save = save.on_disabled_hover_ui(|ui| {
                    ui.label(if self.scanning {
                        "Scanning for devices…".to_owned()
                    } else if let Some(error) = &self.scan_error {
                        format!("Device scan failed: {error}")
                    } else if self.drives.is_empty() {
                        "No devices found. Connect a USB drive and rescan.".to_owned()
                    } else {
                        "Select a device before saving its contents.".to_owned()
                    });
                    if self.busy.is_none() && ui.add(button("Retry scan")).clicked() {
                        self.retry_scan();
                    }
                });
                if save.clicked()
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
            },
        );

        let boot_label = ui.label(t("IDS_BOOT_SELECTION_TXT"));
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 36.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                let name = match &self.image {
                    Some(p) => p
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    None => tf("MSG_281", &[&t("MSG_280")]),
                };
                let before = self.boot;
                let w = (ui.available_width() - 32.0 - 122.0 - ui.spacing().item_spacing.x * 2.0)
                    .max(1.0);
                ui.allocate_ui(egui::vec2(w, 36.0), |ui| {
                    ui.set_width(w);
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
                            (self.fs, self.scheme, self.target) =
                                (Fs::Fat32, Scheme::Mbr, Target::Bios)
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
                if hash.clicked() {
                    self.compute_hashes(ui.ctx());
                }
                ui.add_enabled_ui(self.boot == BootSel::Image, |ui| {
                    let select_text = t("IDC_SELECT");
                    let download = t("MSG_040");
                    let text = if self.download_mode {
                        &download
                    } else {
                        &select_text
                    };
                    let mut main_clicked = false;
                    ui.allocate_ui(egui::vec2(122.0, 36.0), |ui| {
                        ui.set_width(122.0);
                        ButtonGroup::show(ui, |ui| {
                            main_clicked = ui.add(button(text).fixed_width(90.0)).clicked();
                            let menu = ui.add(
                                icon_button("Image source", "chevron-down")
                                    .fixed_width(32.0)
                                    .variant(ButtonVariant::Outline)
                                    .has_popup(true),
                            );
                            DropdownMenu::show_value(
                                ui,
                                &menu,
                                &[
                                    (false, action_label(&select_text)),
                                    (true, action_label(&download)),
                                ],
                                |mode| self.download_mode = mode,
                            );
                        });
                    });
                    if main_clicked {
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
            },
        );

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
            let persistence_label = ui.label(t("MSG_123"));
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
                .add_enabled(
                    max > 0,
                    Slider::f32(&mut slot, 0.0..=persistence_slots(max) as f32).step(1.0),
                )
                .labelled_by(persistence_label.id)
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
    }

    pub(super) fn footer(&mut self, ui: &mut egui::Ui) {
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 36.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                let lang = ui.add(icon_button(t("MSG_273"), "languages").has_popup(true));
                popover(ui, &lang, 280.0, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(280.0)
                        .show(ui, |ui| {
                            for language in loc::languages() {
                                // Language names are data, not action labels: preserve their casing.
                                let response =
                                    menu_item(ui, &language.name, language.code == loc::current());
                                response.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Button,
                                        response.enabled(),
                                        &language.name,
                                    )
                                });
                                if response.clicked() {
                                    loc::set_language(&language.code);
                                    egui::Popup::close_all(ui.ctx());
                                }
                            }
                        });
                });
                if ui.add(icon_button(t("MSG_302"), "info")).clicked() {
                    self.show_about = true;
                }
                let theme = ui.add(icon_button("Theme", "theme").has_popup(true));
                popover(ui, &theme, 160.0, |ui| {
                    let current = match self.theme_preference {
                        egui::ThemePreference::System => "System",
                        egui::ThemePreference::Light => "Light",
                        egui::ThemePreference::Dark => "Dark",
                    };
                    ui.label(format!("Theme: {current}"));
                    for (preference, label) in [
                        (egui::ThemePreference::System, "System"),
                        (egui::ThemePreference::Light, "Light"),
                        (egui::ThemePreference::Dark, "Dark"),
                    ] {
                        if menu_item(ui, label, self.theme_preference == preference).clicked() {
                            self.theme_preference = preference;
                            ui.ctx().set_theme(preference);
                            egui::Popup::close_all(ui.ctx());
                        }
                    }
                });
                let updates = ui.add(icon_button("Check for updates", "updates").has_popup(true));
                let checking = self.update_rx.is_some();
                let mut check_updates = updates.clicked() && !checking;
                popover(ui, &updates, 260.0, |ui| {
                    if checking {
                        ui.horizontal(|ui| {
                            spinner(ui);
                            ui.label("Checking for updates…");
                        });
                    } else if !self.update_note.is_empty() {
                        ui.add(egui::Label::new(&self.update_note).wrap());
                    }
                    check_updates |= ui
                        .add_enabled(!checking, button("Check for updates"))
                        .clicked();
                });
                if check_updates {
                    self.check_updates(ui.ctx(), true);
                }
                if ui
                    .add(icon_button(t("MSG_303"), "logs").selected(self.show_log))
                    .clicked()
                {
                    self.show_log = !self.show_log;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(self.busy.is_none(), button(t("IDCANCEL")).fixed_width(80.0))
                        .clicked()
                    {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if let Some((_, cancel)) = &self.busy {
                        if ui.add(button(t("MSG_007")).fixed_width(80.0)).clicked() {
                            cancel.store(true, Ordering::Relaxed);
                        }
                    } else {
                        let ready = self.scan_ready()
                            && self.drive().is_some()
                            && (self.boot != BootSel::Image || self.image.is_some());
                        if ui
                            .add_enabled(
                                ready,
                                button(t("IDC_START"))
                                    .variant(ButtonVariant::Default)
                                    .fixed_width(80.0),
                            )
                            .on_hover_text(t("MSG_171"))
                            .clicked()
                        {
                            self.on_start();
                        }
                    }
                });
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{persistence_mb_from_slot, persistence_slots};

    fn footer_harness() -> egui_kittest::Harness<'static, super::App> {
        let (_tx, rx) = std::sync::mpsc::channel();
        let app = super::App::with_inputs(
            rx,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            Some(super::super::update::Settings::default()),
        );
        egui_kittest::Harness::builder()
            .with_size(eframe::egui::vec2(470.0, 300.0))
            .build_ui_state(|ui, app| app.footer(ui), app)
    }

    #[test]
    fn footer_theme_choices_apply_without_a_settings_dialog() {
        use egui_kittest::kittest::Queryable;

        let mut harness = footer_harness();
        for (label, preference) in [
            ("Dark", eframe::egui::ThemePreference::Dark),
            ("Light", eframe::egui::ThemePreference::Light),
            ("System", eframe::egui::ThemePreference::System),
        ] {
            harness.get_by_label("Theme").click();
            harness.run();
            harness.get_by_label(label).click();
            harness.run();
            assert_eq!(harness.state().theme_preference, preference);
        }
    }

    #[test]
    fn footer_update_popover_does_not_restart_an_active_check() {
        use egui_kittest::kittest::Queryable;

        let mut harness = footer_harness();
        let (_tx, rx) = std::sync::mpsc::channel();
        harness.state_mut().update_rx = Some(rx);
        let log = harness.state().log.clone();
        harness.get_by_label("Check for updates").click();
        harness.run_steps(3);
        harness.get_by_label("Checking for updates…");
        assert_eq!(harness.state().log, log);
        assert!(harness.state().update_rx.is_some());
    }

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
