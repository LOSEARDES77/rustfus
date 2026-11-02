// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! START confirmation steps, the Fido download dialog, and the Log/About/Settings windows.

use super::{App, Dialog, rustfus_name};
use crate::widgets::{
    button, checkbox, combo, input, modal, ok_cancel, os_window, progress, rufus_style, separator,
};
use eframe::egui::{self, Color32};
use rustfus_core::loc::{t, tf};
use rustfus_core::ops::{self, Msg};
use rustfus_core::update::{Interval, Version};
use rustfus_core::{drives, fido, net, syslinux};
use shadcn_egui::components::{input::Input, textarea::Textarea};
use std::path::PathBuf;
use std::time::Duration;

impl App {
    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        let mut modal_open = true;
        let mut next = None;
        let mut download: Option<(String, PathBuf)> = None;
        let mut stage = None;
        let mut start = false;
        let mut allow_updates = None;
        match &mut self.dialog {
            Dialog::None => {}
            Dialog::Download => {
                let (v, allow) = (self.report.grub2_version.clone(), &mut self.allow_downloads);
                modal(ctx, "download", &mut modal_open, |ui| {
                    ui.heading(t("MSG_115"));
                    ui.label(tf("MSG_116", &[&v, "2.14"]));
                    ui.horizontal(|ui| {
                        if ui.add(button(t("MSG_008"))).clicked() {
                            *allow = true;
                            stage = Some(1);
                        }
                        if ui.add(button(t("MSG_009"))).clicked() {
                            stage = Some(1);
                        }
                        if ui.add(button(t("MSG_007"))).clicked() {
                            next = Some(Dialog::None);
                        }
                    });
                });
            }
            Dialog::SyslinuxDownload(v) => {
                let v = v.clone();
                let embedded = syslinux::embedded_version();
                let allow = &mut self.allow_downloads;
                modal(ctx, "syslinux_download", &mut modal_open, |ui| {
                    ui.heading(t("MSG_115"));
                    ui.label(rustfus_name(&tf("MSG_114", &[&v, "", &embedded, ""])));
                    ui.horizontal(|ui| {
                        if ui.add(button(t("MSG_008"))).clicked() {
                            *allow = true;
                            stage = Some(2);
                        }
                        if ui.add(button(t("MSG_009"))).clicked() {
                            next = Some(Dialog::None);
                        }
                    });
                });
            }
            Dialog::ReplaceC32(i) => {
                let i = *i;
                let old_c32 = ops::obsolete_c32(&self.report, self.fs, self.scheme, self.target);
                let replace = &mut self.replace_c32;
                let path = &old_c32[i];
                let name = path.rsplit('/').next().unwrap_or(path);
                modal(ctx, "replace_c32", &mut modal_open, |ui| {
                    ui.heading(tf("MSG_083", &[name]));
                    ui.label(tf("MSG_084", &[name, name]));
                    ui.horizontal(|ui| {
                        let yes = ui.add(button(t("MSG_008"))).clicked();
                        let no = ui.add(button(t("MSG_009"))).clicked();
                        if yes {
                            replace.push(path.clone());
                        }
                        if yes || no {
                            if i + 1 < old_c32.len() {
                                next = Some(Dialog::ReplaceC32(i + 1));
                            } else {
                                stage = Some(2);
                            }
                        }
                    });
                });
            }
            Dialog::Edition => {
                let (eds, sel) = (&self.editions, &mut self.edition);
                modal(ctx, "edition", &mut modal_open, |ui| {
                    ui.heading(t("MSG_118"));
                    let label = ui.label("Please select the edition you want to install:");
                    combo(ui, "edition_choice", sel, eds, label.id);
                    if let Some(ok) = ok_cancel(ui) {
                        if ok {
                            stage = Some(3)
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
                modal(ctx, "wue", &mut modal_open, |ui| {
                    ui.heading(t("MSG_327"));
                    ui.label(t("MSG_328"));
                    checkbox(ui, &mut o.bypass_hw_checks, t("MSG_329"));
                    checkbox(ui, &mut o.no_online_account, t("MSG_330"));
                    ui.horizontal(|ui| {
                        let account_label = checkbox(ui, use_acct, t("MSG_333"));
                        ui.add_enabled_ui(*use_acct, |ui| {
                            input(ui, user).labelled_by(account_label.id)
                        });
                    });
                    checkbox(ui, &mut o.regional, t("MSG_334"));
                    checkbox(ui, &mut o.no_data_collection, t("MSG_331"));
                    checkbox(ui, &mut o.no_bitlocker, t("MSG_335"));
                    if wtg {
                        checkbox(ui, &mut o.wtg_no_internal, t("MSG_332"));
                    }
                    if let Some(ok) = ok_cancel(ui) {
                        next = Some(if ok { Dialog::Confirm } else { Dialog::None });
                    }
                });
            }
            Dialog::Hybrid { dd } => {
                let mut chosen = None;
                modal(ctx, "hybrid", &mut modal_open, |ui| {
                    ui.heading(tf("MSG_274", &["ISOHybrid"]));
                    let (iso, dd_s) = (t("MSG_036"), t("MSG_095"));
                    let label = ui.label(tf("MSG_275", &[&iso, &dd_s, &iso, &dd_s]));
                    combo(
                        ui,
                        "hybrid_mode",
                        dd,
                        &[
                            (false, tf("MSG_276", &[&iso])),
                            (true, tf("MSG_277", &[&dd_s])),
                        ],
                        label.id,
                    );
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
                let name = self.drive().map(|d| d.label()).unwrap_or_default();
                modal(ctx, "confirm", &mut modal_open, |ui| {
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
                modal(ctx, "hash", &mut modal_open, |ui| {
                    ui.heading("Checksums");
                    match &h {
                        None => {
                            progress(ui, p, format!("{:.0}%", p * 100.0));
                        }
                        Some(h) => {
                            egui::Grid::new("hashes").num_columns(2).show(ui, |ui| {
                                for (name, v) in ["MD5", "SHA1", "SHA256", "SHA512"].iter().zip(h) {
                                    let label = ui.label(*name);
                                    let mut v = v.as_str();
                                    ui.add(
                                        // The shared input edits Strings; checksum values stay read-only.
                                        Input::new(&mut v).max_width(340.0),
                                    )
                                    .labelled_by(label.id);
                                    ui.end_row();
                                }
                            });
                        }
                    }
                    if ui.add_enabled(h.is_some(), button(t("IDOK"))).clicked() {
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
                modal(ctx, "fido", &mut modal_open, |ui| {
                    ui.heading(t("MSG_149"));
                    for (f, v) in fido::FIELDS.iter().zip(&st.chosen) {
                        ui.label(format!("{f}: {v}"));
                    }
                    if let Some(err) = &st.error {
                        ui.colored_label(Color32::RED, err);
                    }
                    if st.rx.is_some() {
                        ui.horizontal(|ui| {
                            // shadcn-egui has no spinner component.
                            ui.spinner();
                            ui.label(t("MSG_142"));
                        });
                    } else if st.chosen.len() < fido::FIELDS.len() && !st.options.is_empty() {
                        let label = ui.label(fido::FIELDS[st.chosen.len()]);
                        let opts: Vec<_> = st.options.iter().cloned().enumerate().collect();
                        combo(ui, "fido_opt", &mut st.pick, &opts, label.id);
                    }
                    ui.horizontal(|ui| {
                        let last = st.chosen.len() + 1 == fido::FIELDS.len();
                        let ready = st.rx.is_none() && !st.options.is_empty();
                        if ui
                            .add_enabled(ready, button(t(if last { "MSG_040" } else { "MSG_140" })))
                            .clicked()
                        {
                            st.chosen.push(st.options[st.pick].clone());
                            query = true;
                        }
                        if ui
                            .add_enabled(
                                st.rx.is_none() && !st.chosen.is_empty(),
                                button(t("MSG_141")),
                            )
                            .clicked()
                        {
                            st.chosen.pop();
                            query = true;
                        }
                        if ui.add(button(t("MSG_006"))).clicked() {
                            next = Some(Dialog::None);
                        }
                    });
                });
                if query && modal_open {
                    st.query(ctx);
                }
                if st.rx.is_some() {
                    ctx.request_repaint_after(Duration::from_millis(200));
                }
            }
            Dialog::UpdatePolicy => {
                modal(ctx, "update_policy", &mut modal_open, |ui| {
                    ui.heading(rustfus_name(&t("MSG_004")));
                    ui.label(rustfus_name(&t("MSG_005")));
                    ui.horizontal(|ui| {
                        if ui.add(button(t("MSG_008"))).clicked() {
                            allow_updates = Some(true);
                        }
                        if ui.add(button(t("MSG_009"))).clicked() {
                            allow_updates = Some(false);
                        }
                    });
                });
            }
            Dialog::NewVersion(r) => {
                modal(ctx, "new_version", &mut modal_open, |ui| {
                    ui.heading(rustfus_name(&t("IDD_NEW_VERSION")));
                    ui.label(t("IDS_NEW_VERSION_AVAIL_TXT"));
                    ui.label(format!("Rustfus {} → {}", Version::current(), r.version));
                    ui.hyperlink_to(t("IDC_WEBSITE"), &r.url);
                    if !r.notes.trim().is_empty() {
                        ui.strong(t("IDS_NEW_VERSION_NOTES_GRP"));
                        egui::ScrollArea::vertical()
                            .id_salt("release_notes")
                            .max_height(200.0)
                            .show(ui, |ui| ui.label(r.notes.trim()));
                    }
                    ui.horizontal(|ui| {
                        if ui.add(button(t("IDC_DOWNLOAD"))).clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(&r.url));
                            next = Some(Dialog::None);
                        }
                        if ui.add(button(t("IDCANCEL"))).clicked() {
                            next = Some(Dialog::None);
                        }
                    });
                });
            }
            Dialog::Error(msg) => {
                let msg = msg.clone();
                modal(ctx, "error", &mut modal_open, |ui| {
                    ui.heading(t("MSG_042"));
                    ui.label(&msg);
                    if ui.add(button(t("IDOK"))).clicked() {
                        next = Some(Dialog::None);
                    }
                });
            }
        }
        // Cancellation always wins, including Escape and approval in the same frame.
        if !modal_open {
            next = Some(Dialog::None);
            stage = None;
            start = false;
            allow_updates = None;
            download = None;
        }
        if let Some(d) = next {
            self.dialog = d;
        }
        if let Some(st) = stage {
            self.dialog = self.next_dialog(st);
        }
        if let Some(allow) = allow_updates {
            self.dialog = Dialog::None;
            if !allow {
                self.update.interval = Interval::Disabled;
            }
            if let Err(err) = self.update.save() {
                self.log
                    .push(format!("Could not save the update settings: {err}"));
            }
            if allow {
                self.check_updates(ctx, /* manual= */ false);
            }
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
                    if ui.add(button(t("IDC_LOG_CLEAR"))).clicked() {
                        log.clear();
                    }
                    if ui.add(button(t("IDC_LOG_SAVE"))).clicked()
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
                        let text = log.join("\n");
                        let mut text = text.as_str();
                        ui.add(Textarea::new(&mut text).desired_width(f32::INFINITY));
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
        let mut policy = self.update.clone();
        let (checking, note) = (self.update_rx.is_some(), &self.update_note);
        let mut check_now = false;
        os_window(
            ctx,
            "settings",
            &mut self.show_settings,
            &t("IDD_UPDATE_POLICY"),
            [320.0, 150.0],
            |ui| {
                checkbox(ui, &mut dark, "Dark mode");
                separator(ui);
                egui::Grid::new("update_policy")
                    .num_columns(2)
                    .show(ui, |ui| {
                        let label = ui.label(t("IDS_UPDATE_FREQUENCY_TXT"));
                        let name = |i: Interval| {
                            if i == Interval::default() {
                                tf("MSG_030", &[&t(i.loc_id())])
                            } else {
                                t(i.loc_id())
                            }
                        };
                        let intervals: Vec<_> =
                            Interval::ALL.into_iter().map(|i| (i, name(i))).collect();
                        combo(
                            ui,
                            "update_interval",
                            &mut policy.interval,
                            &intervals,
                            label.id,
                        );
                        ui.end_row();
                        let label = ui.label(t("IDS_INCLUDE_BETAS_TXT"));
                        checkbox(ui, &mut policy.include_betas, "").labelled_by(label.id);
                        ui.end_row();
                    });
                ui.horizontal(|ui| {
                    check_now = ui
                        .add_enabled(!checking, button(t("IDC_CHECK_NOW")))
                        .clicked();
                    if checking {
                        // shadcn-egui has no spinner component.
                        ui.spinner();
                    }
                    ui.label(note);
                });
            },
        );
        if dark != self.dark {
            self.dark = dark;
            rufus_style(ctx, dark);
        }
        if policy != self.update {
            self.update = policy;
            if let Err(err) = self.update.save() {
                self.log
                    .push(format!("Could not save the update settings: {err}"));
            }
        }
        if check_now {
            self.check_updates(ctx, /* manual= */ true);
        }
    }
}
