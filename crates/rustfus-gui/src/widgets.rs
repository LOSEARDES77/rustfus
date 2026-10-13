// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Rufus-look egui helpers shared by the main window and dialogs.

use eframe::egui::{self, Color32, RichText};
use rustfus_core::image;
use rustfus_core::loc::{t, tf};

pub(crate) const RUFUS_GREEN: Color32 = Color32::from_rgb(0x06, 0xB0, 0x25);

/// Rufus look: taller, bordered controls and roomier rows.
pub(crate) fn rufus_style(ctx: &egui::Context, dark: bool) {
    ctx.set_visuals(if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    });
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing.y = 5.0;
        s.spacing.interact_size.y = 22.0;
        s.spacing.button_padding = egui::vec2(8.0, 4.0);
        let border = egui::Stroke::new(
            1.0,
            if dark {
                Color32::from_gray(90)
            } else {
                Color32::from_gray(170)
            },
        );
        s.visuals.widgets.inactive.bg_stroke = border;
        s.visuals.widgets.noninteractive.bg_stroke = border;
    });
}

/// egui's built-in fonts lack CJK, Thai, Arabic and Hebrew, used by the language
/// names and translations, so add system fonts covering them as fallbacks.
pub(crate) fn add_script_fonts(ctx: &egui::Context) {
    let add = |name: &str, font: Vec<u8>, index: u32| {
        let families =
            [egui::FontFamily::Proportional, egui::FontFamily::Monospace].map(|family| {
                egui::epaint::text::InsertFontFamily {
                    family,
                    priority: egui::epaint::text::FontPriority::Lowest,
                }
            });
        let data = egui::FontData {
            index,
            ..egui::FontData::from_owned(font)
        };
        ctx.add_font(egui::epaint::text::FontInsert::new(
            name,
            data,
            families.into(),
        ));
    };
    if cfg!(target_os = "linux") {
        // One fc-match per script, skipping scripts an already added font covers.
        // Korean first: fonts with Hangul usually cover all of CJK.
        let mut covered = std::collections::HashSet::new();
        for lang in ["ko", "ja", "zh-cn", "zh-tw", "th", "ar", "he", "fa"] {
            if covered.contains(lang) {
                continue;
            }
            let Ok(out) = std::process::Command::new("fc-match")
                .args(["-f", "%{file}\t%{index}\t%{lang}", &format!(":lang={lang}")])
                .output()
            else {
                return;
            };
            let out = String::from_utf8_lossy(&out.stdout);
            let mut it = out.split('\t');
            let (Some(file), Some(index), Some(langs)) = (it.next(), it.next(), it.next()) else {
                continue;
            };
            let langs: Vec<&str> = langs.split('|').collect();
            if langs.contains(&lang)
                && let Ok(font) = std::fs::read(file)
            {
                add(lang, font, index.parse().unwrap_or(0));
                covered.extend(langs.into_iter().map(String::from));
            }
        }
    } else {
        let candidates: &[&str] = if cfg!(windows) {
            &[
                "C:\\Windows\\Fonts\\msyh.ttc",
                "C:\\Windows\\Fonts\\msjh.ttc",
                "C:\\Windows\\Fonts\\YuGothM.ttc",
                "C:\\Windows\\Fonts\\malgun.ttf",
                "C:\\Windows\\Fonts\\tahoma.ttf",
            ]
        } else {
            &[
                "/System/Library/Fonts/Hiragino Sans GB.ttc",
                "/System/Library/Fonts/AppleSDGothicNeo.ttc",
                "/System/Library/Fonts/Supplemental/Tahoma.ttf",
            ]
        };
        for path in candidates {
            if let Ok(font) = std::fs::read(path) {
                add(path, font, 0);
            }
        }
    }
}

/// Rufus section heading: bold title followed by a horizontal rule.
pub(crate) fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(20.0).strong());
        let rect = ui.available_rect_before_wrap();
        let stroke = egui::Stroke::new(2.0, ui.visuals().strong_text_color());
        ui.painter().hline(
            rect.left() + 8.0..=rect.right(),
            rect.center().y + 2.0,
            stroke,
        );
    });
}

pub(crate) fn toggle(ui: &mut egui::Ui, open: &mut bool, what: &str) {
    let text = if *open {
        format!("⏶  {}", tf("MSG_122", &[what]))
    } else {
        format!("⏷  {}", tf("MSG_121", &[what]))
    };
    if ui
        .add(egui::Label::new(text).sense(egui::Sense::click()))
        .clicked()
    {
        *open = !*open;
    }
}

/// Rufus' bad blocks combo labels (MSG_034/035/087); index = `image::BAD_BLOCK_MODES` entry.
pub(crate) fn bad_block_mode(i: u8) -> String {
    match i {
        0 => tf("MSG_034", &["1"]),
        1 => tf("MSG_035", &["2", ""]).trim_end().into(),
        2..=4 => tf(
            "MSG_035",
            &[
                "4",
                &tf("MSG_087", &[["SLC", "MLC", "TLC"][i as usize - 2]]),
            ],
        ),
        _ => image::BAD_BLOCK_MODES[i as usize].0.into(),
    }
}

pub(crate) fn cluster_name(c: u32) -> String {
    match c {
        0 => t("MSG_029"),
        c if c >= 1 << 20 => format!("{} {}", c >> 20, t("MSG_028")),
        c if c >= 1024 => format!("{} {}", c >> 10, t("MSG_027")),
        c => format!("{c} {}", t("MSG_026")),
    }
}

/// Full-width combo box over `(value, text)` items; true when changed.
pub(crate) fn combo<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut T,
    items: &[(T, String)],
) -> bool {
    let before = *value;
    let text = items
        .iter()
        .find(|(v, _)| v == value)
        .map(|(_, s)| s.clone())
        .unwrap_or_default();
    egui::ComboBox::from_id_salt(id)
        .truncate()
        .width(ui.available_width())
        .selected_text(text)
        .show_ui(ui, |ui| {
            for (v, s) in items {
                ui.selectable_value(value, *v, s);
            }
        });
    before != *value
}

pub(crate) fn ok_cancel(ui: &mut egui::Ui) -> Option<bool> {
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

/// Shows a dialog in its own OS window while `*open`; closing the window clears `*open`.
pub(crate) fn os_window(
    ctx: &egui::Context,
    id: &str,
    open: &mut bool,
    title: &str,
    size: [f32; 2],
    mut add: impl FnMut(&mut egui::Ui),
) {
    if !*open {
        return;
    }
    // Open beside the main window. Wayland ignores the position; the app id lets a
    // tiling compositor float these (e.g. a Hyprland rule on class `rustfus-dialog`).
    let mut builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(size)
        .with_app_id("rustfus-dialog");
    if let Some(main) = ctx.input(|i| i.viewport().outer_rect) {
        builder = builder.with_position(main.right_top() + egui::vec2(8.0, 0.0));
    }
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(id), builder, |ui, _| {
        egui::CentralPanel::default_margins().show(ui, |ui| add(ui));
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            *open = false;
        }
    });
}
