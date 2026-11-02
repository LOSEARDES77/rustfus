// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors

use std::{path::PathBuf, rc::Rc, sync::mpsc, time::Duration};

use gpui_kit::component::{
    Disableable, IndexPath,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    select::{SearchableVec, Select, SelectEvent, SelectState},
    spinner::Spinner,
};
use gpui_kit::{
    App, AppContext, Bounds, Context, Entity, FocusHandle, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, Task, TitlebarOptions, WeakEntity, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, size,
};
use rustfus_core::fido;

use crate::assets::icon;

type OnReady = Rc<dyn Fn(String, PathBuf, &mut App)>;

pub(crate) fn open(
    cx: &mut App,
    on_ready: impl Fn(String, PathBuf, &mut App) + 'static,
) -> anyhow::Result<()> {
    let bounds = Bounds::centered(None, size(px(480.), px(650.)), cx);
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

#[derive(Clone, Debug, PartialEq, Eq)]
enum Query {
    Choices(usize),
    Url,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    generation: u64,
    id: u64,
    query: Query,
    chosen: Vec<String>,
}

enum Answer {
    Choices(Vec<String>),
    Url(String),
}

#[derive(Default)]
struct DownloadForm {
    options: [Vec<String>; 5],
    selected: [Option<String>; 5],
    generation: u64,
    request_id: u64,
    pending: Option<Request>,
    failed: Option<Query>,
    error: Option<String>,
    saving: bool,
}

impl DownloadForm {
    fn request(&mut self, query: Query) -> Request {
        self.request_id += 1;
        let count = match query {
            Query::Choices(field) => field,
            Query::Url => 5,
        };
        let request = Request {
            generation: self.generation,
            id: self.request_id,
            query,
            chosen: self.selected[..count]
                .iter()
                .filter_map(Clone::clone)
                .collect(),
        };
        self.pending = Some(request.clone());
        self.failed = None;
        self.error = None;
        request
    }

    fn select(&mut self, field: usize, value: String) -> Option<Request> {
        if self.saving
            || self.selected[field].as_ref() == Some(&value)
            || !self.options[field].contains(&value)
        {
            return None;
        }
        self.generation += 1;
        self.pending = None;
        self.error = None;
        self.failed = None;
        self.selected[field] = Some(value);
        for later in field + 1..5 {
            self.options[later].clear();
            self.selected[later] = None;
        }
        (field < 4).then(|| self.request(Query::Choices(field + 1)))
    }

    fn can_download(&self) -> bool {
        self.selected.iter().all(Option::is_some)
            && self.pending.is_none()
            && self.error.is_none()
            && !self.saving
    }

    fn receive(
        &mut self,
        request: &Request,
        result: Result<Answer, String>,
    ) -> (Option<Request>, Option<String>) {
        // Both coordinates matter: a newer selection can restart the same field.
        if self.pending.as_ref() != Some(request) {
            return (None, None);
        }
        self.pending = None;
        match result {
            Ok(Answer::Choices(options)) if !options.is_empty() => {
                let Query::Choices(field) = request.query else {
                    return (None, None);
                };
                self.selected[field] = options.first().cloned();
                self.options[field] = options;
                let next = (field < 4).then(|| self.request(Query::Choices(field + 1)));
                (next, None)
            }
            Ok(Answer::Url(url)) => (None, Some(url)),
            other => {
                self.error = Some(match other {
                    Err(error) => error,
                    _ => "No download options are available for this selection.".into(),
                });
                self.failed = Some(request.query.clone());
                (None, None)
            }
        }
    }
}

struct Downloader {
    form: DownloadForm,
    choices: Vec<Entity<SelectState<SearchableVec<SharedString>>>>,
    _subscriptions: Vec<Subscription>,
    _poll: Task<()>,
    save: Option<Task<()>>,
    tx: mpsc::Sender<(Request, Result<Answer, String>)>,
    rx: mpsc::Receiver<(Request, Result<Answer, String>)>,
    browser: bool,
    on_ready: Option<OnReady>,
    focus: FocusHandle,
}

impl Downloader {
    fn new(window: &mut Window, cx: &mut Context<Self>, on_ready: OnReady) -> Self {
        let mut subscriptions = Vec::new();
        let choices = (0..5)
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
                        if let Some(value) = value {
                            if let Some(request) = this.form.select(field, value.to_string()) {
                                this.dispatch(request);
                            }
                            this.sync_options(window, cx);
                            cx.notify();
                        }
                    },
                ));
                state
            })
            .collect::<Vec<_>>();
        let executor = cx.background_executor().clone();
        let poll = cx.spawn_in(window, async move |this, cx| {
            loop {
                executor.timer(Duration::from_millis(100)).await;
                if this
                    .update_in(cx, |this, window, cx| {
                        while let Ok((request, answer)) = this.rx.try_recv() {
                            let (next, url) = this.form.receive(&request, answer);
                            if let Some(next) = next {
                                this.dispatch(next);
                            }
                            this.sync_options(window, cx);
                            if let Some(url) = url {
                                if this.browser {
                                    cx.open_url(&url);
                                    this.close(window);
                                    return;
                                }
                                this.choose_path(url, window, cx);
                            }
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let (tx, rx) = mpsc::channel();
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let mut this = Self {
            form: DownloadForm::default(),
            choices,
            _subscriptions: subscriptions,
            _poll: poll,
            save: None,
            tx,
            rx,
            browser: false,
            on_ready: Some(on_ready),
            focus,
        };
        let initial = this.form.request(Query::Choices(0));
        this.dispatch(initial);
        this
    }

    fn dispatch(&self, request: Request) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = match request.query {
                Query::Choices(_) => fido::list(&request.chosen).map(Answer::Choices),
                Query::Url => fido::url(&request.chosen).map(Answer::Url),
            };
            let _ = tx.send((request, result));
        });
    }

    fn sync_options(&self, window: &mut Window, cx: &mut Context<Self>) {
        for (field, choice) in self.choices.iter().enumerate() {
            let items = SearchableVec::new(
                self.form.options[field]
                    .iter()
                    .cloned()
                    .map(SharedString::from)
                    .collect::<Vec<_>>(),
            );
            let selected = self.form.selected[field].as_ref().and_then(|value| {
                self.form.options[field]
                    .iter()
                    .position(|item| item == value)
            });
            choice.update(cx, |state, cx| {
                state.set_items(items, window, cx);
                state.set_selected_index(selected.map(IndexPath::new), window, cx);
            });
        }
    }

    fn choose_path(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        self.form.saving = true;
        let filename = iso_filename(&url);
        let directory = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let prompt = cx.prompt_for_new_path(&directory, Some(&filename));
        self.save = Some(
            cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
                let result = prompt.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.form.saving = false;
                    this.save = None;
                    match result {
                        Ok(Ok(Some(path))) => {
                            if let Some(on_ready) = this.on_ready.take() {
                                on_ready(url, path, cx);
                            }
                            this.close(window);
                        }
                        Ok(Ok(None)) => {}
                        other => {
                            this.form.error = Some(match other {
                                Ok(Err(error)) => error.to_string(),
                                _ => "The save dialog closed unexpectedly.".into(),
                            });
                            this.form.failed = Some(Query::Url);
                        }
                    }
                    cx.notify();
                });
            }),
        );
        cx.notify();
    }

    fn close(&mut self, window: &mut Window) {
        self.form.generation += 1;
        self.form.pending = None;
        self.on_ready = None;
        self.save = None;
        window.remove_window();
    }
}

impl Render for Downloader {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .p_4()
            .gap_4()
            .on_key_down(
                cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, _| {
                    if event.keystroke.key == "escape" {
                        this.close(window);
                    }
                }),
            )
            .child(
                div()
                    .id("download-fields")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .children(self.choices.iter().enumerate().map(|(field, choice)| {
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(fido::FIELDS[field])
                            .child(
                                Select::new(choice).w_full().disabled(
                                    self.form.saving || self.form.options[field].is_empty(),
                                ),
                            )
                    }))
                    .child(
                        Checkbox::new("browser")
                            .label("Download using a browser")
                            .checked(self.browser)
                            .disabled(
                                self.form.saving
                                    || matches!(
                                        self.form.pending.as_ref().map(|r| &r.query),
                                        Some(Query::Url)
                                    ),
                            )
                            .on_click(cx.listener(|this, checked, _, cx| {
                                this.browser = *checked;
                                cx.notify();
                            })),
                    )
                    .when(self.form.pending.is_some() || self.form.saving, |view| {
                        view.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(Spinner::new())
                                .child(if self.form.saving {
                                    "Choose where to save the ISO image."
                                } else {
                                    "Requesting download options…"
                                }),
                        )
                    })
                    .when_some(self.form.error.clone(), |view, error| {
                        view.child(
                            div().flex().flex_col().gap_2().child(error).child(
                                Button::new("retry")
                                    .label("Retry")
                                    .disabled(self.form.failed.is_none() || self.form.saving)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(query) = this.form.failed.clone() {
                                            this.form.generation += 1;
                                            let request = this.form.request(query);
                                            this.dispatch(request);
                                            cx.notify();
                                        }
                                    })),
                            ),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("download")
                            .primary()
                            .label("Download")
                            .icon(icon("download"))
                            .disabled(!self.form.can_download())
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.form.can_download() {
                                    let request = this.form.request(Query::Url);
                                    this.dispatch(request);
                                    cx.notify();
                                }
                            })),
                    )
                    .child(
                        Button::new("back")
                            .label("Back")
                            .on_click(cx.listener(|this, _, window, _| this.close(window))),
                    ),
            )
    }
}

fn iso_filename(url: &str) -> String {
    url.split(['?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit('/')
        .next()
        .filter(|name| name.to_ascii_lowercase().ends_with(".iso") && !name.contains('\\'))
        .unwrap_or("Windows.iso")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn populate(form: &mut DownloadForm) {
        let mut request = form.request(Query::Choices(0));
        for field in 0..5 {
            assert_eq!(request.query, Query::Choices(field));
            let (next, url) = form.receive(
                &request,
                Ok(Answer::Choices(vec![
                    format!("choice-{field}"),
                    format!("alternative-{field}"),
                ])),
            );
            assert!(url.is_none());
            if let Some(next) = next {
                request = next;
            }
        }
    }

    #[test]
    fn all_five_fields_populate_and_download_requires_completion() {
        let mut form = DownloadForm::default();
        assert!(!form.can_download());
        populate(&mut form);
        assert_eq!(
            form.selected,
            std::array::from_fn(|field| Some(format!("choice-{field}")))
        );
        assert!(form.can_download());
        form.saving = true;
        assert!(!form.can_download());
        form.saving = false;
        let request = form.request(Query::Url);
        assert!(!form.can_download());
        form.receive(&request, Err("unavailable".into()));
        assert!(!form.can_download());
    }

    #[test]
    fn earlier_selection_clears_dependents_and_restarts_immediately() {
        let mut form = DownloadForm::default();
        populate(&mut form);
        let request = form.select(1, "alternative-1".into()).unwrap();
        assert_eq!(request.query, Query::Choices(2));
        assert_eq!(request.chosen, vec!["choice-0", "alternative-1"]);
        assert!(form.options[2..].iter().all(Vec::is_empty));
        assert!(form.selected[2..].iter().all(Option::is_none));
        let newer = form.select(0, "alternative-0".into()).unwrap();
        assert_eq!(newer.query, Query::Choices(1));
        assert!(newer.generation > request.generation);
        assert!(!form.can_download());
    }

    #[test]
    fn stale_results_cannot_replace_a_new_generation() {
        let mut form = DownloadForm::default();
        populate(&mut form);
        let old = form.select(0, "alternative-0".into()).unwrap();
        let fresh = form.select(0, "choice-0".into()).unwrap();
        let (next, _) = form.receive(&old, Ok(Answer::Choices(vec!["stale".into()])));
        assert!(next.is_none());
        assert_eq!(form.pending.as_ref(), Some(&fresh));
        assert!(form.options[1].is_empty());
        form.receive(&old, Err("stale error".into()));
        assert!(form.error.is_none());
        form.receive(&fresh, Ok(Answer::Choices(vec!["fresh".into()])));
        assert_eq!(form.selected[1].as_deref(), Some("fresh"));
    }

    #[test]
    fn empty_results_are_retryable() {
        let mut form = DownloadForm::default();
        let request = form.request(Query::Choices(0));
        form.receive(&request, Ok(Answer::Choices(vec![])));
        assert!(form.error.is_some());
        assert_eq!(form.failed, Some(Query::Choices(0)));
        let retry = form.request(Query::Choices(0));
        assert_ne!(request.id, retry.id);
        assert!(form.error.is_none());
    }

    #[test]
    fn suggested_filename_ignores_url_parameters() {
        assert_eq!(
            iso_filename("https://example.test/Windows.iso?token=abc#part"),
            "Windows.iso"
        );
        assert_eq!(
            iso_filename("https://example.test/download?id=42"),
            "Windows.iso"
        );
        assert_eq!(iso_filename("https://example.test/"), "Windows.iso");
    }
}
