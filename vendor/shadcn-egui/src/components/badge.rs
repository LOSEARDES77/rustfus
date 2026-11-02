//! shadcn Badge, ported from shadcn-svelte `ui/badge/badge.svelte` at
//! `tools/stylegen/PIN`. Every value comes from the style's badge
//! recipes: `cn-badge` and `cn-badge-variant-*`, stacked in that order.

use crate::color::Tone;
use crate::components::atom_row::AtomRow;
use crate::components::icon::Icon;
use crate::components::shared::{
    interaction_state, line_width, part_painter, transitioned_text, RecipeBox,
};
use crate::components::text::TextLook;
use crate::style::app::Sealed;
use crate::style::{
    pick, pick_len, AppEnum, AppVariant, Paint, Prop, Recipe, Role, State, Style, VariantEnum,
};
use crate::Theme;
use egui::{Response, Sense, Ui, Vec2, Widget};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum BadgeVariant {
    #[default]
    Default,
    Secondary,
    Destructive,
    Outline,
    Ghost,
    Link,
    /// An app variant: [`BadgeVariant::app`].
    #[doc(hidden)]
    App(AppVariant<BadgeVariant>),
}

impl BadgeVariant {
    /// An app variant, defined in the theme ([`Theme::variant`]).
    pub const fn app(name: &'static str) -> AppVariant<BadgeVariant> {
        AppVariant::new(name)
    }

    /// The variant layer (`cn-badge-variant-*`) of a built-in variant.
    pub(crate) fn layer(self, s: &Style) -> &Recipe {
        match self {
            BadgeVariant::Default | BadgeVariant::App(_) => &s.badge_variant_default,
            BadgeVariant::Secondary => &s.badge_variant_secondary,
            BadgeVariant::Destructive => &s.badge_variant_destructive,
            BadgeVariant::Outline => &s.badge_variant_outline,
            BadgeVariant::Ghost => &s.badge_variant_ghost,
            BadgeVariant::Link => &s.badge_variant_link,
        }
    }
}

impl From<AppVariant<BadgeVariant>> for BadgeVariant {
    fn from(app: AppVariant<BadgeVariant>) -> Self {
        BadgeVariant::App(app)
    }
}

/// The badge's recipes an app variant's edit changes: copies of the
/// base's, in the base's place in the stack.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct BadgeRecipes {
    /// `data-slot="badge"`: the variant's layer.
    pub badge: Recipe,
}

impl Sealed for BadgeVariant {}
impl AppEnum for BadgeVariant {
    type Recipes = BadgeRecipes;
    fn app_name(self) -> Option<&'static str> {
        match self {
            BadgeVariant::App(a) => Some(a.name()),
            _ => None,
        }
    }
    fn parts(r: &mut BadgeRecipes) -> Vec<&mut crate::style::Recipe> {
        vec![&mut r.badge]
    }
    fn recipes(self, style: &Style) -> BadgeRecipes {
        BadgeRecipes {
            badge: self.layer(style).clone(),
        }
    }
}
impl VariantEnum for BadgeVariant {}

pub struct Badge {
    text: String,
    variant: BadgeVariant,
    icon: Option<Icon>,
    icon_end: Option<Icon>,
}

impl Badge {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            variant: BadgeVariant::Default,
            icon: None,
            icon_end: None,
        }
    }
    pub fn variant(mut self, v: impl Into<BadgeVariant>) -> Self {
        self.variant = v.into();
        self
    }
    /// An icon before the text (shadcn `<Icon data-icon="inline-start" />`).
    pub fn icon(mut self, icon: &Icon) -> Self {
        self.icon = Some(*icon);
        self
    }
    /// An icon after the text (shadcn `<Icon data-icon="inline-end" />`).
    pub fn icon_end(mut self, icon: &Icon) -> Self {
        self.icon_end = Some(*icon);
        self
    }
}

/// A badge with the default variant order of the shadcn docs.
pub fn badge(ui: &mut Ui, text: &str, variant: impl Into<BadgeVariant>) -> Response {
    ui.add(Badge::new(text).variant(variant))
}

impl Widget for Badge {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = Theme::current(ui.ctx());
        let variant = t.resolve(self.variant);
        let l = [
            &*t.style.badge,
            variant.part(&t.style, BadgeVariant::layer, |r| &r.badge),
        ];
        let enabled = ui.is_enabled();
        // The style has no `[a]:hover` rules (a badge is not a link in
        // egui): the hover rules it has, and an app variant's, decide. A
        // badge is never focused.
        let context = variant
            .bits()
            .with(State::ICON_START, self.icon.is_some())
            .with(State::ICON_END, self.icon_end.is_some());
        // Geometry never depends on hover: resolve it before allocating.
        let g = t.state(ui.ctx(), context);
        let get = |f: fn(&Recipe) -> &Prop<f32>| pick_len(&l, f, g);
        let (pad_l, pad_r, gap, border_w) = (
            get(|r| &r.pad_left),
            get(|r| &r.pad_right),
            get(|r| &r.gap),
            line_width(get(|r| &r.border_width), ui.ctx().pixels_per_point()),
        );
        let icon_px = pick(&l, |r| &r.icon_size, g).unwrap_or(t.metrics.icon);
        // The text colour depends on the state, known only after allocating:
        // lay out in egui's placeholder colour, paint in the final one.
        let look = TextLook::resolve(ui, &l, g);
        let row = AtomRow::new(ui, self.text.as_str())
            .icons(self.icon, self.icon_end)
            .icon_size(icon_px)
            .gap(gap)
            .inset(
                pad_l + border_w,
                pad_r + border_w,
                get(|r| &r.pad_top) + border_w,
                get(|r| &r.pad_bottom) + border_w,
            )
            .lay_out(ui, &look);
        // No height: CSS's initial `auto`, the content's height.
        let height = pick(&l, |r| &r.height, g).unwrap_or(row.size().y);
        let width = row.size().x;
        // In a justified layout the box takes the layout's width, as egui's
        // `Button` does (conformance 3.1); the content stays centred in it.
        let (id, rect) = crate::layout::allocate_item_space(ui, Vec2::new(width, height));
        let resp = ui.interact(rect, id, Sense::hover());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, enabled, &self.text)
        });
        if !ui.is_rect_visible(rect) {
            return resp;
        }

        let st = interaction_state(&t, &resp, context);
        let painter = part_painter(ui, id, &[&l[..]], st);
        let tone = |f: fn(&Recipe) -> &Prop<Paint>| pick(&l, f, st).map(|p| t.tone(p));
        let inherited = t.role(Role::Foreground); // behaviour-ok: inherits Foreground; upstream sets no text colour
        let fg = tone(|r| &r.text).unwrap_or(Tone::of(inherited));
        let fg = transitioned_text(ui.ctx(), id, fg, &l, st);
        RecipeBox::resolve(ui.ctx(), &t, &l, st)
            .transitioned(ui.ctx(), id, &l, st)
            .paint(&painter, rect);

        // A state's `underline` (the link variant on hover) lays out again.
        row.paint(ui, &painter, rect, &TextLook::resolve(ui, &l, st), fg);
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An app variant's edit gets `BadgeRecipes` and changes only what it
    /// names; it keeps its base's behaviour (Ghost reacts to hover).
    #[test]
    fn edit_compiles_and_changes_its_layer() {
        const BRAND: Role = Role::custom("brand");
        const QUIET: AppVariant<BadgeVariant> = BadgeVariant::app("quiet");
        let t = Theme::vega()
            .color(BRAND, egui::Color32::RED, egui::Color32::BLUE)
            .variant(
                QUIET,
                BadgeVariant::Ghost,
                |r: &mut BadgeRecipes, _style| {
                    r.badge.text.on(State::HOVER, Paint::Role(BRAND, 1.0))
                },
            );
        let v = t.resolve(BadgeVariant::from(QUIET));
        assert_eq!(v.base(), BadgeVariant::Ghost);
        let l = v.part(&t.style, BadgeVariant::layer, |r| &r.badge);
        let ghost = &t.style.badge_variant_ghost;
        assert_eq!(l.bg, ghost.bg);
        assert_eq!(l.text.get(State::HOVER), Some(Paint::Role(BRAND, 1.0)));
        assert_eq!(l.text.get(State::NONE), None);
    }
}
