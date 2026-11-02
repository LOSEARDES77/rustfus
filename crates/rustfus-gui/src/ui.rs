// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).

use std::{path::PathBuf, sync::atomic::Ordering};

use crate::assets::icon;
use crate::model::{BootSel, Dialog, Model};
use gpui_kit::component::{
    ActiveTheme, Disableable, IndexPath, Sizable, ThemeMode, TitleBar, WindowExt,
    button::{Button, ButtonVariants, DropdownButton},
    checkbox::Checkbox,
    group_box::{GroupBox, GroupBoxVariants},
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    select::{SearchableVec, Select, SelectEvent, SelectState},
    slider::{Slider, SliderEvent, SliderState},
};
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Context, Entity, ExternalPaths, Focusable,
    InteractiveElement, IntoElement, MouseButton, ParentElement, PathPromptOptions, Pixels, Render,
    SharedString, Size, StyleRefinement, Styled, Subscription, Task, Window, div, prelude::*, px,
    relative, rgb, size,
};
use rustfus_core::{
    image,
    loc::{self, t, tf},
    ops::{Fs, Scheme, Target},
};

use gpui_kit::base::ElementExt as _;

const TITLEBAR_HEIGHT: f32 = 48.;
const MB: u64 = 1 << 20;
const DEVICE: usize = 0;
const BOOT: usize = 1;
const SCHEME: usize = 2;
const TARGET: usize = 3;
const FILESYSTEM: usize = 4;
const CLUSTER: usize = 5;
const PASSES: usize = 6;
const IMAGE_OPTION: usize = 7;

#[derive(Clone, Debug, PartialEq)]
struct Choice {
    value: usize,
    title: SharedString,
}
fn choices(items: impl IntoIterator<Item = (usize, String)>) -> Vec<Choice> {
    let items = items.into_iter().collect::<Vec<_>>();
    items
        .iter()
        .map(|(value, title)| {
            let title = if items.iter().filter(|(_, other)| other == title).count() > 1 {
                format!("{title} [{}]", value + 1)
            } else {
                title.clone()
            };
            Choice {
                value: *value,
                title: title.into(),
            }
        })
        .collect()
}
struct Control {
    state: Entity<SelectState<SearchableVec<SharedString>>>,
    items: Vec<Choice>,
}
impl Control {
    fn sync(
        &mut self,
        items: Vec<Choice>,
        value: Option<usize>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let changed = self.items != items;
        let selected = self.state.read(cx).selected_value();
        let selection_changed = self
            .items
            .iter()
            .find(|item| Some(&item.title) == selected)
            .map(|item| item.value)
            != value;
        if !changed && !selection_changed {
            return;
        }
        let index = items
            .iter()
            .position(|item| Some(item.value) == value)
            .map(IndexPath::new);
        self.items = items;
        self.state.update(cx, |state, cx| {
            if changed {
                state.set_items(
                    SearchableVec::new(
                        self.items
                            .iter()
                            .map(|item| item.title.clone())
                            .collect::<Vec<_>>(),
                    ),
                    window,
                    cx,
                );
            }
            state.set_selected_index(index, window, cx);
        });
    }
}

pub(crate) struct RustfusView {
    pub(crate) model: Model,
    controls: Vec<Control>,
    label: Entity<InputState>,
    persistence: Entity<SliderState>,
    persistence_max: u64,
    theme: Option<ThemeMode>,
    blur: bool,
    columns: bool,
    compact: bool,
    last_fit: Option<Size<Pixels>>,
    downloader: Option<AnyWindowHandle>,
    checksums: Option<(PathBuf, AnyWindowHandle)>,
    picker: Option<Task<()>>,
    pub(crate) poll_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl RustfusView {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.observe_window_appearance(window, |this, window, cx| {
            if this.theme.is_none() {
                crate::theme::change_theme(None, Some(window), cx);
                cx.notify();
            }
        }));
        let controls = (0..8)
            .map(|field| {
                let state = cx.new(|cx| {
                    SelectState::new(
                        SearchableVec::new(Vec::<SharedString>::new()),
                        None,
                        window,
                        cx,
                    )
                });
                subscriptions.push(cx.subscribe_in(
                    &state,
                    window,
                    move |this, _, event, window, cx| {
                        let SelectEvent::Confirm(value) = event;
                        if this.model.busy.is_none()
                            && let Some(value) = value
                            && let Some(value) = this.controls[field]
                                .items
                                .iter()
                                .find(|item| &item.title == value)
                                .map(|item| item.value)
                        {
                            this.choose(field, value);
                            this.refresh_controls(window, cx);
                            cx.notify();
                        }
                    },
                ));
                Control {
                    state,
                    items: Vec::new(),
                }
            })
            .collect();
        let label = cx.new(|cx| InputState::new(window, cx));
        subscriptions.push(cx.subscribe(&label, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) && this.model.busy.is_none() {
                this.model.label = input.read(cx).value().to_string();
            }
        }));
        let persistence = cx.new(|_| SliderState::new().min(0.).max(1.).step(1.));
        subscriptions.push(cx.subscribe(&persistence, |this, _, event, cx| {
            if let SliderEvent::Change(value) = event
                && this.model.busy.is_none()
            {
                this.model.persistence_mb =
                    persistence_mb_from_slot(value.start(), this.persistence_max);
                cx.notify();
            }
        }));
        let mut view = Self {
            model: Model::new(),
            controls,
            label,
            persistence,
            persistence_max: 0,
            theme: None,
            blur: crate::backdrop::SUPPORTED,
            columns: false,
            compact: false,
            last_fit: None,
            downloader: None,
            checksums: None,
            picker: None,
            poll_task: None,
            _subscriptions: subscriptions,
        };
        view.refresh_controls(window, cx);
        view
    }

    pub(crate) fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.model.poll();
        self.refresh_controls(window, cx);
        // Retained field entities are never rebuilt by this refresh.
        cx.notify();
    }

    fn fit_content(
        &mut self,
        measured: Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.is_fullscreen() || window.is_maximized() {
            return;
        }
        let viewport = window.viewport_size();
        let measured_width = f32::from(measured.width);
        let expected_width = f32::from(viewport.width).min(if self.columns { 1200. } else { 960. });
        if (measured_width - expected_width).abs() > 2. {
            return;
        }
        let available = window
            .display(cx)
            .map(|display| display.visible_bounds().size)
            .unwrap_or(viewport);
        let fit = fit_form(
            f32::from(measured.height),
            self.last_fit.map_or(f32::from(viewport.width), |size| f32::from(size.width)),
            f32::from(available.width),
            f32::from(available.height),
            self.columns,
            self.compact,
        );
        if self.columns != fit.columns || self.compact != fit.compact {
            self.columns = fit.columns;
            self.compact = fit.compact;
            cx.notify();
        }
        let target = size(px(fit.width), px(fit.height));
        if self.last_fit != Some(target)
            && ((f32::from(viewport.height - target.height)).abs() > 1.
                || (f32::from(viewport.width - target.width)).abs() > 1.)
        {
            self.last_fit = Some(target);
            window.resize(target);
        }
    }

    fn choose(&mut self, field: usize, value: usize) {
        let m = &mut self.model;
        match field {
            DEVICE if m.scan_ready() => m.selected = (value < m.drives.len()).then_some(value),
            BOOT => {
                m.boot = match value {
                    1 => BootSel::FreeDos,
                    2 => BootSel::NonBootable,
                    _ => BootSel::Image,
                };
                match m.boot {
                    BootSel::FreeDos => {
                        (m.fs, m.scheme, m.target) = (Fs::Fat32, Scheme::Mbr, Target::Bios)
                    }
                    BootSel::NonBootable => (m.scheme, m.target) = (Scheme::Mbr, Target::BiosUefi),
                    BootSel::Image => {}
                }
                m.dd = m.image_mode() && !m.report.is_iso;
                if !m.image_mode() {
                    m.label = m.drive().map(|d| d.name.clone()).unwrap_or_default();
                }
            }
            SCHEME if !m.dd => {
                m.scheme = if value == 1 { Scheme::Gpt } else { Scheme::Mbr };
                m.target = if m.scheme == Scheme::Gpt {
                    Target::Uefi
                } else {
                    Target::BiosUefi
                };
            }
            TARGET if !m.dd => m.target = target_value(value),
            FILESYSTEM if !m.dd => {
                if let Some(fs) = Fs::ALL.get(value) {
                    m.fs = *fs;
                    m.cluster = 0;
                }
            }
            CLUSTER if !m.dd => m.cluster = value as u32,
            PASSES if value < image::BAD_BLOCK_MODES.len() => m.passes = value as u8,
            IMAGE_OPTION if m.iso_mode() && m.report.windows => {
                m.wtg = value == 1 && cfg!(windows);
                if m.wtg {
                    m.fs = Fs::Ntfs;
                    m.load_editions();
                }
            }
            _ => {}
        }
    }

    pub(crate) fn show_about(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(target_os = "macos")]
        if crate::about::show().is_ok() {
            return;
        }
        window.open_dialog(cx, |dialog, _, _| {
            dialog
                .title("About Rustfus")
                .close_button(true)
                .child(format!("Rustfus {}", env!("CARGO_PKG_VERSION")))
                .child("GPL-3.0-or-later")
                .child("https://github.com/LOSEARDES77/rustfus")
        });
    }

    pub(crate) fn refresh_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let m = &mut self.model;
        self.controls[DEVICE].sync(
            choices(m.drives.iter().enumerate().map(|(i, d)| (i, d.label()))),
            m.selected,
            window,
            cx,
        );
        let name = m
            .image
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| tf("MSG_281", &[&t("MSG_280")]));
        self.controls[BOOT].sync(
            choices([(0, name), (1, "FreeDOS".into()), (2, t("MSG_279"))]),
            Some(match m.boot {
                BootSel::Image => 0,
                BootSel::FreeDos => 1,
                BootSel::NonBootable => 2,
            }),
            window,
            cx,
        );
        self.controls[SCHEME].sync(
            choices([(0, "MBR".into()), (1, "GPT".into())]),
            Some(usize::from(m.scheme == Scheme::Gpt)),
            window,
            cx,
        );
        let targets = if m.scheme == Scheme::Gpt {
            vec![Target::Uefi]
        } else {
            vec![Target::BiosUefi, Target::Bios, Target::Uefi]
        };
        self.controls[TARGET].sync(
            choices(targets.into_iter().map(|v| (target_index(v), v.name()))),
            Some(target_index(m.target)),
            window,
            cx,
        );
        let big = m.drive().is_some_and(|d| d.size > 4 << 30);
        let default_fs = if m.report.large_file && m.image_mode() {
            Fs::Ntfs
        } else {
            Fs::Fat32
        };
        let fss = choices(
            Fs::ALL
                .iter()
                .enumerate()
                .filter(|(_, f)| f.supported() && !(**f == Fs::Fat && big))
                .filter(|(_, f)| m.boot != BootSel::FreeDos || f.is_fat())
                .map(|(i, f)| {
                    (
                        i,
                        if *f == default_fs {
                            tf("MSG_030", &[f.name()])
                        } else {
                            f.name().into()
                        },
                    )
                }),
        );
        self.controls[FILESYSTEM].sync(fss, Fs::ALL.iter().position(|f| *f == m.fs), window, cx);
        self.controls[CLUSTER].sync(
            choices(
                m.fs.clusters()
                    .into_iter()
                    .map(|v| (v as usize, cluster_name(v))),
            ),
            Some(m.cluster as usize),
            window,
            cx,
        );
        self.controls[PASSES].sync(
            choices((0..image::BAD_BLOCK_MODES.len()).map(|i| (i, bad_block_mode(i)))),
            Some(m.passes as usize),
            window,
            cx,
        );
        let mut image_options = vec![(0, t("MSG_117"))];
        if cfg!(windows) {
            image_options.push((1, t("MSG_118")));
        }
        self.controls[IMAGE_OPTION].sync(
            choices(image_options),
            Some(usize::from(m.wtg)),
            window,
            cx,
        );
        if !self.label.focus_handle(cx).is_focused(window)
            && self.label.read(cx).value().as_ref() != m.label
        {
            self.label
                .update(cx, |input, cx| input.set_value(m.label.clone(), window, cx));
        }
        let max = if m.iso_mode() && m.report.persistence_capable() {
            m.drive()
                .map(|d| d.size.saturating_sub(m.report.size.saturating_add(64 * MB)) / MB)
                .unwrap_or(0)
        } else {
            0
        };
        m.persistence_mb = m.persistence_mb.min(max);
        if m.persistence_mb > 0 {
            m.persistence_mb = m.persistence_mb.max(256.min(max));
        }
        let slot = m.persistence_mb.div_ceil(256) as f32;
        if max != self.persistence_max {
            self.persistence_max = max;
            self.persistence.update(cx, |state, cx| {
                *state = SliderState::new()
                    .min(0.)
                    .max(max.div_ceil(256).max(1) as f32)
                    .step(1.)
                    .default_value(slot);
                cx.notify();
            });
        } else if self.persistence.read(cx).value().start() != slot {
            self.persistence
                .update(cx, |state, cx| state.set_value(slot, window, cx));
        }
    }

    fn open_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.busy.is_some() || self.picker.is_some() {
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select disk image".into()),
        });
        self.picker = Some(cx.spawn_in(window, async move |this, cx| {
            let result = prompt.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.picker = None;
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            this.model.select_image(path);
                        }
                    }
                    Ok(Err(err)) => this.model.dialog = Dialog::Error(err.to_string()),
                    _ => {}
                }
                this.refresh_controls(window, cx);
                cx.notify();
            });
        }));
    }

    fn save_path(&mut self, logs: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.is_some()
            || (!logs && (self.model.busy.is_some() || !self.model.scan_ready()))
        {
            return;
        }
        let directory = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let prompt = cx.prompt_for_new_path(
            &directory,
            Some(if logs { "rustfus.log" } else { "drive.vhd" }),
        );
        self.picker = Some(cx.spawn_in(window, async move |this, cx| {
            let result = prompt.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.picker = None;
                match result {
                    Ok(Ok(Some(path))) => {
                        if logs {
                            if let Err(err) = std::fs::write(path, this.model.log.join("\n")) {
                                this.model.dialog = Dialog::Error(err.to_string());
                            }
                        } else {
                            this.model.save_drive(path);
                        }
                    }
                    Ok(Err(err)) => this.model.dialog = Dialog::Error(err.to_string()),
                    _ => {}
                }
                cx.notify();
            });
        }));
    }

    fn show_checksums(&mut self, cx: &mut Context<Self>) {
        if self.model.busy.is_some() {
            return;
        }
        let Some(path) = self.model.image.clone() else {
            return;
        };
        if let Some((current, handle)) = &self.checksums {
            if current == &path {
                if handle
                    .update(cx, |_, window, _| window.activate_window())
                    .is_ok()
                {
                    return;
                }
            } else {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
        }
        match crate::checksums::open(path.clone(), cx) {
            Ok(handle) => self.checksums = Some((path, handle)),
            Err(error) => self
                .model
                .log
                .push(format!("Could not open Checksums: {error}")),
        }
    }

    fn download(&mut self, cx: &mut Context<Self>) {
        if self.model.busy.is_some() {
            return;
        }
        if let Some(handle) = self.downloader
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let before = cx.windows();
        let parent = cx.entity().downgrade();
        match crate::download::open(cx, move |url, path, cx| {
            let _ = parent.update(cx, |this, cx| {
                this.model.download_iso(url, path);
                cx.notify();
            });
        }) {
            Ok(()) => {
                self.downloader = cx
                    .windows()
                    .into_iter()
                    .find(|handle| !before.contains(handle))
            }
            Err(err) => self.model.dialog = Dialog::Error(err.to_string()),
        }
    }

    fn select(&self, field: usize, label: String, disabled: bool, cx: &App) -> impl IntoElement {
        let leading_icon = match field {
            DEVICE => Some("hard-drive"),
            BOOT => Some("file-image"),
            _ => None,
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap_2()
            .child(label.clone())
            .child(
                div()
                    .relative()
                    .w_full()
                    .child(
                        Select::new(&self.controls[field].state)
                            .w_full()
                            .when(leading_icon.is_some(), |this| this.pl_8())
                            .accessibility_label(label)
                            .disabled(disabled),
                    )
                    .when_some(leading_icon, |this, name| {
                        this.child(
                            div()
                                .absolute()
                                .left_2()
                                .top_0()
                                .h_8()
                                .flex()
                                .items_center()
                                .when(disabled, |this| this.opacity(0.5))
                                .child(icon(name).small().text_color(cx.theme().primary)),
                        )
                    }),
            )
    }

    fn checkbox(
        &self,
        id: &'static str,
        label: String,
        value: bool,
        disabled: bool,
        set: impl Fn(&mut Model, bool) + 'static,
        cx: &Context<Self>,
    ) -> Checkbox {
        Checkbox::new(id)
            .label(label)
            .checked(value)
            .disabled(disabled)
            .on_click(cx.listener(move |this, value: &bool, _, cx| {
                if this.model.busy.is_none() {
                    set(&mut this.model, *value);
                    cx.notify();
                }
            }))
    }
}

impl Render for RustfusView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &self.model;
        let busy = m.busy.is_some();
        let formatting_disabled = busy || m.dd;
        let weak = cx.entity().downgrade();
        let source_weak = weak.clone();
        let source_mode = m.download_mode;
        let source = DropdownButton::new("image-source")
            .menu_accessibility_label("Image source")
            .disabled(busy || m.boot != BootSel::Image || self.picker.is_some())
            .button(
                Button::new("select-image")
                    .label(action_label(&t(if m.download_mode {
                        "MSG_040"
                    } else {
                        "IDC_SELECT"
                    })))
                    .on_click(cx.listener(|this, _, w, cx| {
                        if this.model.download_mode {
                            this.download(cx);
                        } else {
                            this.open_image(w, cx);
                        }
                    })),
            )
            .dropdown_menu(move |mut menu, _, cx| {
                for (mode, label) in [
                    (false, action_label(&t("IDC_SELECT"))),
                    (true, action_label(&t("MSG_040"))),
                ] {
                    let weak = source_weak.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .checked(mode == source_mode)
                            .on_click(move |_, _, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.model.download_mode = mode;
                                    cx.notify();
                                });
                            }),
                    );
                }
                let _ = cx;
                menu
            });
        let mut drive = section_card("drive-properties", self.blur, self.compact, cx)
            .child(
                div()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(t("IDS_DRIVE_PROPERTIES_TXT")),
            )
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(self.select(DEVICE, t("IDS_DEVICE_TXT"), busy || !m.scan_ready(), cx))
                    .child(
                        Button::new("retry")
                            .icon(icon("refresh-cw"))
                            .tooltip(
                                m.scan_error
                                    .clone()
                                    .unwrap_or_else(|| "Refresh devices".into()),
                            )
                            .accessibility_label("Refresh devices")
                            .disabled(busy || m.scanning)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.model.retry_scan();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("save-drive")
                            .icon(icon("hard-drive"))
                            .tooltip(t("MSG_304"))
                            .accessibility_label(t("MSG_304"))
                            .disabled(
                                busy || !m.scan_ready()
                                    || m.drive().is_none()
                                    || self.picker.is_some(),
                            )
                            .on_click(cx.listener(|this, _, w, cx| this.save_path(false, w, cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(self.select(BOOT, t("IDS_BOOT_SELECTION_TXT"), busy, cx))
                    .child(
                        Button::new("hash")
                            .icon(icon("file-image"))
                            .tooltip(t("MSG_314"))
                            .accessibility_label(t("MSG_314"))
                            .disabled(busy || !m.image_mode())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_checksums(cx);
                                cx.notify();
                            })),
                    )
                    .child(source),
            );
        if m.iso_mode() && m.report.windows {
            drive = drive.child(self.select(IMAGE_OPTION, t("IDS_IMAGE_OPTION_TXT"), busy, cx));
            if !cfg!(windows) {
                drive = drive.child(
                    div()
                        .text_sm()
                        .child("Windows To Go requires DISM and bcdboot on Windows."),
                );
            }
        }
        if m.iso_mode() && m.report.persistence_capable() {
            drive = drive.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(format!("{}: {} MB", t("MSG_123"), m.persistence_mb))
                    .child(
                        Slider::new(&self.persistence)
                            .accessibility_label(t("MSG_123"))
                            .disabled(busy || self.persistence_max == 0),
                    ),
            );
        }
        drive = drive
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(self.select(
                        SCHEME,
                        t("IDS_PARTITION_TYPE_TXT"),
                        formatting_disabled,
                        cx,
                    ))
                    .child(self.select(
                        TARGET,
                        t("IDS_TARGET_SYSTEM_TXT"),
                        formatting_disabled,
                        cx,
                    )),
            )
            .child(
                Button::new("advanced-drive")
                    .small()
                    .ghost()
                    .flex_none()
                    .self_start()
                    .icon(icon(if m.adv_drive {
                        "chevron-up"
                    } else {
                        "chevron-down"
                    }))
                    .label(action_label(&t("MSG_119")))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.model.adv_drive = !this.model.adv_drive;
                        cx.notify();
                    })),
            );
        if m.adv_drive {
            drive = drive
                .child(self.checkbox(
                    "all-usb",
                    t("IDC_LIST_USB_HDD"),
                    m.list_all.load(Ordering::Relaxed),
                    busy,
                    |m, v| {
                        m.list_all.store(v, Ordering::Relaxed);
                        m.retry_scan();
                    },
                    cx,
                ))
                .child(self.checkbox(
                    "old-bios",
                    t("IDC_OLD_BIOS_FIXES"),
                    m.old_bios,
                    formatting_disabled,
                    |m, v| m.old_bios = v,
                    cx,
                ))
                .child(self.checkbox(
                    "validation",
                    t("IDC_UEFI_MEDIA_VALIDATION"),
                    m.uefi_validation,
                    busy || !(m.iso_mode() && m.report.efi),
                    |m, v| m.uefi_validation = v,
                    cx,
                ))
                .child(self.checkbox(
                    "verify",
                    "Verify write (DD mode)".into(),
                    m.verify,
                    busy || !m.dd,
                    |m, v| m.verify = v,
                    cx,
                ));
        }
        let mut format = section_card("format-options", self.blur, self.compact, cx)
            .child(
                div()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(t("IDS_FORMAT_OPTIONS_TXT")),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(t("IDS_LABEL_TXT"))
                    .child(Input::new(&self.label).disabled(formatting_disabled)),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(self.select(
                        FILESYSTEM,
                        t("IDS_FILE_SYSTEM_TXT"),
                        formatting_disabled,
                        cx,
                    ))
                    .child(self.select(
                        CLUSTER,
                        t("IDS_CLUSTER_SIZE_TXT"),
                        formatting_disabled,
                        cx,
                    )),
            )
            .child(
                Button::new("advanced-format")
                    .small()
                    .ghost()
                    .flex_none()
                    .self_start()
                    .icon(icon(if m.adv_format {
                        "chevron-up"
                    } else {
                        "chevron-down"
                    }))
                    .label(action_label(&t("MSG_120")))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.model.adv_format = !this.model.adv_format;
                        cx.notify();
                    })),
            );
        if m.adv_format {
            format = format
                .child(self.checkbox(
                    "quick",
                    t("IDC_QUICK_FORMAT"),
                    m.quick,
                    formatting_disabled,
                    |m, v| m.quick = v,
                    cx,
                ))
                .child(self.checkbox(
                    "ext-label",
                    t("IDC_EXTENDED_LABEL"),
                    m.ext_label,
                    formatting_disabled,
                    |m, v| m.ext_label = v,
                    cx,
                ))
                .child(self.checkbox(
                    "bad-blocks",
                    t("IDC_BAD_BLOCKS"),
                    m.bad_blocks,
                    busy,
                    |m, v| m.bad_blocks = v,
                    cx,
                ))
                .child(self.select(PASSES, "Bad block passes".into(), busy || !m.bad_blocks, cx));
        }
        let body = div()
            .flex()
            .when(!self.columns, |this| this.flex_col())
            .flex_shrink_0()
            .gap_3()
            .child(div().flex_1().min_w_0().child(drive))
            .child(div().flex_1().min_w_0().child(format));
        let language_weak = weak.clone();
        let language = Button::new("language")
            .icon(icon("language"))
            .accessibility_label("Language")
            .tooltip(t("MSG_273"))
            .dropdown_menu(move |mut menu, _, _| {
                menu = menu.scrollable(true).max_h(px(360.));
                for language in loc::languages() {
                    let code = language.code.clone();
                    let weak = language_weak.clone();
                    menu = menu.item(
                        PopupMenuItem::new(language.name.clone())
                            .checked(language.code == loc::current())
                            .on_click(move |_, w, cx| {
                                loc::set_language(&code);
                                let _ = weak.update(cx, |this, cx| {
                                    this.refresh_controls(w, cx);
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            });
        let theme_weak = weak.clone();
        let current_theme = self.theme;
        let blur = self.blur;
        let theme = Button::new("theme")
            .icon(icon(if cx.theme().is_dark() { "moon" } else { "sun" }))
            .accessibility_label("Theme")
            .tooltip("Theme")
            .dropdown_menu(move |mut menu, _, _| {
                for (mode, label) in [
                    (None, "System"),
                    (Some(ThemeMode::Light), "Light"),
                    (Some(ThemeMode::Dark), "Dark"),
                ] {
                    let weak = theme_weak.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .checked(mode == current_theme)
                            .on_click(move |_, w, cx| {
                                crate::theme::change_theme(mode, Some(w), cx);
                                let _ = weak.update(cx, |this, cx| {
                                    this.theme = mode;
                                    cx.notify();
                                });
                            }),
                    );
                }
                if crate::backdrop::SUPPORTED {
                    let weak = theme_weak.clone();
                    menu = menu.separator().item(
                        PopupMenuItem::new("Window blur").checked(blur).on_click(
                            move |_, window, cx| {
                                crate::backdrop::apply(!blur, window, cx);
                                let _ = weak.update(cx, |this, cx| {
                                    this.blur = !blur;
                                    cx.notify();
                                });
                            },
                        ),
                    );
                }
                menu
            });
        let update_weak = weak.clone();
        let interval = m.update.interval;
        let betas = m.update.include_betas;
        let checking = m.update_rx.is_some();
        let updates = Button::new("updates")
            .icon(icon("refresh-cw"))
            .accessibility_label("Updates")
            .tooltip(if m.update_note.is_empty() {
                "Updates".into()
            } else {
                m.update_note.clone()
            })
            .dropdown_menu(move |mut menu, _, _| {
                let weak = update_weak.clone();
                menu = menu
                    .item(PopupMenuItem::new("Check now").disabled(checking).on_click(
                        move |_, _, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.model.check_updates(true);
                                cx.notify();
                            });
                        },
                    ))
                    .separator()
                    .label("Check for updates");
                for choice in rustfus_core::update::Interval::ALL {
                    let weak = update_weak.clone();
                    menu = menu.item(
                        PopupMenuItem::new(t(choice.loc_id()))
                            .checked(choice == interval)
                            .on_click(move |_, _, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.model.update.interval = choice;
                                    if let Err(error) = this.model.update.save() {
                                        this.model.dialog = Dialog::Error(error.to_string());
                                    }
                                    cx.notify();
                                });
                            }),
                    );
                }
                let weak = update_weak.clone();
                menu.separator().item(
                    PopupMenuItem::new("Include beta versions")
                        .checked(betas)
                        .on_click(move |_, _, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.model.update.include_betas = !betas;
                                if let Err(error) = this.model.update.save() {
                                    this.model.dialog = Dialog::Error(error.to_string());
                                }
                                cx.notify();
                            });
                        }),
                )
            });
        let titlebar = TitleBar::new()
            .h(px(TITLEBAR_HEIGHT))
            .when(cfg!(target_os = "macos"), |this| this.pl(px(88.)))
            .bg(gpui_kit::transparent_black())
            .border_b_0()
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_sm()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(concat!("Rustfus v", env!("CARGO_PKG_VERSION"))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .flex_shrink_0()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(language)
                    .child(updates)
                    .child(theme),
            );
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("about")
                    .small()
                    .icon(icon("info"))
                    .accessibility_label("About Rustfus")
                    .tooltip(t("MSG_302"))
                    .on_click(cx.listener(|this, _, w, cx| this.show_about(w, cx))),
            )
            .child(
                Button::new("logs")
                    .small()
                    .icon(icon("file"))
                    .accessibility_label("Log")
                    .tooltip(t("MSG_303"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.model.show_log = !this.model.show_log;
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .child(
                Button::new("start")
                    .small()
                    .primary()
                    .label(action_label(&t("IDC_START")))
                    .disabled(!m.can_start())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.model.on_start();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("cancel-job")
                    .small()
                    .label(action_label(&t("MSG_007")))
                    .disabled(!busy)
                    .on_click(cx.listener(|this, _, _, _| this.model.cancel_job())),
            );
        let status = m.progress_label();
        let progress_value = m.progress_value();
        let bar_color = if cx.theme().is_dark() {
            rgb(0x236b1d)
        } else {
            rgb(0x4caf3b)
        };
        let seconds = m
            .started
            .map(|s| s.elapsed())
            .unwrap_or(m.elapsed)
            .as_secs();
        let status_card = section_card("status", self.blur, self.compact, cx)
            .child(
                div()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(t("IDS_STATUS_TXT")),
            )
            .child(
                gpui_kit::base::Progress::new("job-progress")
                    .value(progress_value)
                    .accessibility_label(status.clone())
                    .relative()
                    .h(px(28.))
                    .w_full()
                    .flex_shrink_0()
                    .overflow_hidden()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted)
                    .child(
                        gpui_kit::base::ProgressIndicator::new()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left_0()
                            .w(relative(progress_value / 100.))
                            .bg(bar_color),
                    )
                    .child(
                        div()
                            .relative()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_sm()
                            .text_color(cx.theme().foreground)
                            .px_2()
                            .overflow_hidden()
                            .child(status),
                    ),
            )
            .child(footer)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .child(div().flex_1().min_w_0().truncate().child(
                        if busy && !m.current_file.is_empty() {
                            m.current_file.clone()
                        } else {
                            format!("{} device(s)", m.drives.len())
                        },
                    ))
                    .child(div().flex_shrink_0().child(format!(
                        "{:02}:{:02}:{:02}",
                        seconds / 3600,
                        seconds / 60 % 60,
                        seconds % 60
                    ))),
            );
        let mut view = div()
            .flex()
            .flex_col()
            .w_full()
            .flex_shrink_0()
            .max_w(px(if self.columns { 1200. } else { 960. }))
            .min_w_0()
            .p_3()
            .gap_3()
            .child(body)
            .child(status_card);
        if m.show_log {
            view = view
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new("clear-log")
                                .label("Clear")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.model.log.clear();
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("save-log")
                                .label("Save log")
                                .disabled(self.picker.is_some())
                                .on_click(
                                    cx.listener(|this, _, w, cx| this.save_path(true, w, cx)),
                                ),
                        ),
                )
                .child(
                    div()
                        .id("log-dock")
                        .h(px(160.))
                        .flex_shrink_0()
                        .overflow_y_scroll()
                        .bg(rgb(0x000000))
                        .text_color(rgb(0xffffff))
                        .p_3()
                        .text_sm()
                        .children(
                            m.log
                                .iter()
                                .enumerate()
                                .map(|(i, line)| div().id(i).child(line.clone())),
                        ),
                );
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(crate::backdrop::tint(cx.theme().background, self.blur))
            .text_color(cx.theme().foreground)
            .child(titlebar)
            .child(
                div()
                    .id("main-page")
                    .w_full()
                    .flex_none()
                    .flex()
                    .justify_center()
                    .child(view.on_prepaint({
                        let weak = cx.entity().downgrade();
                        move |bounds, window, cx| {
                            let handle = window.window_handle();
                            cx.defer(move |cx| {
                                let _ = handle.update(cx, |_, window, cx| {
                                    let _ = weak.update(cx, |this, cx| {
                                        this.fit_content(bounds.size, window, cx);
                                    });
                                });
                            });
                        }
                    })),
            )
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                if this.model.busy.is_none()
                    && let Some(path) = paths.0.first()
                {
                    this.model.select_image(path.clone());
                    this.refresh_controls(window, cx);
                    cx.notify();
                }
            }))
    }
}

fn section_card(id: &'static str, blur: bool, compact: bool, cx: &App) -> GroupBox {
    GroupBox::new().id(id).fill().flex_shrink_0().content_style(
        StyleRefinement::default()
            .bg(crate::backdrop::card_surface(cx.theme().group_box, blur))
            .border_1()
            .border_color(cx.theme().border)
            .rounded(px(12.))
            .gap(px(if compact { 8. } else { 12. }))
            .p(px(if compact { 8. } else { 12. })),
    )
}

#[derive(Debug, PartialEq)]
struct FormFit {
    width: f32,
    height: f32,
    columns: bool,
    compact: bool,
}

fn fit_form(
    content: f32,
    width: f32,
    screen_width: f32,
    screen_height: f32,
    columns: bool,
    compact: bool,
) -> FormFit {
    let max_height = (screen_height - 16.).max(320.);
    let desired = content.ceil() + TITLEBAR_HEIGHT + 2.;
    let widen = desired > max_height && !columns && screen_width >= 1040.;
    FormFit {
        width: if widen { 1040. } else { width },
        height: desired.min(max_height).max(480.).min(max_height),
        columns: columns || widen,
        compact: compact || (desired > max_height && !widen),
    }
}

pub(crate) fn action_label(text: &str) -> String {
    let text = text.trim();
    if !text.chars().any(char::is_uppercase) || text.chars().any(char::is_lowercase) {
        return text.into();
    }
    let mut first = true;
    text.to_lowercase()
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
fn target_index(target: Target) -> usize {
    match target {
        Target::BiosUefi => 0,
        Target::Bios => 1,
        Target::Uefi => 2,
    }
}
fn target_value(value: usize) -> Target {
    match value {
        1 => Target::Bios,
        2 => Target::Uefi,
        _ => Target::BiosUefi,
    }
}
fn persistence_mb_from_slot(slot: f32, max: u64) -> u64 {
    (slot.round() as u64)
        .min(max.div_ceil(256))
        .saturating_mul(256)
        .min(max)
}
fn cluster_name(c: u32) -> String {
    match c {
        0 => t("MSG_029"),
        c if c >= 1 << 20 => format!("{} {}", c >> 20, t("MSG_028")),
        c if c >= 1024 => format!("{} {}", c >> 10, t("MSG_027")),
        c => format!("{c} {}", t("MSG_026")),
    }
}
fn bad_block_mode(i: usize) -> String {
    match i {
        0 => tf("MSG_034", &["1"]),
        1 => tf("MSG_035", &["2", ""]).trim_end().into(),
        2..=4 => tf(
            "MSG_035",
            &["4", &tf("MSG_087", &[["SLC", "MLC", "TLC"][i - 2]])],
        ),
        _ => image::BAD_BLOCK_MODES[i].0.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn form_height_fits_the_measured_content() {
        let collapsed = fit_form(640., 600., 1440., 1000., false, false);
        assert_eq!(collapsed.height, 690.);
        assert!(!collapsed.columns);
        let expanded = fit_form(830., 600., 1440., 1000., false, false);
        assert_eq!(expanded.height, 880.);
        let collapsed_again = fit_form(640., 600., 1440., 1000., false, false);
        assert_eq!(collapsed_again.height, collapsed.height);
    }

    #[test]
    fn tall_forms_widen_before_using_compact_spacing() {
        let wide = fit_form(980., 600., 1440., 900., false, false);
        assert_eq!(wide.width, 1040.);
        assert!(wide.columns);
        assert!(!wide.compact);
        assert!(wide.height <= 884.);
        let compact = fit_form(980., 600., 900., 900., false, false);
        assert!(compact.compact);
        assert!(!compact.columns);
    }

    #[test]
    fn action_casing_only_changes_shouting() {
        assert_eq!(action_label("  SELECT  "), "Select");
        assert_eq!(action_label("FreeDOS"), "FreeDOS");
        assert_eq!(action_label("UEFI (non CSM)"), "UEFI (non CSM)");
        assert_eq!(action_label("日本語"), "日本語");
    }
    #[test]
    fn duplicate_titles_keep_distinct_domain_values() {
        let items = choices([(3, "USB".into()), (7, "USB".into())]);
        assert_ne!(items[0].value, items[1].value);
        assert_ne!(items[0].title, items[1].title);
    }
    #[test]
    fn persistence_includes_exact_capacity_and_off() {
        for max in [0, 1, 255, 256, 257, 1024, 1100] {
            assert_eq!(persistence_mb_from_slot(0., max), 0);
            assert_eq!(persistence_mb_from_slot(1., max), 256.min(max));
            assert_eq!(persistence_mb_from_slot(max.div_ceil(256) as f32, max), max);
        }
        assert_eq!(persistence_mb_from_slot(f32::NAN, 1100), 0);
        assert_eq!(persistence_mb_from_slot(f32::INFINITY, 1100), 1100);
    }
}
