// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
//! Checksums in a separate native window, independent of the main form's dialogs.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::Duration,
};

use gpui_kit::component::{
    ActiveTheme, Disableable,
    button::Button,
    input::{Input, InputState},
    progress::Progress,
};
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, ClipboardItem, Context, Entity, FocusHandle,
    InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled,
    Task, TitlebarOptions, Window, WindowBounds, WindowOptions, div, px, size,
};
use rustfus_core::image;

use crate::assets::icon;

const ALGORITHMS: [&str; 4] = ["MD5", "SHA-1", "SHA-256", "SHA-512"];

enum Event {
    Progress(f32),
    Done(Result<[String; 4], String>),
}

struct Calculation {
    rx: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    progress: f32,
    result: Option<Result<[String; 4], String>>,
}

impl Calculation {
    fn new(path: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        std::thread::spawn(move || {
            let result = image::hashes(
                &path,
                &|progress| {
                    let _ = tx.send(Event::Progress(progress));
                },
                &worker_cancel,
            )
            .map_err(|error| error.to_string());
            let _ = tx.send(Event::Done(result));
        });
        Self {
            rx,
            cancel,
            progress: 0.,
            result: None,
        }
    }

    fn poll(&mut self) -> bool {
        if self.result.is_some() {
            return false;
        }
        let mut changed = false;
        loop {
            match self.rx.try_recv() {
                Ok(Event::Progress(progress)) => {
                    self.progress = progress.clamp(0., 1.);
                    changed = true;
                }
                Ok(Event::Done(result)) => {
                    if result.is_ok() {
                        self.progress = 1.;
                    }
                    self.result = Some(result);
                    return true;
                }
                Err(TryRecvError::Empty) => return changed,
                Err(TryRecvError::Disconnected) => {
                    self.result = Some(Err("Checksum worker ended without a result".into()));
                    return true;
                }
            }
        }
    }
}

impl Drop for Calculation {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub(crate) fn open(path: PathBuf, cx: &mut App) -> anyhow::Result<AnyWindowHandle> {
    let bounds = Bounds::centered(None, size(px(660.), px(520.)), cx);
    let (window, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(480.), px(460.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Checksums — Rustfus".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        cx,
        |window, cx| cx.new(|cx| Checksums::new(path, window, cx)),
    )?;
    Ok(window)
}

struct Checksums {
    path: PathBuf,
    calculation: Calculation,
    values: [Entity<InputState>; 4],
    focus: FocusHandle,
    _poll: Task<()>,
}

impl Checksums {
    fn new(path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let values = std::array::from_fn(|_| cx.new(|cx| InputState::new(window, cx)));
        let executor = cx.background_executor().clone();
        let poll = cx.spawn_in(window, async move |this, cx| {
            loop {
                executor.timer(Duration::from_millis(100)).await;
                let finished = this.update_in(cx, |this, window, cx| {
                    if this.calculation.poll() {
                        if let Some(Ok(hashes)) = &this.calculation.result {
                            for (field, value) in this.values.iter().zip(hashes) {
                                field.update(cx, |field, cx| {
                                    field.set_value(value.clone(), window, cx);
                                });
                            }
                        }
                        cx.notify();
                    }
                    this.calculation.result.is_some()
                });
                if !matches!(finished, Ok(false)) {
                    break;
                }
            }
        });
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            calculation: Calculation::new(path.clone()),
            path,
            values,
            focus,
            _poll: poll,
        }
    }
}

impl Render for Checksums {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pending = self.calculation.result.is_none();
        let hashes = self
            .calculation
            .result
            .as_ref()
            .and_then(|r| r.as_ref().ok());
        let status = match &self.calculation.result {
            None => format!(
                "Calculating checksums: {:.0}%",
                self.calculation.progress * 100.
            ),
            Some(Ok(_)) => "Checksums calculated".into(),
            Some(Err(error)) => format!("Could not calculate checksums: {error}"),
        };
        let mut body = div()
            .id("checksums-window")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .p_4()
            .gap_3()
            .on_key_down(|event, window, cx| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                    cx.stop_propagation();
                }
            })
            .child(div().text_sm().child(self.path.display().to_string()))
            .child(
                Progress::new("checksum-progress")
                    .value(self.calculation.progress * 100.)
                    .accessibility_label(status.clone()),
            )
            .child(div().text_sm().child(status));
        let mut results = div()
            .id("checksum-results")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_1()
            .gap_3();
        for (index, name) in ALGORITHMS.into_iter().enumerate() {
            let value = hashes.map(|hashes| hashes[index].clone());
            results = results.child(
                div().flex().flex_col().gap_1().child(name).child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(&self.values[index])
                                    .aria_label(name)
                                    .readonly(true),
                            ),
                        )
                        .child(
                            Button::new(("copy-checksum", index))
                                .icon(icon("copy"))
                                .label("Copy")
                                .accessibility_label(format!("Copy {name} checksum"))
                                .disabled(value.is_none())
                                .on_click(move |_, _, cx| {
                                    if let Some(value) = &value {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            value.clone(),
                                        ));
                                    }
                                }),
                        ),
                ),
            );
        }
        body = body.child(results).child(
            div().flex().justify_end().child(
                Button::new("close-checksums")
                    .label(if pending { "Cancel" } else { "Close" })
                    .on_click(|_, window, _| window.remove_window()),
            ),
        );
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (mpsc::Sender<Event>, Calculation) {
        let (tx, rx) = mpsc::channel();
        (
            tx,
            Calculation {
                rx,
                cancel: Arc::new(AtomicBool::new(false)),
                progress: 0.,
                result: None,
            },
        )
    }

    #[test]
    fn progress_and_digests_are_retained_in_the_checksum_window() {
        let (tx, mut calculation) = fixture();
        tx.send(Event::Progress(0.5)).unwrap();
        assert!(calculation.poll());
        assert_eq!(calculation.progress, 0.5);
        let hashes = std::array::from_fn(|i| format!("digest-{i}"));
        tx.send(Event::Done(Ok(hashes.clone()))).unwrap();
        assert!(calculation.poll());
        assert_eq!(calculation.result, Some(Ok(hashes)));
        assert_eq!(calculation.progress, 1.);
        assert!(!calculation.poll());
    }

    #[test]
    fn closing_cancels_work_and_rejects_late_results() {
        let (tx, calculation) = fixture();
        let cancel = calculation.cancel.clone();
        drop(calculation);
        assert!(cancel.load(Ordering::Relaxed));
        assert!(tx.send(Event::Progress(1.)).is_err());
    }

    #[test]
    fn read_errors_and_worker_disconnects_are_not_digest_results() {
        let (tx, mut calculation) = fixture();
        tx.send(Event::Done(Err("File not found".into()))).unwrap();
        assert!(calculation.poll());
        assert_eq!(calculation.result, Some(Err("File not found".into())));
        let (tx, mut calculation) = fixture();
        drop(tx);
        assert!(calculation.poll());
        assert!(matches!(calculation.result, Some(Err(_))));
    }

    #[test]
    fn worker_computes_all_four_digests() {
        let path = std::env::temp_dir().join(format!(
            "rustfus-checksums-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"abc").unwrap();
        let calculation = Calculation::new(path.clone());
        let result = loop {
            match calculation.rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                Event::Progress(_) => {}
                Event::Done(result) => break result,
            }
        };
        std::fs::remove_file(path).unwrap();
        let hashes = result.unwrap();
        assert_eq!(hashes[0], "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hashes[1], "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            hashes[2],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(hashes[3].len(), 128);
    }
}
