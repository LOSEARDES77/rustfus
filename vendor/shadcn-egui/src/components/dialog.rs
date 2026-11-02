//! shadcn Dialog, ported from shadcn-svelte `ui/dialog/*.svelte` at
//! `tools/stylegen/PIN`. Width, padding, gap, radius, ring, text, the footer's
//! strip and gap, and the close button's position come from the style's
//! `cn-dialog-*` recipes.
//!
//! Built on `egui::Modal`, which keeps backdrop and panel in one area (so their
//! z-order cannot flip) and blocks input to everything beneath. Closes on Esc,
//! a backdrop click, or the close button.
//!
//! The overlay's `backdrop-blur-xs` is not drawn: egui cannot blur what lies
//! beneath, so only the overlay colour applies.

use crate::components::button::{Button, ButtonSize, ButtonVariant};
use crate::components::context;
use crate::components::free_content;
use crate::components::shared::{content_ui, opacity, part_painter, part_ui, RecipeBox};
use crate::components::text::{content_scope, TextLook};
use crate::style::app::{default_variant, Resolved};
use crate::style::{pick, pick_len, Recipe, State};
use crate::{layout, Theme};
use egui::{Align, Color32, Id, Layout, Rect, Shape, TextWrapMode, Ui, UiBuilder};

default_variant!(
    /// The dialog's variant: upstream has none, so only app variants
    /// ([`DialogVariant::app`]) differ from `Default`.
    DialogVariant,
    /// The dialog's recipes an app variant's edit changes.
    DialogRecipes {
        /// `data-slot="dialog-overlay"`: the backdrop.
        dialog_overlay,
        /// `data-slot="dialog-content"`: the panel.
        dialog_content,
        /// `data-slot="dialog-close"`: the close button, over its button
        /// recipes.
        dialog_close,
        /// `data-slot="dialog-header"`.
        dialog_header,
        /// `data-slot="dialog-footer"`.
        dialog_footer,
        /// `data-slot="dialog-title"`.
        dialog_title,
        /// `data-slot="dialog-description"`.
        dialog_description,
    }
);

/// The context a dialog gives its parts: its variant.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct DialogContext {
    variant: DialogVariant,
}

/// The recipes of the dialog around `ui`; outside a dialog, the default's.
fn dialog_recipes(ui: &Ui, t: &Theme) -> Resolved<DialogVariant> {
    let cx = context::find::<DialogContext>(ui).unwrap_or_default();
    t.resolve(cx.variant)
}

pub struct Dialog<'a> {
    id: Id,
    variant: DialogVariant,
    title: &'a str,
    description: Option<&'a str>,
    width: Option<f32>,
    backdrop: Option<Color32>,
    show_close_button: bool,
}

impl<'a> Dialog<'a> {
    pub fn new(id_salt: impl egui::AsId, title: &'a str) -> Self {
        Self {
            id: Id::new(id_salt),
            variant: DialogVariant::Default,
            title,
            description: None,
            width: None,
            backdrop: None,
            show_close_button: true,
        }
    }
    /// An app variant ([`DialogVariant::app`]). The dialog's parts inside
    /// take it.
    pub fn variant(mut self, v: impl Into<DialogVariant>) -> Self {
        self.variant = v.into();
        self
    }
    pub fn description(mut self, text: &'a str) -> Self {
        self.description = Some(text);
        self
    }
    /// Width of the dialog box, padding included; default the style's
    /// `max-w-*`, capped to keep clear of the window edges.
    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }
    /// Backdrop color; default the style's overlay. Under a CPU rasterizer a
    /// translucent full-window backdrop is re-blended every frame the dialog
    /// is open; pass `Color32::TRANSPARENT` and dim the background once in the
    /// compositor instead (see `cpu-rendering.md`). It still blocks clicks.
    pub fn backdrop(mut self, color: Color32) -> Self {
        self.backdrop = Some(color);
        self
    }
    /// Whether the close button shows in the corner; default true.
    pub fn show_close_button(mut self, yes: bool) -> Self {
        self.show_close_button = yes;
        self
    }

    /// Show the dialog while `*open`; clears `open` when the user dismisses it.
    /// Returns the body's value while open. Widgets of the caller's own
    /// in `body` stand for the element upstream's demos put there, a block
    /// (rules, Container): they stack with no gap, the style's gap from the
    /// header and from [`footer`], which comes last (a `debug_assert!`
    /// checks it). Wrap them in [`layout::vstack`] for a gap of their own.
    pub fn show<R>(
        self,
        ctx: &egui::Context,
        open: &mut bool,
        body: impl FnOnce(&mut Ui) -> R,
    ) -> Option<R> {
        if !*open {
            return None;
        }
        let t = Theme::current(ctx);
        let v = t.resolve(self.variant);
        let g = t.state(ctx, v.bits());
        let l = [v.part(&t.style, |_, s| &s.dialog_content, |r| &r.dialog_content)];
        let px = |f: fn(&Recipe) -> &crate::style::Prop<f32>| pick_len(&l, f, g);
        let (pad_t, pad_r, pad_b, pad_l) = (
            px(|r| &r.pad_top),
            px(|r| &r.pad_right),
            px(|r| &r.pad_bottom),
            px(|r| &r.pad_left),
        );
        let gap = px(|r| &r.gap);
        // The frame paints outside the panel's `Ui`: fade its colours here.
        let alpha = opacity(&[&l[..]], g);

        let max_width = self
            .width
            .or_else(|| pick(&l, |r| &r.max_width, g))
            .unwrap_or(f32::INFINITY);
        let width =
            layout::overlay_width(ctx, max_width, px(|r| &r.max_width_inset), pad_l + pad_r);
        let inner_width = width - pad_l - pad_r;

        let panel = RecipeBox::resolve(ctx, &t, &l, g).faded(alpha);
        let frame = panel.frame([pad_t, pad_r, pad_b, pad_l]);
        let overlay_l = [v.part(&t.style, |_, s| &s.dialog_overlay, |r| &r.dialog_overlay)];
        let overlay = pick(&overlay_l, |r| &r.bg, g).map_or(Color32::TRANSPARENT, |p| t.paint(p));
        // The overlay's opacity fades the backdrop, the app's colour too.
        let backdrop = self
            .backdrop
            .unwrap_or(overlay)
            .gamma_multiply(opacity(&[&overlay_l[..]], g));

        let resp = egui::Modal::new(self.id)
            .backdrop_color(backdrop)
            .frame(frame)
            .show(ctx, |ui| {
                let slot = RecipeBox::reserve(ui);
                ui.set_width(inner_width);
                ui.spacing_mut().item_spacing.y = gap;
                let parts = DialogContext {
                    variant: self.variant,
                };
                ui.scope_builder(context::push(UiBuilder::new(), parts), |ui| {
                    // The panel's `text-*` and text colour reach its content.
                    let look = TextLook::resolve(ui, &l, g);
                    content_scope(ui, &look, |ui| {
                        content_ui(ui, self.id.with("content"), &[&l[..]], g, |ui| {
                            let inner = free_content::body(ui, gap, 0.0, "dialog", |ui| {
                                free_content::section(ui, |ui| {
                                    header(ui, &t, &v, self.title, self.description)
                                });
                                body(ui)
                            });
                            let close =
                                self.show_close_button && close_button(ui, &t, &v, (pad_t, pad_r));
                            (inner, close, slot)
                        })
                    })
                    .inner
                })
                .inner
            });
        let dismissed = resp.backdrop_response.clicked() || resp.should_close();
        let (inner, close, slot) = resp.inner;
        panel.paint_reserved(ctx, resp.response.layer_id, slot, resp.response.rect);
        if close || dismissed {
            *open = false;
        }
        Some(inner)
    }
}

/// `dialog-header` with its title and description.
fn header(
    ui: &mut Ui,
    t: &Theme,
    v: &Resolved<DialogVariant>,
    title: &str,
    description: Option<&str>,
) {
    let g = t.state(ui.ctx(), v.bits());
    let header_l = [v.part(&t.style, |_, s| &s.dialog_header, |r| &r.dialog_header)];
    let gap = pick_len(&header_l, |r| &r.gap, g);
    let heading = [v.part(&t.style, |_, s| &s.dialog_title, |r| &r.dialog_title)];
    // upstream: dialog-title carries `cn-font-heading`; without a `text-*`
    // it takes the panel's, which the content scope passes.
    let title_look = TextLook::resolve(ui, &heading, g).heading(true);
    let desc = [v.part(
        &t.style,
        |_, s| &s.dialog_description,
        |r| &r.dialog_description,
    )];
    let desc_look = TextLook::resolve(ui, &desc, g);
    // The title and the description sit in the header.
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = gap;
        content_ui(
            ui,
            ui.id().with("shadcn_egui::dialog::header"),
            &[&header_l[..]],
            g,
            |ui| {
                part_ui(
                    ui,
                    ui.id().with("shadcn_egui::dialog::title"),
                    &[&heading[..]],
                    g,
                    |ui| title_look.label(ui, title, TextWrapMode::Wrap),
                );
                if let Some(d) = description {
                    part_ui(
                        ui,
                        ui.id().with("shadcn_egui::dialog::description"),
                        &[&desc[..]],
                        g,
                        |ui| desc_look.label(ui, d, TextWrapMode::Wrap),
                    );
                }
            },
        );
    });
}

/// The close button in the panel's top-right corner, `inset_*` from its
/// edges, painted with `cn-dialog-close` over its button recipes. Returns
/// true when clicked. It is placed without moving the cursor,
/// so it does not grow the panel.
fn close_button(ui: &mut Ui, t: &Theme, v: &Resolved<DialogVariant>, pad: (f32, f32)) -> bool {
    let g = t.state(ui.ctx(), v.bits());
    let close = v.part(&t.style, |_, s| &s.dialog_close, |r| &r.dialog_close);
    let at = [close];
    let (top, right) = (
        pick_len(&at, |r| &r.inset_top, g),
        pick_len(&at, |r| &r.inset_right, g),
    );
    let inner = ui.max_rect();
    let rect = Rect::from_min_max(
        egui::pos2(inner.left(), inner.top() - pad.0 + top),
        egui::pos2(inner.right() + pad.1 - right, inner.bottom()),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::right_to_left(Align::Min))
            .id_salt("shadcn_egui_dialog_close"),
    );
    child
        .add(
            Button::new("Close")
                .icon(&t.icons.close)
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::IconSm)
                .extra_layer(close.clone()),
        )
        .clicked()
}

/// `dialog-footer`: add the primary action last. In a window at least
/// `sm` (640 points) wide the actions are aligned right, left to right in
/// the order the code adds them (upstream's `sm:flex-row sm:justify-end`),
/// at the top of the row (no `items-*`: `align-items: stretch`); the row
/// is a crate row, so [`layout::stretch_height`] and shrinking
/// [`layout::vstack`]s work there as in a [`layout::row`]. In a narrower
/// window they stack, each as wide as the footer, the last one the code
/// adds on top (`flex-col-reverse`). In a `Ui` of unbounded width the
/// footer is as wide as its actions: from `sm` up as the row of them,
/// below as the widest one, each stacked action stretched to that width.
/// In a style with a footer strip
/// (nova) the strip bleeds to the dialog's edges and bottom.
///
/// The row is placed at the end from its width in the previous pass, the
/// stack from its height (and, in unbounded width, its widest action's
/// width); where that size changes, the pass is discarded and redone.
pub fn footer(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    free_content::closing_section(ui, |ui| footer_part(ui, add));
}

fn footer_part(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let id = ui.next_auto_id().with("shadcn_egui::dialog::footer");
    let t = Theme::current(ui.ctx());
    let v = dialog_recipes(ui, &t);
    let g = t.state(ui.ctx(), v.bits());
    let l = [v.part(&t.style, |_, s| &s.dialog_footer, |r| &r.dialog_footer)];
    let px = |f: fn(&Recipe) -> &crate::style::Prop<f32>| pick_len(&l, f, g);
    let (pad_t, pad_r, pad_b, pad_l) = (
        px(|r| &r.pad_top),
        px(|r| &r.pad_right),
        px(|r| &r.pad_bottom),
        px(|r| &r.pad_left),
    );
    let (m_r, m_b, m_l) = (
        px(|r| &r.margin_right),
        px(|r| &r.margin_bottom),
        px(|r| &r.margin_left),
    );
    let gap = pick_len(&l, |r| &r.gap, g);

    let at = ui.max_rect();
    let top = ui.cursor().top();
    let (left, right) = (at.left() + m_l, at.right() - m_r);
    let strip_bg = ui.painter().add(Shape::Noop);
    let inner = Rect::from_min_max(
        egui::pos2(left + pad_l, top + pad_t),
        egui::pos2(right - pad_r, top + pad_t + ui.spacing().interact_size.y),
    );
    let sm = g.contains(State::SM);
    // From `sm` up a crate row (`flex-row`, `align-items: stretch`: a
    // control with its own height sits at the top), so its items stretch
    // and shrink as in the browser.
    let mut row = ui.new_child(if sm {
        layout::row_builder(inner, Layout::left_to_right(Align::Min))
    } else {
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::left_to_right(Align::Center))
    });
    if sm {
        row.spacing_mut().item_spacing.x = gap;
        let items = layout::RowItems::begin(&row);
        layout::shrink_stacks(&mut row, |ui| {
            let width = layout::end_aligned(
                ui,
                id,
                Align::Min,
                "dialog::footer actions width changed",
                |ui| {
                    content_ui(ui, id, &[&l[..]], g, |ui| {
                        layout::hold_row_items(ui);
                        add(ui)
                    })
                },
            )
            .response
            .rect
            .width();
            ((), width)
        });
        items.finish(&row, false);
    } else {
        row.spacing_mut().item_spacing.y = gap;
        layout::reverse_column(
            &mut row,
            id.with("column"),
            "dialog::footer actions height changed",
            |ui| content_ui(ui, id, &[&l[..]], g, add),
        );
    }
    let bottom = row.min_rect().bottom().max(inner.top()) + pad_b;
    // Unbounded room leaves `justify-end` no free space: the footer is as
    // wide as its actions.
    let (right, outer_right) = if right.is_finite() {
        (right, at.right())
    } else {
        let right = row.min_rect().right() + pad_r;
        (right, right + m_r)
    };

    let strip = Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom));
    let painter = part_painter(ui, id, &[&l[..]], g);
    let shapes =
        RecipeBox::resolve(ui.ctx(), &t, &l, g).shapes_at(ui.ctx(), strip, painter.opacity());
    painter.set(strip_bg, Shape::Vec(shapes));
    // The negative bottom margin hands the panel's own bottom padding to the strip.
    ui.allocate_rect(
        Rect::from_min_max(
            egui::pos2(at.left(), top),
            egui::pos2(outer_right, bottom + m_b),
        ),
        egui::Sense::hover(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{AppVariant, Paint, Role, State};

    const BRAND: AppVariant<DialogVariant> = DialogVariant::app("brand");

    fn brand(r: &mut DialogRecipes, _: crate::StyleName) {
        r.dialog_footer.bg.set(Paint::Role(Role::Primary, 1.0));
    }

    /// An app variant's edit gets `DialogRecipes` and changes only what it
    /// names.
    #[test]
    fn edit_compiles_and_changes_its_part() {
        let t = Theme::vega().variant(BRAND, DialogVariant::Default, brand);
        let v = t.resolve(DialogVariant::from(BRAND));
        let footer = v.part(&t.style, |_, s| &s.dialog_footer, |r| &r.dialog_footer);
        assert_eq!(
            footer.bg.get(State::NONE),
            Some(Paint::Role(Role::Primary, 1.0))
        );
        let content = v.part(&t.style, |_, s| &s.dialog_content, |r| &r.dialog_content);
        assert_eq!(content, &*t.style.dialog_content);
    }

    /// The footer inside the dialog's body takes the dialog's variant.
    #[test]
    fn parts_read_the_dialogs_variant() {
        let ctx = egui::Context::default();
        let t = Theme::vega().variant(BRAND, DialogVariant::Default, brand);
        t.apply(&ctx);
        let _ = crate::test_ui::run(&ctx, egui::RawInput::default(), |ui| {
            let mut open = true;
            let seen = Dialog::new("d", "Title")
                .variant(BRAND)
                .show(ui.ctx(), &mut open, |ui| {
                    let t = Theme::current(ui.ctx());
                    let v = dialog_recipes(ui, &t);
                    v.part(&t.style, |_, s| &s.dialog_footer, |r| &r.dialog_footer)
                        .bg
                        .get(State::NONE)
                });
            assert_eq!(seen, Some(Some(Paint::Role(Role::Primary, 1.0))));
            let t = Theme::current(ui.ctx());
            assert_eq!(dialog_recipes(ui, &t).base(), DialogVariant::Default);
        });
    }
}
