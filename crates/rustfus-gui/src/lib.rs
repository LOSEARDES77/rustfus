// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
// Based on Rufus, Copyright © 2011-2026 Pete Batard <pete@akeo.ie> (GPL-3.0-or-later).
//! Rustfus' native GPUI Kit interface.

extern crate gpui_kit as gpui;

#[cfg(target_os = "macos")]
mod about;
mod assets;
mod backdrop;
mod checksums;
mod dialogs;
mod download;
mod model;
mod theme;
mod ui;

use gpui_kit::component::TitleBar;
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Entity, KeyBinding, Menu, MenuItem, TitlebarOptions,
    WeakEntity, WindowBounds, WindowOptions, px, size,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

gpui_kit::actions!(
    rustfus,
    [
        Quit,
        About,
        NewWindow,
        OpenBootable,
        CheckUpdates,
        GithubSource
    ]
);

#[derive(Clone)]
struct MainWindow {
    window: AnyWindowHandle,
    view: WeakEntity<ui::RustfusView>,
}
type MainWindows = Rc<RefCell<Vec<MainWindow>>>;

fn active_main(windows: &MainWindows, cx: &App) -> Option<MainWindow> {
    let windows = windows.borrow();
    let alive = |entry: &&MainWindow| entry.view.upgrade().is_some();
    windows
        .iter()
        .filter(alive)
        .find(|entry| Some(entry.window) == cx.active_window())
        .or_else(|| windows.iter().rev().find(alive))
        .cloned()
}

fn open_main(
    windows: &MainWindows,
    quitting: Rc<Cell<bool>>,
    cx: &mut App,
) -> anyhow::Result<MainWindow> {
    let bounds = Bounds::centered(None, size(px(600.), px(740.)), cx);
    let (window, view) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_background: backdrop::appearance(true),
            window_min_size: Some(size(px(480.), px(480.))),
            titlebar: Some(TitlebarOptions {
                title: Some(concat!("Rustfus v", env!("CARGO_PKG_VERSION")).into()),
                traffic_light_position: Some(gpui_kit::point(px(16.), px(16.))),
                ..TitleBar::title_bar_options()
            }),
            ..TitleBar::window_options()
        },
        cx,
        |window, cx| cx.new(|cx| ui::RustfusView::new(window, cx)),
    )?;
    window.update(cx, |_, window, cx| backdrop::apply(true, window, cx))?;
    let closing = Rc::new(Cell::new(false));
    let close_requested = closing.clone();
    let close_view = view.downgrade();
    window.update(cx, |_, window, cx| {
        window.on_window_should_close(cx, move |_, cx| {
            if let Some(view) = close_view.upgrade()
                && view.read(cx).model.busy.is_some()
            {
                close_requested.set(true);
                view.read(cx).model.cancel_job();
                return false;
            }
            true
        });
    })?;
    let weak = view.downgrade();
    let poll = cx.spawn(async move |cx| {
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
                        view.update(cx, |view, cx| {
                            view.poll(window, cx);
                            if quitting.get() || closing.get() {
                                view.model.cancel_job();
                            }
                        });
                        if (quitting.get() || closing.get()) && view.read(cx).model.busy.is_none() {
                            window.remove_window();
                        } else {
                            dialogs::sync(&view, &mut shown, window, cx);
                        }
                    })
                    .is_ok()
            });
            if !alive {
                break;
            }
        }
    });
    view.update(cx, |view, _| view.poll_task = Some(poll));
    let entry = MainWindow {
        window,
        view: view.downgrade(),
    };
    windows
        .borrow_mut()
        .retain(|entry| entry.view.upgrade().is_some());
    windows.borrow_mut().push(entry.clone());
    cx.activate(true);
    Ok(entry)
}

fn with_main(
    windows: &MainWindows,
    quitting: Rc<Cell<bool>>,
    cx: &mut App,
    action: impl FnOnce(&Entity<ui::RustfusView>, &mut gpui_kit::Window, &mut App),
) {
    let entry = active_main(windows, cx)
        .map(Ok)
        .unwrap_or_else(|| open_main(windows, quitting, cx));
    match entry {
        Ok(entry) => {
            let _ = entry.window.update(cx, |_, window, cx| {
                window.activate_window();
                if let Some(view) = entry.view.upgrade() {
                    action(&view, window, cx);
                }
            });
        }
        Err(error) => eprintln!("Could not open Rustfus: {error}"),
    }
}

pub fn run() -> anyhow::Result<()> {
    let failure = Rc::new(RefCell::new(None));
    let launch_failure = failure.clone();
    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            theme::init(cx);
            let windows: MainWindows = Rc::new(RefCell::new(Vec::new()));
            let quitting = Rc::new(Cell::new(false));
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("cmd-n", NewWindow, None),
                KeyBinding::new("cmd-o", OpenBootable, None),
            ]);
            let registry = windows.clone();
            let quit_flag = quitting.clone();
            cx.on_action(move |_: &Quit, cx| {
                quit_flag.set(true);
                let busy = registry.borrow().iter().any(|entry| {
                    entry.view.upgrade().is_some_and(|view| {
                        view.read(cx).model.cancel_job();
                        view.read(cx).model.busy.is_some()
                    })
                });
                if !busy {
                    cx.quit();
                }
            });
            let registry = windows.clone();
            let flag = quitting.clone();
            cx.on_action(move |_: &NewWindow, cx| {
                let registry = registry.clone();
                let flag = flag.clone();
                cx.defer(move |cx| {
                    if let Err(error) = open_main(&registry, flag, cx) {
                        eprintln!("Could not open Rustfus: {error}");
                    }
                });
            });
            let registry = windows.clone();
            let flag = quitting.clone();
            cx.on_action(move |_: &OpenBootable, cx| {
                let registry = registry.clone();
                let flag = flag.clone();
                cx.defer(move |cx| {
                    with_main(&registry, flag, cx, |view, window, cx| {
                        view.update(cx, |view, cx| view.open_image(window, cx));
                    })
                });
            });
            let registry = windows.clone();
            let flag = quitting.clone();
            cx.on_action(move |_: &CheckUpdates, cx| {
                let registry = registry.clone();
                let flag = flag.clone();
                cx.defer(move |cx| {
                    with_main(&registry, flag, cx, |view, _, cx| {
                        view.update(cx, |view, cx| {
                            view.model.check_updates(true);
                            cx.notify();
                        });
                    })
                });
            });
            let registry = windows.clone();
            let flag = quitting.clone();
            cx.on_action(move |_: &About, cx| {
                let registry = registry.clone();
                let flag = flag.clone();
                cx.defer(move |cx| {
                    #[cfg(target_os = "macos")]
                    {
                        if let Err(error) = about::show() {
                            eprintln!("Could not show About Rustfus: {error}");
                        }
                        let _ = (registry, flag, cx);
                    }
                    #[cfg(not(target_os = "macos"))]
                    with_main(&registry, flag, cx, |view, window, cx| {
                        view.update(cx, |view, cx| view.show_about(window, cx));
                    });
                });
            });
            cx.on_action(|_: &GithubSource, cx| cx.open_url(env!("CARGO_PKG_REPOSITORY")));
            cx.set_menus([
                Menu::new("Rustfus").items([
                    MenuItem::action("About Rustfus", About),
                    MenuItem::separator(),
                    MenuItem::os_submenu("Services", gpui_kit::SystemMenuType::Services),
                    MenuItem::separator(),
                    MenuItem::action("Quit Rustfus", Quit),
                ]),
                Menu::new("File").items([
                    MenuItem::action("New Window", NewWindow),
                    MenuItem::action("Open File (Bootable)…", OpenBootable),
                ]),
                Menu::new("Help").items([
                    MenuItem::action("Check for Updates…", CheckUpdates),
                    MenuItem::action("GitHub Source", GithubSource),
                ]),
            ]);
            let registry = windows.clone();
            let quit_flag = quitting.clone();
            cx.on_window_closed(move |cx, _| {
                registry
                    .borrow_mut()
                    .retain(|entry| entry.view.upgrade().is_some());
                if cx.windows().is_empty() || (quit_flag.get() && registry.borrow().is_empty()) {
                    cx.quit();
                }
            })
            .detach();
            if let Err(error) = open_main(&windows, quitting, cx) {
                *launch_failure.borrow_mut() = Some(error);
                cx.quit();
            }
        });
    failure.borrow_mut().take().map_or(Ok(()), Err)
}
