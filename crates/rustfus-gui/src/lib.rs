// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Rustfus' native GPUI Kit interface.

extern crate gpui_kit as gpui;

#[cfg(target_os = "macos")]
mod about;
mod assets;
mod dialogs;
mod download;
mod model;
mod theme;
mod ui;

use gpui_kit::{
    AppContext, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds, WindowOptions,
    px, size,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

gpui_kit::actions!(rustfus, [Quit, About]);

pub fn run() -> anyhow::Result<()> {
    let failure = Rc::new(RefCell::new(None));
    let launch_failure = failure.clone();
    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            theme::init(cx);
            cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
            let bounds = Bounds::centered(None, size(px(600.), px(740.)), cx);
            let result = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(480.), px(480.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Rustfus".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| ui::RustfusView::new(window, cx)),
            );
            let (window, view) = match result {
                Ok(opened) => opened,
                Err(error) => {
                    *launch_failure.borrow_mut() = Some(error);
                    cx.quit();
                    return;
                }
            };

            let quit_requested = Rc::new(Cell::new(false));
            let quit_for_action = quit_requested.clone();
            let view_for_action = view.downgrade();
            cx.on_action(move |_: &Quit, cx| {
                let busy = view_for_action
                    .upgrade()
                    .is_some_and(|view| view.read(cx).model.busy.is_some());
                if busy {
                    quit_for_action.set(true);
                    if let Some(view) = view_for_action.upgrade() {
                        cx.update_entity(&view, |view, _| view.model.cancel_job());
                    }
                } else {
                    cx.quit();
                }
            });

            let about_view = view.downgrade();
            cx.on_action(move |_: &About, cx| {
                let _ = window.update(cx, |_, window, cx| {
                    let _ = about_view.update(cx, |view, cx| view.show_about(window, cx));
                });
            });
            cx.set_menus([Menu::new("Rustfus").items([
                MenuItem::action("About Rustfus", About),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", gpui_kit::SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Quit Rustfus", Quit),
            ])]);

            let quit_for_close = quit_requested.clone();
            let view_for_close = view.downgrade();
            let _ = window.update(cx, |_, window, cx| {
                window.on_window_should_close(cx, move |_, cx| {
                    let busy = view_for_close
                        .upgrade()
                        .is_some_and(|view| view.read(cx).model.busy.is_some());
                    if busy {
                        quit_for_close.set(true);
                        if let Some(view) = view_for_close.upgrade() {
                            cx.update_entity(&view, |view, _| view.model.cancel_job());
                        }
                        false
                    } else {
                        true
                    }
                });
            });
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let weak = view.downgrade();
            let quit_for_poll = quit_requested.clone();
            let task = cx.spawn(async move |cx| {
                let mut shown = None;
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                    let alive = cx.update(|cx| {
                        let Some(view) = weak.upgrade() else {
                            return false;
                        };
                        window
                            .update(cx, |_, window, cx| {
                                view.update(cx, |this, cx| this.poll(window, cx));
                                dialogs::sync(&view, &mut shown, window, cx);
                                if quit_for_poll.get() && view.read(cx).model.busy.is_none() {
                                    window.remove_window();
                                }
                            })
                            .is_ok()
                    });
                    if !alive {
                        break;
                    }
                }
            });
            view.update(cx, |view, _| view.poll_task = Some(task));
            cx.activate(true);
        });
    match failure.borrow_mut().take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
