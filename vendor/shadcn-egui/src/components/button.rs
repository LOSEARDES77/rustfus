//! shadcn Button, ported from shadcn-svelte `ui/button/button.svelte` at
//! `tools/stylegen/PIN`. Every value comes from the style's button
//! recipes: `cn-button`, `cn-button-variant-*` and `cn-button-size-*`,
//! stacked in that order.

use crate::color::Tone;
use crate::components::atom_row::AtomRow;
use crate::components::icon::Icon;
use crate::components::motion::transition;
use crate::components::shared::{
    interaction_state, line_width, part_painter, pressable, report_invalid, shift,
    transitioned_text, ActivationKeys, RecipeBox,
};
use crate::components::text::TextLook;
use crate::style::app::Sealed;
use crate::style::Transition;
use crate::style::{
    pick, pick_len, AppEnum, AppVariant, Paint, Prop, Recipe, Role, SizeEnum, State, Style,
    VariantEnum,
};
use crate::Theme;
use egui::{Response, Sense, Ui, Vec2, Widget};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ButtonVariant {
    #[default]
    Default,
    Outline,
    Secondary,
    Ghost,
    Destructive,
    Link,
    /// An app variant: [`ButtonVariant::app`].
    #[doc(hidden)]
    App(AppVariant<ButtonVariant>),
}

impl ButtonVariant {
    /// An app variant, defined in the theme ([`Theme::variant`]).
    pub const fn app(name: &'static str) -> AppVariant<ButtonVariant> {
        AppVariant::new(name)
    }

    /// The variant layer (`cn-button-variant-*`) of a built-in variant.
    pub(crate) fn layer(self, s: &Style) -> &Recipe {
        match self {
            ButtonVariant::Default | ButtonVariant::App(_) => &s.button_variant_default,
            ButtonVariant::Outline => &s.button_variant_outline,
            ButtonVariant::Secondary => &s.button_variant_secondary,
            ButtonVariant::Ghost => &s.button_variant_ghost,
            ButtonVariant::Destructive => &s.button_variant_destructive,
            ButtonVariant::Link => &s.button_variant_link,
        }
    }
}

impl From<AppVariant<ButtonVariant>> for ButtonVariant {
    fn from(app: AppVariant<ButtonVariant>) -> Self {
        ButtonVariant::App(app)
    }
}

/// The button's recipes an app variant's edit changes: copies of the
/// base's, in the base's place in the stack.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ButtonRecipes {
    /// `data-slot="button"`: the variant's (or size's) layer.
    pub button: Recipe,
}

impl Sealed for ButtonVariant {}
impl AppEnum for ButtonVariant {
    type Recipes = ButtonRecipes;
    fn app_name(self) -> Option<&'static str> {
        match self {
            ButtonVariant::App(a) => Some(a.name()),
            _ => None,
        }
    }
    fn parts(r: &mut ButtonRecipes) -> Vec<&mut crate::style::Recipe> {
        vec![&mut r.button]
    }
    fn recipes(self, style: &Style) -> ButtonRecipes {
        ButtonRecipes {
            button: self.layer(style).clone(),
        }
    }
}
impl VariantEnum for ButtonVariant {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ButtonSize {
    Xs,
    Sm,
    #[default]
    Default,
    Lg,
    /// Square, icon only; the text is the accessible name.
    Icon,
    IconXs,
    IconSm,
    IconLg,
    /// An app size: [`ButtonSize::app`].
    #[doc(hidden)]
    App(AppVariant<ButtonSize>),
}

impl ButtonSize {
    /// An app size, defined in the theme ([`Theme::size`]).
    pub const fn app(name: &'static str) -> AppVariant<ButtonSize> {
        AppVariant::new(name)
    }

    /// The size layer (`cn-button-size-*`) of a built-in size.
    pub(crate) fn layer(self, s: &Style) -> &Recipe {
        match self {
            ButtonSize::Xs => &s.button_size_xs,
            ButtonSize::Sm => &s.button_size_sm,
            ButtonSize::Default | ButtonSize::App(_) => &s.button_size_default,
            ButtonSize::Lg => &s.button_size_lg,
            ButtonSize::Icon => &s.button_size_icon,
            ButtonSize::IconXs => &s.button_size_icon_xs,
            ButtonSize::IconSm => &s.button_size_icon_sm,
            ButtonSize::IconLg => &s.button_size_icon_lg,
        }
    }

    /// A square, icon-only size.
    fn is_icon(self) -> bool {
        matches!(
            self,
            ButtonSize::Icon | ButtonSize::IconXs | ButtonSize::IconSm | ButtonSize::IconLg
        )
    }
}

impl From<AppVariant<ButtonSize>> for ButtonSize {
    fn from(app: AppVariant<ButtonSize>) -> Self {
        ButtonSize::App(app)
    }
}

impl Sealed for ButtonSize {}
impl AppEnum for ButtonSize {
    type Recipes = ButtonRecipes;
    fn app_name(self) -> Option<&'static str> {
        match self {
            ButtonSize::App(a) => Some(a.name()),
            _ => None,
        }
    }
    fn parts(r: &mut ButtonRecipes) -> Vec<&mut crate::style::Recipe> {
        vec![&mut r.button]
    }
    fn recipes(self, style: &Style) -> ButtonRecipes {
        ButtonRecipes {
            button: self.layer(style).clone(),
        }
    }
}
impl SizeEnum for ButtonSize {}

pub struct Button {
    text: String,
    variant: ButtonVariant,
    size: ButtonSize,
    enabled: bool,
    icon: Option<Icon>,
    icon_end: Option<Icon>,
    expanded: bool,
    invalid: bool,
    has_popup: bool,
    extra: Option<Recipe>,
}

impl Button {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            variant: ButtonVariant::Default,
            size: ButtonSize::Default,
            enabled: true,
            icon: None,
            icon_end: None,
            expanded: false,
            invalid: false,
            has_popup: false,
            extra: None,
        }
    }
    pub fn variant(mut self, v: impl Into<ButtonVariant>) -> Self {
        self.variant = v.into();
        self
    }
    pub fn size(mut self, s: impl Into<ButtonSize>) -> Self {
        self.size = s.into();
        self
    }
    /// Disabled buttons do not sense clicks, cannot take focus, show no
    /// hover or press styling, and paint at the style's disabled opacity.
    pub fn enabled(mut self, yes: bool) -> Self {
        self.enabled = yes;
        self
    }
    /// An icon before the text (shadcn `<Icon data-icon="inline-start" />`).
    /// With an icon size (`ButtonSize::Icon`, …) only the icon is drawn and
    /// the text is the button's accessible name.
    pub fn icon(mut self, icon: &Icon) -> Self {
        self.icon = Some(*icon);
        self
    }
    /// An icon after the text (shadcn `<Icon data-icon="inline-end" />`).
    pub fn icon_end(mut self, icon: &Icon) -> Self {
        self.icon_end = Some(*icon);
        self
    }
    /// `aria-expanded`: the menu or popover this button opens is open.
    pub fn expanded(mut self, yes: bool) -> Self {
        self.expanded = yes;
        self
    }
    /// `aria-invalid`: the destructive ring and border.
    pub fn invalid(mut self, yes: bool) -> Self {
        self.invalid = yes;
        self
    }
    /// `aria-haspopup`: the button opens a popup, so it does not nudge down
    /// when pressed.
    pub fn has_popup(mut self, yes: bool) -> Self {
        self.has_popup = yes;
        self
    }

    /// A recipe stacked over the button's own, for a part that puts its
    /// class on a button (`cn-dialog-close`): it wins ties of equal rank,
    /// so its base value still loses to a button rule for a matching state.
    pub(crate) fn extra_layer(mut self, layer: Recipe) -> Self {
        self.extra = Some(layer);
        self
    }

    /// The recipe stack, with the variant layer `v`, the size layer `z`
    /// and the extra layer, and how many of its layers are in use.
    fn layers<'s>(
        s: &'s Style,
        v: &'s Recipe,
        z: &'s Recipe,
        extra: Option<&'s Recipe>,
    ) -> ([&'s Recipe; 4], usize) {
        match extra {
            Some(extra) => ([&*s.button, v, z, extra], 4),
            None => ([&*s.button, v, z, z], 3),
        }
    }

    /// The icon of an icon size (`ButtonSize::Icon`, …): drawn alone, the
    /// text is not. `None` for text sizes. An icon size without an icon is a
    /// caller error (debug builds panic); release builds fall back to a
    /// button as wide as its text.
    /// `size` is the built-in size the button draws with (an app size's
    /// base).
    fn icon_only(&self, size: ButtonSize) -> Option<Icon> {
        let square = size.is_icon();
        let icon = self.icon.or(self.icon_end);
        debug_assert!(
            !square || icon.is_some(),
            "Button {:?}: an icon size needs .icon(..)",
            self.text
        );
        if square {
            icon
        } else {
            None
        }
    }
}

impl Widget for Button {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = Theme::current(ui.ctx());
        let variant = t.resolve(self.variant);
        let size = t.resolve(self.size);
        let v = variant.part(&t.style, ButtonVariant::layer, |r| &r.button);
        let z = size.part(&t.style, ButtonSize::layer, |r| &r.button);
        let (layers, n) = Self::layers(&t.style, v, z, self.extra.as_ref());
        let l = &layers[..n];
        let enabled = self.enabled && ui.is_enabled();
        let context = (variant.bits() | size.bits())
            .with(State::ICON_START, self.icon.is_some())
            .with(State::ICON_END, self.icon_end.is_some())
            .with(State::EXPANDED, self.expanded)
            .with(State::INVALID, self.invalid)
            .with(State::HAS_POPUP, self.has_popup);
        // Geometry never depends on hover or press: resolve it before allocating.
        let g = t.state(ui.ctx(), context);
        let get = |f: fn(&Recipe) -> &Prop<f32>| pick_len(l, f, g);
        let (pad_l, pad_r, gap, border_w) = (
            get(|r| &r.pad_left),
            get(|r| &r.pad_right),
            get(|r| &r.gap),
            line_width(get(|r| &r.border_width), ui.ctx().pixels_per_point()),
        );
        let icon_px = pick(l, |r| &r.icon_size, g).unwrap_or(t.metrics.icon);
        // The text colour depends on the state, known only after allocating:
        // lay out in egui's placeholder colour, paint in the final one.
        let look = TextLook::resolve(ui, l, g);
        let icon_only = self.icon_only(size.base());
        let row = match icon_only {
            Some(icon) => AtomRow::new(ui, "").icons(Some(icon), None),
            None => AtomRow::new(ui, self.text.as_str()).icons(self.icon, self.icon_end),
        };
        let (pad_t, pad_b) = (get(|r| &r.pad_top), get(|r| &r.pad_bottom));
        let row = row
            .icon_size(icon_px)
            .gap(gap)
            .inset(
                pad_l + border_w,
                pad_r + border_w,
                pad_t + border_w,
                pad_b + border_w,
            )
            .lay_out(ui, &look);
        // No height: CSS's initial `auto`, the content's height.
        let height = pick(l, |r| &r.height, g).unwrap_or(row.size().y);
        let width = if icon_only.is_some() {
            pick(l, |r| &r.width, g).unwrap_or(height)
        } else {
            row.size().x
        };
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        // In a justified layout the box takes the layout's width, as egui's
        // `Button` does (conformance 3.1); the content stays centred in it.
        let (id, rect) = crate::layout::allocate_item_space(ui, Vec2::new(width, height));
        let mut resp = ui.interact(rect, id, sense);
        pressable(&mut resp, ActivationKeys::Button);
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &self.text)
        });
        report_invalid(&resp, self.invalid);
        if !ui.is_rect_visible(rect) {
            return resp;
        }

        let st = interaction_state(&t, &resp, context.with(State::DISABLED, !enabled));
        let painter = part_painter(ui, id, &[l], st);
        let tone = |f: fn(&Recipe) -> &Prop<Paint>| pick(l, f, st).map(|p| t.tone(p));
        let inherited = t.role(Role::Foreground); // behaviour-ok: inherits Foreground; upstream sets no text colour
        let ctx = ui.ctx();
        let key = |property: &str| id.with(("transition", property));
        let fg = tone(|r| &r.text).unwrap_or(Tone::of(inherited));
        let fg = transitioned_text(ctx, id, fg, l, st);
        // `active:translate-y-px` moves as the transition says.
        let shifted = shift(
            rect,
            pick(l, |r| &r.shift_x, st),
            pick(l, |r| &r.shift_y, st),
        );
        let offset = shifted.min - rect.min;
        let rect = rect.translate(Vec2::new(
            transition(
                ctx,
                key("translate-x"),
                offset.x,
                l,
                Transition::TRANSFORM,
                st,
            ),
            transition(
                ctx,
                key("translate-y"),
                offset.y,
                l,
                Transition::TRANSFORM,
                st,
            ),
        ));
        RecipeBox::resolve(ctx, &t, l, st)
            .transitioned(ctx, id, l, st)
            .paint(&painter, rect);
        // A state's `underline` (the link variant on hover) lays out again.
        row.paint(ui, &painter, rect, &TextLook::resolve(ui, l, st), fg);
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{Paint, Role, State, Style};

    fn layers(s: &Style, v: ButtonVariant, z: ButtonSize) -> Vec<&Recipe> {
        let (l, n) = Button::layers(s, v.layer(s), z.layer(s), None);
        l[..n].to_vec()
    }

    #[test]
    fn nova_icon_start_tightens_left_padding() {
        let s = Style::nova();
        let l = layers(&s, ButtonVariant::Default, ButtonSize::Default);
        assert_eq!(pick(&l, |r| &r.pad_left, State::NONE), Some(10.0));
        assert_eq!(pick(&l, |r| &r.pad_left, State::ICON_START), Some(8.0));
    }

    #[test]
    fn outline_hover_in_dark_uses_input_tint() {
        let s = Style::nova();
        let l = layers(&s, ButtonVariant::Outline, ButtonSize::Default);
        assert_eq!(
            pick(&l, |r| &r.bg, State::DARK | State::HOVER),
            Some(Paint::Role(Role::Input, 0.5))
        );
        assert_eq!(
            pick(&l, |r| &r.bg, State::HOVER),
            Some(Paint::Role(Role::Muted, 1.0))
        );
    }

    #[test]
    fn press_nudge_only_without_popup() {
        let s = Style::vega();
        let l = layers(&s, ButtonVariant::Default, ButtonSize::Default);
        assert!(pick(&l, |r| &r.shift_y, State::ACTIVE).is_some());
        assert!(pick(&l, |r| &r.shift_y, State::ACTIVE | State::HAS_POPUP).is_none());
    }

    /// An app variant's and an app size's edit get `ButtonRecipes` and
    /// change only what they name.
    #[test]
    fn edit_compiles_and_changes_its_layer() {
        const DANGER_GHOST: AppVariant<ButtonVariant> = ButtonVariant::app("danger-ghost");
        const TALL: AppVariant<ButtonSize> = ButtonSize::app("tall");
        let t = Theme::vega()
            .variant(DANGER_GHOST, ButtonVariant::Ghost, |r, _style| {
                r.button.text.set(Paint::Role(Role::Destructive, 1.0))
            })
            .size(TALL, ButtonSize::Icon, |r: &mut ButtonRecipes, _| {
                r.button.height.set(48.0)
            });
        let v = t.resolve(ButtonVariant::from(DANGER_GHOST));
        let z = t.resolve(ButtonSize::from(TALL));
        let (v, z, base) = (
            v.part(&t.style, ButtonVariant::layer, |r| &r.button),
            z.part(&t.style, ButtonSize::layer, |r| &r.button),
            z.base(),
        );
        assert_eq!(
            v.text.get(State::NONE),
            Some(Paint::Role(Role::Destructive, 1.0))
        );
        assert_eq!(v.bg, t.style.button_variant_ghost.bg);
        assert_eq!(z.height.get(State::NONE), Some(48.0));
        assert_eq!(z.width, t.style.button_size_icon.width);
        // An app size from an icon size draws icon only.
        assert_eq!(base, ButtonSize::Icon);
        let icon = crate::icons::lucide::X;
        assert!(Button::new("x").icon(&icon).icon_only(base).is_some());
    }
}
