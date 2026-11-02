//! shadcn v4 (OKLCH) design tokens and the `Theme` (style x palette x radius), materialized for egui.

use crate::color::{oklch_to_srgb as c, oklch_to_srgb_a as ca, Tone};
use crate::style::app::{AppMap, Resolved, ResolvedPair};
use crate::style::{
    pick, AppEnum, AppVariant, Edit, Paint, Radius, Role, Rung, ShadowLevel, SizeEnum, State,
    Style, TextSize, VariantEnum, Weight,
};
use egui::Color32;
use shadcn_egui_core::IconSlots;

/// The semantic shadcn color tokens. Surfaces come in `x` / `x_foreground` pairs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub background: Color32,
    pub foreground: Color32,
    pub card: Color32,
    pub card_foreground: Color32,
    pub popover: Color32,
    pub popover_foreground: Color32,
    pub primary: Color32,
    pub primary_foreground: Color32,
    pub secondary: Color32,
    pub secondary_foreground: Color32,
    pub muted: Color32,
    pub muted_foreground: Color32,
    pub accent: Color32,
    pub accent_foreground: Color32,
    pub destructive: Color32,
    pub destructive_foreground: Color32,
    pub border: Color32,
    pub input: Color32,
    pub ring: Color32,
    /// `chart-1` … `chart-5`.
    pub chart: [Color32; 5],
}

impl Palette {
    pub fn light() -> Self {
        Self {
            background: c(1.0, 0.0, 0.0),
            foreground: c(0.145, 0.0, 0.0),
            card: c(1.0, 0.0, 0.0),
            card_foreground: c(0.145, 0.0, 0.0),
            popover: c(1.0, 0.0, 0.0),
            popover_foreground: c(0.145, 0.0, 0.0),
            primary: c(0.205, 0.0, 0.0),
            primary_foreground: c(0.985, 0.0, 0.0),
            secondary: c(0.97, 0.0, 0.0),
            secondary_foreground: c(0.205, 0.0, 0.0),
            muted: c(0.97, 0.0, 0.0),
            muted_foreground: c(0.556, 0.0, 0.0),
            accent: c(0.97, 0.0, 0.0),
            accent_foreground: c(0.205, 0.0, 0.0),
            destructive: c(0.577, 0.245, 27.325),
            destructive_foreground: c(0.985, 0.0, 0.0),
            border: c(0.922, 0.0, 0.0),
            input: c(0.922, 0.0, 0.0),
            ring: c(0.708, 0.0, 0.0),
            chart: [
                c(0.87, 0.0, 0.0),
                c(0.556, 0.0, 0.0),
                c(0.439, 0.0, 0.0),
                c(0.371, 0.0, 0.0),
                c(0.269, 0.0, 0.0),
            ],
        }
    }

    pub fn dark() -> Self {
        Self {
            background: c(0.145, 0.0, 0.0),
            foreground: c(0.985, 0.0, 0.0),
            card: c(0.205, 0.0, 0.0),
            card_foreground: c(0.985, 0.0, 0.0),
            popover: c(0.205, 0.0, 0.0),
            popover_foreground: c(0.985, 0.0, 0.0),
            primary: c(0.922, 0.0, 0.0),
            primary_foreground: c(0.205, 0.0, 0.0),
            secondary: c(0.269, 0.0, 0.0),
            secondary_foreground: c(0.985, 0.0, 0.0),
            muted: c(0.269, 0.0, 0.0),
            muted_foreground: c(0.708, 0.0, 0.0),
            accent: c(0.269, 0.0, 0.0),
            accent_foreground: c(0.985, 0.0, 0.0),
            destructive: c(0.704, 0.191, 22.216),
            destructive_foreground: c(0.985, 0.0, 0.0),
            border: ca(1.0, 0.0, 0.0, 0.10),
            input: ca(1.0, 0.0, 0.0, 0.15),
            ring: c(0.556, 0.0, 0.0), // solid mid-gray (border/input above are translucent white)
            chart: [
                c(0.87, 0.0, 0.0),
                c(0.556, 0.0, 0.0),
                c(0.439, 0.0, 0.0),
                c(0.371, 0.0, 0.0),
                c(0.269, 0.0, 0.0),
            ],
        }
    }
}

impl Palette {
    /// The palette the shadcn site builds for a base colour, a theme and a
    /// chart colour (upstream `buildRegistryTheme`): the `base` colour's
    /// vars, the `theme` on top (a colour theme, or a base name, which
    /// overlays that whole base), then `chart-1..5` from the `chart` theme.
    /// Ids are upstream's (`"neutral"`, `"blue"`); `None` when one is
    /// unknown. Menu accent is separate: [`Palette::with_bold_menu_accent`].
    pub fn from_upstream(base: &str, theme: &str, chart: &str, dark: bool) -> Option<Palette> {
        crate::palette::merge(base, theme, chart, dark)
    }

    /// shadcn `menuAccent: "bold"`: menus highlight with the primary pair.
    pub fn with_bold_menu_accent(mut self) -> Self {
        self.accent = self.primary;
        self.accent_foreground = self.primary_foreground;
        self
    }
}

/// A shadcn theme: a style, the light and the dark palette, the radius,
/// fonts and icons. [`Theme::apply`] installs both modes; the crate follows
/// egui's light/dark choice (`ctx.set_theme(ThemePreference::Light)`, or the
/// system's), and [`Theme::current`] resolves in the context's mode.
/// Cloning is cheap: the style sits behind an `Arc`.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    /// The shadcn style (vega, nova, …): every component part's recipe.
    pub style: Arc<Style>,
    /// The light mode's colours.
    pub light: Palette,
    /// The dark mode's colours.
    pub dark: Palette,
    /// shadcn `--radius`; the radius scale multiplies from it.
    pub radius: f32,
    pub metrics: Metrics,
    /// Body and heading font; [`Theme::apply`] installs them.
    pub fonts: ThemeFonts,
    /// The icons components draw (default: built-in Lucide).
    pub icons: IconSlots,
    /// Menu content draws in dark mode in both modes (upstream
    /// `menuColor: "inverted"`; today the Select list).
    pub menu_inverted: bool,
    /// The mode colours and `dark:` rules resolve in.
    mode: egui::Theme,
    /// App variants, app sizes and app colours, shared by every clone.
    pub(crate) apps: Arc<AppMap>,
}

/// Size tokens, in logical px. Defaults are shadcn v4's Tailwind classes; the
/// class each one stands for is noted. Components read sizes from here, never
/// from literals, so one change restyles every screen. Control heights,
/// paddings and gaps belong to the [`Style`]. Geometry that belongs to
/// a single widget's drawing (a checkmark path, a knob inset) stays local.
///
/// Override them on a theme value, with struct update syntax for the
/// `Metrics` (a `Theme { .. }` literal does not compile: some fields are
/// private):
///
/// ```
/// use shadcn_egui::{Metrics, Theme};
/// let mut theme = Theme::vega();
/// theme.metrics = Metrics { icon: 20.0, ..Metrics::default() };
/// assert_eq!(theme.metrics.icon, 20.0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// `text-xs`: badges, captions (`TextStyle::Small`).
    pub text_xs: f32,
    /// `text-sm`: body text, labels, controls (`TextStyle::Body`/`Button`).
    pub text_sm: f32,
    /// `text-base`: card titles.
    pub text_base: f32,
    /// `text-lg`: dialog titles.
    pub text_lg: f32,
    /// Tailwind's line height that comes with `text-xs`.
    pub line_xs: f32,
    /// Line height of `text-sm`.
    pub line_sm: f32,
    /// Line height of `text-base`.
    pub line_base: f32,
    /// Line height of `text-lg`.
    pub line_lg: f32,
    /// `TextStyle::Monospace`.
    pub text_mono: f32,
    /// `TextStyle::Heading`.
    pub text_heading: f32,
    /// Default maximum width of text fields and selects.
    pub field_max_width: f32,
    /// `size-4`: icons and checkbox boxes.
    pub icon: f32,
    /// Icon stroke width in icon units (Lucide's 24-unit grid draws with 2).
    pub icon_stroke: f32,
    /// `border`: hairline width.
    pub border: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            text_xs: 12.0,
            text_sm: 14.0,
            text_base: 16.0,
            text_lg: 18.0,
            line_xs: 16.0,
            line_sm: 20.0,
            line_base: 24.0,
            line_lg: 28.0,
            text_mono: 13.0,
            text_heading: 20.0,
            field_max_width: 280.0,
            icon: 16.0,
            icon_stroke: 2.0,
            border: 1.0,
        }
    }
}

impl Theme {
    /// A theme from a style, the light and the dark palette and the radius
    /// (`--radius` in px). It resolves in dark mode, egui's fallback, until
    /// [`Theme::current`] gives it the context's mode.
    pub fn new(style: Style, light: Palette, dark: Palette, radius: f32) -> Self {
        // `Arc::new` copies the large `Style` through two stack frames in a
        // debug build; a box moves it to the heap once.
        let style = Arc::from(Box::new(style));
        Self {
            style,
            light,
            dark,
            radius,
            metrics: Metrics::default(),
            fonts: ThemeFonts::inherit(&shadcn_egui_font_inter::FONT),
            icons: crate::icons::lucide::SLOTS,
            menu_inverted: false,
            mode: egui::Theme::Dark,
            apps: Arc::default(),
        }
    }
    /// The vega style with the neutral palettes and a 10 px radius.
    pub fn vega() -> Self {
        Self::new(Style::vega(), Palette::light(), Palette::dark(), 10.0)
    }

    /// This theme resolving colours and `dark:` rules in `mode`: how
    /// [`Theme::current`] gives it the context's mode, and how the crate's
    /// unit tests pick one without a context. Apps switch modes with
    /// `ctx.set_theme(..)`; [`Theme::apply`] installs both modes whatever
    /// this says.
    pub(crate) fn in_mode(mut self, mode: egui::Theme) -> Self {
        self.mode = mode;
        self
    }

    /// The mode this theme resolves in.
    pub fn mode(&self) -> egui::Theme {
        self.mode
    }

    /// Whether this theme resolves in dark mode.
    pub fn is_dark(&self) -> bool {
        self.mode == egui::Theme::Dark
    }

    /// The palette of the mode this theme resolves in.
    pub fn palette(&self) -> &Palette {
        self.palette_of(self.mode)
    }

    /// The palette of `mode`.
    pub fn palette_of(&self, mode: egui::Theme) -> &Palette {
        match mode {
            egui::Theme::Dark => &self.dark,
            egui::Theme::Light => &self.light,
        }
    }

    /// Body font and heading font (`None`: headings use the body font,
    /// upstream `fontHeading: "inherit"`).
    pub fn fonts(mut self, body: &'static FontFiles, heading: Option<&'static FontFiles>) -> Self {
        self.fonts = ThemeFonts {
            body,
            heading: heading.unwrap_or(body),
        };
        self
    }

    /// The icon set components draw from (an icon crate's `SLOTS`).
    pub fn icons(mut self, slots: IconSlots) -> Self {
        self.icons = slots;
        self
    }

    /// Menu content in dark mode in both modes (upstream
    /// `menuColor: "inverted"`).
    pub fn menu_inverted(mut self, inverted: bool) -> Self {
        self.menu_inverted = inverted;
        self
    }

    /// The theme menu content draws with: this one, in dark mode when the
    /// menu is inverted.
    pub fn menu_theme(&self) -> Theme {
        let mut t = self.clone();
        if self.menu_inverted {
            t.mode = egui::Theme::Dark;
        }
        t
    }

    /// Define the app variant `app` from the built-in variant `base`: the
    /// edit gets copies of `base`'s recipes and the style the theme builds
    /// for, and changes what it names; the rest stays `base`'s, in every
    /// state, mode and style. Both modes share it; a later definition of
    /// the same name replaces it.
    ///
    /// ```
    /// use shadcn_egui::components::button::{Button, ButtonVariant};
    /// use shadcn_egui::style::{AppVariant, Paint, Role};
    /// use shadcn_egui::Theme;
    /// # fn f(ui: &mut egui::Ui) {
    /// const DANGER_GHOST: AppVariant<ButtonVariant> = ButtonVariant::app("danger-ghost");
    /// const DANGER_OUTLINE: AppVariant<ButtonVariant> = ButtonVariant::app("danger-outline");
    /// let theme = Theme::vega()
    ///     .variant(DANGER_GHOST, ButtonVariant::Ghost, |r, _style| {
    ///         r.button.text.set(Paint::Role(Role::Destructive, 1.0)); // Ghost's hover colours stay
    ///     })
    ///     // Red where Outline sets a text colour: on hover and when expanded.
    ///     .variant(DANGER_OUTLINE, ButtonVariant::Outline, |r, _| r.button.recolor(Role::Foreground, Role::Destructive));
    /// ui.add(Button::new("Delete").variant(DANGER_GHOST));
    /// # }
    /// ```
    ///
    /// A size is no variant:
    ///
    /// ```compile_fail,E0277
    /// use shadcn_egui::components::button::ButtonVariant;
    /// const WIDE: shadcn_egui::style::AppVariant<ButtonVariant> = ButtonVariant::app("wide");
    /// let _ = shadcn_egui::Theme::vega().size(WIDE, ButtonVariant::Default, |_, _| {});
    /// ```
    pub fn variant<V: VariantEnum>(mut self, app: AppVariant<V>, base: V, edit: Edit<V>) -> Self {
        Arc::make_mut(&mut self.apps).insert(app, base, edit);
        self
    }

    /// Define the app size `app` from the built-in size `base`, as
    /// [`Theme::variant`] does for a variant.
    pub fn size<Z: SizeEnum>(mut self, app: AppVariant<Z>, base: Z, edit: Edit<Z>) -> Self {
        Arc::make_mut(&mut self.apps).insert(app, base, edit);
        self
    }

    /// Define the app colour `role` ([`Role::custom`]) as `light` in light
    /// mode and `dark` in dark mode.
    pub fn color(mut self, role: Role, light: Color32, dark: Color32) -> Self {
        match role {
            Role::Custom(name) => Arc::make_mut(&mut self.apps).set_color(name.name(), light, dark),
            _ => debug_assert!(
                false,
                "Theme::color({role:?}): a palette role, not Role::custom"
            ),
        }
        self
    }

    /// What a component draws a variant or size with: the recipes and state
    /// bits of the built-in case, or of the app case's edited copies.
    pub(crate) fn resolve<X: AppEnum>(&self, x: X) -> Resolved<X> {
        self.apps.resolve(&self.style, x)
    }

    /// The recipes and state bits of a variant and a size that edit the
    /// same parts ([`crate::style::app::AppMap::resolve_pair`]).
    pub(crate) fn resolve_pair<V: AppEnum, Z: AppEnum<Recipes = V::Recipes>>(
        &self,
        v: V,
        z: Z,
    ) -> ResolvedPair<V, Z> {
        self.apps.resolve_pair(&self.style, v, z)
    }

    /// The component state `s` plus this theme's [`State::DARK`], and the
    /// `sm:` flag (the window is at least 640 points wide) and `md:` flag
    /// (768 points), as [`crate::layout::window_at_least`] tests them.
    pub fn state(&self, ctx: &egui::Context, s: State) -> State {
        s.with(State::DARK, self.is_dark())
            .or(crate::layout::breakpoints(ctx))
    }

    /// A radius rung in px: shadcn's scale multiplies `--radius`.
    pub fn rung(&self, r: Rung) -> f32 {
        let k = match r {
            Rung::None => 0.0,
            Rung::Sm => 0.6,
            Rung::Md => 0.8,
            Rung::Lg => 1.0,
            Rung::Xl => 1.4,
            Rung::Xl2 => 1.8,
            Rung::Xl3 => 2.2,
            Rung::Xl4 => 2.6,
            Rung::Full => return f32::INFINITY,
        };
        self.radius * k
    }
    /// Corner radius in px, clamped to `max` (half the part's short side)
    /// and to egui's `u8` corner radius.
    pub fn radius_px(&self, r: Radius, max: f32) -> f32 {
        let v = match r {
            Radius::Rung(r) => self.rung(r),
            Radius::Px(v) => v,
            Radius::Min(r, v) => self.rung(r).min(v),
        };
        v.min(max).clamp(0.0, 255.0)
    }
    pub fn radius_sm(&self) -> f32 {
        self.rung(Rung::Sm)
    }
    pub fn radius_md(&self) -> f32 {
        self.rung(Rung::Md)
    }
    pub fn radius_lg(&self) -> f32 {
        self.rung(Rung::Lg)
    }
    pub fn radius_xl(&self) -> f32 {
        self.rung(Rung::Xl)
    }

    /// The palette colour of a style role.
    pub fn role(&self, r: Role) -> Color32 {
        let p = self.palette();
        match r {
            Role::Background => p.background,
            Role::Foreground => p.foreground,
            Role::Card => p.card,
            Role::CardForeground => p.card_foreground,
            Role::Popover => p.popover,
            Role::PopoverForeground => p.popover_foreground,
            Role::Primary => p.primary,
            Role::PrimaryForeground => p.primary_foreground,
            Role::Secondary => p.secondary,
            Role::SecondaryForeground => p.secondary_foreground,
            Role::Muted => p.muted,
            Role::MutedForeground => p.muted_foreground,
            Role::Accent => p.accent,
            Role::AccentForeground => p.accent_foreground,
            Role::Destructive => p.destructive,
            Role::DestructiveForeground => p.destructive_foreground,
            Role::Border => p.border,
            Role::Input => p.input,
            Role::Ring => p.ring,
            Role::White => Color32::WHITE,
            Role::Black => Color32::BLACK,
            Role::Transparent => Color32::TRANSPARENT,
            Role::Custom(name) => {
                let c = self.apps.color(name.name(), self.is_dark());
                debug_assert!(
                    c.is_some(),
                    "app colour {name:?} is not defined in the theme"
                );
                c.unwrap_or(p.foreground)
            }
        }
    }
    /// A style colour as egui paints it. Alpha scales the premultiplied colour,
    /// which matches CSS `/alpha` compositing in sRGB.
    pub fn paint(&self, p: Paint) -> Color32 {
        self.tone(p).color
    }
    /// [`Theme::paint`] with the unclipped colour it stands for, which a
    /// fade or mix takes: the role's token at the paint's alpha.
    pub(crate) fn tone(&self, p: Paint) -> Tone {
        match p {
            Paint::Role(r, a) if a >= 1.0 => Tone::of(self.role(r)),
            Paint::Role(r, a) => Tone::of(self.role(r)).faded(a),
            Paint::Mix(a, b, t) => {
                Tone::mix_oklch(Tone::of(self.role(a)), Tone::of(self.role(b)), t)
            }
        }
    }
    /// A text size in px, from [`Metrics`].
    pub fn text_px(&self, s: TextSize) -> f32 {
        let m = &self.metrics;
        match s {
            TextSize::Xs => m.text_xs,
            TextSize::Sm => m.text_sm,
            TextSize::Base => m.text_base,
            TextSize::Lg => m.text_lg,
            TextSize::Px(v) => v,
        }
    }
    /// The line height Tailwind pairs with a text size (used when a recipe
    /// has no `leading-*`). An arbitrary pixel size keeps the base ratio.
    pub fn line_px(&self, s: TextSize) -> f32 {
        let m = &self.metrics;
        match s {
            TextSize::Xs => m.line_xs,
            TextSize::Sm => m.line_sm,
            TextSize::Base => m.line_base,
            TextSize::Lg => m.line_lg,
            TextSize::Px(v) => v * m.line_base / m.text_base,
        }
    }
    /// Tailwind v4's `shadow-*` scale with every layer, first layer on top
    /// as CSS paints it. [`RecipeBox`](crate::components::shared::RecipeBox)
    /// draws them to the browser's pixels.
    pub fn shadow(&self, s: ShadowLevel) -> &'static [ShadowLayer] {
        const fn l(y: f32, blur: f32, spread: f32, alpha: f32) -> ShadowLayer {
            ShadowLayer::new(egui::vec2(0.0, y), blur, spread, Color32::BLACK, alpha)
        }
        // Tailwind v4 theme.css `--shadow-*`.
        const XS2: &[ShadowLayer] = &[l(1.0, 0.0, 0.0, 0.05)];
        const XS: &[ShadowLayer] = &[l(1.0, 2.0, 0.0, 0.05)];
        const SM: &[ShadowLayer] = &[l(1.0, 3.0, 0.0, 0.1), l(1.0, 2.0, -1.0, 0.1)];
        const MD: &[ShadowLayer] = &[l(4.0, 6.0, -1.0, 0.1), l(2.0, 4.0, -2.0, 0.1)];
        const LG: &[ShadowLayer] = &[l(10.0, 15.0, -3.0, 0.1), l(4.0, 6.0, -4.0, 0.1)];
        const XL: &[ShadowLayer] = &[l(20.0, 25.0, -5.0, 0.1), l(8.0, 10.0, -6.0, 0.1)];
        const XL2: &[ShadowLayer] = &[l(25.0, 50.0, -12.0, 0.25)];
        match s {
            ShadowLevel::None => &[],
            ShadowLevel::Xs2 => XS2,
            ShadowLevel::Xs => XS,
            ShadowLevel::Sm => SM,
            ShadowLevel::Md => MD,
            ShadowLevel::Lg => LG,
            ShadowLevel::Xl => XL,
            ShadowLevel::Xl2 => XL2,
        }
    }
}

/// One layer of a CSS `box-shadow` (outer): offset, blur radius and spread
/// in px, colour and alpha. The blur is CSS's: a Gaussian with a standard
/// deviation of half the blur radius. The alpha is kept apart from the
/// colour so `0.1` stays exact (egui's colours have 8-bit alpha).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ShadowLayer {
    offset: egui::Vec2,
    blur: f32,
    spread: f32,
    color: Color32,
    alpha: f32,
}

impl ShadowLayer {
    /// A layer as CSS writes it: `<x> <y> <blur> <spread> <color> / <alpha>`.
    /// `color`'s own alpha is ignored.
    pub const fn new(
        offset: egui::Vec2,
        blur: f32,
        spread: f32,
        color: Color32,
        alpha: f32,
    ) -> Self {
        Self {
            offset,
            blur,
            spread,
            color,
            alpha,
        }
    }
    /// The x and y offset in px.
    #[inline]
    pub fn offset(&self) -> egui::Vec2 {
        self.offset
    }
    /// The blur radius in px (CSS's; the Gaussian's deviation is half).
    #[inline]
    pub fn blur(&self) -> f32 {
        self.blur
    }
    /// The spread in px; negative shrinks the shadow.
    #[inline]
    pub fn spread(&self) -> f32 {
        self.spread
    }
    /// The colour; its alpha is not used.
    #[inline]
    pub fn color(&self) -> Color32 {
        self.color
    }
    /// The alpha, 0 to 1.
    #[inline]
    pub fn alpha(&self) -> f32 {
        self.alpha
    }
}

// ---- Font + apply + accessor ----

/// Item spacing of egui-native widgets when the style's tabs recipe sets no gap (`gap-2`).
const DEFAULT_ITEM_GAP: f32 = 8.0;

use egui::{FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle};
use std::sync::Arc;

fn theme_id() -> egui::Id {
    static ID: std::sync::LazyLock<egui::Id> =
        std::sync::LazyLock::new(|| egui::Id::new("shadcn_egui::active_theme"));
    *ID
}

use egui::epaint::text::VariationCoords;
use shadcn_egui_core::{FontFiles, FontKind, WEIGHTS};

/// A theme's fonts. `heading == body` is upstream `fontHeading: "inherit"`.
/// Two values are equal when they name the same fonts.
#[derive(Clone, Copy)]
pub struct ThemeFonts {
    pub body: &'static FontFiles,
    pub heading: &'static FontFiles,
}

impl ThemeFonts {
    /// Headings in the body font.
    pub fn inherit(body: &'static FontFiles) -> Self {
        Self {
            body,
            heading: body,
        }
    }
}

impl PartialEq for ThemeFonts {
    fn eq(&self, other: &Self) -> bool {
        self.body.name == other.body.name && self.heading.name == other.heading.name
    }
}

impl std::fmt::Debug for ThemeFonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThemeFonts")
            .field("body", &self.body.name)
            .field("heading", &self.heading.name)
            .finish()
    }
}

/// Named font families. Body regular is `FontFamily::Proportional`.
pub const FAMILY_MEDIUM: &str = "shadcn-body-medium";
pub const FAMILY_SEMIBOLD: &str = "shadcn-body-semibold";
pub const FAMILY_BOLD: &str = "shadcn-body-bold";
pub const FAMILY_HEADING: &str = "shadcn-heading-regular";
pub const FAMILY_HEADING_MEDIUM: &str = "shadcn-heading-medium";
pub const FAMILY_HEADING_SEMIBOLD: &str = "shadcn-heading-semibold";
pub const FAMILY_HEADING_BOLD: &str = "shadcn-heading-bold";

/// Families per [`WEIGHTS`] entry; `None` is `Proportional`.
const BODY_FAMILIES: [Option<&str>; 4] = [
    None,
    Some(FAMILY_MEDIUM),
    Some(FAMILY_SEMIBOLD),
    Some(FAMILY_BOLD),
];
const HEADING_FAMILIES: [&str; 4] = [
    FAMILY_HEADING,
    FAMILY_HEADING_MEDIUM,
    FAMILY_HEADING_SEMIBOLD,
    FAMILY_HEADING_BOLD,
];

fn weight_index(w: Weight) -> usize {
    match w {
        Weight::Normal => 0,
        Weight::Medium => 1,
        Weight::Semibold => 2,
        Weight::Bold => 3,
    }
}

/// Return the named font family if it's registered (via `Theme::apply`), else
/// fall back to Proportional. egui defers `set_fonts` to the next frame, so on
/// the very first frame after `apply` a named family may not be in the atlas yet.
pub fn family(ctx: &egui::Context, name: &str) -> egui::FontFamily {
    let fam = egui::FontFamily::Name(name.into());
    if ctx.fonts(|f| f.definitions().families.contains_key(&fam)) {
        fam
    } else {
        egui::FontFamily::Proportional
    }
}

/// The body font family for a weight.
pub fn family_for(ctx: &egui::Context, w: Weight) -> egui::FontFamily {
    match BODY_FAMILIES[weight_index(w)] {
        None => egui::FontFamily::Proportional,
        Some(name) => family(ctx, name),
    }
}

/// The heading font family for a weight (upstream `cn-font-heading`).
pub fn heading_family_for(ctx: &egui::Context, w: Weight) -> egui::FontFamily {
    family(ctx, HEADING_FAMILIES[weight_index(w)])
}

/// The font data key of one theme face: `shadcn-{body,heading}-{weight}`.
fn face_key(role: &str, weight: u16) -> String {
    format!("shadcn-{role}-{weight}")
}

/// One face of `files` at weight `w`, with its `wght` coordinate and
/// baseline correction.
fn face_data(files: &FontFiles, w: u16) -> Arc<FontData> {
    let face = files.face(w);
    let mut tweak = egui::FontTweak {
        y_offset_factor: files.y_offset_factor,
        ..Default::default()
    };
    if let Some(v) = face.wght {
        tweak.coords = VariationCoords::new([(b"wght", v)]);
    }
    Arc::new(FontData::from_static(face.data).tweak(tweak))
}

/// `base` plus the theme's fonts: one registration per weight with the
/// `wght` coordinate, each family followed by Inter at its weight (a face
/// `shadcn-fallback-{weight}`, unless the family's font is Inter) and then
/// `base`'s Proportional list, so a glyph the font lacks takes Inter's, as
/// Chrome takes another font's (P4), and glyphs Inter lacks too (emoji,
/// CJK) still render. A mono body's face in Monospace is followed by
/// Inter's regular face the same way. A theme's
/// faces and families already in `base` (the context's current
/// definitions) are replaced, so a reinstall neither stacks them nor keeps
/// an earlier theme's. Only the exact keys and families a theme installs
/// count as the theme's: an app's own family that lists a theme face keeps
/// it while the new install still defines it, and an app font named
/// `shadcn-…` is left alone. A mono body puts its regular face first in
/// Monospace, once; `mono_installed` says the last install on this context
/// did so, which makes a leading body face in `base`'s Monospace the
/// theme's, to be removed. Any other is the app's.
fn font_definitions(
    fonts: ThemeFonts,
    mut base: FontDefinitions,
    mono_installed: bool,
) -> FontDefinitions {
    let inherit = fonts.heading == fonts.body;
    let theme_keys: Vec<String> = ["body", "heading", "fallback"]
        .into_iter()
        .flat_map(|role| WEIGHTS.map(|w| face_key(role, w)))
        .collect();
    let is_theme_key = |key: &str| theme_keys.iter().any(|k| k == key);
    let inter = &shadcn_egui_font_inter::FONT;
    let needs_fallback = |files: &FontFiles| files.name != inter.name;
    let fallback_used = needs_fallback(fonts.body) || needs_fallback(fonts.heading);
    // An inherited heading registers no faces of its own, and only a font
    // other than Inter registers Inter's faces as its fallback.
    let defined = |key: &str| {
        if key.starts_with("shadcn-fallback-") {
            fallback_used
        } else {
            !inherit || key.starts_with("shadcn-body-")
        }
    };
    base.font_data.retain(|key, _| !is_theme_key(key));
    let is_theme_family =
        |name: &str| BODY_FAMILIES.contains(&Some(name)) || HEADING_FAMILIES.contains(&name);
    base.families
        .retain(|family, _| !matches!(family, FontFamily::Name(name) if is_theme_family(name)));
    let mono_key = face_key("body", WEIGHTS[0]);
    let mono_fallback = face_key("fallback", WEIGHTS[0]);
    if let Some(mono) = base.families.get_mut(&FontFamily::Monospace) {
        if mono_installed && mono.first() == Some(&mono_key) {
            mono.remove(0);
            if mono.first() == Some(&mono_fallback) {
                mono.remove(0);
            }
        }
    }
    for list in base.families.values_mut() {
        list.retain(|key| !is_theme_key(key) || defined(key));
    }
    let mut fallback = base
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    fallback.retain(|key| !is_theme_key(key));
    for (role, files) in [("body", fonts.body), ("heading", fonts.heading)] {
        let data_role = if role == "heading" && inherit {
            "body"
        } else {
            role
        };
        for (i, w) in WEIGHTS.into_iter().enumerate() {
            let key = face_key(data_role, w);
            if data_role == role {
                base.font_data.insert(key.clone(), face_data(files, w));
            }
            let family = match (role, i) {
                ("body", 0) => FontFamily::Proportional,
                ("body", _) => {
                    FontFamily::Name(BODY_FAMILIES[i].expect("named body family").into())
                }
                _ => FontFamily::Name(HEADING_FAMILIES[i].into()),
            };
            let mut list = vec![key];
            if needs_fallback(files) {
                let inter_key = face_key("fallback", w);
                base.font_data
                    .entry(inter_key.clone())
                    .or_insert_with(|| face_data(inter, w));
                list.push(inter_key);
            }
            list.extend(fallback.iter().cloned());
            base.families.insert(family, list);
        }
    }
    if fonts.body.kind == FontKind::Mono {
        let mono = base.families.entry(FontFamily::Monospace).or_default();
        mono.retain(|key| *key != mono_key && *key != mono_fallback);
        let mut lead = vec![mono_key];
        if needs_fallback(fonts.body) {
            lead.push(mono_fallback);
        }
        mono.splice(0..0, lead);
    }
    base
}

/// The fonts last installed on a context by [`Theme::apply`].
#[derive(Clone, PartialEq)]
struct InstalledFonts {
    body: &'static str,
    heading: &'static str,
    /// The body is mono: the install put its face first in Monospace.
    mono_body: bool,
}

/// Whether the context's current fonts bind `family`. Only valid once the
/// context has fonts (`Context::fonts` panics before its first pass).
fn is_bound(ctx: &egui::Context, family: &str) -> bool {
    let fam = FontFamily::Name(family.into());
    ctx.fonts(|f| f.definitions().families.contains_key(&fam))
}

/// Install the theme's fonts. With `base: None` (plain `apply`) nothing
/// happens when the same fonts are installed already, so a light/dark toggle
/// neither rebuilds the atlas nor drops fonts an app added through
/// `apply_with_fonts`. When the app has replaced the fonts since
/// (`ctx.set_fonts`), the theme's families are installed again on top of
/// the app's current fonts, which keeps the app's own families.
fn install_fonts(ctx: &egui::Context, fonts: ThemeFonts, base: Option<FontDefinitions>) {
    let id = egui::Id::new("shadcn_egui::installed_fonts");
    let key = InstalledFonts {
        body: fonts.body.name,
        heading: fonts.heading.name,
        mono_body: fonts.body.kind == FontKind::Mono,
    };
    let last = ctx.data(|d| d.get_temp::<InstalledFonts>(id));
    let base = match base {
        Some(base) => base,
        None if last.as_ref() != Some(&key) => FontDefinitions::default(),
        // Before the first pass the fonts are still the ones installed last.
        None if ctx.cumulative_pass_nr() == 0 => return,
        None => {
            let all = BODY_FAMILIES.iter().flatten().chain(&HEADING_FAMILIES);
            if all.into_iter().all(|f| is_bound(ctx, f)) {
                return;
            }
            ctx.fonts(|f| f.definitions().clone())
        }
    };
    let mono_installed = last.is_some_and(|f| f.mono_body);
    let defs = Arc::new(font_definitions(fonts, base, mono_installed));
    let pass = ctx.cumulative_pass_nr();
    // Fonts that change nothing count as no install, or an app that
    // applies the same theme in every pass gets every pass discarded. After
    // the first pass this is egui's own test in `set_fonts`, which then does
    // nothing; before and in the first pass egui may have no fonts yet, so
    // the comparison is with the last install.
    let unchanged = if pass > 0 {
        ctx.fonts(|f| f.definitions() == &*defs)
    } else {
        ctx.data(|d| d.get_temp::<Arc<FontDefinitions>>(last_defs_id()))
            .is_some_and(|last| *last == *defs)
    };
    ctx.data_mut(|d| d.insert_temp(id, key));
    if unchanged {
        if pass == 0 {
            ctx.set_fonts((*defs).clone());
        }
        return;
    }
    ctx.set_fonts((*defs).clone());
    ctx.data_mut(|d| d.insert_temp(last_defs_id(), defs));
    // A pass that has started sees the count change (`check_mode_switch`)
    // and is redone. Where no crate call has seen this pass yet, a call
    // between two passes cannot be told from the first call of a pass:
    // repaint, so a pass that drew in the old fonts is not the last one.
    // egui repaints after the first pass anyway.
    let seen = ctx.data(|d| {
        d.get_temp::<(u64, egui::Theme, u64)>(pass_mode_id())
            .is_some_and(|(p, ..)| p == pass)
    });
    if pass > 0 && !seen {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| {
        let n = d.get_temp::<u64>(font_installs_id()).unwrap_or(0);
        d.insert_temp(font_installs_id(), n + 1);
    });
}

/// `TextStyle::Heading` waits in `Proportional` until the heading family is
/// bound: `set_fonts` takes effect only at the start of the next pass, and
/// egui panics on a family the current fonts lack. Holds the stand-in
/// `FontId` of the dark and the light style, which an app may have given
/// different headings, so an app's own later change to a heading is left
/// alone.
#[derive(Clone)]
struct PendingHeading {
    dark: Option<FontId>,
    light: Option<FontId>,
}

impl PendingHeading {
    fn both(id: FontId) -> Self {
        Self {
            dark: Some(id.clone()),
            light: Some(id),
        }
    }

    fn of(&self, theme: egui::Theme) -> Option<&FontId> {
        match theme {
            egui::Theme::Dark => self.dark.as_ref(),
            egui::Theme::Light => self.light.as_ref(),
        }
    }
}

fn pending_heading_id() -> egui::Id {
    egui::Id::new("shadcn_egui::pending_heading")
}

/// Set once the heading family was seen bound. Every install defines it,
/// but an app's own `ctx.set_fonts` drops it: [`unbind_lost_heading`]
/// clears the flag again.
fn heading_bound_id() -> egui::Id {
    egui::Id::new("shadcn_egui::heading_bound")
}

fn heading_bound(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(heading_bound_id()).unwrap_or(false))
}

/// The family `TextStyle::Heading` may name now: the heading family once it
/// is known to be bound, else `Proportional`, upgraded by [`bind_heading`].
fn heading_family_now(ctx: &egui::Context) -> FontFamily {
    if heading_bound(ctx) {
        if is_bound(ctx, FAMILY_HEADING_SEMIBOLD) {
            return FontFamily::Name(FAMILY_HEADING_SEMIBOLD.into());
        }
        // The app replaced the fonts; this apply installs the theme's again.
        ctx.data_mut(|d| d.remove::<bool>(heading_bound_id()));
    }
    let hook_id = egui::Id::new("shadcn_egui::heading_hook");
    if !ctx.data(|d| d.get_temp::<bool>(hook_id).unwrap_or(false)) {
        ctx.data_mut(|d| d.insert_temp(hook_id, true));
        ctx.on_begin_pass(
            "shadcn_egui::bind_heading",
            Arc::new(|ui: &mut egui::Ui| {
                if let Some(id) = check_heading(ui.ctx()) {
                    // The root ui was built before this hook ran.
                    ui.style_mut().text_styles.insert(TextStyle::Heading, id);
                }
            }),
        );
    }
    FontFamily::Proportional
}

/// Once per pass: a heading whose family the app's own fonts dropped
/// ([`unbind_lost_heading`]) falls back, a waiting one binds
/// ([`bind_heading`]). Returns the new heading `FontId` when it changed.
fn check_heading(ctx: &egui::Context) -> Option<FontId> {
    let id = egui::Id::new("shadcn_egui::heading_checked");
    let pass = ctx.cumulative_pass_nr();
    if ctx.data(|d| d.get_temp::<u64>(id)) == Some(pass) {
        return None;
    }
    ctx.data_mut(|d| d.insert_temp(id, pass));
    let fallback = unbind_lost_heading(ctx);
    bind_heading(ctx).or(fallback)
}

/// `TextStyle::Heading` back to `Proportional` when the context's fonts no
/// longer bind the heading family: the app called `ctx.set_fonts` after
/// [`Theme::apply`], and egui panics on a family its fonts lack. Each
/// style's heading waits as a [`PendingHeading`] until the family is bound
/// again. Returns the active style's stand-in.
fn unbind_lost_heading(ctx: &egui::Context) -> Option<FontId> {
    if !heading_bound(ctx) || is_bound(ctx, FAMILY_HEADING_SEMIBOLD) {
        return None;
    }
    let warned = egui::Id::new("shadcn_egui::lost_heading_warned");
    ctx.data_mut(|d| {
        d.remove::<bool>(heading_bound_id());
        if !d.get_temp::<bool>(warned).unwrap_or(false) {
            d.insert_temp(warned, true);
            log::warn!(
                "shadcn-egui: the fonts no longer bind the theme's families \
                 (ctx.set_fonts after Theme::apply?); headings fall back to \
                 Proportional. Use Theme::apply_with_fonts for fonts of your own."
            );
        }
    });
    let name = FontFamily::Name(FAMILY_HEADING_SEMIBOLD.into());
    let fall_back = |theme| {
        let mut stand_in = None;
        ctx.style_mut_of(theme, |s| {
            if let Some(h) = s.text_styles.get_mut(&TextStyle::Heading) {
                if h.family == name {
                    h.family = FontFamily::Proportional;
                    stand_in = Some(h.clone());
                }
            }
        });
        stand_in
    };
    let pending = PendingHeading {
        dark: fall_back(egui::Theme::Dark),
        light: fall_back(egui::Theme::Light),
    };
    if pending.dark.is_none() && pending.light.is_none() {
        return None;
    }
    let active = pending.of(ctx.theme()).cloned();
    ctx.data_mut(|d| d.insert_temp(pending_heading_id(), pending));
    active
}

/// Give `TextStyle::Heading` the heading family once the fonts bind it, in
/// each style that still holds its stand-in. Returns the active style's new
/// heading `FontId` when it changed.
fn bind_heading(ctx: &egui::Context) -> Option<FontId> {
    let pending: PendingHeading = ctx.data(|d| d.get_temp(pending_heading_id()))?;
    let name = FontFamily::Name(FAMILY_HEADING_SEMIBOLD.into());
    if !ctx.fonts(|f| f.definitions().families.contains_key(&name)) {
        return None;
    }
    ctx.data_mut(|d| {
        d.remove::<PendingHeading>(pending_heading_id());
        d.insert_temp(heading_bound_id(), true);
    });
    let mut active = None;
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let Some(stand_in) = pending.of(theme) else {
            continue;
        };
        let id = FontId::new(stand_in.size, name.clone());
        let mut changed = false;
        ctx.style_mut_of(theme, |s| {
            if s.text_styles.get(&TextStyle::Heading) == Some(stand_in) {
                s.text_styles.insert(TextStyle::Heading, id.clone());
                changed = true;
            }
        });
        if changed && theme == ctx.theme() {
            active = Some(id);
        }
    }
    active
}

impl Theme {
    /// Read the active theme stored by [`Theme::apply`], resolving in the
    /// context's mode (`ctx.theme()`). Falls back to [`Theme::vega`],
    /// shared so the fallback does not build a [`Style`] per call.
    ///
    /// Also binds a heading that still waits for its fonts, and lets one
    /// whose family the app's own fonts dropped fall back, for apps whose
    /// loop does not run egui's pass hooks (`Context::begin_pass`). Needs
    /// the context's fonts, which exist once its first pass has ended. In
    /// such a loop, a heading in a `Ui` built before the first shadcn
    /// component of the pass can still panic after an app's own
    /// `ctx.set_fonts`; use [`Theme::apply_with_fonts`].
    ///
    /// A `ctx.set_theme(..)` in the middle of a pass asks egui to discard
    /// that pass (`Context::run_ui` runs it again) and to repaint, so no
    /// frame shows widgets of both modes. A pass hook checks this at the
    /// end of the pass too; an app's own `begin_pass`/`end_pass` loop runs
    /// no hook, so there only a crate widget after the call catches it.
    pub fn current(ctx: &egui::Context) -> Theme {
        static FALLBACK: std::sync::LazyLock<Theme> = std::sync::LazyLock::new(Theme::vega);
        // Before the first pass ends, fonts may not exist yet; the hook binds then.
        if ctx.cumulative_pass_nr() > 0 {
            let _ = check_heading(ctx);
        }
        let mode = check_mode_switch(ctx);
        let theme = ctx.data(|d| d.get_temp::<Theme>(theme_id()));
        theme.unwrap_or_else(|| FALLBACK.clone()).in_mode(mode)
    }

    /// Push this theme into the egui context: fonts, type scale, spacing,
    /// and the colors of both modes (egui's dark and light style). The
    /// crate's widgets and plain egui widgets then follow
    /// `ctx.set_theme(..)` and the system's mode.
    ///
    /// Call once, right after the context is created, and again only when the
    /// theme changes. Not every frame: that would silently undo any other
    /// code's style writes. Fonts are rebuilt from egui's defaults plus the
    /// theme's fonts, and only when they differ from the last install on
    /// this context. **An app that installs fonts of its own must use
    /// [`Theme::apply_with_fonts`]**. A `ctx.set_fonts` of its own after
    /// `apply` drops the theme's families: headings then fall back to
    /// `Proportional` (with a log warning) until a later `apply` of the same
    /// fonts, in a later pass, installs the theme's families on top of the
    /// app's fonts again. In a loop that does not run egui's pass hooks, a
    /// heading in a `Ui` built before the first shadcn component of that pass
    /// can still panic. Calling `apply` and `set_fonts` every frame rebuilds
    /// the font atlas on every other frame.
    ///
    /// egui swaps in new fonts only at the next pass. Called in the middle
    /// of a pass, `apply` of other fonts asks egui to discard that pass and
    /// to repaint, as a mid-pass `ctx.set_theme(..)` does; layout that the
    /// crate takes from the previous pass then needs one more pass, so the
    /// first frame of such a change can show it unsettled. Called before
    /// the first pass (in eframe's app creator, on `cc.egui_ctx`), the
    /// first frame is in the theme's fonts. An app's own `ctx.set_fonts`
    /// in a pass is not seen: egui asks for no repaint, so the screen keeps
    /// the old fonts until the next input. Install the fonts through
    /// [`Theme::apply_with_fonts`] instead, or call `ctx.request_repaint()`
    /// after `set_fonts`.
    pub fn apply(&self, ctx: &egui::Context) {
        self.apply_inner(ctx, None);
    }

    /// Like [`Theme::apply`], starting from the app's own font definitions
    /// instead of egui's defaults. Always builds the fonts again from
    /// `base`; egui takes them only when they differ from its own, so the
    /// same call in every pass costs no extra pass.
    pub fn apply_with_fonts(&self, ctx: &egui::Context, base: FontDefinitions) {
        self.apply_inner(ctx, Some(base));
    }

    fn apply_inner(&self, ctx: &egui::Context, base: Option<FontDefinitions>) {
        install_fonts(ctx, self.fonts, base);
        crate::components::icon_atom::install(ctx);
        crate::components::shared::install_focus_modality(ctx);
        let m = &self.metrics;
        let hairline = crate::components::shared::line_width(m.border, ctx.pixels_per_point());
        let heading = FontId::new(m.text_heading, heading_family_now(ctx));
        if heading.family == FontFamily::Proportional {
            ctx.data_mut(|d| {
                d.insert_temp(pending_heading_id(), PendingHeading::both(heading.clone()))
            });
        }

        for mode in [egui::Theme::Dark, egui::Theme::Light] {
            let dark = mode == egui::Theme::Dark;
            let p = self.palette_of(mode);
            ctx.style_mut_of(mode, |s| self.style_egui(s, p, dark, hairline, &heading));
        }

        ctx.data_mut(|d| {
            if d.get_temp::<Theme>(theme_id()).as_ref() != Some(self) {
                let generation = d.get_temp::<u64>(generation_id()).unwrap_or(0);
                d.insert_temp(generation_id(), generation + 1);
            }
            d.insert_temp(theme_id(), self.clone());
            d.insert_temp(hairline_id(), hairline);
        });
        install_hairline_hook(ctx);
        install_mode_switch_hook(ctx);
    }

    /// One mode's egui style: type scale, spacing and colours.
    fn style_egui(
        &self,
        s: &mut egui::Style,
        p: &Palette,
        dark: bool,
        hairline: f32,
        heading: &FontId,
    ) {
        let m = &self.metrics;
        use FontFamily::Proportional;
        s.text_styles = [
            (TextStyle::Small, FontId::new(m.text_xs, Proportional)),
            (TextStyle::Body, FontId::new(m.text_sm, Proportional)),
            (TextStyle::Button, FontId::new(m.text_sm, Proportional)),
            (
                TextStyle::Monospace,
                FontId::new(m.text_mono, FontFamily::Monospace),
            ),
            (TextStyle::Heading, heading.clone()),
        ]
        .into();

        // egui-native widgets follow the style's default button: its
        // height and horizontal padding, and the tabs gap between items.
        // A style without a value gets egui's default, also when an
        // earlier theme had set one.
        let egui_default = egui::style::Spacing::default();
        let button = [&*self.style.button_size_default];
        // Set once, not per pass: no window breakpoint applies.
        let none = State::NONE.with(State::DARK, dark);
        let gap = pick(&[&*self.style.tabs], |r| &r.gap, none).unwrap_or(DEFAULT_ITEM_GAP);
        let pad_x = pick(&button, |r| &r.pad_left, none).unwrap_or(egui_default.button_padding.x);
        let height = pick(&button, |r| &r.height, none);
        // egui's button is its text row plus twice `button_padding.y`, and
        // at least `interact_size.y`: keep the padding within the height.
        let pad_y = height.map_or(egui_default.button_padding.y, |h| {
            ((h - self.line_px(TextSize::Sm)) / 2.0).max(0.0)
        });
        s.spacing.item_spacing = egui::vec2(gap, gap);
        s.spacing.button_padding = egui::vec2(pad_x, pad_y);
        s.spacing.window_margin = egui::Margin::same(0);
        s.spacing.interact_size.y = height.unwrap_or(egui_default.interact_size.y);

        // Plain egui widgets fade as the style's controls do
        // (`disabled:opacity-50` on the button).
        let control = State::NONE.with(State::DARK, dark).or(State::DISABLED);
        let disabled_alpha = pick(&[&*self.style.button], |r| &r.opacity, control);
        let v = &mut s.visuals;
        v.disabled_alpha = disabled_alpha.unwrap_or(v.disabled_alpha);
        v.dark_mode = dark;
        // The ink of Chrome's grayscale anti-aliasing: light text on dark
        // takes `coverage^0.81`, dark text on light the plain coverage
        // (tests/text_coverage.rs). egui's dark default (`2c - c²`) draws
        // 3 to 25 % more.
        v.text_options.color_transfer_function = if dark {
            egui::epaint::FontColorTransferFunction::Gamma(0.81)
        } else {
            egui::epaint::FontColorTransferFunction::Off
        };
        v.panel_fill = p.background;
        v.window_fill = p.card;
        v.window_stroke = Stroke::new(hairline, p.border);
        v.extreme_bg_color = p.input;
        v.faint_bg_color = p.muted;
        v.override_text_color = Some(p.foreground);
        v.hyperlink_color = p.primary;
        v.selection.bg_fill = p.primary.gamma_multiply(0.35);
        v.selection.stroke = Stroke::new(hairline, p.ring);
        // Chrome's caret: 1 css px of the text colour (`caret-color: auto`).
        v.text_cursor.stroke = Stroke::new(1.0_f32, p.foreground);
        v.widgets.noninteractive.bg_fill = p.background;
        v.widgets.noninteractive.bg_stroke = Stroke::new(hairline, p.border);
        v.widgets.noninteractive.fg_stroke = Stroke::new(hairline, p.foreground);
        for w in [
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.bg_fill = p.secondary;
            w.weak_bg_fill = p.secondary;
            w.bg_stroke = Stroke::new(hairline, p.border);
            w.fg_stroke = Stroke::new(hairline, p.foreground);
            w.corner_radius = egui::CornerRadius::same(self.radius_md() as u8);
        }
        // shadcn uses the `accent` token for hover/active feedback so
        // native widgets (e.g. ComboBox dropdown items) don't look inert.
        for w in [&mut v.widgets.hovered, &mut v.widgets.active] {
            w.bg_fill = p.accent;
            w.weak_bg_fill = p.accent;
        }
    }
}

fn hairline_id() -> egui::Id {
    egui::Id::new("shadcn_egui::hairline")
}

/// The pass number, and the mode and the font install the pass started
/// with, as first seen by [`check_mode_switch`].
fn pass_mode_id() -> egui::Id {
    egui::Id::new("shadcn_egui::pass_mode")
}

/// The font definitions [`install_fonts`] installed last.
fn last_defs_id() -> egui::Id {
    egui::Id::new("shadcn_egui::last_font_definitions")
}

/// How many times [`install_fonts`] has installed fonts that differ from
/// the ones egui has.
fn font_installs_id() -> egui::Id {
    egui::Id::new("shadcn_egui::font_installs")
}

fn font_installs(ctx: &egui::Context) -> u64 {
    ctx.data(|d| d.get_temp::<u64>(font_installs_id()).unwrap_or(0))
}

/// The context's mode, checked against the mode its pass started in. A
/// `ctx.set_theme(..)` since then leaves the widgets drawn before it in the
/// other mode: discard the pass, as the browser restyles the whole page at
/// once, and repaint for a loop that cannot redo it. A font install since
/// the pass started does the same: egui swaps in new fonts only at the
/// next pass, so the whole pass is in the old fonts. Runs at the start and
/// end of every pass ([`install_mode_switch_hook`]) and in every
/// [`Theme::current`], so an app's own loop, which runs no pass hook, is
/// caught by the next crate widget.
fn check_mode_switch(ctx: &egui::Context) -> egui::Theme {
    let mode = ctx.theme();
    let pass = ctx.cumulative_pass_nr();
    let fonts = font_installs(ctx);
    match ctx.data(|d| d.get_temp::<(u64, egui::Theme, u64)>(pass_mode_id())) {
        Some((p, start, start_fonts)) if p == pass => {
            if start != mode {
                ctx.request_discard("shadcn-egui: light/dark mode changed in the pass");
                ctx.request_repaint();
            }
            if start_fonts != fonts {
                ctx.request_discard("shadcn-egui: the theme's fonts changed in the pass");
                ctx.request_repaint();
            }
        }
        _ => {
            ctx.data_mut(|d| d.insert_temp(pass_mode_id(), (pass, mode, fonts)));
        }
    }
    mode
}

/// Run [`check_mode_switch`] at the start and end of every pass of `ctx`,
/// so a switch after the pass's last crate widget, or before a plain egui
/// `Area`, does not leave a mixed or stale frame; installed once.
fn install_mode_switch_hook(ctx: &egui::Context) {
    let hook_id = egui::Id::new("shadcn_egui::mode_switch_hook");
    if ctx.data(|d| d.get_temp::<bool>(hook_id).unwrap_or(false)) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(hook_id, true));
    let check = |ui: &mut egui::Ui| {
        check_mode_switch(ui.ctx());
    };
    ctx.on_begin_pass("shadcn_egui::mode_switch", Arc::new(check));
    ctx.on_end_pass("shadcn_egui::mode_switch", Arc::new(check));
}

fn generation_id() -> egui::Id {
    egui::Id::new("shadcn_egui::theme_generation")
}

/// What the colours of the context's pass resolve from: the applied
/// theme (counted up by each [`Theme::apply`] of a different theme) and
/// the mode. A transition that sees it change jumps.
pub(crate) fn theme_stamp(ctx: &egui::Context) -> u64 {
    let generation = ctx.data(|d| d.get_temp::<u64>(generation_id()).unwrap_or(0));
    generation * 2 + u64::from(ctx.theme() == egui::Theme::Dark)
}

/// Every stroke [`Theme::apply`] gives the theme's hairline (`border`).
fn set_hairline(v: &mut egui::Visuals, width: f32) {
    v.window_stroke.width = width;
    v.selection.stroke.width = width;
    v.widgets.noninteractive.bg_stroke.width = width;
    v.widgets.noninteractive.fg_stroke.width = width;
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.bg_stroke.width = width;
        w.fg_stroke.width = width;
    }
}

/// Keep the hairline of plain egui widgets on whole device pixels, as the
/// crate's own borders: `Theme::apply` runs once, often before the window's
/// scale is known, and the scale can change later (zoom, another monitor).
/// Each pass checks the scale and rewrites the stroke widths only when the
/// snapped width changes, so style writes of the app's own stay.
fn install_hairline_hook(ctx: &egui::Context) {
    let hook_id = egui::Id::new("shadcn_egui::hairline_hook");
    if ctx.data(|d| d.get_temp::<bool>(hook_id).unwrap_or(false)) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(hook_id, true));
    ctx.on_begin_pass(
        "shadcn_egui::hairline",
        Arc::new(|ui: &mut egui::Ui| {
            if let Some(width) = snap_hairline(ui.ctx()) {
                // The root ui was built before this hook ran.
                set_hairline(&mut ui.style_mut().visuals, width);
            }
        }),
    );
}

/// The hairline for the context's current scale, written to both styles
/// when it differs from the last one written. Returns the new width then.
fn snap_hairline(ctx: &egui::Context) -> Option<f32> {
    let border = ctx
        .data(|d| d.get_temp::<Theme>(theme_id()))?
        .metrics
        .border;
    let width = crate::components::shared::line_width(border, ctx.pixels_per_point());
    if ctx.data(|d| d.get_temp::<f32>(hairline_id())) == Some(width) {
        return None;
    }
    ctx.data_mut(|d| d.insert_temp(hairline_id(), width));
    ctx.all_styles_mut(|s| set_hairline(&mut s.visuals, width));
    Some(width)
}

/// Convenience accessor for components.
pub fn theme(ctx: &egui::Context) -> Theme {
    Theme::current(ctx)
}
