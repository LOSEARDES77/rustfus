// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! GPUI component dialogs for the model's START sequence and status messages.

use crate::model::{Dialog, Model};
use crate::ui::RustfusView;
use gpui_kit::component::{
    IndexPath, WindowExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    radio::Radio,
    select::{SearchableVec, Select, SelectState},
};
use gpui_kit::{
    App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window,
    div, prelude::FluentBuilder, px,
};
use rustfus_core::loc::{t, tf};
use rustfus_core::{syslinux, update::Version};
use std::cell::Cell;

/// Identity of an opening, not of its live progress or editable choices.
pub(crate) fn key(dialog: &Dialog) -> String {
    match dialog {
        Dialog::None => "none".into(),
        Dialog::Download => "download".into(),
        Dialog::SyslinuxDownload(version) => format!("syslinux:{version}"),
        Dialog::ReplaceC32(index) => format!("replace-c32:{index}"),
        Dialog::Edition => "edition".into(),
        Dialog::Wue => "wue".into(),
        Dialog::Hybrid { .. } => "hybrid".into(),
        Dialog::Confirm => "confirm".into(),
        Dialog::NewVersion(release) => format!("release:{}:{}", release.version, release.url),
        Dialog::Error(message) => format!("error:{message}"),
    }
}

fn title(dialog: &Dialog, model: &Model) -> String {
    match dialog {
        Dialog::None => String::new(),
        Dialog::Download | Dialog::SyslinuxDownload(_) => t("MSG_115"),
        Dialog::ReplaceC32(index) => {
            let paths = model.obsolete_c32();
            let path = paths.get(*index).map(String::as_str).unwrap_or("");
            tf("MSG_083", &[path.rsplit('/').next().unwrap_or(path)])
        }
        Dialog::Edition => t("MSG_118"),
        Dialog::Wue => t("MSG_327"),
        Dialog::Hybrid { .. } => tf("MSG_274", &["ISOHybrid"]),
        Dialog::Confirm => "Warning".into(),
        Dialog::NewVersion(_) => t("IDD_NEW_VERSION").replace("Rufus", "Rustfus"),
        Dialog::Error(_) => t("MSG_042"),
    }
}

/// Call after polling the model, outside the parent's entity update/render.
/// `shown` belongs to the caller and starts at `None`.
pub(crate) fn sync(
    parent: &Entity<RustfusView>,
    shown: &mut Option<String>,
    window: &mut Window,
    cx: &mut App,
) {
    let dialog = parent.read(cx).model.dialog.clone();
    let next = (!matches!(dialog, Dialog::None)).then(|| key(&dialog));
    if *shown == next {
        return;
    }
    if shown.take().is_some() {
        window.close_dialog(cx);
    }
    *shown = next;
    if !matches!(dialog, Dialog::None) {
        open(parent.downgrade(), dialog, window, cx);
    }
}

/// The builder retains this entity and its input state for the whole opening.
pub(crate) fn open(
    parent: WeakEntity<RustfusView>,
    dialog: Dialog,
    window: &mut Window,
    cx: &mut App,
) {
    if matches!(dialog, Dialog::None) {
        return;
    }
    let Some(entity) = parent.upgrade() else {
        return;
    };
    if key(&entity.read(cx).model.dialog) != key(&dialog) {
        return;
    }
    let heading = title(&dialog, &entity.read(cx).model);
    let expected = key(&dialog);
    let content = cx.new(|cx| DialogView::new(parent.clone(), expected.clone(), window, cx));
    let focus = content.read(cx).cancel_focus.clone();
    let initial_focus = Cell::new(true);
    window.open_dialog(cx, move |surface, window, cx| {
        if initial_focus.replace(false) {
            focus.focus(window, cx);
        }
        let parent = parent.clone();
        let expected = expected.clone();
        surface
            .title(heading.clone())
            .width(px(560.0))
            .close_button(true)
            .overlay_closable(false)
            // Enter must never approve a destructive operation from dialog-level focus.
            .on_ok(|_, _, _| false)
            .child(content.clone())
            .on_close(move |_, window, cx| {
                if let Some(parent) = parent.upgrade() {
                    parent.update(cx, |view, cx| {
                        if key(&view.model.dialog) == expected {
                            view.model.cancel_dialog();
                            view.refresh_controls(window, cx);
                            cx.notify();
                        }
                    });
                }
            })
    });
}

pub(crate) struct DialogView {
    parent: WeakEntity<RustfusView>,
    expected: String,
    username: Entity<InputState>,
    edition: Entity<SelectState<SearchableVec<SharedString>>>,
    edition_ids: Vec<u32>,
    cancel_focus: FocusHandle,
    _parent_observer: Subscription,
    _username_subscription: Subscription,
}

impl DialogView {
    fn new(
        parent: WeakEntity<RustfusView>,
        expected: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let entity = parent
            .upgrade()
            .expect("dialog parent exists while opening");
        let (username, edition_ids, selected, editions) = {
            let model = &entity.read(cx).model;
            let edition_ids = model.editions.iter().map(|(id, _)| *id).collect::<Vec<_>>();
            let selected = edition_ids.iter().position(|id| *id == model.edition);
            let editions = model
                .editions
                .iter()
                .map(|(_, label)| SharedString::from(label.clone()))
                .collect::<Vec<_>>();
            (model.wue_user.clone(), edition_ids, selected, editions)
        };
        let username = cx.new(|cx| InputState::new(window, cx).default_value(username));
        let username_subscription = cx.subscribe(&username, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) && this.expected == key(&Dialog::Wue) {
                let value = input.read(cx).value().to_string();
                this.change(cx, |model| model.wue_user = value);
            }
        });
        let edition = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(editions),
                selected.map(IndexPath::new),
                window,
                cx,
            )
            .searchable(true)
        });
        let observer = cx.observe(&entity, |_, _, cx| cx.notify());
        Self {
            parent,
            expected,
            username,
            edition,
            edition_ids,
            cancel_focus: cx.focus_handle(),
            _parent_observer: observer,
            _username_subscription: username_subscription,
        }
    }

    fn change(&self, cx: &mut App, change: impl FnOnce(&mut Model)) {
        if let Some(parent) = self.parent.upgrade() {
            parent.update(cx, |view, cx| {
                if key(&view.model.dialog) == self.expected {
                    change(&mut view.model);
                    cx.notify();
                }
            });
        }
    }

    fn respond(&self, approve: Option<bool>, window: &mut Window, cx: &mut App) {
        let Some(parent) = self.parent.upgrade() else {
            return;
        };
        if key(&parent.read(cx).model.dialog) != self.expected {
            return;
        }
        let username = self.username.read(cx).value().to_string();
        let edition = self
            .edition
            .read(cx)
            .selected_index(cx)
            .and_then(|index| self.edition_ids.get(index.row).copied());
        // Close the old surface before advancing; it must not close the next stage.
        window.close_dialog(cx);
        parent.update(cx, |view, cx| {
            if key(&view.model.dialog) != self.expected {
                return;
            }
            let model = &mut view.model;
            if approve == Some(true) {
                match model.dialog {
                    Dialog::Wue => model.wue_user = username,
                    Dialog::Edition => {
                        let Some(edition) = edition else {
                            model.cancel_dialog();
                            return;
                        };
                        model.edition = edition;
                    }
                    _ => {}
                }
            }
            match approve {
                Some(yes) => model.approve(yes),
                None => model.cancel_dialog(),
            }
            view.refresh_controls(window, cx);
            cx.notify();
        });
    }

    fn cancel(&self, label: String, cx: &Context<Self>) -> Button {
        Button::new("cancel-dialog")
            .label(label)
            .track_focus(&self.cancel_focus)
            .on_click(cx.listener(|this, _, window, cx| this.respond(None, window, cx)))
    }

    fn approve(&self, label: String, destructive: bool, cx: &Context<Self>) -> Button {
        Button::new("approve-dialog")
            .label(label)
            .when(destructive, |button| button.danger())
            .when(!destructive, |button| button.primary())
            .on_click(cx.listener(|this, _, window, cx| this.respond(Some(true), window, cx)))
    }

    fn checkbox(
        &self,
        id: &'static str,
        checked: bool,
        label: String,
        set: fn(&mut Model, bool),
        cx: &Context<Self>,
    ) -> Checkbox {
        Checkbox::new(id)
            .checked(checked)
            .label(label)
            .on_click(cx.listener(move |this, checked, _, cx| {
                this.change(cx, |model| set(model, *checked));
            }))
    }
}

impl Render for DialogView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = div().flex().flex_col().gap_3().w_full().min_w_0();
        let Some(parent) = self.parent.upgrade() else {
            return body;
        };
        let (dialog, wue, use_account, windows_11, wtg, drive_name, download_message) = {
            let model = &parent.read(cx).model;
            if key(&model.dialog) != self.expected {
                return body;
            }
            let message = match &model.dialog {
                Dialog::Download => tf("MSG_116", &[&model.report.grub2_version, "2.14"]),
                Dialog::SyslinuxDownload(version) => {
                    tf("MSG_114", &[version, "", &syslinux::embedded_version(), ""])
                        .replace("Rufus", "Rustfus")
                }
                Dialog::ReplaceC32(index) => {
                    let paths = model.obsolete_c32();
                    let path = paths.get(*index).map(String::as_str).unwrap_or("");
                    let name = path.rsplit('/').next().unwrap_or(path);
                    tf("MSG_084", &[name, name])
                }
                _ => String::new(),
            };
            (
                model.dialog.clone(),
                model.wue.clone(),
                model.wue_use_account,
                model.report.windows_11,
                model.wtg,
                model
                    .confirmation_drive
                    .as_ref()
                    .map(|drive| drive.label())
                    .unwrap_or_default(),
                message,
            )
        };
        let mut footer = div().flex().justify_end().gap_2().mt_2();
        match dialog {
            Dialog::None => return body,
            Dialog::Download | Dialog::SyslinuxDownload(_) | Dialog::ReplaceC32(_) => {
                body = body.child(download_message);
                footer = footer
                    .child(self.cancel(crate::ui::action_label(&t("MSG_007")), cx))
                    .child(
                        Button::new("decline-dialog")
                            .label(crate::ui::action_label(&t("MSG_009")))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.respond(Some(false), window, cx)
                            })),
                    )
                    .child(self.approve(crate::ui::action_label(&t("MSG_008")), false, cx));
            }
            Dialog::Edition => {
                body = body
                    .child("Please select the edition you want to install:")
                    .child(Select::new(&self.edition).w_full());
                footer = footer
                    .child(self.cancel(crate::ui::action_label(&t("IDCANCEL")), cx))
                    .child(self.approve(crate::ui::action_label(&t("IDOK")), false, cx));
            }
            Dialog::Wue => {
                body = body.child(t("MSG_328"));
                if windows_11 {
                    body = body
                        .child(self.checkbox(
                            "hardware",
                            wue.bypass_hw_checks,
                            t("MSG_329"),
                            |m, v| m.wue.bypass_hw_checks = v,
                            cx,
                        ))
                        .child(self.checkbox(
                            "online-account",
                            wue.no_online_account,
                            t("MSG_330"),
                            |m, v| m.wue.no_online_account = v,
                            cx,
                        ));
                }
                body = body
                    .child(self.checkbox(
                        "local-account",
                        use_account,
                        t("MSG_333"),
                        |m, v| m.wue_use_account = v,
                        cx,
                    ))
                    .child(Input::new(&self.username).disabled(!use_account))
                    .child(self.checkbox(
                        "regional",
                        wue.regional,
                        t("MSG_334"),
                        |m, v| m.wue.regional = v,
                        cx,
                    ))
                    .child(self.checkbox(
                        "data-collection",
                        wue.no_data_collection,
                        t("MSG_331"),
                        |m, v| m.wue.no_data_collection = v,
                        cx,
                    ))
                    .child(self.checkbox(
                        "bitlocker",
                        wue.no_bitlocker,
                        t("MSG_335"),
                        |m, v| m.wue.no_bitlocker = v,
                        cx,
                    ));
                if wtg {
                    body = body.child(self.checkbox(
                        "internal-drives",
                        wue.wtg_no_internal,
                        t("MSG_332"),
                        |m, v| m.wue.wtg_no_internal = v,
                        cx,
                    ));
                }
                footer = footer
                    .child(self.cancel(crate::ui::action_label(&t("IDCANCEL")), cx))
                    .child(self.approve(crate::ui::action_label(&t("IDOK")), false, cx));
            }
            Dialog::Hybrid { dd } => {
                let iso = t("MSG_036");
                let dd_label = t("MSG_095");
                body = body.child(tf("MSG_275", &[&iso, &dd_label, &iso, &dd_label]));
                for (value, id, label) in [
                    (false, "iso-mode", tf("MSG_276", &[&iso])),
                    (true, "dd-mode", tf("MSG_277", &[&dd_label])),
                ] {
                    body = body.child(Radio::new(id).checked(dd == value).label(label).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.change(cx, |model| model.dialog = Dialog::Hybrid { dd: value });
                        }),
                    ));
                }
                footer = footer
                    .child(self.cancel(crate::ui::action_label(&t("IDCANCEL")), cx))
                    .child(self.approve(crate::ui::action_label(&t("IDOK")), false, cx));
            }
            Dialog::Confirm => {
                body = body.child(tf("MSG_003", &[&drive_name]));
                footer = footer
                    .child(self.cancel(crate::ui::action_label(&t("IDCANCEL")), cx))
                    .child(self.approve(crate::ui::action_label(&t("IDOK")), true, cx));
            }
            Dialog::NewVersion(release) => {
                body = body.child(t("IDS_NEW_VERSION_AVAIL_TXT")).child(format!(
                    "Rustfus {} → {}",
                    Version::current(),
                    release.version
                ));
                if !release.notes.trim().is_empty() {
                    body = body.child(t("IDS_NEW_VERSION_NOTES_GRP")).child(
                        div()
                            .id("release-notes")
                            .max_h(px(200.0))
                            .overflow_y_scroll()
                            .child(release.notes.trim().to_string()),
                    );
                }
                footer = footer
                    .child(self.cancel(crate::ui::action_label(&t("IDCANCEL")), cx))
                    .child(
                        Button::new("download-release")
                            .primary()
                            .label(crate::ui::action_label(&t("IDC_DOWNLOAD")))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(parent) = this.parent.upgrade()
                                    && key(&parent.read(cx).model.dialog) == this.expected
                                {
                                    cx.open_url(&release.url);
                                    this.respond(None, window, cx);
                                }
                            })),
                    );
            }
            Dialog::Error(message) => {
                #[cfg(target_os = "macos")]
                let privacy_error = rustfus_core::macos_auth::is_disk_privacy_error(&message);
                body = body.child(message);
                #[cfg(target_os = "macos")]
                if privacy_error {
                    footer = footer.child(
                        Button::new("open-macos-privacy")
                            .label("Open Privacy Settings")
                            .on_click(|_, _, cx| {
                                cx.open_url(rustfus_core::macos_auth::PRIVACY_SETTINGS_URL);
                            }),
                    );
                }
                footer = footer.child(self.cancel(crate::ui::action_label(&t("IDOK")), cx));
            }
        }
        body.child(footer)
    }
}

#[cfg(test)]
mod tests {
    use super::key;
    use crate::model::Dialog;

    #[test]
    fn live_changes_keep_the_same_opening() {
        assert_eq!(
            key(&Dialog::Hybrid { dd: false }),
            key(&Dialog::Hybrid { dd: true })
        );
    }

    #[test]
    fn sequential_prompts_and_changed_errors_are_distinct() {
        assert_ne!(key(&Dialog::ReplaceC32(0)), key(&Dialog::ReplaceC32(1)));
        assert_ne!(
            key(&Dialog::SyslinuxDownload("6.03".into())),
            key(&Dialog::SyslinuxDownload("6.04".into()))
        );
        assert_ne!(
            key(&Dialog::Error("first".into())),
            key(&Dialog::Error("second".into()))
        );
        assert_ne!(key(&Dialog::Confirm), key(&Dialog::None));
    }
}
