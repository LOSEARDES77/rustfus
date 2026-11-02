//! The frame every text-entry field shares (Input, Textarea): a frameless
//! egui `TextEdit` inside a box painted from the part's recipe. The frame
//! resolves the box insets, the part's fade, the text look and its typed
//! and placeholder colours, then shows the widget's `TextEdit` in the
//! allocated rect with Chrome's caret and paints the box under it in the
//! field's state. The widget decides only what differs: the box height,
//! how its text lays out, and its hint.

use crate::components::shared::{
    inset_frame, line_width, part_opacity, part_painter, report_invalid, text_entry_state,
    RecipeBox,
};
use crate::components::text::{Leading, TextLook};
use crate::style::{pick, pick_len, Prop, Recipe, Role, State};
use crate::{layout, Theme};
use egui::text_edit::TextEditOutput;
use egui::{Color32, FontId, Galley, Sense, Shape, TextEdit, Ui, Vec2};
use std::sync::Arc;

/// Where a field clips its text, its hint and its caret, as Chrome does.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextClip {
    /// An `<input>`: at the content box. The text scrolls so a caret at its
    /// end stays inside.
    Content,
    /// A `<textarea>`: at the padding box; a wide glyph paints into the
    /// padding.
    Padding,
}

/// A text-entry field's resolved frame. Build it with
/// [`TextEntry::resolve`], size the box from its fields, then
/// [`TextEntry::show`] the widget's `TextEdit` in it.
pub(crate) struct TextEntry<'a> {
    theme: &'a Theme,
    /// The part's recipes, in upstream's class order.
    pub(crate) layers: &'a [&'a Recipe],
    /// The variant's bits and `aria-invalid`.
    context: State,
    /// The state geometry resolves in: never focus, so it is known before
    /// the field is allocated.
    pub(crate) geometry: State,
    pub(crate) enabled: bool,
    /// Top, right, bottom, left: padding plus border.
    pub(crate) inset: [f32; 4],
    /// The border width, in points on whole device pixels.
    border: f32,
    clip: TextClip,
    pub(crate) look: TextLook,
    pub(crate) font: FontId,
    fade: f32,
    /// The colour of the hint text (`placeholder:`).
    pub(crate) placeholder: Color32,
    /// The colour of the typed text.
    pub(crate) typed: Color32,
    /// The narrowest box: the padding box around one glyph.
    pub(crate) content_min: f32,
}

impl<'a> TextEntry<'a> {
    /// The frame of a field drawn from `layers` in `theme`, in `context`
    /// (the variant's bits, `aria-invalid`), disabled where `ui` is or
    /// `enabled` is false, its text placed in its line box as `leading`
    /// says, its text clipped as `clip` says. `name` keys the part's fade
    /// transition.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn resolve(
        ui: &Ui,
        theme: &'a Theme,
        layers: &'a [&'a Recipe],
        context: State,
        enabled: bool,
        leading: Leading,
        clip: TextClip,
        name: &str,
    ) -> Self {
        let enabled = enabled && ui.is_enabled();
        let geometry = theme.state(ui.ctx(), context);
        let get = |f: fn(&Recipe) -> &Prop<f32>| pick_len(layers, f, geometry);
        let border_w = line_width(get(|r| &r.border_width), ui.ctx().pixels_per_point());
        let inset = [
            get(|r| &r.pad_top),
            get(|r| &r.pad_right),
            get(|r| &r.pad_bottom),
            get(|r| &r.pad_left),
        ]
        .map(|pad| pad + border_w);
        let look = TextLook::resolve(ui, layers, geometry).leading(leading);
        let font = look.font_id(ui.ctx());
        // The `TextEdit` is the field's own text: it paints at the part's opacity.
        let fade = part_opacity(
            ui,
            ui.next_auto_id().with(name),
            &[layers],
            geometry.with(State::DISABLED, !enabled),
        );
        // Upstream's text-entry parts all carry
        // `placeholder:text-muted-foreground`; a recipe without a
        // placeholder colour keeps it.
        let muted = theme.role(Role::MutedForeground); // behaviour-ok: upstream's placeholder colour
        let placeholder = pick(layers, |r| &r.text, geometry.or(State::PLACEHOLDER))
            .map_or(muted, |p| theme.paint(p));
        let inherited = theme.role(Role::Foreground); // behaviour-ok: inherits Foreground; reads the part first
        let typed = pick(layers, |r| &r.text, geometry).map_or(inherited, |p| theme.paint(p));
        let content_min = inset[3] + inset[1] + ui.ctx().fonts_mut(|f| f.glyph_width(&font, 'M'));
        Self {
            theme,
            layers,
            context,
            geometry,
            enabled,
            inset,
            border: border_w,
            clip,
            look,
            font,
            fade,
            placeholder,
            typed,
            content_min,
        }
    }

    /// The recipe's `f` in the geometry state, 0 where it sets none.
    pub(crate) fn get(&self, f: fn(&Recipe) -> &Prop<f32>) -> f32 {
        pick_len(self.layers, f, self.geometry)
    }

    /// The width between the side insets of a box `width` wide.
    pub(crate) fn content_width(&self, width: f32) -> f32 {
        width - self.inset[3] - self.inset[1]
    }

    /// The room an input keeps at the end of its scrolled text for the
    /// caret, in whole points: Blink adds the caret's width to the
    /// scrollable width of an editable box, so a caret after the last
    /// glyph of a full field stays inside it. None in a textarea, whose
    /// lines wrap.
    pub(crate) fn caret_room(&self, ctx: &egui::Context) -> f32 {
        match self.clip {
            TextClip::Content => {
                (caret_width(ctx.pixels_per_point()) / ctx.pixels_per_point()).ceil()
            }
            TextClip::Padding => 0.0,
        }
    }

    /// Allocate the box at `size`, show `edit` in it (`id_salt` gives it
    /// a stable id), and paint the box under it, at the allocated rect, in
    /// the field's state.
    /// Raised to its content minimum, the field overflows its room without
    /// widening the parent (conformance 3.6).
    pub(crate) fn show(
        self,
        ui: &mut Ui,
        size: Vec2,
        edit: TextEdit<'_>,
        id_salt: Option<egui::Id>,
    ) -> TextEditOutput {
        let enabled = self.enabled;
        let mut edit_inset = self.inset;
        edit_inset[1] += self.caret_room(ui.ctx());
        // The box goes under the text: reserve its place in paint order, fill
        // it once the focus state is known.
        let slot = ui.painter().add(Shape::Noop);
        let (rect, out) = ui
            .scope(|ui| {
                ui.set_opacity(self.fade);
                // As `add_sized`, with the id made in the `Ui` that shows
                // the field: `TextEdit::id_salt` would hash the stored `Id`
                // again.
                let (rect, _) = layout::allocate_overflowing(ui, size, Sense::hover());
                let edit = edit
                    .interactive(enabled)
                    .frame(inset_frame(edit_inset))
                    .font(self.font.clone())
                    .text_color(self.typed);
                let mut field = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect)
                        .layout(egui::Layout::centered_and_justified(ui.layout().main_dir())),
                );
                let edit = match id_salt {
                    Some(salt) => edit.id(field.make_persistent_id(salt)),
                    None => edit,
                };
                // egui's caret spans the row and 1.5 pt beyond it; the
                // field paints Chrome's instead.
                let blink = field.visuals().text_cursor.clone();
                field.visuals_mut().text_cursor.stroke = egui::Stroke::NONE;
                field.visuals_mut().text_cursor.blink = false;
                // Chrome's clip for the text, the hint and the caret: egui
                // clips the text a point outside the content box and the
                // hint not at all.
                let clip = match self.clip {
                    TextClip::Content => egui::Rect::from_x_y_ranges(
                        rect.left() + self.inset[3]..=rect.right() - self.inset[1],
                        rect.top() + self.inset[0]..=rect.bottom() - self.inset[2],
                    ),
                    TextClip::Padding => rect.shrink(self.border),
                }
                .intersect(field.clip_rect());
                let layer = field.layer_id();
                let first = field.ctx().graphics_mut(|g| g.entry(layer).next_idx());
                let out = edit.show(&mut field);
                field.ctx().graphics_mut(|g| {
                    let list = g.entry(layer);
                    for i in first.0..list.next_idx().0 {
                        list.mutate_shape(egui::layers::ShapeIdx(i), |c| {
                            if matches!(c.shape, Shape::Text(_)) {
                                c.clip_rect = clip;
                            }
                        });
                    }
                });
                if enabled {
                    paint_caret(
                        &field,
                        &out,
                        &blink,
                        self.look.caret_span(field.ctx()),
                        self.typed,
                        clip,
                    );
                }
                (rect, out)
            })
            .inner;

        let resp = &out.response.response;
        report_invalid(resp, self.context.contains(State::INVALID));
        let st = text_entry_state(
            self.theme,
            resp,
            self.context.with(State::DISABLED, !enabled),
        );
        let l = self.layers;
        let painter = part_painter(ui, resp.id, &[l], st);
        let shapes = RecipeBox::resolve(ui.ctx(), self.theme, l, st)
            .transitioned(ui.ctx(), resp.id, l, st)
            .shapes_at(ui.ctx(), rect, painter.opacity());
        painter.set(slot, Shape::Vec(shapes));
        out
    }
}

/// `galley`, measured at most `width` wide: the text the field shows or
/// hints. A glyph wider than the content box (a `W` in a field at its
/// one-`M` content minimum) would widen the `TextEdit` past the box and
/// move its response outside the allocation; Chrome keeps the field and
/// clips the glyph inside it. Only the measured rect shrinks: the rows
/// and the glyphs stay where they were laid out.
pub(crate) fn within(galley: Arc<Galley>, width: f32) -> Arc<Galley> {
    if galley.rect.width() <= width {
        return galley;
    }
    let mut fit = (*galley).clone();
    fit.rect.max.x = fit.rect.min.x + width.max(0.0);
    Arc::new(fit)
}

/// The caret as Blink paints it in a text field: 1 css px wide, from the
/// font's ascent above the baseline to its descent below (`span`, below the
/// top of the galley's row, the editor line), in the text colour
/// (`caret-color: auto`), snapped to device pixels where the painter puts
/// the galley. Blinks as egui's caret does, from the last edit or cursor
/// move.
fn paint_caret(
    ui: &Ui,
    out: &TextEditOutput,
    cursor: &egui::style::TextCursorStyle,
    span: Option<std::ops::Range<f32>>,
    color: Color32,
    clip: egui::Rect,
) {
    let Some(range) = out.cursor_range else {
        return;
    };
    let focused = out.response.response.has_focus() && ui.input(|i| i.focused);
    if !focused {
        return;
    }
    let now = ui.input(|i| i.time);
    let id = out.response.response.id.with("shadcn_egui::input::caret");
    let since = ui.ctx().data_mut(|d| {
        let last = d.get_temp_mut_or_insert_with(id, || (range, now));
        if out.response.response.changed() || last.0 != range {
            *last = (range, now);
        }
        now - last.1
    });
    if cursor.blink {
        let total = cursor.on_duration + cursor.off_duration;
        let phase = (since % f64::from(total)) as f32;
        let (shown, wake) = if phase < cursor.on_duration {
            (true, cursor.on_duration - phase)
        } else {
            (false, total - phase)
        };
        ui.ctx().request_repaint_after_secs(wake);
        if !shown {
            return;
        }
    }
    let ppp = ui.ctx().pixels_per_point();
    let origin = out.galley_pos - egui::vec2(out.galley.rect.left(), 0.0);
    let row = out.galley.pos_from_cursor(range.primary);
    let span = span.unwrap_or(0.0..row.height());
    // The painter rounds the galley origin to the device pixel; the row
    // is whole device pixels tall.
    let top = (origin.y * ppp).round();
    let width = caret_width(ppp);
    // The field keeps the caret's room after its text (`caret_room`), in
    // whole points; a caret the rounding puts past the clip moves in by
    // that last device pixel.
    let left = ((origin.x + row.min.x) * ppp)
        .round()
        .min((clip.right() * ppp).floor() - width);
    let rect = egui::Rect::from_min_max(
        egui::pos2(left, top + (row.min.y + span.start) * ppp),
        egui::pos2(left + width, top + (row.min.y + span.end) * ppp),
    );
    let rect = egui::Rect::from_min_max(rect.min / ppp, rect.max / ppp);
    ui.painter_at(clip).rect_filled(rect, 0.0, color);
}

/// Blink's caret width in device pixels: 1 css px, at least one device
/// pixel.
fn caret_width(ppp: f32) -> f32 {
    ppp.round().max(1.0) // behaviour-ok: the browser's caret width
}
