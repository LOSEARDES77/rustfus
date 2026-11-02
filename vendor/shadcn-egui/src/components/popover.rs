//! shadcn Popover, ported from shadcn-svelte `ui/popover/*.svelte` at
//! `tools/stylegen/PIN`. Width, padding, gap, radius, ring, shadow and text
//! come from the style's `cn-popover-content`, `cn-popover-header`,
//! `cn-popover-title` and `cn-popover-description` recipes.
//!
//! Behaviour follows bits-ui Popover: a click on the trigger toggles it, Esc
//! and a click outside close it. bits-ui also moves keyboard focus into the
//! content; egui does not, so focus stays where it was.
//!
//! egui remembers one open popup at a time, so opening a popover closes any
//! other popup (a menu, a combo box). Nest them only through `open_bool`
//! state of your own.

use crate::components::context;
use crate::components::free_content;
use crate::components::shared::{content_ui, opacity, part_ui, RecipeBox, DEFAULT_SIDE_OFFSET};
use crate::components::text::{content_scope, TextLook};
use crate::style::app::{default_variant, Resolved};
use crate::style::{pick, pick_len};
use crate::Theme;
use egui::{PopupCloseBehavior, Response, TextWrapMode, Ui, UiBuilder};

default_variant!(
    /// The popover's variant: upstream has none, so only app variants
    /// ([`PopoverVariant::app`]) differ from `Default`.
    PopoverVariant,
    /// The popover's recipes an app variant's edit changes.
    PopoverRecipes {
        /// `data-slot="popover-content"`: the panel.
        popover_content,
        /// `data-slot="popover-header"`.
        popover_header,
        /// `data-slot="popover-title"`.
        popover_title,
        /// `data-slot="popover-description"`.
        popover_description,
    }
);

/// The context a popover gives its parts: its variant.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct PopoverContext {
    variant: PopoverVariant,
}

/// The recipes of the popover around `ui`; outside a popover, the
/// default's.
fn popover_recipes(ui: &Ui, t: &Theme) -> Resolved<PopoverVariant> {
    let cx = context::find::<PopoverContext>(ui).unwrap_or_default();
    t.resolve(cx.variant)
}

pub struct Popover<'a> {
    trigger: &'a Response,
    variant: PopoverVariant,
    width: Option<f32>,
    side_offset: f32,
}

impl<'a> Popover<'a> {
    /// A popover toggled by clicks on `trigger`.
    pub fn new(trigger: &'a Response) -> Self {
        Self {
            trigger,
            variant: PopoverVariant::Default,
            width: None,
            side_offset: DEFAULT_SIDE_OFFSET,
        }
    }
    /// An app variant ([`PopoverVariant::app`]). The popover's parts inside
    /// take it.
    pub fn variant(mut self, v: impl Into<PopoverVariant>) -> Self {
        self.variant = v.into();
        self
    }
    /// Width of the popover box, padding included; default the style's
    /// `w-72`.
    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }
    /// Gap between the trigger and the popover (shadcn `sideOffset`, default 4).
    pub fn side_offset(mut self, px: f32) -> Self {
        self.side_offset = px;
        self
    }
    /// Show the content while open. Returns its value while open. Widgets
    /// of the caller's own stand for the element upstream's demos put
    /// there, a block (rules, Container): they stack with no gap, the
    /// style's gap from [`popover_header`]. Wrap them in
    /// [`crate::layout::vstack`] for a gap of their own.
    pub fn show<R>(self, content: impl FnOnce(&mut Ui) -> R) -> Option<R> {
        let t = Theme::current(&self.trigger.ctx);
        let v = t.resolve(self.variant);
        let g = t.state(&self.trigger.ctx, v.bits());
        let l = [v.part(&t.style, |_, s| &s.popover_content, |r| &r.popover_content)];
        let px = |f: fn(&crate::style::Recipe) -> &crate::style::Prop<f32>| pick_len(&l, f, g);
        let (pad_t, pad_r, pad_b, pad_l) = (
            px(|r| &r.pad_top),
            px(|r| &r.pad_right),
            px(|r| &r.pad_bottom),
            px(|r| &r.pad_left),
        );
        let gap = px(|r| &r.gap);
        // The frame paints outside the content's `Ui`: fade its colours here.
        let alpha = opacity(&[&l[..]], g);
        let panel = RecipeBox::resolve(&self.trigger.ctx, &t, &l, g).faded(alpha);
        let frame = panel.frame([pad_t, pad_r, pad_b, pad_l]);
        // No width from the builder or the recipe: CSS's initial `auto`,
        // the content's width.
        let inner_width = self
            .width
            .or_else(|| pick(&l, |r| &r.width, g))
            .map(|w| (w - pad_l - pad_r).max(0.0));
        let shown = egui::Popup::from_toggle_button_response(self.trigger)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .gap(self.side_offset)
            .frame(frame)
            .show(|ui| {
                let slot = RecipeBox::reserve(ui);
                if let Some(w) = inner_width {
                    ui.set_width(w);
                }
                ui.spacing_mut().item_spacing.y = gap;
                let parts = PopoverContext {
                    variant: self.variant,
                };
                let inner = ui.scope_builder(context::push(UiBuilder::new(), parts), |ui| {
                    // The content's `text-*` and text colour reach what it holds.
                    let look = TextLook::resolve(ui, &l, g);
                    content_scope(ui, &look, |ui| {
                        content_ui(
                            ui,
                            ui.id().with("shadcn_egui::popover::content"),
                            &[&l[..]],
                            g,
                            |ui| free_content::body(ui, gap, 0.0, "popover", content),
                        )
                    })
                    .inner
                });
                (inner.inner, slot)
            })?;
        let (inner, slot) = shown.inner;
        panel.paint_reserved(
            &self.trigger.ctx,
            shown.response.layer_id,
            slot,
            shown.response.rect,
        );
        Some(inner)
    }
}

/// shadcn `PopoverHeader` with its `PopoverTitle` and `PopoverDescription`.
/// Each line takes its size and line height from its own recipe, over the
/// header's.
pub fn popover_header(ui: &mut Ui, title: &str, description: &str) {
    free_content::section(ui, |ui| header(ui, title, description));
}

fn header(ui: &mut Ui, title: &str, description: &str) {
    let t = Theme::current(ui.ctx());
    let v = popover_recipes(ui, &t);
    let g = t.state(ui.ctx(), v.bits());
    let header = v.part(&t.style, |_, s| &s.popover_header, |r| &r.popover_header);
    let title_r = v.part(&t.style, |_, s| &s.popover_title, |r| &r.popover_title);
    let desc_r = v.part(
        &t.style,
        |_, s| &s.popover_description,
        |r| &r.popover_description,
    );
    let content = v.part(&t.style, |_, s| &s.popover_content, |r| &r.popover_content);
    let title_l = [header, title_r];
    let desc_l = [header, desc_r];
    let gap = pick_len(&[header], |r| &r.gap, g);
    let title_look = TextLook::resolve(ui, &title_l, g);
    let content_color = pick(&[content], |r| &r.text, g).map(|p| t.paint(p));
    let title_look = match (title_look.recipe_color(), content_color) {
        (None, Some(c)) => title_look.color(c),
        _ => title_look,
    };
    let desc_look = TextLook::resolve(ui, &desc_l, g);
    // The title and the description sit in the header.
    let (title_l, desc_l2, header_l) = ([title_r], [desc_r], [header]);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = gap;
        content_ui(
            ui,
            ui.id().with("shadcn_egui::popover::header"),
            &[&header_l[..]],
            g,
            |ui| {
                part_ui(
                    ui,
                    ui.id().with("shadcn_egui::popover::title"),
                    &[&title_l[..]],
                    g,
                    |ui| title_look.label(ui, title, TextWrapMode::Wrap),
                );
                part_ui(
                    ui,
                    ui.id().with("shadcn_egui::popover::description"),
                    &[&desc_l2[..]],
                    g,
                    |ui| desc_look.label(ui, description, TextWrapMode::Wrap),
                );
            },
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{AppVariant, Paint, Role, State, Style, TextSize};

    /// The title and description sizes come from their own recipes, which
    /// differ from the header's in lyra and maia.
    #[test]
    fn header_lines_read_their_own_recipes() {
        let lyra = Style::lyra();
        assert_eq!(
            pick(&[&lyra.popover_header], |r| &r.text_size, State::NONE),
            Some(TextSize::Xs)
        );
        assert_eq!(
            pick(
                &[&lyra.popover_header, &lyra.popover_title],
                |r| &r.text_size,
                State::NONE
            ),
            Some(TextSize::Sm)
        );
        assert_eq!(
            pick(
                &[&lyra.popover_header, &lyra.popover_description],
                |r| &r.line_height,
                State::NONE
            ),
            Some(1.625)
        );
        let maia = Style::maia();
        assert_eq!(
            pick(
                &[&maia.popover_header, &maia.popover_title],
                |r| &r.text_size,
                State::NONE
            ),
            Some(TextSize::Base)
        );
    }

    /// An app variant's edit gets `PopoverRecipes` and changes only what it
    /// names.
    #[test]
    fn edit_compiles_and_changes_its_part() {
        const BRAND: AppVariant<PopoverVariant> = PopoverVariant::app("brand");
        let t = Theme::vega().variant(
            BRAND,
            PopoverVariant::Default,
            |r: &mut PopoverRecipes, _| r.popover_title.text.set(Paint::Role(Role::Primary, 1.0)),
        );
        let v = t.resolve(PopoverVariant::from(BRAND));
        let title = v.part(&t.style, |_, s| &s.popover_title, |r| &r.popover_title);
        assert_eq!(
            title.text.get(State::NONE),
            Some(Paint::Role(Role::Primary, 1.0))
        );
        let header = v.part(&t.style, |_, s| &s.popover_header, |r| &r.popover_header);
        assert_eq!(header, &*t.style.popover_header);
    }
}
