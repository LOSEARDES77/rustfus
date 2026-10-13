// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! START confirmation steps, the Fido download dialog, and the Log/About/Settings windows.

use super::*;

impl App {
    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        let modal = |id: &str, f: &mut dyn FnMut(&mut egui::Ui)| {
            egui::Modal::new(egui::Id::new(id)).show(ctx, |ui| {
                ui.set_max_width(440.0);
                egui::ScrollArea::vertical()
                    .max_height(ctx.content_rect().height() - 60.0)
                    .show(ui, |ui| f(ui));
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
                        if ok {
                            stage = Some(2)
                        } else {
                            next = Some(Dialog::None)
                        }
                    }
                });
            }
            Dialog::Wue => {
                let (o, user, use_acct, wtg) = (
                    &mut self.wue,
                    &mut self.wue_user,
                    &mut self.wue_use_account,
                    self.wtg,
                );
                modal("wue", &mut |ui| {
                    ui.heading(t("MSG_327"));
                    ui.label(t("MSG_328"));
                    ui.checkbox(&mut o.bypass_hw_checks, t("MSG_329"));
                    ui.checkbox(&mut o.no_online_account, t("MSG_330"));
                    ui.horizontal(|ui| {
                        ui.checkbox(use_acct, t("MSG_333"));
                        ui.add_enabled(
                            *use_acct,
                            egui::TextEdit::singleline(user).desired_width(110.0),
                        );
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
                let name = self
                    .selected
                    .and_then(|i| self.drives.get(i))
                    .map(|d| d.label())
                    .unwrap_or_default();
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
                                    ui.add(
                                        egui::TextEdit::singleline(&mut v)
                                            .desired_width(340.0)
                                            .font(egui::TextStyle::Monospace),
                                    );
                                    ui.end_row();
                                }
                            });
                        }
                    }
                    if ui
                        .add_enabled(h.is_some(), egui::Button::new(t("IDOK")))
                        .clicked()
                    {
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
                            let name = fido::file_name(&url);
                            if let Some(path) = rfd::FileDialog::new()
                                .set_file_name(&name)
                                .add_filter(t("MSG_036"), &["iso"])
                                .save_file()
                            {
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
                        egui::ComboBox::from_id_salt("fido_opt")
                            .truncate()
                            .width(300.0)
                            .selected_text(opts[st.pick.min(opts.len() - 1)].clone())
                            .show_ui(ui, |ui| {
                                for (i, o) in opts.iter().enumerate() {
                                    ui.selectable_value(&mut st.pick, i, o);
                                }
                            });
                    }
                    ui.horizontal(|ui| {
                        let last = st.chosen.len() + 1 == fido::FIELDS.len();
                        let ready = st.rx.is_none() && !st.options.is_empty();
                        if ui
                            .add_enabled(
                                ready,
                                egui::Button::new(t(if last { "MSG_040" } else { "MSG_140" })),
                            )
                            .clicked()
                        {
                            st.chosen.push(st.options[st.pick].clone());
                            query = true;
                        }
                        if ui
                            .add_enabled(
                                st.rx.is_none() && !st.chosen.is_empty(),
                                egui::Button::new(t("MSG_141")),
                            )
                            .clicked()
                        {
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
                    tx.send(Msg::Progress(p, tf("MSG_241", &[&drives::human(got)])))
                        .ok();
                };
                net::download(&url, &path, &progress, c)?;
                Ok(format!("Downloaded {}", path.display()))
            });
        }
        if start {
            self.start_job(ctx);
        }

        let log = &mut self.log;
        os_window(
            ctx,
            "log",
            &mut self.show_log,
            &t("IDD_LOG"),
            [520.0, 300.0],
            |ui| {
                ui.horizontal(|ui| {
                    if ui.button(t("IDC_LOG_CLEAR")).clicked() {
                        log.clear();
                    }
                    if ui.button(t("IDC_LOG_SAVE")).clicked()
                        && let Some(p) = rfd::FileDialog::new()
                            .set_file_name("rustfus.log")
                            .save_file()
                    {
                        std::fs::write(p, log.join("\n")).ok();
                    }
                });
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for l in log.iter() {
                            ui.monospace(l);
                        }
                    });
            },
        );
        os_window(
            ctx,
            "about",
            &mut self.show_about,
            "About Rustfus",
            [440.0, 220.0],
            |ui| {
                ui.heading(format!("Rustfus {}", env!("CARGO_PKG_VERSION")));
                ui.label("The Reliable USB Formatting Utility, rewritten in Rust.");
                ui.label("Based on Rufus © 2011-2026 Pete Batard. Not affiliated with Rufus.");
                ui.label(
                "Includes ms-sys boot records, Syslinux, GRUB 2, FreeDOS, UEFI:NTFS (ntfs-3g, EfiFs), \
                 uefi-md5sum and the Rufus translations. See THIRD_PARTY_NOTICES.md.",
                );

                ui.hyperlink_to("https://rufus.ie", "https://rufus.ie");

                ui.label("License: GNU General Public License (GPL) version 3 or later.");
            },
        );
        let mut dark = self.dark;
        os_window(
            ctx,
            "settings",
            &mut self.show_settings,
            &t("IDS_UPDATE_SETTINGS_GRP"),
            [260.0, 60.0],
            |ui| {
                ui.checkbox(&mut dark, "Dark mode");
            },
        );
        if dark != self.dark {
            self.dark = dark;
            rufus_style(ctx, dark);
        }
    }
}
