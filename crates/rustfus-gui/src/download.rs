// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors

use std::{path::PathBuf, rc::Rc, time::Duration};

use gpui_kit::{
    App, AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Render, SharedString,
    FocusHandle, Styled, Subscription, Task, TitlebarOptions, WeakEntity, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, size,
};
use gpui_kit::component::{
    Disableable, IndexPath,
    button::{Button, ButtonVariants},
    select::{SearchableVec, Select, SelectEvent, SelectState},
    spinner::Spinner,
};
use rustfus_core::fido;

use crate::{assets::icon, model::FidoState};

type OnReady = Rc<dyn Fn(String, PathBuf, &mut App)>;

pub(crate) fn open(
    cx: &mut App,
    on_ready: impl Fn(String, PathBuf, &mut App) + 'static,
) -> anyhow::Result<()> {
    let bounds = Bounds::centered(None, size(px(440.), px(420.)), cx);
    gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Download ISO image".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        cx,
        |window, cx| cx.new(|cx| Downloader::new(window, cx, Rc::new(on_ready))),
    )?;
    Ok(())
}

struct Downloader {
    fido: FidoState,
    choice: Entity<SelectState<SearchableVec<SharedString>>>,
    options: Vec<String>,
    _selection: Subscription,
    _poll: Task<()>,
    save: Option<Task<()>>,
    on_ready: OnReady,
    focus: FocusHandle,
}

impl Downloader {
    fn new(window: &mut Window, cx: &mut Context<Self>, on_ready: OnReady) -> Self {
        let choice = cx.new(|cx| {
            SelectState::new(SearchableVec::new(Vec::<SharedString>::new()), None, window, cx)
        });
        let selection = cx.subscribe(&choice, |this, _, event, cx| {
            let SelectEvent::Confirm(value) = event;
            if this.fido.rx.is_none() && this.save.is_none() {
                if let Some(index) = value.as_ref().and_then(|value| {
                    this.fido.options.iter().position(|item| item == value.as_ref())
                }) {
                    this.fido.pick = index;
                }
                cx.notify();
            }
        });
        let executor = cx.background_executor().clone();
        let poll = cx.spawn_in(window, async move |this, cx| {
            loop {
                executor.timer(Duration::from_millis(150)).await;
                if this.update_in(cx, |this, window, cx| {
                    let was_loading = this.fido.rx.is_some();
                    let url = this.fido.poll();
                    this.sync_options(window, cx);
                    if let Some(url) = url {
                        this.choose_path(url, window, cx);
                    }
                    if was_loading != this.fido.rx.is_some() {
                        cx.notify();
                    }
                }).is_err() {
                    break;
                }
            }
        });
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            fido: FidoState::new(),
            choice,
            options: Vec::new(),
            _selection: selection,
            _poll: poll,
            save: None,
            on_ready,
            focus,
        }
    }

    fn sync_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.options == self.fido.options {
            return;
        }
        self.options.clone_from(&self.fido.options);
        let items = SearchableVec::new(self.options.iter().cloned().map(SharedString::from).collect::<Vec<_>>());
        self.choice.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(
                (!self.options.is_empty()).then(|| IndexPath::new(self.fido.pick)),
                window,
                cx,
            );
        });
        cx.notify();
    }

    fn choose_path(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        let filename = iso_filename(&url);
        let directory = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let prompt = cx.prompt_for_new_path(&directory, Some(&filename));
        self.save = Some(cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
            let result = prompt.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(Ok(Some(path))) => {
                        this.fido.cancel();
                        (this.on_ready)(url, path, cx);
                        window.remove_window();
                    }
                    other => {
                        // The last selection must be available again after dismissing Save.
                        this.fido.chosen.pop();
                        if let Ok(Err(error)) = other {
                            this.fido.error = Some(error.to_string());
                        } else if other.is_err() {
                            this.fido.error = Some("The save dialog closed unexpectedly.".into());
                        }
                        this.save = None;
                        cx.notify();
                    }
                }
            });
        }));
        cx.notify();
    }

    fn cancel(&mut self, window: &mut Window) {
        self.fido.cancel();
        self.save = None;
        window.remove_window();
    }
}

impl Drop for Downloader {
    fn drop(&mut self) {
        self.fido.cancel();
    }
}

impl Render for Downloader {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.fido.rx.is_some() || self.save.is_some();
        let field = fido::FIELDS.get(self.fido.chosen.len()).copied().unwrap_or("ISO image");
        div()
            .track_focus(&self.focus)
            .flex().flex_col().size_full().p_4().gap_4()
            .on_key_down(cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, _| {
                if event.keystroke.key == "escape" {
                    this.cancel(window);
                }
            }))
            .child(div().child("Select the Windows ISO image to download."))
            .children(self.fido.chosen.iter().enumerate().map(|(index, value)| {
                div().child(format!("{}: {value}", fido::FIELDS[index]))
            }))
            .child(div().flex().flex_col().gap_2()
                .child(field)
                .child(Select::new(&self.choice).w_full().disabled(busy)))
            .when(busy, |view| view.child(div().flex().items_center().gap_2()
                .child(Spinner::new())
                .child(if self.save.is_some() { "Choose where to save the ISO image." } else { "Requesting download options…" })))
            .when(!busy && self.fido.options.is_empty() && self.fido.error.is_none(), |view| {
                view.child("No download options are available.")
            })
            .when_some(self.fido.error.clone(), |view, error| view.child(div().child(error)))
            .child(div().flex_1())
            .child(div().flex().gap_2().justify_end()
                .child(Button::new("cancel").label("Cancel")
                    .on_click(cx.listener(|this, _, window, _| this.cancel(window))))
                .child(Button::new("back").label("Back")
                    .disabled(busy || self.fido.chosen.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| { this.fido.back(); cx.notify(); })))
                .child(Button::new("continue").primary()
                    .label(if self.fido.error.is_some() { "Retry" } else if self.fido.chosen.len() + 1 == fido::FIELDS.len() { "Download" } else { "Continue" })
                    .icon(icon("download"))
                    .disabled(busy || (self.fido.options.is_empty() && self.fido.error.is_none()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.fido.error.is_some() { this.fido.query(); } else { this.fido.next(); }
                        cx.notify();
                    }))))
    }
}

fn iso_filename(url: &str) -> String {
    url.split(['?', '#']).next().unwrap_or_default()
        .rsplit('/').next()
        .filter(|name| name.to_ascii_lowercase().ends_with(".iso") && !name.contains('\\'))
        .unwrap_or("Windows.iso").to_owned()
}

#[cfg(test)]
mod tests {
    use super::iso_filename;

    #[test]
    fn suggested_filename_ignores_url_parameters() {
        assert_eq!(iso_filename("https://example.test/Windows.iso?token=abc#part"), "Windows.iso");
        assert_eq!(iso_filename("https://example.test/download?id=42"), "Windows.iso");
        assert_eq!(iso_filename("https://example.test/"), "Windows.iso");
    }
}
