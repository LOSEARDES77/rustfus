// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Rufus-look egui helpers shared by the main window and dialogs.

use eframe::egui::{self, Color32, RichText};
use rustfus_core::image;
use rustfus_core::loc::{t, tf};
use shadcn_egui::{Theme, components::{button::{Button, ButtonSize, ButtonVariant}, checkbox::Checkbox,
    dialog::Dialog, input::Input, label::Label, progress::{Progress, ProgressVariant},
    select::{Select, SelectSize}, separator::Separator}, style::{AppVariant, Paint, Role}};

pub(crate) const RUFUS_GREEN: Color32 = Color32::from_rgb(0x06, 0xb0, 0x25);
const STATUS: AppVariant<ProgressVariant> = ProgressVariant::app("rufus-status");

fn rufus_theme() -> Theme {
    let mut theme = Theme::vega().color(Role::custom("rufus-green"), RUFUS_GREEN, RUFUS_GREEN).variant(STATUS, ProgressVariant::Default, |r, _| {
        r.progress_indicator.bg.set(Paint::Role(Role::custom("rufus-green"), 1.0));
    });
    theme.radius = 4.0;
    theme.metrics.text_heading = 20.0;
    theme.light.border = Color32::from_gray(128);
    theme.light.input = Color32::from_gray(128);
    theme.light.ring = Color32::from_rgb(0, 112, 24);
    theme.dark.border = Color32::from_gray(140);
    theme.dark.input = Color32::from_gray(140);
    theme.dark.ring = RUFUS_GREEN;
    theme
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::{Queryable, NodeT}};

    #[test]
    fn action_labels_are_sentence_case() {
        assert_eq!(action_label("SELECT"), "Select");
        assert_eq!(action_label("CHECK NOW"), "Check now");
        assert_eq!(action_label("FreeDOS"), "FreeDOS");
        assert_eq!(action_label(""), "");
    }

    #[test]
    fn shadcn_button_is_keyboard_activatable() {
        let mut activated = false;
        let mut harness = Harness::new_ui(|ui| {
            if ui.add(button("START")).clicked() {
                activated = true;
            }
        });
        let start = harness.get_by_label("Start");
        start.focus();
        harness.run();
        harness.key_press(egui::Key::Enter);
        harness.run();
        drop(harness);
        assert!(activated);
    }

    #[test]
    fn shadcn_checkbox_exposes_toggle_state() {
        let mut checked = false;
        let mut harness = Harness::new_ui(|ui| {
            checkbox(ui, &mut checked, "Verify write");
        });
        let verify = harness.get_by_label("Verify write");
        assert_eq!(verify.accesskit_node().toggled(), Some(egui::accesskit::Toggled::False));
        verify.focus();
        harness.run();
        harness.key_press(egui::Key::Space);
        harness.run();
        drop(harness);
        assert!(checked);
    }
}

pub(crate) fn rufus_style(ctx: &egui::Context, dark: bool) {
    let installed = egui::Id::new("rustfus-theme-installed");
    if !ctx.data(|d| d.get_temp::<bool>(installed).unwrap_or(false)) {
        rufus_theme().apply(ctx);
        ctx.all_styles_mut(|s| {
            s.spacing.item_spacing.y = 6.0;
            s.spacing.window_margin = egui::Margin::same(12);
        });
        ctx.data_mut(|d| d.insert_temp(installed, true));
    }
    ctx.set_theme(if dark { egui::ThemePreference::Dark } else { egui::ThemePreference::Light });
}

/// Normalize shouting action translations without rewriting technical option labels.
fn action_label(text: &str) -> String {
    let text = text.trim();
    if !text.chars().any(char::is_uppercase) || text.chars().any(char::is_lowercase) {
        return text.into();
    }
    let lower = text.to_lowercase();
    let mut first = true;
    lower.chars().flat_map(|c| {
        if first && c.is_alphabetic() {
            first = false;
            c.to_uppercase().collect::<Vec<_>>()
        } else { vec![c] }
    }).collect()
}

pub(crate) fn button(text: impl AsRef<str>) -> Button {
    Button::new(action_label(text.as_ref())).variant(ButtonVariant::Outline).size(ButtonSize::Sm)
}

pub(crate) fn icon_button(text: impl AsRef<str>, name: &str) -> Button {
    use shadcn_egui::icons::lucide;
    let icon = match name {
        "save" => lucide::SAVE,
        "hash" => lucide::HASH,
        "languages" => lucide::LANGUAGES,
        "info" => lucide::INFO,
        "settings" => lucide::SETTINGS,
        "logs" => lucide::LOGS,
        "chevron-down" => lucide::CHEVRON_DOWN,
        _ => panic!("unknown application icon: {name}"),
    };
    button(text).variant(ButtonVariant::Ghost).size(ButtonSize::IconSm).icon(&icon)
}

pub(crate) fn checkbox(ui: &mut egui::Ui, value: &mut bool, text: impl Into<String>) -> egui::Response {
    ui.horizontal_wrapped(|ui| {
        let mut response = ui.add(Checkbox::new(value));
        let label = ui.add(Label::new(text));
        response = response.labelled_by(label.id);
        if ui.interact(label.rect, label.id.with("toggle"), egui::Sense::click()).clicked() && ui.is_enabled() {
            *value = !*value;
            response.mark_changed();
        }
        response
    }).inner
}

pub(crate) fn input(ui: &mut egui::Ui, value: &mut String) -> egui::Response {
    ui.add(Input::new(value).max_width(f32::INFINITY))
}

pub(crate) fn select(ui: &mut egui::Ui, id: &str, value: &mut usize, choices: &[String], placeholder: &str) -> egui::Response {
    let options: Vec<_> = choices.iter().map(String::as_str).collect();
    ui.add(Select::new(id, value, &options).size(SelectSize::Sm).width(ui.available_width()).placeholder(placeholder).enabled(!options.is_empty()))
}

pub(crate) fn progress(ui: &mut egui::Ui, value: f32, text: impl AsRef<str>) -> egui::Response {
    let label = ui.label(RichText::new(text.as_ref()).strong());
    ui.add(Progress::new(value.clamp(0.0, 1.0)).variant(STATUS)).labelled_by(label.id)
}

pub(crate) fn separator(ui: &mut egui::Ui) -> egui::Response {
    ui.add(Separator::new())
}

pub(crate) fn modal(ctx: &egui::Context, id: &str, open: &mut bool, body: impl FnOnce(&mut egui::Ui)) {
    Dialog::new(id, "").width(460.0).show_close_button(false).show(ctx, open, |ui| {
        egui::ScrollArea::vertical().max_height((ctx.content_rect().height() - 100.0).max(80.0)).show(ui, body);
    });
}

/// egui's built-in fonts lack CJK, Thai, Arabic and Hebrew, used by the language
/// names and translations, so add system fonts covering them as fallbacks.
pub(crate) fn add_script_fonts(ctx: &egui::Context) {
    let add = |name: &str, font: Vec<u8>, index: u32| {
        let families: Vec<_> = ctx.fonts(|fonts| fonts.definitions().families.keys().cloned().collect());
        let families = families.into_iter().map(|family| {
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
            families.collect(),
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

/// Keep Rufus' heading-and-rule hierarchy without custom painting.
pub(crate) fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(20.0).strong());
        separator(ui);
    });
}

pub(crate) fn toggle(ui: &mut egui::Ui, open: &mut bool, what: &str) {
    let text = tf(if *open { "MSG_122" } else { "MSG_121" }, &[what]);
    let icon = if *open { shadcn_egui::icons::lucide::CHEVRON_UP } else { shadcn_egui::icons::lucide::CHEVRON_DOWN };
    if ui.add(button(text).variant(ButtonVariant::Ghost).icon(&icon).expanded(*open)).clicked() {
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
    let mut selected = items.iter().position(|(v, _)| *v == before).unwrap_or(usize::MAX);
    let choices: Vec<_> = items.iter().map(|(_, text)| text.clone()).collect();
    if select(ui, id, &mut selected, &choices, "").changed()
        && let Some((chosen, _)) = items.get(selected) {
        *value = *chosen;
    }
    before != *value
}

pub(crate) fn ok_cancel(ui: &mut egui::Ui) -> Option<bool> {
    let mut r = None;
    ui.horizontal(|ui| {
        if ui.add(button(t("IDOK"))).clicked() {
            r = Some(true);
        }
        if ui.add(button(t("MSG_007"))).clicked() {
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
        if ui.ctx().input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)) {
            *open = false;
        }
    });
}
