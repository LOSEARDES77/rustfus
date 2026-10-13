// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! The main window: drive properties, format options and status sections.

use super::*;

impl App {
    pub(super) fn drive_section(&mut self, ui: &mut egui::Ui) {
        section(ui, &t("IDS_DRIVE_PROPERTIES_TXT"));
        ui.label(t("IDS_DEVICE_TXT"));
        ui.horizontal(|ui| {
            let text = self.drive().map(|d| d.label()).unwrap_or_default();
            egui::ComboBox::from_id_salt("device")
                .truncate()
                .width(ui.available_width() - 28.0)
                .selected_text(text)
                .show_ui(ui, |ui| {
                    for (i, d) in self.drives.iter().enumerate() {
                        ui.selectable_value(&mut self.selected, Some(i), d.label())
                            .on_hover_text(&d.path);
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
                self.log
                    .push(format!("Saving {} to {}", d.path, out.display()));
                let ctx = ui.ctx().clone();
                self.spawn(&ctx, move |tx, c| ops::save_drive(&d, &out, tx, c));
            }
        });

        ui.label(t("IDS_BOOT_SELECTION_TXT"));
        ui.horizontal(|ui| {
            let name = match (self.boot, &self.image) {
                (BootSel::Image, Some(p)) => p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                (BootSel::Image, None) => tf("MSG_281", &[&t("MSG_280")]),
                (BootSel::FreeDos, _) => "FreeDOS".into(),
                (BootSel::NonBootable, _) => t("MSG_279"),
            };
            let before = self.boot;
            // Fixed width so long translated image names truncate instead of pushing SELECT off-screen.
            let w = ui.available_width() - 165.0;
            ui.allocate_ui(egui::vec2(w, ui.spacing().interact_size.y), |ui| {
                ui.set_max_width(w);
                egui::ComboBox::from_id_salt("boot")
                    .truncate()
                    .width(w)
                    .selected_text(name)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.boot, BootSel::Image, t("MSG_280"));
                        ui.selectable_value(&mut self.boot, BootSel::FreeDos, "FreeDOS");
                        ui.selectable_value(&mut self.boot, BootSel::NonBootable, t("MSG_279"));
                    });
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
            let check = RichText::new("✔")
                .size(16.0)
                .color(Color32::from_rgb(0x1E, 0x7F, 0xD8));
            let hash = ui.add_enabled(self.image_mode(), egui::Button::new(check).frame(false));
            if hash.on_hover_text(t("MSG_314")).clicked() {
                self.compute_hashes(ui.ctx());
            }
            // Rufus' split button: SELECT, or DOWNLOAD (Fido) from the arrow menu.
            ui.add_enabled_ui(self.boot == BootSel::Image, |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let (select, download) =
                    (t("IDC_SELECT").to_uppercase(), t("MSG_040").to_uppercase());
                let text = if self.download_mode {
                    &download
                } else {
                    &select
                };
                let w = (ui.available_width() - 22.0).max(40.0);
                let main = ui.add_sized(
                    [w, ui.spacing().interact_size.y],
                    egui::Button::new(text).truncate(),
                );
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
            ui.label(t("IDS_IMAGE_OPTION_TXT"));
            let before = self.wtg;
            egui::ComboBox::from_id_salt("imgopt")
                .truncate()
                .width(ui.available_width() / 2.0 - 4.0)
                .selected_text(t(if self.wtg { "MSG_118" } else { "MSG_117" }))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.wtg, false, t("MSG_117"));
                    ui.add_enabled_ui(cfg!(windows), |ui| {
                        ui.selectable_value(&mut self.wtg, true, t("MSG_118"))
                    })
                    .response
                    .on_disabled_hover_text(
                        "Windows To Go needs DISM and bcdboot, which only exist on Windows",
                    );
                });
            if self.wtg && !before {
                self.fs = Fs::Ntfs;
                self.load_editions();
            }
        }
        if self.iso_mode() && self.report.persistence_capable() {
            ui.label(t("MSG_123"));
            let max = self
                .drive()
                .map(|d| d.size.saturating_sub(self.report.size + 64 * MB) / MB)
                .unwrap_or(0);
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
                let schemes = [
                    (Scheme::Mbr, "MBR".to_string()),
                    (Scheme::Gpt, "GPT".to_string()),
                ];
                if combo(&mut c[0], "scheme", &mut self.scheme, &schemes) {
                    self.target = if self.scheme == Scheme::Gpt {
                        Target::Uefi
                    } else {
                        Target::BiosUefi
                    };
                }
                c[1].label(t("IDS_TARGET_SYSTEM_TXT"));
                let targets: Vec<(Target, String)> = match self.scheme {
                    Scheme::Gpt => vec![(Target::Uefi, Target::Uefi.name())],
                    Scheme::Mbr => [Target::BiosUefi, Target::Bios, Target::Uefi]
                        .iter()
                        .map(|t| (*t, t.name()))
                        .collect(),
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
            ui.add_enabled(
                formatting,
                egui::Checkbox::new(&mut self.old_bios, t("IDC_OLD_BIOS_FIXES")),
            );
            let can_validate = self.iso_mode() && self.report.efi;
            ui.add_enabled(
                can_validate,
                egui::Checkbox::new(&mut self.uefi_validation, t("IDC_UEFI_MEDIA_VALIDATION")),
            );
            ui.add_enabled(
                self.dd,
                egui::Checkbox::new(&mut self.verify, "Verify write (DD mode)"),
            );
        }
    }

    pub(super) fn format_section(&mut self, ui: &mut egui::Ui) {
        section(ui, &t("IDS_FORMAT_OPTIONS_TXT"));
        let formatting = !self.dd;
        ui.add_enabled_ui(formatting, |ui| {
            ui.label(t("IDS_LABEL_TXT"));
            ui.add(egui::TextEdit::singleline(&mut self.label).desired_width(f32::INFINITY));
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
                c[0].label(t("IDS_FILE_SYSTEM_TXT"));
                if combo(&mut c[0], "fs", &mut self.fs, &fss) {
                    self.cluster = 0;
                }
                c[1].label(t("IDS_CLUSTER_SIZE_TXT"));
                let cl: Vec<(u32, String)> = self
                    .fs
                    .clusters()
                    .into_iter()
                    .map(|v| (v, cluster_name(v)))
                    .collect();
                combo(&mut c[1], "cluster", &mut self.cluster, &cl);
            });
        });
        toggle(ui, &mut self.adv_format, &t("MSG_120"));
        if self.adv_format {
            ui.add_enabled(
                formatting,
                egui::Checkbox::new(&mut self.quick, t("IDC_QUICK_FORMAT")),
            );
            ui.add_enabled(
                formatting,
                egui::Checkbox::new(&mut self.ext_label, t("IDC_EXTENDED_LABEL")),
            );
            ui.columns(2, |c| {
                c[0].checkbox(&mut self.bad_blocks, t("IDC_BAD_BLOCKS"));
                c[1].add_enabled_ui(self.bad_blocks, |ui| {
                    let passes: Vec<(u8, String)> = (0..image::BAD_BLOCK_MODES.len() as u8)
                        .map(|i| (i, bad_block_mode(i)))
                        .collect();
                    combo(ui, "passes", &mut self.passes, &passes);
                });
            });
        }
    }

    pub(super) fn status_section(&mut self, ui: &mut egui::Ui) {
        section(ui, &t("IDS_STATUS_TXT"));
        ui.add_space(4.0);
        let bar = if self.busy.is_some() {
            egui::ProgressBar::new(self.progress).text(RichText::new(&self.status).strong())
        } else {
            egui::ProgressBar::new(1.0)
                .fill(RUFUS_GREEN)
                .text(RichText::new(t("MSG_210")).color(Color32::WHITE).strong())
        };
        ui.add(bar.desired_height(22.0).corner_radius(0.0));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let icon = |ui: &mut egui::Ui, s: &str, tip: &str| {
                ui.add(egui::Button::new(RichText::new(s).size(18.0)).frame(false))
                    .on_hover_text(tip)
                    .clicked()
            };
            let lang = ui.menu_button(RichText::new("🌐").size(18.0), |ui| {
                let cur = loc::current();
                egui::ScrollArea::vertical()
                    .max_height(ui.ctx().content_rect().height() - 60.0)
                    .show(ui, |ui| {
                        for l in loc::languages() {
                            if ui.radio(l.code == cur, &l.name).clicked() {
                                loc::set_language(&l.code);
                                ui.close();
                            }
                        }
                    });
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
                    if ui
                        .add(
                            egui::Button::new(t("MSG_007").to_uppercase())
                                .truncate()
                                .min_size(size),
                        )
                        .clicked()
                    {
                        cancel.store(true, Ordering::Relaxed);
                    }
                } else if ui
                    .add(
                        egui::Button::new(t("IDCANCEL").to_uppercase())
                            .truncate()
                            .min_size(size),
                    )
                    .clicked()
                {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                let ready = self.busy.is_none()
                    && self.drive().is_some()
                    && (self.boot != BootSel::Image || self.image.is_some());
                if ui
                    .add_enabled(
                        ready,
                        egui::Button::new(t("IDC_START").to_uppercase())
                            .truncate()
                            .min_size(size),
                    )
                    .on_hover_text(t("MSG_171"))
                    .clicked()
                {
                    self.on_start();
                }
            });
        });
    }
}
