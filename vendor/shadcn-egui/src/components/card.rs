//! shadcn Card, ported from shadcn-svelte `ui/card/*.svelte` at
//! `tools/stylegen/PIN`. Padding,
//! gap, radius, ring, shadow, the footer strip and the text of the title and
//! description come from the style's `cn-card*` recipes.
//!
//! The card is a column of header, content and footer, the `cn-card` `gap`
//! apart. egui has no flex column, so [`Card::show`] lays the sections out
//! by hand: the card paints no padding on the sides of its sections, each
//! section insets itself, and the footer strip can therefore reach the
//! card's edges. Inside [`card_content`], only vega spaces the children
//! (`cn-card-content` `gap`); see its docs.
//!
//! Header, content and footer all inset their children by the same side
//! padding in every style (`px-(--card-spacing)`), so [`Card::show`] insets
//! the body once and [`card_header`] adds nothing of its own on the sides.
//!
//! `cn-card-header`'s `rounded-*` (vega `rounded-t-xl`, lyra `rounded-none`)
//! is not read: the header paints no fill or border in any style, so its
//! corners have nothing to shape.

use crate::components::context;
use crate::components::free_content::{self, section};
use crate::components::shared::{content_ui, part_painter, part_ui, RecipeBox};
use crate::components::text::{content_scope, TextLook};
use crate::style::app::{default_variant, state_size, ResolvedPair};
use crate::style::{pick_len, State};
use crate::{layout, Theme};
use egui::{
    Align, Layout, Rect, Response, Sense, Shape, TextWrapMode, Ui, UiBuilder, WidgetInfo,
    WidgetType,
};

/// `data-size` of a card: `default` or `sm`, or an app size
/// ([`CardSize::app`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CardSize {
    #[default]
    Default,
    Sm,
    /// An app size: [`CardSize::app`].
    #[doc(hidden)]
    App(crate::style::AppVariant<CardSize>),
}

state_size!(CardSize, CardVariant);

default_variant!(
    /// The card's variant: upstream has none, so only app variants
    /// ([`CardVariant::app`]) differ from `Default`.
    CardVariant,
    /// The card's recipes an app variant's or app size's edit changes.
    CardRecipes {
        /// `data-slot="card"`: the surface.
        card,
        /// `data-slot="card-header"`.
        card_header,
        /// `data-slot="card-title"`.
        card_title,
        /// `data-slot="card-description"`.
        card_description,
        /// `data-slot="card-action"`.
        card_action,
        /// `data-slot="card-content"`.
        card_content,
        /// `data-slot="card-footer"`.
        card_footer,
    }
);

/// The context a card gives its parts: its variant and its size
/// (`group-data-[size=sm]/card`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct CardContext {
    variant: CardVariant,
    size: CardSize,
}

/// The context of the card around `ui`; outside a card, the default.
fn card_context(ui: &Ui) -> CardContext {
    context::find::<CardContext>(ui).unwrap_or_default()
}

/// The recipes of the card around `ui`.
fn card_recipes(ui: &Ui, t: &Theme) -> ResolvedPair<CardVariant, CardSize> {
    let cx = card_context(ui);
    t.resolve_pair(cx.variant, cx.size)
}

type FooterFn<'a> = Box<dyn FnOnce(&mut Ui) + 'a>;

/// A shadcn card: `Card::new().size(..).footer(..).show(ui, |ui| ..)`.
pub struct Card<'a> {
    size: CardSize,
    variant: CardVariant,
    footer: Option<FooterFn<'a>>,
}

impl Default for Card<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Card<'a> {
    pub fn new() -> Self {
        Self {
            size: CardSize::Default,
            variant: CardVariant::Default,
            footer: None,
        }
    }
    /// `data-size`: `Default` or the tighter `Sm`, or an app size
    /// ([`CardSize::app`]). The card's parts inside take it.
    pub fn size(mut self, size: impl Into<CardSize>) -> Self {
        self.size = size.into();
        self
    }
    /// An app variant ([`CardVariant::app`]). The card's parts inside take
    /// it.
    pub fn variant(mut self, v: impl Into<CardVariant>) -> Self {
        self.variant = v.into();
        self
    }
    /// A `card-footer` row: its children sit in a row, centred on the
    /// cross axis. In a style with a footer strip (nova) the strip reaches
    /// the card's edges.
    pub fn footer(mut self, footer: impl FnOnce(&mut Ui) + 'a) -> Self {
        self.footer = Some(Box::new(footer));
        self
    }

    /// Show the card. `add` fills the body: [`card_header`] and
    /// [`card_content`] sections, `gap` apart. Widgets of the caller's own
    /// between two sections stand for one more `card-content` (rules,
    /// Container): they are `cn-card-content`'s gap apart, as inside
    /// [`card_content`], and `gap` from the sections around them.
    ///
    /// Upstream's card sets no width (conformance 3.1): in a column it
    /// spans the available width, as a block does; in a crate row
    /// ([`layout::row`], [`layout::wrap_row`], [`layout::space_between`],
    /// a card footer) it is a flex item as a [`layout::vstack`] is, as
    /// wide as its content and shrinking where the row has no room for
    /// it. Every style's card is `overflow-hidden`, so it shrinks below
    /// its narrowest content (a scroll container's minimum width in a
    /// flex row is 0) and, in a row as in a column, clips content wider
    /// than itself at its box. Where the width is unbounded (an
    /// `allocate_ui` of infinite width, a crate row measuring it) it takes
    /// its content's width. A raw egui widget that fills whatever width it
    /// gets (`add_sized` with an infinite width) makes a card in unbounded
    /// room infinitely wide, as it makes an egui `Frame` around it; a crate
    /// row with room gives such a card the room its other items leave.
    pub fn show<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
        let id = ui.next_auto_id().with("shadcn_egui::components::card");
        match layout::as_flex_item(ui, id, true, move |ui: &mut Ui| self.show_box(ui, add)) {
            Ok(inner) => inner,
            Err(show) => show(ui),
        }
    }

    fn show_box<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
        let t = Theme::current(ui.ctx());
        let has_footer = self.footer.is_some();
        let parts = CardContext {
            variant: self.variant,
            size: self.size,
        };
        let v = t.resolve_pair(self.variant, self.size);
        let st = t.state(
            ui.ctx(),
            v.bits()
                .with(State::HAS_FOOTER, has_footer)
                .with(State::DISABLED, !ui.is_enabled()),
        );
        let card = [v.part(&t.style, |s| &s.card, |r| &r.card)];
        let content = [v.part(&t.style, |s| &s.card_content, |r| &r.card_content)];
        let foot = [v.part(&t.style, |s| &s.card_footer, |r| &r.card_footer)];

        let (pad_top, pad_bottom, gap) = (
            pick_len(&card, |r| &r.pad_top, st),
            pick_len(&card, |r| &r.pad_bottom, st),
            pick_len(&card, |r| &r.gap, st),
        );
        let (side_l, side_r) = (
            pick_len(&content, |r| &r.pad_left, st),
            pick_len(&content, |r| &r.pad_right, st),
        );
        // The card's `text-*` and text colour reach its content.
        let look = TextLook::resolve(ui, &card, st);
        // Header, content and footer sit in the card; the card's parts
        // key their transitions under its own id.
        let id = ui.next_auto_id();
        let painter = part_painter(ui, id.with("shadcn_egui::card"), &[&card[..]], st);

        // A block fills the room left; where that is unbounded, the card
        // is laid out unbounded and takes its content's width.
        let bounded = ui.available_width().is_finite();
        let fill = layout::capped_width(ui, f32::INFINITY, side_l + side_r);
        let outer_w = if bounded { fill } else { f32::INFINITY };
        let (origin, mut frame) = crate::layout::begin_item(ui);
        let back = ui.painter().add(Shape::Noop);
        // Every style's card is `overflow-hidden`: content wider than a
        // bounded card is cut at its box (no style gives it a border, so
        // the padding box is the border box).
        let clip = |child: &mut Ui| {
            if bounded {
                let mut r = child.clip_rect();
                r.min.x = r.min.x.max(origin.x);
                r.max.x = r.max.x.min(origin.x + outer_w);
                child.set_clip_rect(r);
            }
        };

        // Header and content.
        let body_rect = Rect::from_min_max(
            egui::pos2(origin.x + side_l, origin.y + pad_top),
            egui::pos2(
                origin.x + outer_w - side_r,
                origin.y + pad_top + ui.available_height().max(0.0),
            ),
        );
        let mut body = frame.new_child(context::push(
            UiBuilder::new()
                .max_rect(body_rect)
                .layout(Layout::top_down(Align::Min)),
            parts,
        ));
        // Free content stands for a `card-content` (rules, Container): its
        // widgets are `cn-card-content`'s gap apart, and the parts keep the
        // card's gap around themselves.
        let content_gap = pick_len(&content, |r| &r.gap, st);
        clip(&mut body);
        let result = content_scope(&mut body, &look, |ui| {
            content_ui(
                ui,
                id.with("shadcn_egui::card::content"),
                &[&card[..]],
                st,
                |ui| free_content::body(ui, gap, content_gap, "card", add),
            )
        })
        .inner;
        let mut bottom = body.min_rect().bottom().max(body_rect.top());
        let mut content_w = body.min_rect().right() - origin.x + side_r;

        // Footer.
        let mut footer_shapes = None;
        if let Some(footer) = self.footer {
            let (f_top, f_right, f_bottom, f_left) = (
                pick_len(&foot, |r| &r.pad_top, st),
                pick_len(&foot, |r| &r.pad_right, st),
                pick_len(&foot, |r| &r.pad_bottom, st),
                pick_len(&foot, |r| &r.pad_left, st),
            );
            let strip_top = bottom + gap;
            let strip_bg = ui.painter().add(Shape::Noop);
            // A crate row as high as its content, as `layout::row`.
            let band_id = id.with("shadcn_egui::card::footer_height");
            let (stored, band, align) = crate::layout::row_band(ui, band_id);
            let inner = Rect::from_min_max(
                egui::pos2(origin.x + f_left, strip_top + f_top),
                egui::pos2(origin.x + outer_w - f_right, strip_top + f_top + band),
            );
            let mut row = frame.new_child(context::push(
                crate::layout::row_builder(inner, Layout::left_to_right(align)),
                parts,
            ));
            row.spacing_mut().item_spacing.x = pick_len(&foot, |r| &r.gap, st);
            clip(&mut row);
            let items = crate::layout::RowItems::begin(&row);
            crate::layout::shrink_stacks(&mut row, |ui| {
                content_scope(ui, &look, |ui| {
                    content_ui(
                        ui,
                        id.with("shadcn_egui::card::footer"),
                        &[&foot[..], &card[..]],
                        st,
                        |ui| {
                            crate::layout::hold_row_items(ui);
                            footer(ui);
                        },
                    )
                });
                ((), ui.min_rect().width())
            });
            let lines = items.finish(&row, false);
            crate::layout::settle_row_band(
                ui,
                band_id,
                stored,
                band,
                &lines,
                "card::footer content height changed",
            );
            content_w = content_w.max(row.min_rect().right() - origin.x + f_right);
            let strip_bottom = row.min_rect().bottom().max(inner.top()) + f_bottom;
            footer_shapes = Some((strip_bg, strip_top, strip_bottom));
            bottom = strip_bottom;
        }
        let card_bottom = bottom + pad_bottom;
        // Bounded, the card keeps its width and clips what overflows it,
        // also as a flex item: a scroll container's automatic minimum
        // width is 0, so the row learns no narrowest width from it.
        let outer_w = if bounded {
            outer_w
        } else {
            content_w.max(side_l + side_r)
        };
        let outer = Rect::from_min_max(origin, egui::pos2(origin.x + outer_w, card_bottom));

        // Paint behind the sections: the card's box, then the footer strip.
        painter.set(
            back,
            Shape::Vec(RecipeBox::resolve(ui.ctx(), &t, &card, st).shapes_at(
                ui.ctx(),
                outer,
                painter.opacity(),
            )),
        );
        if let Some((idx, top, bottom)) = footer_shapes {
            let strip = Rect::from_min_max(
                egui::pos2(outer.left(), top),
                egui::pos2(outer.right(), bottom),
            );
            let painter = part_painter(
                ui,
                id.with("shadcn_egui::card::footer"),
                &[&foot[..], &card[..]],
                st,
            );
            let shapes = RecipeBox::resolve(ui.ctx(), &t, &foot, st).shapes_at(
                ui.ctx(),
                strip,
                painter.opacity(),
            );
            painter.set(idx, Shape::Vec(shapes));
        }

        let response = crate::layout::end_item(ui, frame, outer, Sense::hover());
        response.widget_info(|| WidgetInfo::new(WidgetType::Panel));
        result
    }
}

/// `card-content` section. Its children are the style's `cn-card-content`
/// `gap` apart, which only vega sets (12 px); in nova, maia, lyra, mira,
/// luma and rhea they touch. Wrap them in [`crate::layout::vstack`] for the
/// design's spacing.
pub fn card_content<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    section(ui, |ui| content_section(ui, add))
}

fn content_section<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Theme::current(ui.ctx());
    let v = card_recipes(ui, &t);
    let st = card_state(ui, &v);
    let l = [v.part(&t.style, |s| &s.card_content, |r| &r.card_content)];
    let gap = pick_len(&l, |r| &r.gap, st);
    let id = ui.next_auto_id().with("shadcn_egui::card::part");
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = gap;
        content_ui(ui, id, &[&l[..]], st, add)
    })
    .inner
}

/// `card-header` with its title and description.
pub fn card_header(ui: &mut Ui, title: &str, description: &str) {
    section(ui, |ui| header(ui, title, description));
}

fn header(ui: &mut Ui, title: &str, description: &str) {
    let t = Theme::current(ui.ctx());
    let v = card_recipes(ui, &t);
    let header = [v.part(&t.style, |s| &s.card_header, |r| &r.card_header)];
    content_ui(
        ui,
        ui.next_auto_id().with("shadcn_egui::card::header"),
        &[&header[..]],
        card_state(ui, &v),
        |ui| header_column(ui, title, description),
    );
}

/// The state of a card part: the current card's variant and size bits,
/// disabled with the `Ui`.
fn card_state(ui: &Ui, v: &ResolvedPair<CardVariant, CardSize>) -> State {
    let t = Theme::current(ui.ctx());
    t.state(ui.ctx(), v.bits().with(State::DISABLED, !ui.is_enabled()))
}

/// `card-header` with a `card-action` to the right of the title and
/// description, aligned to the top. The action's widgets show left to
/// right in the order the code adds them.
///
/// The action is placed at the end from its width in the previous pass;
/// where that width changes, the pass is discarded and redone.
pub fn card_header_with_action(
    ui: &mut Ui,
    title: &str,
    description: &str,
    action: impl FnOnce(&mut Ui),
) {
    section(ui, |ui| header_with_action(ui, title, description, action));
}

fn header_with_action(ui: &mut Ui, title: &str, description: &str, action: impl FnOnce(&mut Ui)) {
    // The action sits in the header, beside the title and description.
    let t = Theme::current(ui.ctx());
    let v = card_recipes(ui, &t);
    let st = card_state(ui, &v);
    let header = [v.part(&t.style, |s| &s.card_header, |r| &r.card_header)];
    let card_action = [v.part(&t.style, |s| &s.card_action, |r| &r.card_action)];
    let id = ui.next_auto_id().with("shadcn_egui::card::action");
    content_ui(ui, id, &[&header[..]], st, |ui| {
        ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
            // The title column takes the room the action left in the
            // previous pass; the action keeps the order the code adds its
            // widgets in and ends at the header's end.
            let action_w =
                layout::end_aligned_width(ui, id).map_or(0.0, |w| w + ui.spacing().item_spacing.x);
            let width = ui.available_width() - action_w;
            ui.allocate_ui_with_layout(
                egui::vec2(width, 0.0),
                Layout::top_down(Align::Min),
                |ui| header_column(ui, title, description),
            );
            layout::end_aligned(
                ui,
                id,
                Align::Min,
                "card::card_header_with_action action width changed",
                |ui| content_ui(ui, id.with("action"), &[&card_action[..]], st, action),
            );
        })
    });
}

fn header_column(ui: &mut Ui, title: &str, description: &str) {
    let t = Theme::current(ui.ctx());
    let v = card_recipes(ui, &t);
    let gap = pick_len(
        &[v.part(&t.style, |s| &s.card_header, |r| &r.card_header)],
        |r| &r.gap,
        t.state(ui.ctx(), v.bits()),
    );
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = gap;
        card_title(ui, title);
        card_description(ui, description);
    });
}

/// `card-title`: size, weight and line height of the style.
pub fn card_title(ui: &mut Ui, text: &str) -> Response {
    section(ui, |ui| title(ui, text))
}

fn title(ui: &mut Ui, text: &str) -> Response {
    let t = Theme::current(ui.ctx());
    let v = card_recipes(ui, &t);
    let st = t.state(ui.ctx(), v.bits());
    let l = [v.part(&t.style, |s| &s.card_title, |r| &r.card_title)];
    // upstream: card-title carries `cn-font-heading`
    let look = TextLook::resolve(ui, &l, st).heading(true);
    part_ui(
        ui,
        ui.next_auto_id().with("shadcn_egui::card::title"),
        &[&l[..]],
        st,
        |ui| look.label(ui, text, TextWrapMode::Wrap),
    )
}

/// `card-description`: muted text at the style's size.
pub fn card_description(ui: &mut Ui, text: &str) -> Response {
    section(ui, |ui| description(ui, text))
}

fn description(ui: &mut Ui, text: &str) -> Response {
    let t = Theme::current(ui.ctx());
    let v = card_recipes(ui, &t);
    let st = t.state(ui.ctx(), v.bits());
    let l = [v.part(&t.style, |s| &s.card_description, |r| &r.card_description)];
    let look = TextLook::resolve(ui, &l, st);
    part_ui(
        ui,
        ui.next_auto_id().with("shadcn_egui::card::description"),
        &[&l[..]],
        st,
        |ui| look.label(ui, text, TextWrapMode::Wrap),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{pick, AppVariant, Paint, Recipe, Role};

    /// An app variant's edit gets `CardRecipes` and changes only what it
    /// names; an app size edits the same copy after it.
    #[test]
    fn edit_compiles_and_changes_its_part() {
        const BRAND: AppVariant<CardVariant> = CardVariant::app("brand");
        const ROOMY: AppVariant<CardSize> = CardSize::app("roomy");
        let t = Theme::vega()
            .variant(BRAND, CardVariant::Default, |r: &mut CardRecipes, _| {
                r.card_title.text.set(Paint::Role(Role::Primary, 1.0))
            })
            .size(ROOMY, CardSize::Sm, |r, _| {
                r.card.gap.on(State::SIZE_SM, 40.0)
            });
        let v = t.resolve_pair(CardVariant::from(BRAND), CardSize::from(ROOMY));
        assert_eq!(v.bits(), State::SIZE_SM);
        let title = v.part(&t.style, |s| &s.card_title, |r| &r.card_title);
        assert_eq!(
            title.text.get(State::NONE),
            Some(Paint::Role(Role::Primary, 1.0))
        );
        let card = v.part(&t.style, |s| &s.card, |r| &r.card);
        assert_eq!(pick(&[card], |r| &r.gap, State::SIZE_SM), Some(40.0));
        // An unedited part is `Sm`'s, with `Sm`'s values unconditional.
        let footer = v.part(&t.style, |s| &s.card_footer, |r| &r.card_footer);
        let mut sm = (*t.style.card_footer).clone();
        sm.fold_state(State::SIZE_SM);
        assert_eq!(footer, &sm);
    }

    /// An app size on `Sm` draws `Sm`'s values, and its `set` replaces
    /// them, though `Sm` gives them through `data-[size=sm]:` rules.
    #[test]
    fn app_size_set_replaces_its_base_size_value() {
        const BRAND: AppVariant<CardVariant> = CardVariant::app("brand");
        const TIGHT: AppVariant<CardSize> = CardSize::app("tight");
        let t = Theme::vega()
            .variant(BRAND, CardVariant::Default, |_, _| {})
            .size(TIGHT, CardSize::Sm, |r, _| r.card.gap.set(8.0));
        let sm =
            |p: fn(&Recipe) -> &crate::style::Prop<f32>| pick(&[&*t.style.card], p, State::SIZE_SM);
        for v in [CardVariant::Default, CardVariant::from(BRAND)] {
            let r = t.resolve_pair(v, CardSize::from(TIGHT));
            let card = r.part(&t.style, |s| &s.card, |r| &r.card);
            assert_eq!(pick(&[card], |r| &r.gap, r.bits()), Some(8.0));
            assert_eq!(pick(&[card], |r| &r.pad_top, r.bits()), sm(|r| &r.pad_top));
        }
    }

    /// The card's parts take its variant from the context.
    #[test]
    fn parts_read_the_cards_variant() {
        const BRAND: AppVariant<CardVariant> = CardVariant::app("brand");
        let ctx = egui::Context::default();
        Theme::vega()
            .variant(BRAND, CardVariant::Default, |_, _| {})
            .apply(&ctx);
        let _ = crate::test_ui::run(&ctx, egui::RawInput::default(), |ui| {
            Card::new().variant(BRAND).show(ui, |ui| {
                assert_eq!(card_context(ui).variant, CardVariant::from(BRAND));
                Card::new().show(ui, |ui| {
                    assert_eq!(card_context(ui).variant, CardVariant::Default)
                });
            });
        });
    }

    /// Each card part reads the size of its nearest card; outside a card
    /// it reads the default.
    #[test]
    fn parts_read_the_nearest_cards_size() {
        let ctx = egui::Context::default();
        let _ = crate::test_ui::run(&ctx, egui::RawInput::default(), |ui| {
            assert_eq!(card_context(ui).size, CardSize::Default);
            Card::new()
                .size(CardSize::Sm)
                .footer(|ui| assert_eq!(card_context(ui).size, CardSize::Sm))
                .show(ui, |ui| {
                    assert_eq!(card_context(ui).size, CardSize::Sm);
                    Card::new().show(ui, |ui| {
                        assert_eq!(card_context(ui).size, CardSize::Default);
                        Card::new()
                            .size(CardSize::Sm)
                            .show(ui, |ui| assert_eq!(card_context(ui).size, CardSize::Sm));
                    });
                    assert_eq!(card_context(ui).size, CardSize::Sm);
                });
            assert_eq!(card_context(ui).size, CardSize::Default);
        });
    }
}
