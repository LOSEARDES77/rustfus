//! Flexbox substitutes for egui's single-pass layout.
//! `gap` maps to item_spacing; space-between right-aligns via a nested layout.

use crate::components::context;
use egui::Ui;
use egui_extras::{Size, StripBuilder};

/// A row height or stack width change below this is float noise, not a
/// reason to redo a pass.
const ROW_HEIGHT_EPSILON: f32 = 0.01; // behaviour-ok: float tolerance, not a design token

/// Whether two widths are the same up to [`ROW_HEIGHT_EPSILON`]. Equal
/// infinite widths (an item holding a widget that fills unbounded room)
/// are the same, where their difference is NaN.
fn same_width(a: f32, b: f32) -> bool {
    a == b || (a - b).abs() <= ROW_HEIGHT_EPSILON
}

/// How much narrower epaint reports text at a finite width than it
/// measured unwrapped: it wraps at that width rounded to whole points
/// (plus this, so it never wraps earlier) and reports the galley no
/// wider than the rounded width (`Fonts::layout_job`,
/// `Galley::round_output_to_gui`).
const TEXT_WIDTH_ROUNDING: f32 = 0.5; // behaviour-ok: epaint's wrap-width rounding, not a design token

/// Whether a pass of `ui` may replace what a layout helper stored from
/// an earlier pass (`stored`). A sizing pass (the first frame of an
/// `egui::Area` or a `Grid`, or one the app forces) lays out in a room and
/// an alignment of its own, invisibly: it fills a slot nothing filled
/// yet, so that the visible pass it discards into starts from it, but it
/// never replaces what a visible pass measured, nor redoes a pass that is
/// not shown.
fn may_store(ui: &Ui, stored: bool) -> bool {
    !(stored && ui.is_sizing_pass())
}

/// Horizontal stack with an explicit gap (flex-direction: row; gap: N;
/// align-items: center).
///
/// The row is as high as its tallest child, never egui's `interact_size`
/// (conformance 3.3). egui centres each child in the row height so far,
/// so a child taller than the children before it would leave them above
/// centre. The row therefore starts from the content height of the last
/// pass (0 in its first pass), and asks egui to redo a pass whose content
/// height changed.
///
/// A widget that fills the available height (a `ScrollArea`) takes the
/// row's current height as its own, so a row that holds one does not
/// shrink after a taller child goes; a `ScrollArea` grows the row to its
/// full content height. Content that registers no
/// widget (shapes painted directly, `allocate_space`) is not measured.
///
/// A child wider than the room left pushes the children after it past its
/// whole width, and the row takes no more than its room in the parent, so
/// the overflow does not widen the parent: the browser's flex row. A plain
/// `ui.horizontal` cannot do that (see [`allocate_overflowing`]).
///
/// A child that is as high as the row's other children (`self-stretch`,
/// as `separator_vertical`) takes its height from [`stretch_height`].
pub fn row<R>(ui: &mut Ui, gap: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let id = ui.next_auto_id().with("shadcn_egui::layout::row");
    let (stored, height, align) = row_band(ui, id);
    let layout = if ui.layout().prefer_right_to_left() {
        egui::Layout::right_to_left(align)
    } else {
        egui::Layout::left_to_right(align)
    };
    // The content height: the tallest item of the row, measured from the
    // widgets registered while `add` runs (see `row_lines`), so it does not
    // depend on the row's own height.
    let size = egui::vec2(ui.available_size_before_wrap().x, height);
    let (inner, lines) = overflow_row(ui, size, layout, None, |_, ui| {
        let items = RowItems::begin(ui);
        ui.spacing_mut().item_spacing.x = gap;
        let inner = shrink_stacks(ui, |ui| {
            let inner = add(ui);
            (inner, ui.min_rect().width())
        });
        (inner, items.finish(ui, false))
    });
    settle_row_band(
        ui,
        id,
        stored,
        height,
        &lines,
        "layout::row content height changed",
    );
    inner
}

/// The height a content-height row (one line: [`row`], [`space_between`],
/// a card footer) lays its children out in, stored under `id`: what it
/// stored, its content height of the last pass (0 in its first pass), and
/// the cross alignment of its items. The first pass, redone once it has
/// measured the content, puts its items at the top of the empty band:
/// egui moves an item taller than the band down to the row's top, and
/// that move turns an item that a filling egui widget pushed to an
/// infinite place (a `Separator` in a [`vstack`] measuring its content)
/// into a NaN rect.
pub(crate) fn row_band(ui: &Ui, id: egui::Id) -> (Option<f32>, f32, egui::Align) {
    let stored = ui.data(|d| d.get_temp::<f32>(id));
    let align = if stored.is_some() {
        egui::Align::Center
    } else {
        egui::Align::Min
    };
    (stored, stored.unwrap_or(0.0), align)
}

/// Store the content height `lines` measured for the row under `id` that
/// laid out in `band` ([`row_band`]), and redo the pass with `why` where
/// it differs from `band`.
pub(crate) fn settle_row_band(
    ui: &Ui,
    id: egui::Id,
    stored: Option<f32>,
    band: f32,
    lines: &[Line],
    why: &'static str,
) {
    let measured = lines.iter().map(|l| l.height).fold(0.0, f32::max);
    if may_store(ui, stored.is_some())
        && stored.is_none_or(|h| (h - measured).abs() > ROW_HEIGHT_EPSILON)
    {
        ui.data_mut(|d| d.insert_temp(id, measured));
        if (band - measured).abs() > ROW_HEIGHT_EPSILON {
            ui.ctx().request_discard(why);
        }
    }
}

fn stretch_id(row: egui::Id) -> egui::Id {
    row.with("shadcn_egui::layout::stretch")
}

fn stretch_pass_id(row: egui::Id) -> egui::Id {
    row.with("shadcn_egui::layout::stretch_pass")
}

/// What a crate row gives its stretch items in a pass and learns about
/// them: the height the previous pass found for each [`stretch_height`]
/// call, in call order; the place on the layer of the next entry after
/// each call, which marks the item that asked; and the `Ui`s whose
/// children are the row's items: `roots` by the `id` their children name
/// as parent, `skip` the unique ids of the `Ui`s between a root and the
/// row, which are no items themselves, and of the pass's hit areas,
/// which take no part in the row's layout.
#[derive(Clone, Debug, Default)]
struct Stretch {
    heights: Vec<f32>,
    marks: Vec<usize>,
    roots: Vec<egui::Id>,
    skip: Vec<egui::Id>,
}

/// One line of a crate row's items in a pass: the height of its tallest
/// item and of its first.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Line {
    height: f32,
    first: f32,
}

/// A crate row's pass over its items ([`row`], [`wrap_row`],
/// [`space_between`], a card or dialog footer), for [`stretch_height`]:
/// begun on the row's `Ui` before its items, finished after them.
pub(crate) struct RowItems {
    /// The row `Ui`'s unique id, which [`flex_row_of`] finds.
    row: egui::Id,
    layer: egui::LayerId,
    before: usize,
    stored: Option<Vec<f32>>,
}

impl RowItems {
    pub(crate) fn begin(ui: &Ui) -> Self {
        let (row, layer) = (ui.unique_id(), ui.layer_id());
        let before = ui
            .ctx()
            .viewport(|v| v.this_pass.widgets.get_layer(layer).count());
        let stored = ui.data(|d| d.get_temp::<Vec<f32>>(stretch_id(row)));
        let pass = Stretch {
            heights: stored.clone().unwrap_or_default(),
            roots: vec![ui.id()],
            ..Stretch::default()
        };
        ui.data_mut(|d| d.insert_temp(stretch_pass_id(row), pass));
        Self {
            row,
            layer,
            before,
            stored,
        }
    }

    /// Ends the pass: keeps each stretch item's height for the next pass
    /// (the tallest other item of its line) and redoes the pass where one
    /// changed. Gives the row's lines: one, or where the row `wraps` one
    /// per line its items were placed on.
    pub(crate) fn finish(self, ui: &Ui, wraps: bool) -> Vec<Line> {
        let pass = ui
            .data_mut(|d| d.remove_temp::<Stretch>(stretch_pass_id(self.row)))
            .unwrap_or_default();
        let (added, end) = ui.ctx().viewport(|v| {
            let layer = v.this_pass.widgets.get_layer(self.layer);
            let added: Vec<(usize, egui::WidgetRect)> = layer
                .enumerate()
                .skip(self.before)
                .map(|(i, w)| (i, *w))
                .collect();
            let end = self.before + added.len();
            (added, end)
        });
        // A hit area takes no part in the row's layout.
        let mut pass = pass;
        pass.skip.extend(crate::components::shared::hit_areas_among(
            ui.ctx(),
            added.iter().map(|(_, w)| w.id),
        ));
        let (lines, heights) = row_lines(&added, end, &pass, wraps);
        let changed = self.stored.as_ref().is_none_or(|old| {
            old.len() != heights.len()
                || old
                    .iter()
                    .zip(&heights)
                    .any(|(a, b)| (a - b).abs() > ROW_HEIGHT_EPSILON)
        });
        if changed && may_store(ui, self.stored.is_some()) {
            ui.data_mut(|d| d.insert_temp(stretch_id(self.row), heights));
            if !pass.marks.is_empty() {
                ui.ctx()
                    .request_discard("layout: a row's stretch height changed");
            }
        }
        lines
    }
}

/// The lines of a crate row's items from the entries `added` on its layer
/// in a pass (each with its place on the layer, `end` the place after the
/// last), and the height for each of the pass's stretch marks: the
/// tallest other item of the mark's line.
fn row_lines(
    added: &[(usize, egui::WidgetRect)],
    end: usize,
    pass: &Stretch,
    wraps: bool,
) -> (Vec<Line>, Vec<f32>) {
    // An item is a child of a root: a leaf widget, or a Ui (a scope,
    // `add_enabled_ui`, a nested `vstack`) that counts as the union of the
    // leaf widgets inside it, not as its own rect. A Ui's own rect cannot
    // serve: in a centred row egui makes it as high as the row, so a row
    // holding one would never shrink. Neither can the union of all
    // widgets: a nested `vstack` sits at the top while a widget beside it
    // is centred, and the union grew on every pass.
    //
    // Rebuild the tree from the creation order: a Ui registers before what
    // it holds, and an entry names its Ui's `id` as parent, not the
    // `unique_id` the Ui registered under. So an entry whose next entry
    // names a parent not on the stack opens a Ui.
    //
    // A frame (egui's `Frame`, a card) registers the `Ui`s of its content,
    // then the box around them: the box is an item of its own on the line
    // of its content, and the line and a stretch item beside it are as
    // high as the box.
    let mut stack = vec![pass.roots[0]];
    // Each item's first place on the layer and its rect.
    let mut items: Vec<(usize, egui::Rect)> = Vec::new();
    for (i, (at, w)) in added.iter().enumerate() {
        while stack.len() > 1 && stack.last() != Some(&w.parent_id) {
            stack.pop();
        }
        let skipped = pass.skip.contains(&w.id);
        if !skipped && stack.last().is_some_and(|p| pass.roots.contains(p)) {
            items.push((*at, egui::Rect::NOTHING));
        }
        match added.get(i + 1).map(|(_, n)| n.parent_id) {
            Some(p) if !stack.contains(&p) => stack.push(p),
            _ if !skipped => {
                if let Some((_, c)) = items.last_mut() {
                    *c = c.union(w.rect);
                }
            }
            _ => {}
        }
    }
    // An item starts a line when it is placed below the line so far.
    let mut line_of = vec![0; items.len()];
    let mut lines: Vec<Line> = Vec::new();
    let mut bottom = f32::NEG_INFINITY;
    for (k, (_, r)) in items.iter().enumerate() {
        if !r.is_finite() {
            line_of[k] = lines.len().saturating_sub(1);
            continue;
        }
        if lines.is_empty() || (wraps && r.top() >= bottom) {
            lines.push(Line {
                height: 0.0,
                first: r.height(),
            });
            bottom = f32::NEG_INFINITY;
        }
        bottom = bottom.max(r.bottom());
        line_of[k] = lines.len() - 1;
        let line = lines.last_mut().expect("a line was pushed");
        line.height = line.height.max(r.height());
    }
    // A stretch item is the item that holds the first entry registered
    // after its mark.
    let item_at = |m: usize| {
        (0..items.len()).find(|&k| {
            let next = items.get(k + 1).map_or(end, |c| c.0);
            (items[k].0..next).contains(&m)
        })
    };
    let marked: Vec<Option<usize>> = pass.marks.iter().map(|&m| item_at(m)).collect();
    let stretches = |k: usize| marked.contains(&Some(k));
    let tallest_other = |line: Option<usize>| {
        (0..items.len())
            .filter(|&k| !stretches(k) && line.is_none_or(|l| line_of[k] == l))
            .map(|k| items[k].1.height())
            .fold(0.0, f32::max)
    };
    let heights = marked
        .iter()
        .map(|k| tallest_other(k.map(|k| line_of[k])))
        .collect();
    (lines, heights)
}

/// Makes the children of `ui` items of the crate row `ui` sits in, through
/// `Ui`s of the crate's own (`space_between`'s right side, a card or dialog
/// footer's text scope), so that [`stretch_height`] counts them one by
/// one.
pub(crate) fn hold_row_items(ui: &Ui) {
    let mut skip = Vec::new();
    for s in ui.stack().iter() {
        if context::on_frame::<RowContext>(s).is_some() {
            let root = ui.id();
            ui.data_mut(|d| {
                if let Some(mut pass) = d.get_temp::<Stretch>(stretch_pass_id(s.id)) {
                    pass.roots.push(root);
                    pass.skip.extend(skip);
                    d.insert_temp(stretch_pass_id(s.id), pass);
                }
            });
            return;
        }
        skip.push(s.id);
    }
}

/// The height of a `self-stretch` flex item: in a crate row ([`row`],
/// [`space_between`], a card footer, a dialog footer from `sm` up), the
/// height of the row's tallest other child in the previous pass (a
/// vertical rule beside small text is as high as the text); in a
/// [`wrap_row`], the tallest other child of the item's own line. The
/// child that calls this before it draws counts as a stretch item, and
/// stretch items do not count as other children; where the row's other
/// children change height, the row redoes the pass.
///
/// `None` where the item gives no height: in a column (a [`vstack`], a
/// vertical `Ui`), in a box of its own on the way to the row (a frame, a
/// scroll area, a `StripBuilder` or table cell), outside a crate row, and
/// in the row's first pass. The item then takes its own minimum.
///
/// ```
/// # egui::__run_test_ui(|ui| {
/// shadcn_egui::layout::row(ui, 8.0, |ui| {
///     ui.label("Ctrl");
///     let height = shadcn_egui::layout::stretch_height(ui).unwrap_or(0.0);
///     ui.allocate_exact_size(egui::vec2(1.0, height), egui::Sense::hover());
///     ui.label("K");
/// });
/// # });
/// ```
#[doc(alias = "self-stretch")]
#[must_use]
pub fn stretch_height(ui: &Ui) -> Option<f32> {
    let row = flex_row_of(ui)?;
    let layer = ui.layer_id();
    let at = ui
        .ctx()
        .viewport(|v| v.this_pass.widgets.get_layer(layer).count());
    ui.data_mut(|d| {
        let mut pass = d.get_temp::<Stretch>(stretch_pass_id(row))?;
        let height = pass.heights.get(pass.marks.len()).copied();
        pass.marks.push(at);
        d.insert_temp(stretch_pass_id(row), pass);
        height
    })
}

/// Horizontal stack that wraps onto further lines when it runs out of width
/// (flex-direction: row; flex-wrap: wrap; gap: N; align-items: center).
/// The gap applies both between items on a line and between wrapped lines.
/// A [`vstack`] item wraps as a whole, as a flex item does, and so does an
/// egui `Label`: egui text in a `wrap_row` does not wrap, except in a
/// [`vstack`] too wide for a whole line.
///
/// Each line starts from its height in the last pass, so every item is
/// centred on its line and a line is as high as its tallest item, as in a
/// [`row`]; where a line's height changes, the row redoes the pass. The
/// crate starts a line this way before a crate widget or a [`vstack`] that
/// does not fit the line before.
///
/// Limits, where egui gives the crate no say between two egui widgets:
/// - A line that a plain egui widget starts (an egui `Label`, `Button`,
///   `allocate_space`) keeps the height of the line before it where that
///   is taller, and centres its items in that height.
/// - A plain egui `Label` longer than a whole line overflows the line;
///   the browser would shrink it to the line and wrap its text.
pub fn wrap_row<R>(ui: &mut Ui, gap: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    // As `horizontal_wrapped`, but each line is as high as its tallest item:
    // egui's own starts every line `interact_size.y` high, which puts more
    // than `gap` between lines of smaller items (badges).
    let id = ui.next_auto_id().with("shadcn_egui::layout::wrap_row");
    let stored = ui.data(|d| d.get_temp::<Vec<f32>>(id));
    let heights = stored.clone().unwrap_or_default();
    let band = heights.first().copied().unwrap_or(0.0);
    let layout = if ui.layout().prefer_right_to_left() {
        egui::Layout::right_to_left(egui::Align::Center)
    } else {
        egui::Layout::left_to_right(egui::Align::Center)
    };
    let size = egui::vec2(ui.available_size_before_wrap().x, band);
    let wrap = WrapContext(ui.wrap_mode());
    let row = layout.with_main_wrap(true);
    let (inner, lines, started) = overflow_row(ui, size, row, Some(wrap), |_, ui| {
        let key = wrap_lines_id(ui.unique_id());
        let before = ui
            .ctx()
            .viewport(|v| v.this_pass.widgets.get_layer(ui.layer_id()).count());
        let pass = WrapLines {
            heights,
            started: vec![(0, band)],
            before,
        };
        ui.data_mut(|d| d.insert_temp(key, pass));
        let items = RowItems::begin(ui);
        ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        let inner = add(ui);
        let pass = ui
            .data_mut(|d| d.remove_temp::<WrapLines>(key))
            .unwrap_or_default();
        (inner, items.finish(ui, true), pass.started)
    });
    let measured: Vec<f32> = lines.iter().map(|l| l.height).collect();
    let changed = stored.as_ref().is_none_or(|old| {
        old.len() != measured.len()
            || old
                .iter()
                .zip(&measured)
                .any(|(a, b)| (a - b).abs() > ROW_HEIGHT_EPSILON)
    });
    if changed && may_store(ui, stored.is_some()) {
        ui.data_mut(|d| d.insert_temp(id, measured));
        // egui grows the line to each item as it comes, so a line that
        // started lower than it ends places its items as one that started
        // at its height only when the first item is the tallest.
        let misplaced = started.iter().any(|&(k, band)| {
            lines.get(k).is_some_and(|l| {
                band > l.height + ROW_HEIGHT_EPSILON
                    || (band < l.height - ROW_HEIGHT_EPSILON
                        && l.first < l.height - ROW_HEIGHT_EPSILON)
            })
        });
        if misplaced {
            ui.ctx()
                .request_discard("layout::wrap_row line height changed");
        }
    }
    inner
}

/// The context on the `Ui` of a [`wrap_row`]: the text wrap mode of the
/// `Ui` around it, which a [`vstack`] too wide for a whole line gives its
/// text back.
#[derive(Clone, Copy)]
struct WrapContext(egui::TextWrapMode);

/// What a [`wrap_row`] knows about its lines in a pass: each line's height
/// in the previous pass, the lines that started at it (line and height),
/// and the place on the layer of the row's first item.
#[derive(Clone, Debug, Default)]
struct WrapLines {
    heights: Vec<f32>,
    started: Vec<(usize, f32)>,
    before: usize,
}

fn wrap_lines_id(row: egui::Id) -> egui::Id {
    row.with("shadcn_egui::layout::wrap_lines")
}

/// Whether the next item of the wrapping `ui` starts its line.
fn at_line_start(ui: &Ui) -> bool {
    let (cursor, max) = (ui.cursor(), ui.max_rect());
    if ui.layout().prefer_right_to_left() {
        cursor.max.x >= max.max.x
    } else {
        cursor.min.x <= max.min.x
    }
}

/// Where `ui` is a [`wrap_row`] and an item `width` wide does not fit the
/// rest of its line, starts the next line for it (see [`start_line`]).
/// The crate's allocation helpers ([`allocate_item`],
/// [`allocate_item_space`], [`allocate_overflowing`]) call this before
/// they allocate, so that the crate, not egui, wraps a crate widget.
fn wrap_item(ui: &mut Ui, width: f32) {
    if context::on_frame::<WrapContext>(ui.stack()).is_some() {
        wrap_line(ui, width);
    }
}

/// Where an item `width` wide does not fit the rest of the line of the
/// wrapping `ui`, starts the next line for it (see [`start_line`]): the
/// one line break of [`wrap_item`], [`as_flex_item`] and [`begin_block`].
fn wrap_line(ui: &mut Ui, width: f32) {
    if !at_line_start(ui) && width > ui.available_size_before_wrap().x {
        start_line(ui);
    }
}

/// The width of a whole line where `ui` is a [`wrap_row`]: the most a flex
/// item shrinks to there, so crate text longer than it wraps at it.
pub(crate) fn wrap_line_width(ui: &Ui) -> Option<f32> {
    context::on_frame::<WrapContext>(ui.stack()).map(|_| ui.max_rect().width())
}

/// The width crate text in `ui` wraps at outside a [`wrap_row`], as egui's
/// `Label` wraps there: none where the `Ui` does not wrap text, a whole
/// line in a wrapping horizontal layout (egui moves a box too wide for the
/// rest of the line to the next), else the room left.
pub(crate) fn text_wrap_width(ui: &Ui) -> Option<f32> {
    match ui.wrap_mode() {
        egui::TextWrapMode::Extend => None,
        _ if ui.layout().is_horizontal() && ui.layout().main_wrap() => Some(ui.max_rect().width()),
        _ => Some(ui.available_width()),
    }
}

/// Whether an item added to `ui` is a flex item of a crate row that
/// shrinks as a [`vstack`] does there ([`row`], [`space_between`], a card
/// footer; not a [`wrap_row`], whose items wrap whole).
pub(crate) fn in_flex_row(ui: &Ui) -> bool {
    !ui.layout().main_wrap() && flex_row_of(ui).is_some()
}

/// Ends the line of the wrapping `ui` and, in a [`wrap_row`], starts the
/// next at its height in the previous pass: egui would start it at least
/// as high as the line before.
fn start_line(ui: &mut Ui) {
    ui.end_row();
    let key = wrap_lines_id(ui.unique_id());
    let Some(mut pass) = ui.data(|d| d.get_temp::<WrapLines>(key)) else {
        return;
    };
    // The new line's number: the lines the row's items so far are on.
    let parent = ui.id();
    let line = ui.ctx().viewport(|v| {
        let mut bottom = f32::NEG_INFINITY;
        let mut lines = 0;
        for w in v
            .this_pass
            .widgets
            .get_layer(ui.layer_id())
            .skip(pass.before)
            .filter(|w| w.parent_id == parent && w.rect.is_finite())
        {
            if w.rect.top() >= bottom {
                lines += 1;
                bottom = f32::NEG_INFINITY;
            }
            bottom = bottom.max(w.rect.bottom());
        }
        lines
    });
    let band = pass.heights.get(line).copied().unwrap_or(0.0);
    ui.set_row_height(band);
    pass.started.push((line, band));
    ui.data_mut(|d| d.insert_temp(key, pass));
}

/// The context on the `Ui` of a crate row ([`row`], [`wrap_row`],
/// [`space_between`], a card or dialog footer), where
/// [`allocate_overflowing`]
/// allocates a widget's whole width.
#[derive(Clone, Copy)]
struct RowContext;

/// The `UiBuilder` of a crate row's `Ui`: `max_rect` and `layout`, tagged
/// as a row. The caller allocates only the part of the row's `min_rect`
/// that fits its room, so an overflowing child pushes its siblings without
/// widening the parent.
pub(crate) fn row_builder(max_rect: egui::Rect, layout: egui::Layout) -> egui::UiBuilder {
    egui::UiBuilder::new()
        .max_rect(max_rect)
        .layout(layout)
        .ui_stack_info(context::info(egui::UiStackInfo::default(), RowContext))
}

/// The context on the `Ui` of a [`vstack`]: a flex column, which grows to
/// the width of its widest child inside a crate row.
#[derive(Clone, Copy)]
struct VstackContext;

/// Whether `ui` is a crate row, or sits inside one through horizontal
/// `Ui`s (a scope, an enabled scope) and [`vstack`]s: a flex item of the
/// row, which the browser grows to its content's minimum width. A
/// `StripBuilder` or table cell on the way keeps its width, as a sized box
/// does in a flex row; an egui `Frame` grows with its content, as a flex
/// item does.
fn in_row(ui: &Ui) -> bool {
    for s in ui.stack().iter() {
        if context::on_frame::<RowContext>(s).is_some() {
            return true;
        }
        if s.kind() == Some(egui::UiKind::TableCell)
            || (!s.layout_direction.is_horizontal()
                && !context::on_frame::<VstackContext>(s).is_some())
        {
            return false;
        }
    }
    false
}

/// A crate row of `size` in `layout`, placed as `allocate_ui_with_layout`
/// places it. `add` gets the `Ui` that holds the room and the row's own
/// `Ui`, built inside it with [`row_builder`] (and the `wrap` context of
/// a [`wrap_row`]). The row's children may run
/// past the room; the room takes the row's `min_rect` cut to its own
/// width, and the parent allocates that. A row that is itself a flex item
/// of a crate row (in a [`vstack`] in a row) is not cut: the outer row
/// takes the overflow, as the browser grows a flex item to its content.
fn overflow_row<R>(
    ui: &mut Ui,
    size: egui::Vec2,
    layout: egui::Layout,
    wrap: Option<WrapContext>,
    add: impl FnOnce(&Ui, &mut Ui) -> R,
) -> R {
    ui.allocate_ui_with_layout(size, layout, |room| {
        let max = room.max_rect();
        let mut builder = row_builder(max, layout);
        if let Some(wrap) = wrap {
            let info = builder.ui_stack_info.clone();
            builder = builder.ui_stack_info(context::inherited_info(info, wrap));
        }
        let mut row = room.new_child(builder);
        let inner = add(room, &mut row);
        // Dropping the row registers its `min_rect`.
        let used = row.min_rect();
        drop(row);
        let grows = in_row(room);
        let x = |v: f32| {
            if grows {
                v
            } else {
                v.clamp(max.left(), max.right())
            }
        };
        room.advance_cursor_after_rect(egui::Rect::from_x_y_ranges(
            x(used.left())..=x(used.right()),
            used.y_range(),
        ));
        inner
    })
    .inner
}

/// Vertical stack with an explicit gap (flex-direction: column; gap: N).
///
/// As an item of a [`wrap_row`] it behaves as a flex item: its text takes
/// its widest, unwrapped width, and the stack starts a new line when that
/// width does not fit the rest of the line. egui wraps only an item whose
/// size it knows before the item draws, so the stack takes its width from
/// the previous pass and redoes a pass when it changes. Alone on a line too
/// narrow for it, its text wraps (flex-shrink).
///
/// Inside a crate row ([`row`], [`wrap_row`], [`space_between`], a card
/// footer) it grows to its widest child, as a flex item does: a child
/// raised past the room ([`allocate_overflowing`]) widens the stack and
/// pushes the row's next children, and the row keeps the overflow from
/// its own parent. Outside a crate row the child overflows the stack.
///
/// In a [`row`], [`space_between`], a card footer or a dialog footer from
/// `sm` up the stack shrinks as a flex item (`flex-shrink: 1`,
/// `min-width: auto`): its text keeps its
/// widest, unwrapped width while the row has room for it, and wraps at the
/// width the row leaves after its other children when it has not. Several
/// stacks in one row share the shortfall in proportion to their unwrapped
/// widths, none below its narrowest (its longest word or widest child).
/// A stack that fits is laid out at its unwrapped width, so a child that
/// fills (`w-full`: a horizontal separator) spans the stack. The widths
/// come from the previous pass, and a pass that changes them is redone;
/// where the content of a stack that fits changes, the pass is redone
/// unwrapped, so its text never shows wrapped at the old width. When the stack's content takes another size or paints
/// another text or image at the same width (its text, image, font or
/// scale changed), the next frame measures it unwrapped again in a pass
/// that is not shown.
///
/// In a `StripBuilder` or table cell (directly, or through scopes that
/// keep the cell's layout) the stack takes the cell's width, as a block in
/// a grid track or a sized flex item fills it, and its text wraps there.
pub fn vstack<R>(ui: &mut Ui, gap: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let id = ui.next_auto_id().with("shadcn_egui::layout::vstack");
    let add = |ui: &mut Ui| {
        ui.spacing_mut().item_spacing.y = gap;
        add(ui)
    };
    match as_flex_item(ui, id, false, add) {
        Ok(inner) => inner,
        Err(add) => {
            let fill = in_cell(ui);
            ui.scope_builder(flex_column(), |ui| {
                if fill {
                    ui.set_min_width(ui.available_width());
                }
                add(ui)
            })
            .inner
        }
    }
}

/// `ui.vertical`, tagged as a flex column: the `Ui` of a [`vstack`], and
/// of a box laid out with [`as_flex_item`].
fn flex_column() -> egui::UiBuilder {
    egui::UiBuilder::new()
        .layout(egui::Layout::top_down(egui::Align::Min))
        .ui_stack_info(context::info(egui::UiStackInfo::default(), VstackContext))
}

/// Lays `add` out as a flex item of the crate row or [`wrap_row`] `ui`
/// is in, as a [`vstack`] is (see there), in a column that keeps what it
/// learns under `id`; outside such a row, hands `add` back unrun.
///
/// With `unbounded`, a column that fits its [`wrap_row`] line is laid
/// out without a width limit, as a [`row`] measures its items, so a box
/// that fills its room (a card) takes its content width there.
///
/// A content-sized container (a card: no width upstream) is laid out
/// with this; a `w-full` container (an alert) with [`begin_block`].
pub(crate) fn as_flex_item<R, F: FnOnce(&mut Ui) -> R>(
    ui: &mut Ui,
    id: egui::Id,
    unbounded: bool,
    add: F,
) -> Result<R, F> {
    if !ui.layout().main_wrap() {
        let Some(row) = flex_row_of(ui) else {
            return Err(add);
        };
        return Ok(flex_item(ui, row, id, flex_column(), add));
    }
    let stored = ui.data(|d| d.get_temp::<f32>(id));
    if let Some(w) = stored {
        wrap_line(ui, w);
    }
    let fits = stored.is_none_or(|w| w <= ui.available_size_before_wrap().x);
    let wrap = context::on_frame::<WrapContext>(ui.stack()).map(|w| w.0);
    let response = ui.scope_builder(flex_column(), |ui| {
        if fits {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            if unbounded {
                ui.set_max_width(f32::INFINITY);
            }
        } else if let Some(mode) = wrap {
            ui.style_mut().wrap_mode = Some(mode);
        }
        add(ui)
    });
    // Only an unwrapped pass measures the stack's own width.
    let width = response.response.rect.width();
    if fits && may_store(ui, stored.is_some()) && stored.is_none_or(|w| !same_width(w, width)) {
        ui.data_mut(|d| d.insert_temp(id, width));
        ui.ctx()
            .request_discard("layout::vstack width changed in a wrap_row");
    }
    Ok(response.inner)
}

/// The crate row whose flex item a [`vstack`] started in `ui` is: the
/// row's `Ui` unique id, reached through horizontal `Ui`s (a scope, an enabled
/// scope, `space_between`'s right side), not through a [`vstack`], a
/// vertical `Ui` or a box of its own (a `StripBuilder` or table cell, a
/// frame, a scroll area: any `Ui` of a [`egui::UiKind`]), whose children
/// are not the row's items.
fn flex_row_of(ui: &Ui) -> Option<egui::Id> {
    for s in ui.stack().iter() {
        if context::on_frame::<RowContext>(s).is_some() {
            return Some(s.id);
        }
        if s.kind().is_some()
            || !s.layout_direction.is_horizontal()
            || context::on_frame::<VstackContext>(s).is_some()
        {
            return None;
        }
    }
    None
}

/// Whether a [`vstack`] started in `ui` is the content of a `StripBuilder`
/// or table cell: `ui` is the cell, or reaches it through scopes that keep
/// their parent's layout and are no crate row or stack.
fn in_cell(ui: &Ui) -> bool {
    let mut s = ui.stack();
    loop {
        match s.kind() {
            Some(egui::UiKind::TableCell) => return true,
            Some(_) => return false,
            None => {}
        }
        let Some(parent) = s.parent.as_ref() else {
            return false;
        };
        if !s.tags().0.is_empty() || s.layout_direction != parent.layout_direction {
            return false;
        }
        s = parent;
    }
}

/// Reports a leaf widget that is a flex item of the crate row `ui` sits in
/// (see [`in_flex_row`]; crate text) to the row as a [`vstack`] item is
/// reported: its unwrapped width `basis` and its narrowest width `min`
/// (its longest word). Gives the width the row's last plan gives it where
/// the row shrinks its items, `None` where they fit or `ui` is no such row.
/// The widget takes that width, or `basis` without one.
pub(crate) fn flex_leaf(ui: &Ui, basis: f32, min: impl FnOnce() -> f32) -> Option<f32> {
    if ui.layout().main_wrap() {
        return None;
    }
    let row = flex_row_of(ui)?;
    let min = min();
    let index = ui.data(|d| {
        d.get_temp::<Vec<FlexItem>>(flex_items_id(row))
            .map_or(0, |items| items.len())
    });
    let target = ui
        .data(|d| d.get_temp::<Vec<Option<f32>>>(flex_plan_id(row)))
        .and_then(|plan| plan.get(index).copied().flatten())
        .map(|w| w.max(min));
    ui.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<FlexItem>>(flex_items_id(row))
            .push(FlexItem {
                width: target.unwrap_or(basis),
                basis,
                min,
            });
    });
    target
}

/// What a row learns about one of its [`vstack`] items in a pass.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct FlexItem {
    /// The width it took.
    width: f32,
    /// Its unwrapped width (CSS max-content), the flex base size.
    basis: f32,
    /// Its narrowest width (CSS min-content), as far as known: what it
    /// overflowed to at a smaller width, else 0.
    min: f32,
}

/// What a [`vstack`] keeps of its own content between passes.
#[derive(Clone, Copy, Debug, Default)]
struct Intrinsic {
    /// The width it took unwrapped.
    max: Option<f32>,
    /// The width it overflowed to when made narrower (exact), or lowered
    /// to the width it wrapped to (an upper bound).
    min: Option<f32>,
    /// The last pass at a planned width: that width, the scale, the
    /// content's size and the hash of the text and images it painted
    /// ([`content_hash`]). The same width and scale giving another size or
    /// hash means the content or its font changed, and `max` and `min`
    /// with it, also where the wrapped block kept its size.
    seen: Option<([f32; 4], u64)>,
    /// The last unwrapped pass: the content's size and the scale. A stack that fits is laid out at its
    /// unwrapped width, where its content must have this size.
    unwrapped: Option<(egui::Vec2, f32)>,
    /// Set when `seen` or `unwrapped` no longer match: the next pass
    /// measures the content again.
    stale: bool,
}

impl Intrinsic {
    /// Records a pass laid out at `w`, the unwrapped width `max` holds,
    /// whose content had `size` and painted `hash` ([`content_hash`]) at
    /// the scale `ppp`. Such a pass must find the content as large as
    /// the unwrapped pass did and paint what the last pass at this width
    /// painted, unless the content, its font or the scale changed: then
    /// the memo turns stale and this returns `true`, so the caller
    /// discards the pass and the next one measures again, as text may
    /// wrap at the old width.
    fn fitted_pass(&mut self, w: f32, ppp: f32, size: egui::Vec2, hash: u64) -> bool {
        let seen = ([w, ppp, size.x, size.y], hash);
        // Its text may report up to `TEXT_WIDTH_ROUNDING` narrower than
        // unwrapped, at the same height.
        let changed = self.unwrapped.is_some_and(|(old, p)| {
            p != ppp
                || size.x > old.x + ROW_HEIGHT_EPSILON
                || size.x < old.x - TEXT_WIDTH_ROUNDING - ROW_HEIGHT_EPSILON
                || (old.y - size.y).abs() > ROW_HEIGHT_EPSILON
        }) || self
            .seen
            .is_some_and(|s| s.0[..2] == seen.0[..2] && s != seen);
        self.seen = Some(seen);
        self.stale |= changed;
        changed
    }
}

/// A hash of what decides the unwrapped width of the content painted on
/// `layer` from shape `first` on: each galley's text and per section its
/// byte range, font and letter spacing (not its colour, which a hover
/// changes), each glyph's advance (the theme swaps a font's data under
/// the same `FontId`), and each image's texture, its size and the part
/// (uv) drawn.
/// A shrunk image of another intrinsic size, or an icon atom (an image
/// per icon) swapped for another, can draw at the same size.
///
/// Only `layer` counts: what the stack shows on another layer (a popup,
/// a tooltip) is out of flow, as an absolutely positioned box is in CSS,
/// and never sizes the stack. Shapes without a texture (an icon a crate
/// widget paints as paths, a frame) draw at a size their widget fixes,
/// so the content's size already tells a change.
///
/// The hash covers what one pass painted, so it is compared only between
/// passes at the same planned width: a pass that paints nothing (egui's
/// invisible sizing pass of an `Area`) hashes as empty content.
fn content_hash(ctx: &egui::Context, layer: egui::LayerId, first: usize) -> u64 {
    use std::hash::{Hash, Hasher};
    fn image(
        id: egui::TextureId,
        uv: impl Iterator<Item = egui::Pos2>,
        h: &mut impl Hasher,
        images: &mut Vec<egui::TextureId>,
    ) {
        // The font atlas (the default texture) grows as glyphs are added;
        // text is hashed by its galley.
        if id != egui::TextureId::default() {
            id.hash(h);
            uv.for_each(|p| [p.x.to_bits(), p.y.to_bits()].hash(h));
            images.push(id);
        }
    }
    fn add(shape: &egui::Shape, h: &mut impl Hasher, images: &mut Vec<egui::TextureId>) {
        match shape {
            egui::Shape::Text(text) => {
                let job = &text.galley.job;
                job.text.hash(h);
                for section in &job.sections {
                    section.byte_range.hash(h);
                    section.leading_space.to_bits().hash(h);
                    section.format.font_id.hash(h);
                    section.format.extra_letter_spacing.to_bits().hash(h);
                }
                for row in &text.galley.rows {
                    row.glyphs
                        .iter()
                        .for_each(|g| g.advance_width.to_bits().hash(h));
                }
            }
            egui::Shape::Rect(rect) => {
                if let Some(brush) = &rect.brush {
                    let uv = brush.uv;
                    image(
                        brush.fill_texture_id,
                        [uv.min, uv.max].into_iter(),
                        h,
                        images,
                    );
                }
            }
            egui::Shape::Mesh(mesh) => {
                image(
                    mesh.texture_id,
                    mesh.vertices.iter().map(|v| v.uv),
                    h,
                    images,
                );
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| add(s, h, images)),
            _ => {}
        }
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut images = Vec::new();
    ctx.graphics(|g| {
        if let Some(list) = g.get(layer) {
            list.all_entries()
                .skip(first)
                .for_each(|c| add(&c.shape, &mut h, &mut images));
        }
    });
    // The texture's own size: the same id can be given another image.
    let textures = ctx.tex_manager();
    let textures = textures.read();
    for id in images {
        textures.meta(id).map(|m| m.size).hash(&mut h);
    }
    h.finish()
}

fn flex_items_id(row: egui::Id) -> egui::Id {
    row.with("shadcn_egui::layout::flex_items")
}

fn flex_plan_id(row: egui::Id) -> egui::Id {
    row.with("shadcn_egui::layout::flex_plan")
}

/// Runs `add` in a crate row's `ui` and resolves the widths of its
/// [`vstack`] items for the next pass as the browser shrinks flex items.
/// `add` returns its result and the row's content width: the sum of its
/// items and the gaps between them, without free space.
pub(crate) fn shrink_stacks<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> (R, f32)) -> R {
    // The id a `UiStack` entry carries, which `flex_row_of` finds.
    let row = ui.unique_id();
    ui.data_mut(|d| d.insert_temp(flex_items_id(row), Vec::<FlexItem>::new()));
    let room = ui.max_rect().width();
    let (inner, content) = add(ui);
    let items = ui
        .data_mut(|d| d.remove_temp::<Vec<FlexItem>>(flex_items_id(row)))
        .unwrap_or_default();
    let stacks: f32 = items.iter().map(|i| i.width).sum();
    // A stack that filled the unbounded room (egui's `Separator`) took an
    // infinite width and pushed the rest of the row out of reach; its plan
    // gives it a finite one, and the next pass measures the rest.
    let others = Some(content - stacks).filter(|w| w.is_finite());
    let plan = shrink(room - others.unwrap_or(0.0), &items);
    let stored = ui.data(|d| d.get_temp::<Vec<Option<f32>>>(flex_plan_id(row)));
    let same = stored.as_ref().is_some_and(|old| {
        old.len() == plan.len()
            && old.iter().zip(&plan).all(|(a, b)| match (a, b) {
                (Some(a), Some(b)) => same_width(*a, *b),
                (a, b) => a.is_none() && b.is_none(),
            })
    });
    if !same && may_store(ui, stored.is_some()) {
        ui.data_mut(|d| d.insert_temp(flex_plan_id(row), plan));
        if !items.is_empty() {
            ui.ctx()
                .request_discard("layout: a vstack's width in a row changed");
        }
    }
    inner
}

/// The widths of flex items that share `room`, by CSS's "resolve flexible
/// lengths" for a shrinking line (`flex-shrink: 1`, no grow): each its
/// base when the bases fit, else each base less its share of the
/// shortfall in proportion to its base, an item held at its min-content
/// leaving the rest to the others.
///
/// An item whose base is infinite (it holds a widget that fills whatever
/// room it gets, as egui's `Separator`) takes the room the others leave,
/// as that widget fills a plain egui row, at least its min-content.
fn shrink(room: f32, items: &[FlexItem]) -> Vec<Option<f32>> {
    let fills = items.iter().filter(|i| i.basis.is_infinite()).count();
    if fills > 0 {
        let (fill, rest): (Vec<_>, Vec<_>) = items.iter().partition(|i| i.basis.is_infinite());
        let rest: Vec<FlexItem> = rest.into_iter().copied().collect();
        let held: f32 = fill.iter().map(|i| i.min).sum();
        let mut plan = shrink(room - held, &rest).into_iter();
        let used: f32 = rest
            .iter()
            .zip(plan.clone())
            .map(|(i, w)| w.unwrap_or(i.basis))
            .sum();
        let share = ((room - used) / fills as f32).max(0.0);
        return items
            .iter()
            .map(|i| {
                if i.basis.is_infinite() {
                    Some(share.max(i.min))
                } else {
                    plan.next().flatten()
                }
            })
            .collect();
    }
    let bases: f32 = items.iter().map(|i| i.basis).sum();
    if bases <= room + ROW_HEIGHT_EPSILON {
        return items.iter().map(|i| Some(i.basis)).collect();
    }
    let mut frozen = vec![false; items.len()];
    let mut width: Vec<f32> = items.iter().map(|i| i.basis).collect();
    loop {
        let held: f32 = (0..items.len())
            .filter(|&i| frozen[i])
            .map(|i| width[i])
            .sum();
        let open: f32 = (0..items.len())
            .filter(|&i| !frozen[i])
            .map(|i| items[i].basis)
            .sum();
        if open <= 0.0 {
            break;
        }
        let free = room - held;
        let mut clamped = false;
        for (i, item) in items.iter().enumerate() {
            if frozen[i] {
                continue;
            }
            width[i] = item.basis * free / open;
            if width[i] < item.min {
                width[i] = item.min;
                frozen[i] = true;
                clamped = true;
            }
        }
        if !clamped {
            break;
        }
    }
    width.into_iter().map(Some).collect()
}

/// A [`vstack`] as an item of the crate row `row`: laid out unwrapped
/// when the row's last plan gives it no width, else at that width, and
/// reported to the row.
///
/// The measuring pass of a stack whose content changed is discarded, so
/// it is not shown. Where it is egui's last allowed pass (`max_passes`
/// 2), egui ignores the discard and shows it for one frame: its text
/// unwrapped, a `w-full` child at its content minimum. The next frame
/// lays the stack out at its measured width.
fn flex_item<R>(
    ui: &mut Ui,
    row: egui::Id,
    id: egui::Id,
    column: egui::UiBuilder,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let index = ui.data_mut(|d| {
        let items = d.get_temp_mut_or_default::<Vec<FlexItem>>(flex_items_id(row));
        items.push(FlexItem::default());
        items.len() - 1
    });
    let stored = ui.data(|d| d.get_temp::<Intrinsic>(id));
    let store = may_store(ui, stored.is_some());
    let mut known = stored.unwrap_or_default();
    let target = ui
        .data(|d| d.get_temp::<Vec<Option<f32>>>(flex_plan_id(row)))
        .and_then(|plan| plan.get(index).copied().flatten())
        .filter(|_| !known.stale);
    if known.stale && store {
        // A measuring pass: not shown where egui has a pass left, else
        // followed by a frame at the measured width.
        ui.ctx()
            .request_discard("layout: a vstack in a row measures its content");
        ui.ctx().request_repaint();
        known = Intrinsic::default();
    }
    let mut content = egui::Vec2::ZERO;
    let layer = ui.layer_id();
    let first = ui
        .ctx()
        .graphics(|g| g.get(layer).map_or(0, |l| l.next_idx().0));
    let response = ui.scope_builder(column, |ui| {
        // Unbounded, text does not wrap and a filling widget takes its
        // content minimum (`capped_width`): the stack's max-content.
        ui.set_max_width(target.unwrap_or(f32::INFINITY));
        let inner = add(ui);
        content = ui.min_rect().size();
        // A flex item is as wide as the row made it, also where its
        // content wraps narrower.
        if let Some(w) = target {
            ui.set_min_width(w);
        }
        inner
    });
    if store {
        let (ppp, hash) = (
            ui.ctx().pixels_per_point(),
            content_hash(ui.ctx(), layer, first),
        );
        match target {
            None => known.unwrapped = Some((content, ppp)),
            Some(w) if known.max.is_some_and(|m| same_width(m, w)) => {
                // A stack that fits is laid out at its unwrapped width; a
                // changed content is redone as a measuring pass, never
                // shown.
                if known.fitted_pass(w, ppp, content, hash) {
                    ui.ctx()
                        .request_discard("layout: a fitting vstack's content changed");
                }
            }
            Some(w) => {
                let seen = ([w, ppp, content.x, content.y], hash);
                if known
                    .seen
                    .is_some_and(|s| s.0[..2] == seen.0[..2] && s != seen)
                {
                    // A shrunk stack wraps anyway: measured next frame, so
                    // this one keeps a valid layout.
                    known.stale = true;
                    ui.ctx().request_repaint();
                }
                known.seen = Some(seen);
            }
        }
    }
    let content = content.x;
    match target {
        None => known.max = Some(content),
        Some(w) if content > w + ROW_HEIGHT_EPSILON => known.min = Some(content),
        Some(_) => {
            if known.min.is_some_and(|m| content < m - ROW_HEIGHT_EPSILON) {
                known.min = Some(content);
            }
        }
    }
    let width = response.response.rect.width();
    ui.data_mut(|d| {
        if store {
            d.insert_temp(id, known);
        }
        if let Some(item) = d
            .get_temp_mut_or_default::<Vec<FlexItem>>(flex_items_id(row))
            .get_mut(index)
        {
            *item = FlexItem {
                width,
                basis: known.max.unwrap_or(width),
                min: known.min.unwrap_or(0.0),
            };
        }
    });
    response.inner
}

/// The width a widget takes (conformance 3.1; the one place a widget's
/// width arithmetic lives, rule 6 of the porting standard): `requested`
/// (the caller's width setter, else the widget's default), capped to the
/// width `ui` has left, then raised to `content_min`, what the widget
/// cannot shrink below (its padding and fixed parts). `f32::INFINITY`
/// requests the full width; where that is unbounded too (an
/// `allocate_ui` of infinite width), the widget takes `content_min`. A NaN request is no
/// request: the widget takes `content_min`, never NaN.
#[must_use]
pub fn capped_width(ui: &Ui, requested: f32, content_min: f32) -> f32 {
    fit_width(requested, ui.available_width(), content_min)
}

/// [`capped_width`] for a floating overlay (dialog, popover): `requested`
/// capped to the window's width less `inset` (upstream's
/// `max-w-[calc(100%-Npx)]`, 0 without one), then raised to
/// `content_min`.
#[must_use]
pub fn overlay_width(ctx: &egui::Context, requested: f32, inset: f32, content_min: f32) -> f32 {
    fit_width(requested, ctx.content_rect().width() - inset, content_min)
}

/// Allocates `size` as the browser places a box wider than its room: the
/// returned rect (and the response's) is `size`, but the parent `Ui` takes
/// at most the width it has left, so a widget raised to its content
/// minimum ([`capped_width`]) overflows without widening its parent and
/// the widgets that fill it. It overflows as a flex item does: past the
/// end in a left-aligned column or a left-to-right row, past the start in
/// a right-aligned column or a right-to-left row, and evenly on both
/// sides in a centred column.
///
/// In a crate row ([`row`], [`wrap_row`], [`space_between`], a card
/// footer), or a horizontal scope or a [`vstack`] inside one, the widget
/// takes its whole width and the next widget starts after it, as in the
/// browser; the row keeps the overflow from its own parent. A
/// `StripBuilder` or table cell in the row keeps its width: the widget
/// overflows the cell, and the row's next widget stays after the cell. In a plain `ui.horizontal` the
/// next widget starts after the room the parent gave, so it covers the
/// overflowing part: egui can move a row's cursor only by growing the row,
/// and that would widen the row's parent.
pub fn allocate_overflowing(
    ui: &mut Ui,
    size: egui::Vec2,
    sense: egui::Sense,
) -> (egui::Rect, egui::Response) {
    wrap_item(ui, size.x);
    let room = ui.available_width().max(0.0);
    if size.x <= room || in_row(ui) {
        return ui.allocate_exact_size(size, sense);
    }
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(room, size.y), sense);
    let layout = ui.layout();
    let stretched = layout.is_vertical() && layout.cross_justify();
    let left = match layout.horizontal_placement() {
        _ if stretched => rect.left(),
        egui::Align::Min => rect.left(),
        egui::Align::Center => rect.center().x - size.x / 2.0,
        egui::Align::Max => rect.right() - size.x,
    };
    // The response reports the widget's own rect; it senses only the part
    // inside the parent.
    response.rect = egui::Rect::from_min_size(egui::pos2(left, rect.top()), size);
    (response.rect, response)
}

/// `ui.allocate_exact_size` for a widget that may sit in a [`wrap_row`].
///
/// Call it instead of `allocate_exact_size` when a widget of your own
/// (a ported component, a custom-painted control) allocates its rect:
/// in a [`wrap_row`] the crate, not egui, then moves the widget to the
/// next line when it does not fit the rest of its line, and that line is
/// as high as its own tallest item, as for a crate widget. Elsewhere it
/// is `allocate_exact_size`. A widget wider than its room allocates with
/// [`allocate_overflowing`] instead, which wraps the same way.
pub fn allocate_item(
    ui: &mut Ui,
    size: egui::Vec2,
    sense: egui::Sense,
) -> (egui::Rect, egui::Response) {
    wrap_item(ui, size.x);
    ui.allocate_exact_size(size, sense)
}

/// `ui.allocate_space` for a widget that may sit in a [`wrap_row`]: it
/// wraps as [`allocate_item`] does, and like `allocate_space` it senses
/// nothing and returns an id to interact with and a rect that takes a
/// justified layout's width.
pub fn allocate_item_space(ui: &mut Ui, size: egui::Vec2) -> (egui::Id, egui::Rect) {
    wrap_item(ui, size.x);
    ui.allocate_space(size)
}

/// Begins a container (an alert, a card): a box that draws its content
/// before it knows its height and allocates its box with [`end_item`]
/// after it. Returns the box's top left corner and the container's
/// frame, the `Ui` its content `Ui`s are made from.
///
/// A content-sized container (a card) begins with this inside
/// [`as_flex_item`], which wraps it; a `w-full` container (an alert)
/// begins with [`begin_block`].
///
/// The frame is the container's accessible node, made before its
/// content's, so that the content is inside it, as upstream's children
/// are inside its `div` (egui gives a node the nearest ancestor that has
/// one when the node is made).
pub(crate) fn begin_item(ui: &mut Ui) -> (egui::Pos2, Ui) {
    let origin = ui.cursor().min;
    let frame = ui.new_child(egui::UiBuilder::new().max_rect(ui.available_rect_before_wrap()));
    ui.ctx().accesskit_node_builder(frame.unique_id(), |_| {});
    (origin, frame)
}

/// [`begin_item`] for a `w-full` container (an alert) `width` wide: in
/// a wrapping `Ui` (a [`wrap_row`] or egui's `horizontal_wrapped`) it
/// wraps from its width before its content is drawn, as [`as_flex_item`]
/// wraps a card (rule 6 of the porting standard): egui wraps only what it
/// allocates, not a container drawn in a child `Ui`. In a [`row`] it
/// fills the room the row's other items leave. A content-sized container
/// (a card) goes through [`as_flex_item`] instead.
pub(crate) fn begin_block(ui: &mut Ui, width: f32) -> (egui::Pos2, Ui) {
    if ui.layout().main_wrap() {
        wrap_line(ui, width);
    }
    begin_item(ui)
}

/// Allocates the box `rect` of a container that [`begin_item`] began and
/// returns the response of its `frame`, the container's accessible node.
/// The box is an item of its own after the content, as egui's `Frame`
/// allocates its box: a crate row measures it.
pub(crate) fn end_item(
    ui: &mut Ui,
    frame: Ui,
    rect: egui::Rect,
    sense: egui::Sense,
) -> egui::Response {
    let id = frame.unique_id();
    drop(frame);
    let _ = ui.allocate_rect(rect, sense);
    ui.interact(rect, id, sense)
}

/// The clamp order of [`capped_width`]: cap first, then the content
/// minimum, so a widget wider than its room overflows instead of
/// truncating.
fn fit_width(requested: f32, available: f32, content_min: f32) -> f32 {
    // `f32::max` and `f32::min` return the other operand for NaN.
    let content_min = content_min.max(0.0);
    if requested.is_nan() {
        return content_min;
    }
    let w = requested.min(available);
    if w.is_finite() {
        w.max(content_min)
    } else {
        content_min
    }
}

/// Tailwind's `sm` breakpoint (40rem).
const BREAKPOINT_SM: f32 = 640.0; // behaviour-ok: Tailwind default --breakpoint-sm
/// Tailwind's `md` breakpoint (48rem).
const BREAKPOINT_MD: f32 = 768.0; // behaviour-ok: Tailwind default --breakpoint-md

/// The breakpoint helper for media queries (`sm:`, `md:`,
/// `@media (width >= N)`): whether the window is at least `px` points
/// wide. It reads what a media query reads, the whole viewport
/// (`Context::viewport_rect`), safe-area insets included. Upstream's
/// breakpoints are `min-width` queries, so the boundary counts as wide.
#[must_use]
pub fn window_at_least(ctx: &egui::Context, px: f32) -> bool {
    ctx.viewport_rect().width() >= px
}

/// The breakpoint helper for container queries (`@md/field-group`,
/// `@container (width >= N)`): whether the width `ui` has left is at
/// least `px` points; the boundary counts as wide. With
/// [`window_at_least`], the one written-down exception to single-pass
/// sizing: a widget reads its room here, never with arithmetic of its own.
#[must_use]
pub fn container_at_least(ui: &Ui, px: f32) -> bool {
    ui.available_width() >= px
}

/// The media-query flags of the window: [`State::SM`] at 640 px and up,
/// [`State::MD`] at 768 px and up. [`Theme::state`](crate::Theme::state) adds them to every
/// state it resolves.
///
/// [`State::SM`]: crate::style::State::SM
/// [`State::MD`]: crate::style::State::MD
#[must_use]
pub(crate) fn breakpoints(ctx: &egui::Context) -> crate::style::State {
    use crate::style::State;
    State::NONE
        .with(State::SM, window_at_least(ctx, BREAKPOINT_SM))
        .with(State::MD, window_at_least(ctx, BREAKPOINT_MD))
}

/// A row whose `left` content hugs the start and `right` content hugs the end
/// (justify-content: space-between). Both sides keep the order the code
/// adds their widgets in. Where the width is unbounded, the row is as wide
/// as its two sides, the right one after the left.
///
/// The right side is placed at the end from its width in the previous
/// pass; where that width changes, the pass is discarded and redone.
pub fn space_between(ui: &mut Ui, left: impl FnOnce(&mut Ui), right: impl FnOnce(&mut Ui)) {
    let id = ui.next_auto_id().with("shadcn_egui::layout::space_between");
    // A crate row as high as its content, as `row`.
    let band_id = id.with("height");
    let (stored, band, align) = row_band(ui, band_id);
    let size = egui::vec2(ui.available_size_before_wrap().x, band);
    let layout = egui::Layout::left_to_right(align);
    let lines = overflow_row(ui, size, layout, None, |_, ui| {
        let items = RowItems::begin(ui);
        shrink_stacks(ui, |ui| {
            left(ui);
            let start = ui.min_rect().width();
            // Unbounded room (a crate row measuring a card around it)
            // leaves no free space to put between the sides, as a
            // `justify-between` row in max-content sizing: the right side
            // follows the left one.
            let end = end_aligned(
                ui,
                id,
                egui::Align::Center,
                "layout::space_between right side width changed",
                |ui| {
                    hold_row_items(ui);
                    right(ui);
                },
            )
            .response
            .rect
            .width();
            // The free space between the two sides is not content.
            let gap = if end > 0.0 {
                ui.spacing().item_spacing.x
            } else {
                0.0
            };
            ((), start + gap + end)
        });
        items.finish(ui, false)
    });
    settle_row_band(
        ui,
        band_id,
        stored,
        band,
        &lines,
        "layout::space_between content height changed",
    );
}

/// The width [`end_aligned`] took under `id` in the previous pass; `None`
/// before its first pass.
pub(crate) fn end_aligned_width(ui: &Ui, id: egui::Id) -> Option<f32> {
    ui.data(|d| d.get_temp::<f32>(id))
}

/// Lays `add` out left to right, in the order the code adds its widgets,
/// flush with the end of the room `ui` has left (`justify-content:
/// flex-end`): it starts where its width from the previous pass, stored
/// under `id`, ends it at the room's right edge, never before the room's
/// left edge. Where that width changes, the pass is discarded once with
/// `why` (conformance 3.2), as `flex_item` does; a sizing pass never
/// replaces a width a visible pass measured. Before its first pass, and
/// in unbounded room, it starts at the room's left edge.
pub(crate) fn end_aligned<R>(
    ui: &mut Ui,
    id: egui::Id,
    align: egui::Align,
    why: &'static str,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let room = ui.available_rect_before_wrap();
    let stored = end_aligned_width(ui, id);
    let x = stored
        .filter(|_| room.right().is_finite())
        .map_or(room.left(), |w| (room.right() - w).max(room.left()));
    let inner = ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_max(
                egui::pos2(x, room.top()),
                room.max,
            ))
            .layout(egui::Layout::left_to_right(align)),
        add,
    );
    let width = inner.response.rect.width();
    if room.right().is_finite()
        && may_store(ui, stored.is_some())
        && stored.is_none_or(|w| !same_width(w, width))
    {
        ui.data_mut(|d| d.insert_temp(id, width));
        ui.ctx().request_discard(why);
    }
    inner
}

/// Lays `add` out as a reverse column (`flex-direction: column-reverse`,
/// `align-items: stretch`): the first widget the code adds at the bottom,
/// each later one above it, each as wide as the room `ui` has left. It
/// starts the column where its height from the previous pass, stored under
/// `id`, ends it; where that height changes, the pass is discarded once
/// with `why` (conformance 3.2), as [`end_aligned`] does on the x axis; a
/// sizing pass never replaces a height a visible pass measured. Before its
/// first pass the column ends at the room's top edge.
///
/// In room of unbounded width the column is as wide as its widest widget
/// (max-content), and each widget as wide as the column. It learns that
/// width as a [`vstack`] in a row learns its unwrapped width: a measuring
/// pass lays the widgets out at their own widths and is discarded; later
/// passes stretch them to the width found, and measure again where the
/// content, its font or the scale changed.
pub(crate) fn reverse_column<R>(
    ui: &mut Ui,
    id: egui::Id,
    why: &'static str,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let room = ui.available_rect_before_wrap();
    let stored = ui.data(|d| d.get_temp::<f32>(id));
    let height = stored.unwrap_or(0.0);
    // Unbounded: the widest widget's width, kept as a fitting stack's.
    let fit_id = id.with("shadcn_egui::layout::reverse_column::fit");
    let unbounded = !room.width().is_finite();
    let fit = unbounded.then(|| ui.data(|d| d.get_temp::<Intrinsic>(fit_id)));
    let store_fit = may_store(ui, fit.flatten().is_some());
    let mut known = fit.flatten().unwrap_or_default();
    if known.stale && store_fit {
        // A measuring pass: not shown where egui has a pass left, else
        // followed by a frame at the measured width.
        ui.ctx().request_discard(why);
        ui.ctx().request_repaint();
        known = Intrinsic::default();
    }
    let width = if unbounded {
        known.max
    } else {
        Some(room.width())
    };
    let layer = ui.layer_id();
    let first = ui
        .ctx()
        .graphics(|g| g.get(layer).map_or(0, |l| l.next_idx().0));
    let inner = ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_size(
                room.min,
                egui::vec2(width.unwrap_or(room.width()), height),
            ))
            .layout(egui::Layout::bottom_up(egui::Align::Min).with_cross_justify(width.is_some())),
        add,
    );
    let measured = inner.response.rect.height();
    if may_store(ui, stored.is_some()) && stored.is_none_or(|h| !same_width(h, measured)) {
        ui.data_mut(|d| d.insert_temp(id, measured));
        ui.ctx().request_discard(why);
    }
    if fit.is_some() && store_fit {
        let (ppp, size) = (ui.ctx().pixels_per_point(), inner.response.rect.size());
        match known.max {
            None => {
                known.max = Some(size.x);
                known.unwrapped = Some((size, ppp));
                ui.ctx().request_discard(why);
                ui.ctx().request_repaint();
            }
            Some(w) => {
                if known.fitted_pass(w, ppp, size, content_hash(ui.ctx(), layer, first)) {
                    ui.ctx().request_discard(why);
                }
            }
        }
        ui.data_mut(|d| d.insert_temp(fit_id, known));
    }
    inner
}

/// A shadcn Card at the default size, across the available width of a
/// column and content-sized in a crate row. See
/// [`crate::components::card::Card`] for sizes, a footer and its width.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    crate::components::card::Card::new().show(ui, add)
}

/// A labeled control row: fixed-width label column + control filling the
/// rest, as high as the taller of the two (never egui's `interact_size`),
/// both centred in it. The height comes from the previous pass; where it
/// changes, the pass is redone.
pub fn form_row(ui: &mut Ui, label_width: f32, label: &str, control: impl FnOnce(&mut Ui)) {
    let id = ui.next_auto_id().with("shadcn_egui::layout::form_row");
    let (stored, band, align) = row_band(ui, id);
    let mut lines = Vec::new();
    // A cell measures its widgets, not its own rect: egui makes a centred
    // cell as high as the band, so the row would never shrink.
    let mut cell = |ui: &mut Ui, add: &mut dyn FnMut(&mut Ui)| {
        ui.with_layout(egui::Layout::left_to_right(align), |ui| {
            let items = RowItems::begin(ui);
            add(ui);
            lines.extend(items.finish(ui, false));
        });
    };
    let mut control = Some(control);
    ui.allocate_ui(egui::vec2(ui.available_width(), band), |ui| {
        StripBuilder::new(ui)
            .size(Size::exact(label_width))
            .size(Size::remainder())
            .horizontal(|mut strip| {
                strip.cell(|ui| {
                    cell(ui, &mut |ui| {
                        ui.add(crate::components::label::Label::new(label));
                    })
                });
                strip.cell(|ui| {
                    cell(ui, &mut |ui| {
                        if let Some(control) = control.take() {
                            control(ui);
                        }
                    })
                });
            });
    });
    settle_row_band(
        ui,
        id,
        stored,
        band,
        &lines,
        "layout::form_row content height changed",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_caps_then_raises_to_the_content_minimum() {
        assert_eq!(fit_width(300.0, 200.0, 50.0), 200.0);
        assert_eq!(fit_width(100.0, 200.0, 50.0), 100.0);
        // Cap first, then the minimum: an overflow, not a truncation.
        assert_eq!(fit_width(300.0, 20.0, 50.0), 50.0);
        // Fill.
        assert_eq!(fit_width(f32::INFINITY, 200.0, 50.0), 200.0);
        // Nothing finite requested and unbounded room: the minimum.
        assert_eq!(fit_width(f32::INFINITY, f32::INFINITY, 50.0), 50.0);
        assert_eq!(fit_width(120.0, f32::INFINITY, 50.0), 120.0);
    }

    #[test]
    fn width_is_never_nan() {
        // A NaN request does not fill (`NaN.min(x)` is `x`).
        assert_eq!(fit_width(f32::NAN, 200.0, 50.0), 50.0);
        assert_eq!(fit_width(300.0, f32::NAN, 50.0), 300.0);
        assert_eq!(fit_width(f32::INFINITY, 200.0, f32::NAN), 200.0);
        assert_eq!(fit_width(f32::NAN, f32::NAN, f32::NAN), 0.0);
    }

    #[test]
    fn an_overflowing_widget_keeps_its_parents_width() {
        let ctx = egui::Context::default();
        let mut widths = None;
        let _ = crate::test_ui::run(&ctx, egui::RawInput::default(), |ui| {
            ui.allocate_ui(egui::vec2(100.0, 200.0), |ui| {
                ui.set_max_width(100.0);
                let (rect, response) =
                    allocate_overflowing(ui, egui::vec2(150.0, 20.0), egui::Sense::hover());
                widths = Some((rect.width(), response.rect.width(), ui.min_rect().width()));
            });
        });
        assert_eq!(widths, Some((150.0, 150.0, 100.0)));
    }

    /// `rows` in a column 100 wide: the left edge of the widget after a
    /// 150 wide overflowing one, and the column's width afterwards.
    fn after_overflow(rows: impl Fn(&mut Ui, &mut dyn FnMut(&mut Ui))) -> (f32, f32) {
        let ctx = egui::Context::default();
        let mut out = None;
        let _ = crate::test_ui::run(&ctx, egui::RawInput::default(), |ui| {
            let max = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 200.0));
            let mut column = ui.new_child(egui::UiBuilder::new().max_rect(max));
            let mut next = 0.0;
            rows(&mut column, &mut |ui| {
                allocate_overflowing(ui, egui::vec2(150.0, 20.0), egui::Sense::hover());
                next = ui
                    .allocate_exact_size(egui::vec2(10.0, 20.0), egui::Sense::hover())
                    .0
                    .left();
            });
            out = Some((next, column.min_rect().width()));
        });
        out.expect("ran")
    }

    /// In a crate row an overflowing child pushes its siblings past its
    /// whole width, and the column holding the row keeps its width, as a
    /// flex row in the browser.
    #[test]
    fn a_crate_row_places_siblings_after_the_overflow() {
        assert_eq!(
            after_overflow(|ui, add| row(ui, 4.0, |ui| add(ui))),
            (154.0, 100.0)
        );
        assert_eq!(
            after_overflow(|ui, add| wrap_row(ui, 4.0, |ui| add(ui))),
            // The sibling does not fit the rest of the line: the next one.
            (0.0, 100.0)
        );
        assert_eq!(
            after_overflow(|ui, add| space_between(ui, |ui| add(ui), |_| {})),
            (150.0 + 8.0, 100.0)
        );
        // In a scope inside the row too.
        assert_eq!(
            after_overflow(|ui, add| row(ui, 4.0, |ui| {
                ui.scope(|ui| add(ui));
            })),
            (154.0, 100.0)
        );
    }

    /// A `vstack` in a crate row is a flex item: it grows to an
    /// overflowing child, and the row's next child starts after it, also
    /// when the child sits in a row of its own inside the stack. The
    /// column holding the row keeps its width.
    #[test]
    fn a_vstack_in_a_crate_row_grows_to_its_overflowing_child() {
        let in_stack = |inner_row: bool| {
            let ctx = egui::Context::default();
            let mut out = None;
            let _ = crate::test_ui::run(&ctx, egui::RawInput::default(), |ui| {
                let max = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 200.0));
                let mut column = ui.new_child(egui::UiBuilder::new().max_rect(max));
                let mut stack_width = 0.0;
                let next = row(&mut column, 4.0, |ui| {
                    stack_width = vstack(ui, 0.0, |ui| {
                        let overflow = |ui: &mut Ui| {
                            allocate_overflowing(ui, egui::vec2(150.0, 20.0), egui::Sense::hover());
                        };
                        if inner_row {
                            row(ui, 4.0, overflow);
                        } else {
                            overflow(ui);
                        }
                        ui.min_rect().width()
                    });
                    ui.allocate_exact_size(egui::vec2(10.0, 20.0), egui::Sense::hover())
                        .0
                        .left()
                });
                out = Some((stack_width, next, column.min_rect().width()));
            });
            out.expect("ran")
        };
        assert_eq!(in_stack(false), (150.0, 154.0, 100.0));
        assert_eq!(in_stack(true), (150.0, 154.0, 100.0));
    }

    /// Outside a crate row a `vstack` is a block: the child overflows it.
    #[test]
    fn a_vstack_outside_a_row_keeps_its_width() {
        assert_eq!(
            after_overflow(|ui, add| {
                vstack(ui, 0.0, |ui| add(ui));
            }),
            (0.0, 100.0)
        );
    }

    /// In a plain `ui.horizontal` the next widget covers the overflow,
    /// and a widget that runs past the room still widens the parent.
    #[test]
    fn a_plain_horizontal_row_overlaps_the_overflow() {
        assert_eq!(
            after_overflow(|ui, add| {
                ui.horizontal(|ui| add(ui));
            }),
            (108.0, 118.0)
        );
    }

    /// An egui widget wider than a crate row's room does not widen the
    /// row's parent either.
    #[test]
    fn a_crate_row_keeps_an_egui_widget_overflow_from_its_parent() {
        assert_eq!(
            after_overflow(|ui, _| row(ui, 4.0, |ui| {
                ui.allocate_exact_size(egui::vec2(150.0, 20.0), egui::Sense::hover());
            })),
            (0.0, 100.0)
        );
    }

    /// The overflow goes where a flex item's goes: the end of a left
    /// column, both sides of a centred one, the start of a right column
    /// and of a right-to-left row.
    #[test]
    fn an_overflowing_widget_overflows_as_its_layout_aligns() {
        use egui::{Align, Layout};
        let overflow = |layout: Layout| {
            let ctx = egui::Context::default();
            let mut span = None;
            let _ = crate::test_ui::run(&ctx, egui::RawInput::default(), |ui| {
                let max =
                    egui::Rect::from_min_size(egui::pos2(100.0, 0.0), egui::vec2(100.0, 50.0));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(max).layout(layout));
                let (rect, _) =
                    allocate_overflowing(&mut child, egui::vec2(150.0, 20.0), egui::Sense::hover());
                span = Some((rect.left(), rect.right(), child.min_rect().width()));
            });
            span.expect("ran")
        };
        assert_eq!(
            overflow(Layout::top_down(Align::Min)),
            (100.0, 250.0, 100.0)
        );
        assert_eq!(
            overflow(Layout::top_down(Align::Center)),
            (75.0, 225.0, 100.0)
        );
        assert_eq!(overflow(Layout::top_down(Align::Max)), (50.0, 200.0, 100.0));
        assert_eq!(
            overflow(Layout::top_down_justified(Align::Center)),
            (100.0, 250.0, 100.0)
        );
        assert_eq!(
            overflow(Layout::left_to_right(Align::Center)),
            (100.0, 250.0, 100.0)
        );
        assert_eq!(
            overflow(Layout::right_to_left(Align::Center)),
            (50.0, 200.0, 100.0)
        );
    }
}
