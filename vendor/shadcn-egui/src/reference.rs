//! Reference screens built entirely from the shadcn_egui theme, layout
//! helpers and components: a settings form with tabs ([`settings_ui`], the
//! `settings` example) and a gallery of every component under a live preset
//! picker ([`gallery_ui`], the `gallery` example). The snapshot tests render
//! both.

use crate::components::alert::{alert_action, Alert, AlertVariant};
use crate::components::badge::{badge, BadgeVariant};
use crate::components::button::{Button, ButtonSize, ButtonVariant};
use crate::components::card::{card_description, card_title};
use crate::components::checkbox::checkbox;
use crate::components::dialog::{self, Dialog};
use crate::components::icon::IconView;
use crate::components::input::Input;
use crate::components::kbd::{kbd, Kbd, KbdGroup};
use crate::components::label::{description, label};
use crate::components::popover::{popover_header, Popover};
use crate::components::progress::Progress;
use crate::components::select::Select;
use crate::components::separator::{separator, separator_vertical};
use crate::components::slider::Slider;
use crate::components::switch::toggle;
use crate::components::tabs::Tabs;
use crate::components::text::TextLook;
use crate::components::textarea::Textarea;
use crate::components::tooltip::tooltip;
use crate::preset::{
    BaseColor, FontHeading, FontName, IconLibrary, MenuAccent, MenuColor, Preset, RadiusChoice,
    StyleName, ThemeColor,
};
use crate::style::State;
use crate::{layout, Theme};

pub struct SettingsState {
    pub tab: usize,
    pub name: String,
    pub email: String,
    pub password: String,
    pub role: usize,
    pub marketing_emails: bool,
    pub security_emails: bool,
    pub weekly_digest: bool,
    /// Volume slider on the components tab, 0..=100.
    pub slider: f32,
    /// Region select on the components tab; an index past the options shows its placeholder.
    pub region: usize,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            tab: 0,
            name: String::new(),
            email: String::new(),
            password: String::new(),
            role: 0,
            marketing_emails: false,
            security_emails: false,
            weekly_digest: false,
            slider: 50.0,
            region: usize::MAX,
        }
    }
}

/// Render the settings screen into the given root `Ui`. `Theme::apply` must
/// have been called on the context (once, when it was created). The caller
/// owns the root `Ui` (e.g. from eframe's `App::ui`); this fills it with a
/// `CentralPanel`.
pub fn settings_ui(ui: &mut egui::Ui, state: &mut SettingsState) {
    let t = Theme::current(ui.ctx());
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(t.palette().background)
                .inner_margin(egui::Margin::same(32)),
        )
        .show(ui, |ui| {
            layout::vstack(ui, 24.0, |ui| {
                card_title(ui, "Settings");
                card_description(ui, "Manage your account settings and preferences.");
                // The gap between the tab list and its content comes from the style.
                layout::vstack(ui, Tabs::content_gap(ui), |ui| {
                    ui.add(Tabs::new(
                        &mut state.tab,
                        &["Account", "Notifications", "Components"],
                    ));
                    match state.tab {
                        0 => account_tab(ui, state),
                        1 => notifications_tab(ui, state),
                        _ => components_tab(ui, state),
                    }
                });
            });
        });
}

/// A labelled control: a 6px-gap stack of `label` over the control. The
/// label names the control for screen readers (`labelled_by`).
fn field(ui: &mut egui::Ui, name: &str, control: impl FnOnce(&mut egui::Ui) -> egui::Response) {
    layout::vstack(ui, 6.0, |ui| {
        let name = label(ui, name);
        let _ = control(ui).labelled_by(name.id);
    });
}

/// A label on the left, its control on the right, the label naming the
/// control for screen readers (`labelled_by`).
fn setting_row(
    ui: &mut egui::Ui,
    name: &str,
    control: impl FnOnce(&mut egui::Ui) -> egui::Response,
) {
    let label_id = std::cell::Cell::new(egui::Id::NULL);
    layout::space_between(
        ui,
        |ui| label_id.set(label(ui, name).id),
        |ui| {
            let _ = control(ui).labelled_by(label_id.get());
        },
    );
}

fn account_tab(ui: &mut egui::Ui, state: &mut SettingsState) {
    layout::card(ui, |ui| {
        layout::vstack(ui, 16.0, |ui| {
            card_title(ui, "Account");
            card_description(ui, "Update your account details.");
            separator(ui);
            field(ui, "Name", |ui| {
                ui.add(
                    Input::new(&mut state.name)
                        .hint("Your name")
                        .max_width(f32::INFINITY),
                )
            });
            field(ui, "Email", |ui| {
                ui.add(
                    Input::new(&mut state.email)
                        .hint("you@example.com")
                        .max_width(f32::INFINITY),
                )
            });
            field(ui, "Password", |ui| {
                ui.add(
                    Input::new(&mut state.password)
                        .hint("••••••••")
                        .password(true)
                        .max_width(f32::INFINITY),
                )
            });
            field(ui, "Role", |ui| {
                ui.add(Select::new(
                    "role",
                    &mut state.role,
                    &["Member", "Admin", "Owner"],
                ))
            });
            separator(ui);
            layout::space_between(
                ui,
                |_ui| {},
                |ui| {
                    let _ = ui.add(Button::new("Cancel").variant(ButtonVariant::Outline));
                    let _ = ui.add(Button::new("Save").variant(ButtonVariant::Default));
                },
            );
        });
    });
}

fn notifications_tab(ui: &mut egui::Ui, state: &mut SettingsState) {
    layout::card(ui, |ui| {
        layout::vstack(ui, 16.0, |ui| {
            card_title(ui, "Notifications");
            card_description(ui, "Choose what you want to be notified about.");
            separator(ui);
            setting_row(ui, "Marketing emails", |ui| {
                toggle(ui, &mut state.marketing_emails)
            });
            setting_row(ui, "Security emails", |ui| {
                toggle(ui, &mut state.security_emails)
            });
            setting_row(ui, "Weekly digest", |ui| {
                checkbox(ui, &mut state.weekly_digest)
            });
            separator(ui);
            label(ui, "Channel status");
            layout::wrap_row(ui, 8.0, |ui| {
                badge(ui, "Active", BadgeVariant::Default);
                badge(ui, "Beta", BadgeVariant::Secondary);
                badge(ui, "Deprecated", BadgeVariant::Destructive);
                badge(ui, "Optional", BadgeVariant::Outline);
            });
        });
    });
}

fn components_tab(ui: &mut egui::Ui, state: &mut SettingsState) {
    layout::card(ui, |ui| {
        layout::vstack(ui, 16.0, |ui| {
            card_title(ui, "Components");
            card_description(ui, "Buttons, badges, a slider and a select.");
            separator(ui);
            label(ui, "Button variants");
            layout::wrap_row(ui, 8.0, |ui| {
                let _ = ui.add(Button::new("Default").variant(ButtonVariant::Default));
                let _ = ui.add(Button::new("Secondary").variant(ButtonVariant::Secondary));
                let _ = ui.add(Button::new("Destructive").variant(ButtonVariant::Destructive));
                let _ = ui.add(Button::new("Outline").variant(ButtonVariant::Outline));
                let _ = ui.add(Button::new("Ghost").variant(ButtonVariant::Ghost));
                let _ = ui.add(Button::new("Link").variant(ButtonVariant::Link));
            });
            label(ui, "Button sizes");
            layout::row(ui, 8.0, |ui| {
                let _ = ui.add(
                    Button::new("Small")
                        .variant(ButtonVariant::Secondary)
                        .size(ButtonSize::Sm),
                );
                let _ = ui.add(
                    Button::new("Default")
                        .variant(ButtonVariant::Secondary)
                        .size(ButtonSize::Default),
                );
                let _ = ui.add(
                    Button::new("Large")
                        .variant(ButtonVariant::Secondary)
                        .size(ButtonSize::Lg),
                );
            });
            separator(ui);
            label(ui, "Badges");
            layout::wrap_row(ui, 8.0, |ui| {
                badge(ui, "Default", BadgeVariant::Default);
                badge(ui, "Secondary", BadgeVariant::Secondary);
                badge(ui, "Destructive", BadgeVariant::Destructive);
                badge(ui, "Outline", BadgeVariant::Outline);
            });
            separator(ui);
            field(ui, "Volume", |ui| {
                ui.add(Slider::new(&mut state.slider, 0.0..=100.0).label("Volume"))
            });
            field(ui, "Region", |ui| {
                ui.add(
                    Select::new("region", &mut state.region, &["Europe", "Americas", "Asia"])
                        .placeholder("Select a region"),
                )
            });
        });
    });
}

/// State of [`gallery_ui`]: the preset and mode the pickers show, the preset
/// code field, and the demo widgets' values.
pub struct GalleryState {
    pub preset: Preset,
    /// Light or dark; the gallery passes it to `ctx.set_theme`.
    pub mode: egui::Theme,
    /// The preset code field; the pickers rewrite it when they change.
    pub code: String,
    /// The last Apply's error, shown under the code field.
    code_error: Option<String>,
    /// What the context shows; `None` until the first pass applies it.
    applied: Option<(Preset, egui::Theme)>,
    tab: usize,
    name: String,
    email: String,
    message: String,
    message_invalid: String,
    checked: bool,
    switch_on: bool,
    volume: f32,
    region: usize,
    dialog_open: bool,
    /// When the progress demo first showed: it moves from 13 to 66
    /// half a second later.
    progress_shown: Option<f64>,
    /// The controlled progress demo's value, 0..=100.
    progress: f32,
}

impl GalleryState {
    /// Pickers set to `preset` in `mode`; the first [`gallery_ui`] pass applies it.
    pub fn new(preset: Preset, mode: egui::Theme) -> Self {
        Self {
            preset,
            mode,
            code: preset.to_code().unwrap_or_default(),
            code_error: None,
            applied: None,
            tab: 0,
            name: String::new(),
            email: "ada@example".into(),
            message: String::new(),
            message_invalid: String::new(),
            checked: true,
            switch_on: true,
            volume: 50.0,
            region: usize::MAX,
            dialog_open: false,
            progress_shown: None,
            progress: 50.0,
        }
    }

    /// Apply the preset when it changed since the last apply, never every
    /// frame (each apply rebuilds the theme), and switch the context's mode
    /// when the mode changed.
    fn apply_if_changed(&mut self, ctx: &egui::Context) {
        let last = self.applied;
        if last.map(|(p, _)| p) != Some(self.preset) {
            self.preset.apply(ctx);
        }
        if last.map(|(_, m)| m) != Some(self.mode) {
            ctx.set_theme(self.mode);
        }
        self.applied = Some((self.preset, self.mode));
    }

    /// Decode the code field into the pickers, or keep everything and show
    /// why it is not a code.
    fn apply_code(&mut self) {
        match Preset::from_code(self.code.trim()) {
            Ok(p) => {
                self.preset = p;
                self.code_error = None;
            }
            Err(e) => self.code_error = Some(format!("Not a preset code: {e}")),
        }
    }
}

impl Default for GalleryState {
    /// ui.shadcn.com's default preset, light.
    fn default() -> Self {
        Self::new(Preset::default(), egui::Theme::Light)
    }
}

/// Render the gallery into the given root `Ui`: a preset bar on top (a
/// preset code field and one select per preset part, and the mode) and every
/// component below it in a scroll area. It applies `state.preset` and
/// `state.mode` itself, on its first pass and whenever a picker changed
/// them. Applied in the first pass, the preset's fonts arrive a pass late
/// and the first frame shows the layout unsettled; an app applies both
/// before the first pass too, as the `gallery` example does. Fonts and
/// icon libraries whose feature is off are not offered.
pub fn gallery_ui(ui: &mut egui::Ui, state: &mut GalleryState) {
    // Apply before anything reads the theme.
    state.apply_if_changed(ui.ctx());
    let before = (state.preset, state.mode);
    preset_bar(ui, state);
    if (state.preset, state.mode) != before {
        state.code = state.preset.to_code().unwrap_or_default();
        state.code_error = None;
        // A picker changed after this pass read the theme: redo the pass,
        // which applies the new preset first.
        ui.ctx().request_discard("preset changed");
    }
    let t = Theme::current(ui.ctx());
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(t.palette().background))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    egui::Frame::new()
                        .inner_margin(egui::Margin::same(32))
                        .show(ui, |ui| gallery_components(ui, state));
                });
        });
}

/// Width of a preset part select: fixed, so the bar does not change with the
/// feature set, and wide enough for the longest id (`inverted-translucent`).
const PART_WIDTH: f32 = 192.0;

/// One preset part: its label over a select of `options`, `current` the
/// index of the preset's value (`None`: not offered, shown as the
/// placeholder `missing`), and a muted `help` line under it. Returns the
/// newly picked index.
fn part_select(
    ui: &mut egui::Ui,
    (name, help): (&str, Option<&str>),
    options: &[&str],
    current: Option<usize>,
    missing: &str,
) -> Option<usize> {
    let mut idx = current.unwrap_or(usize::MAX);
    let mut picked = None;
    layout::vstack(ui, 6.0, |ui| {
        // The help line wraps at the select's width.
        ui.set_max_width(PART_WIDTH);
        field(ui, name, |ui| {
            let resp = ui.add(
                Select::new(name, &mut idx, options)
                    .placeholder(missing)
                    .width(PART_WIDTH),
            );
            if resp.changed() && idx < options.len() {
                picked = Some(idx);
            }
            resp
        });
        if let Some(help) = help {
            description(ui, help);
        }
    });
    picked
}

/// [`part_select`] over `all`, named by `id`; the preset's value is `value`.
fn enum_select<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    name: (&str, Option<&str>),
    all: &[T],
    id: impl Fn(T) -> &'static str,
    value: &mut T,
) {
    let names: Vec<&str> = all.iter().map(|&v| id(v)).collect();
    let current = all.iter().position(|v| v == value);
    // A value this build lacks (a code naming a font whose feature is off)
    // shows as the muted placeholder; the theme falls back with a warning.
    let missing = id(*value);
    if let Some(i) = part_select(ui, name, &names, current, missing) {
        *value = all[i];
    }
}

fn preset_bar(ui: &mut egui::Ui, state: &mut GalleryState) {
    let t = Theme::current(ui.ctx());
    egui::Panel::top("gallery_preset_bar")
        .frame(
            egui::Frame::new()
                .fill(t.palette().background)
                .inner_margin(egui::Margin::symmetric(32, 16)),
        )
        .show(ui, |ui| {
            layout::vstack(ui, 12.0, |ui| {
                layout::row(ui, 8.0, |ui| {
                    let name = label(ui, "Preset code");
                    let resp = ui
                        .add(
                            Input::new(&mut state.code)
                                .hint("b0")
                                .max_width(160.0)
                                .id_salt("gallery_preset_code")
                                .invalid(state.code_error.is_some()),
                        )
                        .labelled_by(name.id);
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let apply = ui.add(Button::new("Apply").variant(ButtonVariant::Outline));
                    if enter || apply.clicked() {
                        state.apply_code();
                    }
                    ui.add_space(16.0);
                    let mut dark = usize::from(state.mode == egui::Theme::Dark);
                    if ui.add(Tabs::new(&mut dark, &["Light", "Dark"])).changed() {
                        state.mode = if dark == 1 {
                            egui::Theme::Dark
                        } else {
                            egui::Theme::Light
                        };
                    }
                });
                if let Some(err) = &state.code_error {
                    ui.label(egui::RichText::new(err).color(t.palette().destructive));
                }
                preset_selects(ui, &mut state.preset);
            });
        });
}

/// The preset parts, one select each, in the order of the shadcn create page.
const PART_COUNT: usize = 10;

/// The preset part selects in rows of as many as fit the width, aligned at
/// the top (only some parts have a help line). egui's wrapping layout cannot
/// wrap a label-over-select stack, so the rows are counted here.
fn preset_selects(ui: &mut egui::Ui, p: &mut Preset) {
    let gap = 12.0;
    let per_row = (((ui.available_width() + gap) / (PART_WIDTH + gap)) as usize).max(1);
    let parts: Vec<usize> = (0..PART_COUNT).collect();
    layout::vstack(ui, gap, |ui| {
        for chunk in parts.chunks(per_row) {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for &i in chunk {
                    preset_part(ui, i, p);
                }
            });
        }
    });
}

/// Preset part `i`'s select, listing its enum's own values; fonts and icon
/// libraries this build lacks are left out.
fn preset_part(ui: &mut egui::Ui, i: usize, p: &mut Preset) {
    let fonts = || -> Vec<FontName> {
        FontName::ALL
            .iter()
            .copied()
            .filter(|&f| crate::fonts::files(f).is_some())
            .collect()
    };
    match i {
        0 => enum_select(
            ui,
            ("Style", None),
            StyleName::ALL,
            StyleName::id,
            &mut p.style,
        ),
        1 => enum_select(
            ui,
            (
                "Base colour",
                Some("Greys for backgrounds, text and borders."),
            ),
            BaseColor::ALL,
            BaseColor::id,
            &mut p.base_color,
        ),
        2 => enum_select(
            ui,
            (
                "Theme colour",
                Some("Accent for buttons, focus ring and active controls."),
            ),
            ThemeColor::ALL,
            ThemeColor::id,
            &mut p.theme,
        ),
        3 => enum_select(
            ui,
            (
                "Chart colour (charts only)",
                Some("Used by charts; no effect on these components."),
            ),
            ThemeColor::ALL,
            ThemeColor::id,
            &mut p.chart_color,
        ),
        4 => enum_select(ui, ("Font", None), &fonts(), FontName::id, &mut p.font),
        5 => {
            let headings: Vec<FontHeading> = std::iter::once(FontHeading::Inherit)
                .chain(fonts().into_iter().map(FontHeading::Font))
                .collect();
            let id = |h: FontHeading| match h {
                FontHeading::Inherit => "inherit",
                FontHeading::Font(f) => f.id(),
            };
            enum_select(
                ui,
                ("Heading font", None),
                &headings,
                id,
                &mut p.font_heading,
            );
        }
        6 => {
            let libraries: Vec<IconLibrary> = IconLibrary::ALL
                .iter()
                .copied()
                .filter(|&l| crate::icons::slots(l).is_some())
                .collect();
            enum_select(
                ui,
                ("Icon library", None),
                &libraries,
                IconLibrary::id,
                &mut p.icons,
            );
        }
        // Every listed radius has an id; only `Px`, which no code names, lacks one.
        7 => enum_select(
            ui,
            ("Radius", None),
            RadiusChoice::ALL,
            |r| r.id().unwrap_or("custom"),
            &mut p.radius,
        ),
        8 => enum_select(
            ui,
            ("Menu colour", Some("Normal or inverted menus.")),
            MenuColor::ALL,
            MenuColor::id,
            &mut p.menu_color,
        ),
        _ => enum_select(
            ui,
            (
                "Menu accent",
                Some("Strength of the highlighted menu item."),
            ),
            MenuAccent::ALL,
            MenuAccent::id,
            &mut p.menu_accent,
        ),
    }
}

const BUTTON_VARIANTS: [(ButtonVariant, &str); 6] = [
    (ButtonVariant::Default, "Default"),
    (ButtonVariant::Secondary, "Secondary"),
    (ButtonVariant::Outline, "Outline"),
    (ButtonVariant::Ghost, "Ghost"),
    (ButtonVariant::Destructive, "Destructive"),
    (ButtonVariant::Link, "Link"),
];

const BUTTON_SIZES: [(ButtonSize, &str); 4] = [
    (ButtonSize::Xs, "Size xs"),
    (ButtonSize::Sm, "Size sm"),
    (ButtonSize::Default, "Size default"),
    (ButtonSize::Lg, "Size lg"),
];

const BADGE_VARIANTS: [(BadgeVariant, &str); 6] = [
    (BadgeVariant::Default, "Default"),
    (BadgeVariant::Secondary, "Secondary"),
    (BadgeVariant::Destructive, "Destructive"),
    (BadgeVariant::Outline, "Outline"),
    (BadgeVariant::Ghost, "Ghost"),
    (BadgeVariant::Link, "Link"),
];

/// A component group: a card with a heading-font title and a description.
fn group(ui: &mut egui::Ui, title: &str, about: &str, add: impl FnOnce(&mut egui::Ui)) {
    layout::card(ui, |ui| {
        layout::vstack(ui, 16.0, |ui| {
            layout::vstack(ui, 6.0, |ui| {
                card_title(ui, title);
                card_description(ui, about);
            });
            add(ui);
        });
    });
}

fn gallery_components(ui: &mut egui::Ui, state: &mut GalleryState) {
    layout::vstack(ui, 24.0, |ui| {
        layout::vstack(ui, 6.0, |ui| {
            card_title(ui, "Component gallery");
            card_description(
                ui,
                "Every shadcn-egui component under the preset chosen above.",
            );
        });
        ui.columns_const(|[left, right]| {
            layout::vstack(left, 24.0, |ui| {
                buttons_group(ui);
                badges_group(ui);
                icons_group(ui);
                kbd_gallery(ui);
                overlays_group(ui, state);
                progress_group(ui, state);
            });
            layout::vstack(right, 24.0, |ui| {
                inputs_group(ui, state);
                textarea_group(ui, state);
                controls_group(ui, state);
                alerts_group(ui);
                tabs_group(ui, state);
            });
        });
    });
}

fn buttons_group(ui: &mut egui::Ui) {
    let icons = Theme::current(ui.ctx()).icons;
    group(ui, "Button", "Every variant in every size.", |ui| {
        for (size, size_name) in BUTTON_SIZES {
            layout::vstack(ui, 8.0, |ui| {
                description(ui, size_name);
                layout::wrap_row(ui, 8.0, |ui| {
                    for (variant, name) in BUTTON_VARIANTS {
                        let _ = ui.add(Button::new(name).variant(variant).size(size));
                    }
                });
            });
        }
        layout::vstack(ui, 8.0, |ui| {
            description(ui, "Icon sizes, an icon with text, disabled");
            layout::wrap_row(ui, 8.0, |ui| {
                for size in [
                    ButtonSize::IconXs,
                    ButtonSize::IconSm,
                    ButtonSize::Icon,
                    ButtonSize::IconLg,
                ] {
                    let _ = ui.add(
                        Button::new("Close")
                            .variant(ButtonVariant::Outline)
                            .size(size)
                            .icon(&icons.close),
                    );
                }
                let _ = ui.add(Button::new("Confirm").icon(&icons.check));
                let _ = ui.add(Button::new("Disabled").enabled(false));
            });
        });
    });
}

fn badges_group(ui: &mut egui::Ui) {
    group(ui, "Badge", "Every variant.", |ui| {
        layout::wrap_row(ui, 8.0, |ui| {
            for (variant, name) in BADGE_VARIANTS {
                badge(ui, name, variant);
            }
        });
    });
}

fn alerts_group(ui: &mut egui::Ui) {
    use crate::icons::lucide;
    group(
        ui,
        "Alert",
        "Default with an icon, destructive, and one with an action.",
        |ui| {
            layout::vstack(ui, 16.0, |ui| {
                ui.add(
                    Alert::new()
                        .icon(lucide::CIRCLE_CHECK)
                        .title("Payment successful")
                        .description(
                            "Your payment of $29.99 has been processed. A receipt has been \
                             sent to your email address.",
                        ),
                );
                ui.add(
                    Alert::new()
                        .variant(AlertVariant::Destructive)
                        .icon(lucide::CIRCLE_ALERT)
                        .title("Payment failed")
                        .description(
                            "Your payment could not be processed. Please check your payment \
                             method and try again.",
                        ),
                );
                Alert::new()
                    .title("Dark mode is now available")
                    .description("Enable it under your profile settings to get started.")
                    .show(ui, |ui| {
                        alert_action(ui, |ui| {
                            let _ = ui.add(Button::new("Enable").size(ButtonSize::Xs));
                        });
                    });
            });
        },
    );
}

fn kbd_gallery(ui: &mut egui::Ui) {
    let icons = Theme::current(ui.ctx()).icons;
    group(ui, "Kbd", "Key chips and groups.", |ui| {
        layout::vstack(ui, 16.0, |ui| {
            KbdGroup::new().show(ui, |ui| {
                for k in ["⌘", "⇧", "⌥", "⌃"] {
                    kbd(ui, k);
                }
            });
            KbdGroup::new().show(ui, |ui| {
                kbd(ui, "Ctrl");
                TextLook::resolve(ui, &[], State::NONE).label(ui, "+", egui::TextWrapMode::Extend);
                kbd(ui, "B");
            });
            ui.horizontal_wrapped(|ui| {
                description(ui, "Use");
                KbdGroup::new().show(ui, |ui| {
                    kbd(ui, "Ctrl + B");
                    kbd(ui, "Ctrl + K");
                });
                description(ui, "to open the command palette");
            });
            ui.add(Kbd::icon_only(icons.check, "Enter"));
        });
    });
}

fn icons_group(ui: &mut egui::Ui) {
    let t = Theme::current(ui.ctx());
    let slots = [
        ("check", &t.icons.check),
        ("select_trigger", &t.icons.select_trigger),
        ("chevron_down_plain", &t.icons.chevron_down_plain),
        ("chevron_up", &t.icons.chevron_up),
        ("chevron_left", &t.icons.chevron_left),
        ("chevron_right", &t.icons.chevron_right),
        ("more_horizontal", &t.icons.more_horizontal),
        ("close", &t.icons.close),
    ];
    group(
        ui,
        "Icons",
        "The icons the components draw, from the preset's icon library.",
        |ui| {
            layout::wrap_row(ui, 24.0, |ui| {
                for (slot, icon) in slots {
                    layout::vstack(ui, 4.0, |ui| {
                        let name = label(ui, icon.name);
                        let _ = ui.add(IconView::new(icon).size(24.0)).labelled_by(name.id);
                        description(ui, slot);
                    });
                }
            });
        },
    );
}

fn inputs_group(ui: &mut egui::Ui, state: &mut GalleryState) {
    group(
        ui,
        "Input and Label",
        "Text fields named by their label.",
        |ui| {
            field(ui, "Name", |ui| {
                ui.add(
                    Input::new(&mut state.name)
                        .hint("Your name")
                        .max_width(f32::INFINITY),
                )
            });
            field(ui, "Email (invalid)", |ui| {
                ui.add(
                    Input::new(&mut state.email)
                        .invalid(true)
                        .max_width(f32::INFINITY),
                )
            });
        },
    );
}

/// shadcn-svelte's textarea-demo and textarea-button, and the field's
/// disabled and invalid states.
fn textarea_group(ui: &mut egui::Ui, state: &mut GalleryState) {
    const HINT: &str = "Type your message here.";
    group(
        ui,
        "Textarea",
        "Multi-line text that grows with its content; disabled; invalid.",
        |ui| {
            layout::vstack(ui, 8.0, |ui| {
                ui.add(
                    Textarea::new(&mut state.message)
                        .hint_text(HINT)
                        .desired_width(f32::INFINITY),
                );
                let _ = ui.add(Button::new("Send message"));
            });
            ui.add_enabled(
                false,
                Textarea::new(&mut String::new())
                    .hint_text(HINT)
                    .desired_width(f32::INFINITY),
            );
            ui.add(
                Textarea::new(&mut state.message_invalid)
                    .hint_text(HINT)
                    .invalid(true)
                    .desired_width(f32::INFINITY),
            );
        },
    );
}

fn controls_group(ui: &mut egui::Ui, state: &mut GalleryState) {
    group(
        ui,
        "Checkbox, Switch, Slider and Select",
        "Controls with a value.",
        |ui| {
            setting_row(ui, "Accept terms", |ui| checkbox(ui, &mut state.checked));
            setting_row(ui, "Airplane mode", |ui| toggle(ui, &mut state.switch_on));
            field(ui, "Volume", |ui| {
                ui.add(Slider::new(&mut state.volume, 0.0..=100.0).label("Volume"))
            });
            field(ui, "Region", |ui| {
                ui.add(
                    Select::new(
                        "gallery_region",
                        &mut state.region,
                        &["Europe", "Americas", "Asia"],
                    )
                    .placeholder("Select a region"),
                )
            });
        },
    );
}

/// shadcn-svelte's `progress-demo` (13, then 66 after 500 ms, at 60 % of
/// the width) and `progress-controlled` (a slider drives the value).
fn progress_group(ui: &mut egui::Ui, state: &mut GalleryState) {
    group(
        ui,
        "Progress",
        "A value that moves, and one a slider sets.",
        |ui| {
            let now = ui.input(|i| i.time);
            let since = now - *state.progress_shown.get_or_insert(now);
            let value = if since < 0.5 {
                ui.ctx().request_repaint_after_secs((0.5 - since) as f32);
                13.0
            } else {
                66.0
            };
            let width = ui.available_width() * 0.6;
            ui.add(Progress::from_value(value, 100.0).desired_width(width));
            // progress-controlled: `flex w-full max-w-sm flex-col gap-4`.
            ui.scope(|ui| {
                ui.set_max_width(384.0);
                layout::vstack(ui, 16.0, |ui| {
                    ui.add(Progress::from_value(state.progress, 100.0));
                    ui.add(
                        Slider::new(&mut state.progress, 0.0..=100.0)
                            .step(1.0)
                            .label("Progress"),
                    )
                });
            });
        },
    );
}

fn tabs_group(ui: &mut egui::Ui, state: &mut GalleryState) {
    group(ui, "Tabs and Separator", "Tabs over their content.", |ui| {
        layout::vstack(ui, Tabs::content_gap(ui), |ui| {
            ui.add(Tabs::new(&mut state.tab, &["Overview", "Usage", "Billing"]));
            let text = match state.tab {
                0 => "The overview tab.",
                1 => "The usage tab.",
                _ => "The billing tab.",
            };
            description(ui, text);
        });
        separator(ui);
        layout::row(ui, 12.0, |ui| {
            label(ui, "Blog");
            separator_vertical(ui);
            label(ui, "Docs");
            separator_vertical(ui);
            label(ui, "Source");
        });
    });
}

fn overlays_group(ui: &mut egui::Ui, state: &mut GalleryState) {
    group(
        ui,
        "Dialog, Popover and Tooltip",
        "Overlays opened from a button.",
        |ui| {
            layout::wrap_row(ui, 8.0, |ui| {
                let open = ui.add(Button::new("Open dialog").variant(ButtonVariant::Outline));
                if open.clicked() {
                    state.dialog_open = true;
                }
                let trigger = ui.add(
                    Button::new("Open popover")
                        .variant(ButtonVariant::Outline)
                        .has_popup(true),
                );
                Popover::new(&trigger).show(|ui| {
                    popover_header(ui, "Dimensions", "Set the dimensions for the layer.");
                });
                let _ = tooltip(
                    ui.add(Button::new("Tooltip").variant(ButtonVariant::Outline)),
                    "Add to library",
                );
                description(ui, "hover me");
            });
        },
    );
    let mut close = false;
    Dialog::new("gallery_dialog", "Edit profile")
        .description("Make changes to your profile here. Click save when you are done.")
        .show(ui.ctx(), &mut state.dialog_open, |ui| {
            dialog::footer(ui, |ui| {
                close |= ui
                    .add(Button::new("Cancel").variant(ButtonVariant::Outline))
                    .clicked();
                close |= ui.add(Button::new("Save changes")).clicked();
            });
        });
    if close {
        state.dialog_open = false;
    }
}
