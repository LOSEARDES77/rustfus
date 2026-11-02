//! Free content in a component's closure, between its parts (rules,
//! Container): a run of the caller's own widgets stands for the element
//! upstream's demos put there. The run is one item, the component's gap
//! from the parts around it; its widgets stack as that element stacks its
//! children. The alert counts its runs as grid rows and keeps its own
//! bookkeeping.

use egui::{Id, Ui};

/// What a body knows while its closure runs.
#[derive(Clone, Copy, Debug)]
struct Body {
    /// The body's cursor after the last part.
    mark: f32,
    /// The component's gap, between its items.
    gap: f32,
    /// The gap between the widgets of one run.
    run_gap: f32,
    /// The part upstream places last (a footer) is placed.
    closed: bool,
    /// The component, for the order asserts.
    what: &'static str,
}

fn key(ui: &Ui) -> Id {
    ui.id().with("shadcn_egui::free_content::body")
}

/// The cursor moved past `body.mark`: free content since the last part.
fn moved(ui: &Ui, body: &Body) -> bool {
    ui.cursor().top() > body.mark + 0.5 // behaviour-ok: below half a point the cursor did not move
}

/// Run `add` as the body of `what`: its parts ([`section`],
/// [`closing_section`]) are `gap` apart and `gap` from free content, the
/// widgets of one run of free content `run_gap` apart.
pub(crate) fn body<R>(
    ui: &mut Ui,
    gap: f32,
    run_gap: f32,
    what: &'static str,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    ui.spacing_mut().item_spacing.y = run_gap;
    let (key, mark) = (key(ui), ui.cursor().top());
    let body = Body {
        mark,
        gap,
        run_gap,
        closed: false,
        what,
    };
    ui.data_mut(|d| d.insert_temp(key, body));
    let inner = add(ui);
    let body = ui.data_mut(|d| {
        let body = d.get_temp::<Body>(key);
        d.remove::<Body>(key);
        body
    });
    if let Some(body) = body {
        debug_assert!(
            !(body.closed && moved(ui, &body)),
            "free content in a {} comes before its footer",
            body.what
        );
    }
    inner
}

/// Place a part: directly in a [`body`], it is an item `gap` away from
/// the free content before it and from what comes after it; elsewhere
/// `add` runs as it is.
pub(crate) fn section<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    place(ui, false, add)
}

/// [`section`] for the part upstream places last (a footer): free content
/// or a part after it fails a `debug_assert!` (rules, Part).
pub(crate) fn closing_section<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    place(ui, true, add)
}

fn place<R>(ui: &mut Ui, closing: bool, add: impl FnOnce(&mut Ui) -> R) -> R {
    let key = key(ui);
    let Some(body) = ui.data(|d| d.get_temp::<Body>(key)) else {
        return add(ui);
    };
    let free = moved(ui, &body);
    debug_assert!(
        !body.closed,
        "the footer of a {} comes after its other parts and content",
        body.what
    );
    if free {
        // The run's last widget left the run's gap.
        ui.add_space(body.gap - body.run_gap);
    }
    ui.spacing_mut().item_spacing.y = body.gap;
    let inner = add(ui);
    ui.spacing_mut().item_spacing.y = body.run_gap;
    let mark = ui.cursor().top();
    let closed = body.closed || closing;
    ui.data_mut(|d| {
        d.insert_temp(
            key,
            Body {
                mark,
                closed,
                ..body
            },
        )
    });
    inner
}
