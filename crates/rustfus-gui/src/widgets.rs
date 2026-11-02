// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Rufus-look egui helpers shared by the main window and dialogs.

use eframe::egui::{self, Color32, RichText};
use functora_egui::{
    Button as FunctoraButton, ButtonVariant, Checkbox, ComponentSize, Dialog, LucideIcon, Progress,
    Separator, ShadcnTheme, ShadcnThemeExt, Spinner,
};
use rustfus_core::image;
use rustfus_core::loc::{t, tf};

pub(crate) const RUFUS_GREEN: Color32 = Color32::from_rgb(0x06, 0xb0, 0x25);

fn rufus_theme(dark: bool) -> ShadcnTheme {
    let mut theme = if dark {
        functora_egui::theme::shadcn_theme_dark::dark()
    } else {
        functora_egui::theme::shadcn_theme_light::light()
    };
    theme.radius = 4.0;
    let gray = Color32::from_gray;
    if dark {
        theme.background = gray(24);
        theme.foreground = gray(244);
        theme.card = gray(24);
        theme.popover = gray(28);
        theme.primary = gray(244);
        theme.primary_foreground = gray(24);
        theme.secondary = gray(39);
        theme.muted = gray(39);
        theme.muted_foreground = gray(163);
        theme.accent = gray(48);
        theme.border = gray(58);
        theme.input = gray(70);
        theme.ring = gray(163);
    } else {
        theme.background = Color32::WHITE;
        theme.foreground = gray(24);
        theme.card = Color32::WHITE;
        theme.popover = Color32::WHITE;
        theme.primary = gray(24);
        theme.primary_foreground = gray(250);
        theme.secondary = gray(245);
        theme.muted = gray(245);
        theme.muted_foreground = gray(100);
        theme.accent = gray(240);
        theme.border = gray(228);
        theme.input = gray(212);
        theme.ring = gray(115);
    }

    theme.card_foreground = theme.foreground;
    theme.popover_foreground = theme.foreground;
    theme.secondary_foreground = theme.foreground;
    theme.accent_foreground = theme.foreground;
    theme
}

/// Update palette tokens without changing the user's system/light/dark preference.
pub(crate) fn rufus_style(ctx: &egui::Context, dark: bool) {
    let installed = egui::Id::new("rustfus-theme-installed");
    let previous = ctx.data(|d| d.get_temp::<bool>(installed));
    if previous.is_none() {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "Inter Variable".into(),
            std::sync::Arc::new(
                egui::FontData::from_static(include_bytes!("../../../res/fonts/InterVariable.ttf"))
                    .tweak(egui::FontTweak {
                        coords: egui::epaint::text::VariationCoords::new([
                            (b"wght", 400.0),
                            (b"opsz", 14.0),
                        ]),
                        ..Default::default()
                    }),
            ),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "Inter Variable".into());
        let heading = egui::FontFamily::Name("Inter Heading".into());
        fonts.font_data.insert(
            "Inter Heading".into(),
            std::sync::Arc::new(
                egui::FontData::from_static(include_bytes!("../../../res/fonts/InterVariable.ttf"))
                    .tweak(egui::FontTweak {
                        coords: egui::epaint::text::VariationCoords::new([
                            (b"wght", 550.0),
                            (b"opsz", 20.0),
                        ]),
                        ..Default::default()
                    }),
            ),
        );
        let mut fallback = fonts.families[&egui::FontFamily::Proportional].clone();
        fallback.insert(0, "Inter Heading".into());
        fonts.families.insert(heading.clone(), fallback);
        for (name, data) in script_fonts().font_data {
            fonts.font_data.insert(name.clone(), data);
            for family in fonts.families.values_mut() {
                family.push(name.clone());
            }
        }
        ctx.set_fonts(fonts);
        ctx.all_styles_mut(|s| {
            s.spacing.item_spacing.y = 6.0;
            s.spacing.window_margin = egui::Margin::same(12);
            s.spacing.interact_size.y = 28.0;
            s.spacing.button_padding = egui::vec2(8.0, 4.0);
            s.text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
            s.text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
            s.text_styles
                .insert(egui::TextStyle::Heading, egui::FontId::proportional(20.0));
        });
    }
    if previous != Some(dark) {
        ctx.all_styles_mut(|s| {
            s.visuals = if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            };
        });
        let theme = rufus_theme(dark);
        ctx.set_shadcn_theme(theme.clone());
        ctx.all_styles_mut(|s| {
            for w in [
                &mut s.visuals.widgets.inactive,
                &mut s.visuals.widgets.hovered,
                &mut s.visuals.widgets.active,
            ] {
                w.bg_fill = theme.background;
                w.weak_bg_fill = theme.background;
                w.bg_stroke = egui::Stroke::new(1.0, theme.input);
                w.corner_radius = egui::CornerRadius::same(4);
            }
            s.visuals.widgets.hovered.bg_fill = theme.accent;
            s.visuals.widgets.hovered.weak_bg_fill = theme.accent;
            s.visuals.widgets.active.bg_stroke.color = theme.ring;
            s.visuals.text_cursor.stroke.color = theme.foreground;
            s.visuals.extreme_bg_color = theme.background;
        });
        ctx.data_mut(|d| d.insert_temp(installed, dark));
    }
}

/// Call each frame after egui has resolved the system theme or explicit preference.
pub(crate) fn sync_style(ctx: &egui::Context) {
    rufus_style(ctx, ctx.theme() == egui::Theme::Dark);
}

/// Normalize shouting action translations without rewriting technical option labels.
pub(crate) fn action_label(text: &str) -> String {
    let text = text.trim();
    if !text.chars().any(char::is_uppercase) || text.chars().any(char::is_lowercase) {
        return text.into();
    }
    let lower = text.to_lowercase();
    let mut first = true;
    lower
        .chars()
        .flat_map(|c| {
            if first && c.is_alphabetic() {
                first = false;
                c.to_uppercase().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

pub(crate) struct Button {
    label: String,
    inner: FunctoraButton<'static>,
}

impl Button {
    pub(crate) fn variant(mut self, variant: ButtonVariant) -> Self {
        self.inner = self.inner.variant(variant);
        self
    }
    pub(crate) fn fixed_width(mut self, width: f32) -> Self {
        self.inner = self.inner.fixed_width(width);
        self
    }
    fn icon(mut self, icon: LucideIcon) -> Self {
        self.inner = self.inner.icon(icon);
        self
    }
    fn selected(mut self, selected: bool) -> Self {
        self.inner = self.inner.selected(selected);
        self
    }
}

impl egui::Widget for Button {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let response = ui.add(self.inner);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                response.enabled(),
                self.label.clone(),
            )
        });
        response
    }
}

pub(crate) fn button(text: impl AsRef<str>) -> Button {
    let label = action_label(text.as_ref());
    Button {
        inner: FunctoraButton::new(label.clone())
            .variant(ButtonVariant::Outline)
            .size(ComponentSize::Xs),
        label,
    }
}

pub(crate) struct IconButton {
    label: String,
    icon: LucideIcon,
    has_popup: bool,
    variant: ButtonVariant,
    width: Option<f32>,
}

impl IconButton {
    pub(crate) fn fixed_width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
    pub(crate) fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }
    pub(crate) fn has_popup(mut self, has_popup: bool) -> Self {
        self.has_popup = has_popup;
        self
    }
}

impl egui::Widget for IconButton {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let mut button = FunctoraButton::icon_only(self.icon)
            .variant(self.variant)
            .size(ComponentSize::Xs);
        if let Some(width) = self.width {
            button = button.fixed_width(width);
        }
        let response = ui.add(button);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &self.label)
        });
        if self.has_popup {
            response.ctx.accesskit_node_builder(response.id, |node| {
                node.set_has_popup(egui::accesskit::HasPopup::Menu);
            });
        }
        response.on_hover_text(self.label)
    }
}

pub(crate) fn icon_button(text: impl AsRef<str>, name: &str) -> IconButton {
    let icon = match name {
        "save" => LucideIcon::Save,
        "hash" => LucideIcon::Hash,
        "languages" => LucideIcon::Languages,
        "info" => LucideIcon::Info,
        "settings" => LucideIcon::Settings,
        "logs" => LucideIcon::Logs,
        "theme" => LucideIcon::SunMoon,
        "updates" => LucideIcon::RefreshCw,
        "chevron-down" => LucideIcon::ChevronDown,
        _ => panic!("unknown application icon: {name}"),
    };
    IconButton {
        label: action_label(text.as_ref()),
        icon,
        has_popup: false,
        variant: ButtonVariant::Ghost,
        width: None,
    }
}

pub(crate) fn checkbox(
    ui: &mut egui::Ui,
    value: &mut bool,
    text: impl Into<String>,
) -> egui::Response {
    let text = text.into();
    let response = ui.add(Checkbox::new(value).label(text.clone()));
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, &text)
    });
    response
}

/// Compact desktop field; functora's Input has no size override below 800pt.
pub(crate) fn input(ui: &mut egui::Ui, value: &mut String) -> egui::Response {
    let padding = egui::vec2(8.0, 5.0);
    let width = (ui.available_width() - padding.x * 2.0).max(1.0);
    ui.add(
        egui::TextEdit::singleline(value)
            .font(egui::FontId::proportional(14.0))
            .margin(padding)
            .desired_width(width),
    )
}

pub(crate) fn select(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut usize,
    choices: &[String],
    placeholder: &str,
) -> egui::Response {
    let before = *value;
    let mut selected = choices.get(*value).map(|_| *value);
    let shown = choices
        .get(*value)
        .map(String::as_str)
        .unwrap_or(placeholder);
    // Functora 0.2 fixes narrow-view fields to 44pt without a size builder.
    // Keep native field behavior and use the same palette at desktop density.
    let mut response = ui
        .scope(|ui| {
            ui.spacing_mut().interact_size.y = 28.0;
            ui.spacing_mut().button_padding = egui::vec2(8.0, 4.0);
            ui.add_enabled_ui(!choices.is_empty(), |ui| {
                egui::ComboBox::from_id_salt(id)
                    .width((ui.available_width() - ui.spacing().button_padding.x * 2.0).max(1.0))
                    .height(240.0)
                    .truncate()
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        for (i, text) in choices.iter().enumerate() {
                            ui.selectable_value(&mut selected, Some(i), text);
                        }
                    })
                    .response
            })
            .inner
        })
        .inner;
    // Upstream paints the select but does not publish metadata or mark changes.
    if response.has_focus() && response.enabled() && !choices.is_empty() {
        let next = ui.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                Some(selected.map_or(0, |i| (i + 1).min(choices.len() - 1)))
            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                Some(selected.unwrap_or(0).saturating_sub(1))
            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::Home) {
                Some(0)
            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::End) {
                Some(choices.len() - 1)
            } else {
                None
            }
        });
        if let Some(next) = next {
            selected = Some(next);
        }
    }
    if let Some(selected) = selected {
        *value = selected;
    }
    if before != *value {
        response.mark_changed();
    }
    response.widget_info(|| {
        let mut info =
            egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, response.enabled(), placeholder);
        info.current_text_value = choices.get(*value).cloned();
        info
    });
    response
}

pub(crate) fn progress(ui: &mut egui::Ui, value: f32, text: impl AsRef<str>) -> egui::Response {
    let label = ui.label(RichText::new(text.as_ref()).strong());
    let theme = ui.ctx().shadcn_theme();
    let mut progress_theme = theme.clone();
    progress_theme.primary = RUFUS_GREEN;
    // Progress 0.2 has no fill builder; scope only its token lookup, not egui styles.
    let key = egui::Id::NULL;
    ui.ctx().data_mut(|d| d.insert_temp(key, progress_theme));
    let response = ui.add(Progress::new(value));
    ui.ctx().data_mut(|d| d.insert_temp(key, theme));
    response.labelled_by(label.id)
}

pub(crate) fn spinner(ui: &mut egui::Ui) -> egui::Response {
    ui.add(Spinner::new())
}

pub(crate) fn separator(ui: &mut egui::Ui) -> egui::Response {
    ui.add(Separator::horizontal())
}

pub(crate) fn modal(
    ctx: &egui::Context,
    id: &str,
    open: &mut bool,
    body: impl FnOnce(&mut egui::Ui),
) {
    if !*open {
        return;
    }
    // Dialog 0.2 uses fixed Area IDs and has no focus barrier or Escape handling.
    // The uniquely identified egui modal supplies input isolation; functora owns
    // the visible dialog and all controls. Scope child IDs by the caller's ID.
    let guard = egui::Modal::new(egui::Id::new(("rustfus-modal", id)))
        .backdrop_color(Color32::TRANSPARENT)
        .frame(egui::Frame::NONE)
        .show(ctx, |_| {
            Dialog::new().width(460.0).show(ctx, open, |ui| {
                ui.push_id(id, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height((ctx.content_rect().height() - 100.0).max(80.0))
                        .show(ui, body);
                });
            });
        });
    if guard.should_close() {
        *open = false;
    }
}

/// egui's built-in fonts lack CJK, Thai, Arabic and Hebrew, used by the language
/// names and translations, so add system fonts covering them as fallbacks.
fn script_fonts() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::empty();
    let mut add = |name: &str, font: Vec<u8>, index: u32| {
        let data = egui::FontData {
            index,
            ..egui::FontData::from_owned(font)
        };
        fonts
            .font_data
            .insert(name.into(), std::sync::Arc::new(data));
        for family in fonts.families.values_mut() {
            family.push(name.into());
        }
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
                break;
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
    fonts
}

/// Keep Rufus' heading-and-rule hierarchy without custom painting.
pub(crate) fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let heading = egui::FontFamily::Name("Inter Heading".into());
        let family = if ui
            .ctx()
            .fonts(|f| f.definitions().families.contains_key(&heading))
        {
            heading
        } else {
            egui::FontFamily::Proportional
        };
        ui.label(RichText::new(title).font(egui::FontId::new(18.0, family)));
        separator(ui);
    });
}

pub(crate) fn toggle(ui: &mut egui::Ui, open: &mut bool, what: &str) {
    let text = tf(if *open { "MSG_122" } else { "MSG_121" }, &[what]);
    let icon = if *open {
        LucideIcon::ChevronUp
    } else {
        LucideIcon::ChevronDown
    };
    if ui
        .add(
            button(text)
                .variant(ButtonVariant::Ghost)
                .icon(icon)
                .selected(*open),
        )
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
    label: egui::Id,
) -> bool {
    let before = *value;
    let mut selected = items
        .iter()
        .position(|(v, _)| *v == before)
        .unwrap_or(usize::MAX);
    let choices: Vec<_> = items.iter().map(|(_, text)| text.clone()).collect();
    if select(ui, id, &mut selected, &choices, "")
        .labelled_by(label)
        .changed()
        && let Some((chosen, _)) = items.get(selected)
    {
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
        if ui
            .ctx()
            .input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape))
        {
            *open = false;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };

    #[test]
    fn theme_initializes_before_the_first_frame() {
        let ctx = egui::Context::default();
        rufus_style(&ctx, false);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.add(button("START"));
            checkbox(ui, &mut false, "確認 — בדיקה");
        });
        output.textures_delta.clear();
        assert!(!output.shapes.is_empty());
        rufus_style(&ctx, true);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.add(button("CANCEL"));
        });
        output.textures_delta.clear();
    }

    #[test]
    fn action_labels_are_sentence_case() {
        assert_eq!(action_label("SELECT"), "Select");
        assert_eq!(action_label("CHECK NOW"), "Check now");
        assert_eq!(action_label("FreeDOS"), "FreeDOS");
        assert_eq!(action_label("  CHECK NOW  "), "Check now");
        assert_eq!(action_label(""), "");
    }

    #[test]
    fn status_colors_have_readable_text() {
        fn luminance(c: Color32) -> f32 {
            fn channel(v: u8) -> f32 {
                let v = f32::from(v) / 255.0;
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            }
            0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
        }
        for dark in [false, true] {
            let theme = rufus_theme(dark);
            for (foreground, background) in [
                (theme.foreground, theme.background),
                (theme.primary_foreground, theme.primary),
                (theme.muted_foreground, theme.background),
            ] {
                let a = luminance(foreground);
                let b = luminance(background);
                assert!((a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5);
            }
        }
    }

    #[test]
    fn palette_sync_preserves_preference_and_unrelated_style() {
        let ctx = egui::Context::default();
        ctx.set_theme(egui::ThemePreference::System);
        rufus_style(&ctx, false);
        assert_eq!(
            ctx.options(|o| o.theme_preference),
            egui::ThemePreference::System
        );
        assert_eq!(ctx.shadcn_theme().border, rufus_theme(false).border);
        ctx.style_mut_of(ctx.theme(), |s| s.spacing.item_spacing.y = 17.0);
        rufus_style(&ctx, false);
        assert_eq!(ctx.style_of(ctx.theme()).spacing.item_spacing.y, 17.0);
        rufus_style(&ctx, true);
        assert_eq!(ctx.shadcn_theme().border, rufus_theme(true).border);
        assert_eq!(
            ctx.options(|o| o.theme_preference),
            egui::ThemePreference::System
        );
        assert_eq!(ctx.style_of(ctx.theme()).spacing.item_spacing.y, 17.0);
    }

    #[test]
    fn icon_button_exposes_name_and_keyboard_action() {
        let mut activated = false;
        let mut harness = Harness::new_ui(|ui| {
            if ui.add(icon_button("SETTINGS", "settings")).clicked() {
                activated = true;
            }
        });
        harness.get_by_label("Settings").focus();
        harness.run();
        harness.key_press(egui::Key::Enter);
        harness.run();
        drop(harness);
        assert!(activated);
    }

    #[test]
    fn select_keyboard_preserves_duplicate_label_identity() {
        let mut selected = 0;
        let choices = vec!["Same".into(), "Same".into(), "Other".into()];
        let mut changed = false;
        let mut harness = Harness::new_ui(|ui| {
            changed |= select(ui, "choice", &mut selected, &choices, "Image source").changed();
        });
        harness.get_by_label("Image source").focus();
        harness.run();
        harness.key_press(egui::Key::ArrowDown);
        harness.run();
        drop(harness);
        assert_eq!(selected, 1);
        assert!(changed);
    }

    #[test]
    fn modal_escape_closes_without_activating_background() {
        let mut open = true;
        let mut background_activated = false;
        let mut harness = Harness::new_ui(|ui| {
            if ui.add(button("Background action")).clicked() {
                background_activated = true;
            }
            modal(ui.ctx(), "confirm-write", &mut open, |ui| {
                ui.add(icon_button("Dialog settings", "settings"));
            });
        });
        harness.run();
        harness.get_by_label("Dialog settings").focus();
        harness.run();
        harness.key_press(egui::Key::Escape);
        harness.run();
        drop(harness);
        assert!(!open);
        assert!(!background_activated);
    }

    #[test]
    fn progress_does_not_leak_green_tokens() {
        let ctx = egui::Context::default();
        rufus_style(&ctx, false);
        let primary = ctx.shadcn_theme().primary;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            progress(ui, 0.5, "Ready");
            assert_eq!(ui.ctx().shadcn_theme().primary, primary);
            assert_eq!(ui.visuals().hyperlink_color, primary);
        });
        output.textures_delta.clear();
    }

    #[test]
    fn functora_button_is_keyboard_activatable() {
        let mut activated = false;
        let mut harness = Harness::new_ui(|ui| {
            if ui.add(button("START")).clicked() {
                activated = true;
            }
        });
        harness.key_press(egui::Key::Tab);
        harness.run();
        harness.key_press(egui::Key::Enter);
        harness.run();
        drop(harness);
        assert!(activated);
    }

    #[test]
    fn functora_checkbox_exposes_toggle_state() {
        let mut checked = false;
        let mut harness = Harness::new_ui(|ui| {
            checkbox(ui, &mut checked, "Verify write");
        });
        let verify = harness.get_by_label("Verify write");
        assert_eq!(
            verify.accesskit_node().toggled(),
            Some(egui::accesskit::Toggled::False)
        );
        verify.focus();
        harness.run();
        harness.key_press(egui::Key::Space);
        harness.run();
        drop(harness);
        assert!(checked);
    }
}
