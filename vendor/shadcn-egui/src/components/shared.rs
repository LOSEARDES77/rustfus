//! Shared painting helpers for shadcn-style components.

use crate::box_shadow::{clamp_radii, Layers};
use crate::color::Tone;
use crate::components::motion::{transition, transition_shadow, transition_tone};
use crate::style::{
    pick, pick_len, pick_ranked, Paint, Prop, Radius, Recipe, Shift, State, Transition,
};
use crate::Theme;
use egui::{
    Color32, Context, CornerRadius, Id, Key, Painter, PointerButton, Rect, Response, Shape, Stroke,
    Ui,
};
use std::sync::Arc;

/// CSS's initial `opacity`.
const INITIAL_OPACITY: f32 = 1.0; // behaviour-ok: CSS initial value of opacity

/// shadcn's `bits-ui` `sideOffset` default of `SelectContent` and
/// `PopoverContent`: a component prop, not a class.
pub const DEFAULT_SIDE_OFFSET: f32 = 4.0; // behaviour-ok: upstream prop default, not a class

/// A part's `opacity` under `s`, multiplied down the part tree as CSS does:
/// `chain` is the part's recipe stack, then its parent's, and so on.
pub fn opacity(chain: &[&[&Recipe]], s: State) -> f32 {
    chain
        .iter()
        .map(|stack| pick(stack, |r| &r.opacity, s).unwrap_or(INITIAL_OPACITY))
        .product()
}

/// [`opacity`] on its way to this state's, as each part in `chain` moves
/// its own `opacity` under its own `transition-*` ([`transition`]): a
/// disabled toggle fades where the transition names `opacity`
/// (`transition-all`, `transition-opacity`) and jumps elsewhere. `id` is
/// the widget's.
pub(crate) fn transitioned_opacity(ctx: &Context, id: Id, chain: &[&[&Recipe]], s: State) -> f32 {
    chain
        .iter()
        .enumerate()
        .map(|(i, stack)| {
            let o = pick(stack, |r| &r.opacity, s).unwrap_or(INITIAL_OPACITY);
            transition(
                ctx,
                id.with(("opacity", i)),
                o,
                stack,
                Transition::OPACITY,
                s,
            )
        })
        .product()
}

/// A part's text and icon colour `color` on its way to this state's, under
/// [`Transition::COLOR`] ([`transition_tone`]). `id` is the widget's.
pub(crate) fn transitioned_text(
    ctx: &Context,
    id: Id,
    color: Tone,
    layers: &[&Recipe],
    s: State,
) -> Color32 {
    transition_tone(
        ctx,
        id.with(("transition", "text")),
        color,
        layers,
        Transition::COLOR,
        s,
    )
    .color
}

/// The `Ui`'s opacity without egui's disabled fade. `Ui::disable` multiplies
/// the painter's opacity by `visuals.disabled_alpha`; a nested disable fades
/// again, which the `Ui` does not report, so one fade only is taken out.
fn opacity_without_fade(ui: &Ui) -> f32 {
    let fade = ui.visuals().disabled_alpha();
    if ui.is_enabled() || fade <= 0.0 {
        ui.opacity()
    } else {
        (ui.opacity() / fade).min(1.0) // behaviour-ok: opacity never exceeds 1
    }
}

/// The opacity a crate part paints at in `ui`: the `Ui`'s opacity with one
/// egui disabled fade taken out, times the part's [`opacity`] (`chain` as
/// there) under `s`, with `DISABLED` added in a disabled `Ui`. So a part
/// fades by its style's `disabled:` rule, not by egui's fade, and a part
/// with no such rule stays opaque in a disabled `Ui`, as in a disabled
/// `<fieldset>` (conformance 2.2). `id` is the widget's: a part whose
/// `transition-*` names `opacity` fades to its new value
/// ([`transition`]), as a disabled toggle does in the browser.
///
/// A nested disable fades twice: `ui.add_enabled_ui(false, ..)` inside a
/// `Ui` that is already disabled multiplies egui's fade in again, which the
/// `Ui` does not report, so a part there paints at that one extra fade.
/// Disable an area once.
pub fn part_opacity(ui: &Ui, id: Id, chain: &[&[&Recipe]], s: State) -> f32 {
    let s = s.with(
        State::DISABLED,
        s.contains(State::DISABLED) || !ui.is_enabled(),
    );
    opacity_without_fade(ui) * transitioned_opacity(ui.ctx(), id, chain, s)
}

/// The `Ui`'s painter at the part's [`part_opacity`].
///
/// CSS fades a part as one group; egui fades shape by shape. The part's
/// own box matches the group: [`RecipeBox::paint`] lays fill and border
/// side by side when faded, never one over the other. What a part paints
/// over its box (an icon, text, a switch thumb on its track) does not:
/// with the content `c` over the fill `f` (premultiplied, alpha `c.a` and
/// `f.a`), the opacity `o` and the page `B` under the part, the group
/// gives `o·c + o·f·(1 − c.a) + (1 − o·(c.a + f.a·(1 − c.a)))·B`, shape by
/// shape gives `o·c + (1 − o·c.a)·(o·f + (1 − o·f.a)·B)`, and the second
/// is brighter by `o·(1 − o)·c.a·(f − f.a·B)`. That is 0 only at `o` 0 or
/// 1, for clear content, or where the fill matches the page; otherwise it
/// depends on `B`, which the painter does not know. A disabled checked
/// checkbox (lyra dark, `o` = 0.5) shows its check at about 74 where
/// Chrome shows 16.
pub fn part_painter(ui: &Ui, id: Id, chain: &[&[&Recipe]], s: State) -> Painter {
    let mut painter = ui.painter().clone();
    painter.set_opacity(part_opacity(ui, id, chain, s));
    painter
}

/// Run `add` with `ui` at `opacity`, then restore the `Ui`'s own opacity.
/// A [`Ui::disable`] inside `add` (a disabled card action) keeps its fade:
/// the `Ui` stays disabled, so what it paints afterwards stays faded too.
fn with_opacity<R>(ui: &mut Ui, opacity: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let (before, was_enabled) = (ui.opacity(), ui.is_enabled());
    ui.set_opacity(opacity);
    let inner = add(ui);
    ui.set_opacity(before);
    // Restoring `before` undid the fade egui's `disable` multiplied in.
    if was_enabled && !ui.is_enabled() {
        ui.multiply_opacity(ui.visuals().disabled_alpha());
    }
    inner
}

/// Run `add`, which draws a crate part through egui (a title through
/// `ui.label`), at the part's [`part_opacity`]. Unlike a scope, this adds no
/// layout of its own.
pub fn part_ui<R>(
    ui: &mut Ui,
    id: Id,
    chain: &[&[&Recipe]],
    s: State,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let opacity = part_opacity(ui, id, chain, s);
    with_opacity(ui, opacity, add)
}

/// Run `add`, the caller's content of a container part, at the part's
/// opacity. egui's disabled fade stays, so plain egui widgets in it show the
/// theme's `disabled_alpha`, and crate parts in it take it out again with
/// [`part_opacity`]. Unlike a scope, this adds no layout of its own.
///
/// A nested disable fades twice, here as in [`part_opacity`]:
/// `ui.add_enabled_ui(false, ..)` inside a disabled container fades its
/// widgets by the theme's `disabled_alpha` once more.
pub fn content_ui<R>(
    ui: &mut Ui,
    id: Id,
    chain: &[&[&Recipe]],
    s: State,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let opacity = ui.opacity()
        * transitioned_opacity(
            ui.ctx(),
            id,
            chain,
            s.with(
                State::DISABLED,
                s.contains(State::DISABLED) || !ui.is_enabled(),
            ),
        );
    with_opacity(ui, opacity, add)
}

/// A `border-*` or `outline-*` width of `px` as the browser lays it out on
/// a screen of `ppp` device pixels per point: floored to whole device
/// pixels, and one device pixel where it is above 0 but thinner (Blink's
/// `StyleBuilderConverter::ClampLineWidth`). A 1 px border is 1 device px
/// at a scale of 1.5, a 2 px border 3.
pub fn line_width(px: f32, ppp: f32) -> f32 {
    let device = px * ppp;
    if device <= 0.0 {
        0.0
    } else {
        // A width that is a whole number of device pixels stays whole
        // through the float product; a thinner one becomes one device pixel.
        (device + 1.0e-4).floor().max(1.0) / ppp // behaviour-ok: float tolerance, one device px
    }
}

/// An egui `Frame` that paints nothing and insets its content by `insets`
/// (top, right, bottom, left, in points). egui's inner margin takes whole
/// points only; the fraction the sides share goes into a transparent
/// stroke, which egui adds to every side, so an inset such as 12 px padding
/// plus a border of 1 device px at a scale of 1.5 stays exact. A side whose
/// fraction differs from the left side's is rounded to whole points.
pub fn inset_frame(insets: [f32; 4]) -> egui::Frame {
    let shared = insets[3].max(0.0).fract();
    let [t, r, b, l] = insets.map(|v| (v - shared).round().clamp(0.0, 127.0) as i8); // behaviour-ok: egui's i8 margin
    egui::Frame::NONE
        .inner_margin(egui::Margin {
            left: l,
            right: r,
            top: t,
            bottom: b,
        })
        .stroke(Stroke::new(shared, Color32::TRANSPARENT))
}

/// A part's box resolved from its recipe stack under one [`State`]: fill,
/// border, shadow, focus or invalid ring (`ring-*`) and outline. Every crate
/// widget and panel paints its box through it.
///
/// Resolve geometry (padding, size) with the state without interaction
/// flags, and the box with the [`interaction_state`] of the paint, so the
/// colours follow hover and focus while the size does not. Border and
/// outline widths come out on whole device pixels as the browser lays them
/// out ([`line_width`]).
///
/// ```
/// use shadcn_egui::components::shared::RecipeBox;
/// use shadcn_egui::style::State;
/// use shadcn_egui::Theme;
///
/// fn paint_card(ui: &egui::Ui, rect: egui::Rect) {
///     let theme = Theme::current(ui.ctx());
///     let card = [&*theme.style.card];
///     let state = theme.state(ui.ctx(), State::NONE);
///     RecipeBox::resolve(ui.ctx(), &theme, &card, state).paint(ui.painter(), rect);
/// }
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct RecipeBox {
    /// Corner radii in px before CSS's clamping: nw, ne, sw, se.
    radii: [f32; 4],
    fill: Option<Color32>,
    border_color: Color32,
    /// Border widths in px on whole device pixels: top, right, bottom, left.
    borders: [f32; 4],
    clip_padding: bool,
    /// `box-shadow` layers, first on top, and their fade.
    shadow: Layers,
    shadow_opacity: f32,
    /// The fade [`RecipeBox::faded`] put into every colour.
    opacity: f32,
    ring: Option<Stroke>,
    outline: Option<Stroke>,
    /// The outline's colour, also while it has no width: shadcn's base
    /// `outline-ring/50` where the part sets none.
    outline_color: Color32,
    /// The colours a transition fades from and to.
    tones: BoxTones,
}

/// A [`RecipeBox`]'s colours with the unclipped value each stands for
/// ([`Tone`]); an absent fill or ring is transparent.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BoxTones {
    fill: Tone,
    border: Tone,
    outline: Tone,
    ring: Tone,
}

/// `rounded-full` (`calc(infinity * 1px)`) as a finite length, so CSS's
/// corner clamping can scale it.
const FULL_RADIUS: f32 = 1.0e9; // behaviour-ok: stands in for CSS's infinite radius

impl RecipeBox {
    /// The box of the part whose recipes are `layers` (in upstream's class
    /// order, as for [`pick`]) under `state`, its border and outline widths
    /// on the device pixels of `ctx`.
    ///
    /// A side's border width (`border-t-*`, `border-l-*`) and a corner's
    /// radius (`rounded-t-*`, `rounded-b-*`, `rounded-l-*`, `rounded-r-*`)
    /// win against `border-*` and `rounded-*` by rank, as in CSS; at equal
    /// rank the later layer wins (tailwind-merge drops the earlier class),
    /// and within one layer the side wins (Tailwind sorts the shorthand
    /// first). A border without a colour of its own takes shadcn's base
    /// `border-border`, an outline shadcn's base `outline-ring/50`.
    pub fn resolve(ctx: &egui::Context, theme: &Theme, layers: &[&Recipe], state: State) -> Self {
        let ppp = ctx.pixels_per_point();
        // Candidates in Tailwind's property order: a later one wins a tie.
        fn side<T: Copy>(cands: &[Option<((u128, usize), T)>]) -> Option<T> {
            let mut best: Option<((u128, usize), T)> = None;
            for c in cands.iter().flatten() {
                if best.is_none_or(|(k, _)| c.0 >= k) {
                    best = Some(*c);
                }
            }
            best.map(|(_, v)| v)
        }
        let tone = |f: fn(&Recipe) -> &Prop<Paint>| pick(layers, f, state).map(|p| theme.tone(p));
        let width = |f: fn(&Recipe) -> &Prop<f32>| pick_ranked(layers, f, state);
        let all = width(|r| &r.border_width);
        let top = width(|r| &r.border_top_width);
        let left = width(|r| &r.border_left_width);
        let borders = [
            side(&[all, top]),
            side(&[all]),
            side(&[all]),
            side(&[all, left]),
        ]
        .map(|w| line_width(w.unwrap_or(0.0), ppp));
        let radius = |f: fn(&Recipe) -> &Prop<Radius>| {
            pick_ranked(layers, f, state).map(|(k, r)| {
                let px = match r {
                    Radius::Rung(crate::style::Rung::Full) => FULL_RADIUS,
                    r => theme.radius_px(r, FULL_RADIUS),
                };
                (k, px)
            })
        };
        let (base, t, r, b, l) = (
            radius(|r| &r.radius),
            radius(|r| &r.radius_top),
            radius(|r| &r.radius_right),
            radius(|r| &r.radius_bottom),
            radius(|r| &r.radius_left),
        );
        // Tailwind's order: rounded, rounded-t, -r, -b, -l.
        let corner = |a, b| side(&[base, a, b]).unwrap_or(0.0);
        let ring_w = pick_len(layers, |r| &r.ring_width, state);
        let outline_w = line_width(pick_len(layers, |r| &r.outline_width, state), ppp);
        let stroke = |w: f32, c: Option<Color32>| {
            c.filter(|c| w > 0.0 && c.a() > 0)
                .map(|c| Stroke::new(w, c))
        };
        // shadcn's base layer `* { @apply border-border }`.
        let base_border = theme.tone(Paint::Role(crate::style::Role::Border, 1.0)); // behaviour-ok: CSS base layer colour of every border
                                                                                    // shadcn's base layer `* { @apply outline-ring/50 }`.
        let base_outline = theme.tone(Paint::Role(crate::style::Role::Ring, 0.5)); // behaviour-ok: CSS base layer colour of every outline
        let clear = Tone::of(Color32::TRANSPARENT);
        let fill = tone(|r| &r.bg).filter(|c| c.color.a() > 0);
        let border = tone(|r| &r.border_color).unwrap_or(base_border);
        let outline = tone(|r| &r.outline_color).unwrap_or(base_outline);
        let ring_tone = tone(|r| &r.ring_color);
        let ring = stroke(ring_w, ring_tone.map(|c| c.color));
        Self {
            radii: [corner(t, l), corner(t, r), corner(b, l), corner(r, b)],
            fill: fill.map(|c| c.color),
            border_color: border.color,
            borders,
            clip_padding: pick(layers, |r| &r.bg_clip_padding, state) == Some(true),
            shadow: pick(layers, |r| &r.shadow, state)
                .map_or(Layers::NONE, |s| Layers::new(theme.shadow(s))),
            shadow_opacity: INITIAL_OPACITY,
            opacity: INITIAL_OPACITY,
            ring,
            outline: stroke(outline_w, Some(outline.color)),
            outline_color: outline.color,
            tones: BoxTones {
                fill: fill.unwrap_or(clear),
                border,
                outline,
                ring: ring_tone.filter(|_| ring.is_some()).unwrap_or(clear),
            },
        }
    }

    /// The same box with its colours on the way to this state's, as the
    /// part's `transition-*` moves them
    /// ([`transition_color`](crate::components::motion::transition_color)): the fill
    /// under [`Transition::BACKGROUND`], the border colour under
    /// [`Transition::BORDER_COLOR`], the outline colour under
    /// [`Transition::OUTLINE_COLOR`], the outline's width under
    /// [`Transition::OTHER`] (only `transition-all` names it), and the
    /// `box-shadow`, its elevation layers and the ring's width and colour,
    /// under [`Transition::SHADOW`]. A property the transition does not
    /// name jumps. `id` is the widget's; each property
    /// keys its own transition under it.
    pub fn transitioned(
        mut self,
        ctx: &egui::Context,
        id: egui::Id,
        layers: &[&Recipe],
        state: State,
    ) -> Self {
        let color = |key: &str, c: Tone, p: Transition| {
            transition_tone(ctx, id.with(("transition", key)), c, layers, p, state)
        };
        let tones = &mut self.tones;
        tones.fill = color("bg", tones.fill, Transition::BACKGROUND);
        self.fill = (tones.fill.color.a() > 0).then_some(tones.fill.color);
        tones.border = color("border", tones.border, Transition::BORDER_COLOR);
        self.border_color = tones.border.color;
        let width = |key: &str, w: f32, p: Transition| {
            transition(ctx, id.with(("transition", key)), w, layers, p, state)
        };
        // An absent outline has no width (`outline-style: none`) but keeps
        // its colour: it grows from nothing and its colour moves from or to
        // the resting one (shadcn's base `outline-ring/50`).
        tones.outline = color("outline", tones.outline, Transition::OUTLINE_COLOR);
        self.outline_color = tones.outline.color;
        let w = width(
            "outline-width",
            self.outline.map_or(0.0, |o| o.width),
            Transition::OTHER,
        );
        let outline = Stroke::new(w, self.outline_color);
        self.outline = (outline.width > 0.0 && outline.color.a() > 0).then_some(outline);
        self.shadow = transition_shadow(
            ctx,
            id.with(("transition", "shadow")),
            self.shadow,
            layers,
            state,
        );
        // An absent ring is CSS's `0 0 #0000` shadow: it grows out of
        // nothing and fades back into it.
        let ring_width = self.ring.map_or(0.0, |r| r.width);
        let ring_width = width("ring-width", ring_width, Transition::SHADOW);
        self.tones.ring = color("ring", self.tones.ring, Transition::SHADOW);
        let ring = Stroke::new(ring_width, self.tones.ring.color);
        self.ring = (ring.width > 0.0 && ring.color.a() > 0).then_some(ring);
        self
    }

    /// The same box with every colour faded by `opacity`: for a box an
    /// egui container paints outside the `Ui` that carries the part's fade
    /// (a popup's frame, a modal's panel).
    #[inline]
    pub fn faded(mut self, opacity: f32) -> Self {
        self.fill = self.fill.map(|c| c.gamma_multiply(opacity));
        self.border_color = self.border_color.gamma_multiply(opacity);
        self.outline_color = self.outline_color.gamma_multiply(opacity);
        let tones = &mut self.tones;
        for t in [
            &mut tones.fill,
            &mut tones.border,
            &mut tones.outline,
            &mut tones.ring,
        ] {
            *t = t.faded(opacity);
        }
        self.shadow_opacity *= opacity;
        self.opacity *= opacity;
        for s in [&mut self.ring, &mut self.outline].into_iter().flatten() {
            s.color = s.color.gamma_multiply(opacity);
        }
        self
    }

    /// The corner radii on `rect`, clamped as CSS clamps them: where two
    /// radii on one side add up to more than its length, all four scale
    /// down by the same factor (so `rounded-full` is half the short side).
    pub fn radius_on(&self, rect: Rect) -> CornerRadius {
        let [nw, ne, sw, se] = clamp_radii(self.radii, rect);
        let c = |v: f32| v.clamp(0.0, 255.0).round() as u8; // behaviour-ok: egui's u8 corner radius
        CornerRadius {
            nw: c(nw),
            ne: c(ne),
            sw: c(sw),
            se: c(se),
        }
    }

    /// Paint the box on `rect` cut to the inside of `outer`'s padding box
    /// on `outer_rect`, as CSS paints a child of a part with `overflow:
    /// hidden` (the progress indicator in its track, the slider range): the
    /// cut follows the outer box's rounding exactly, whatever the size and
    /// corners of this box.
    pub(crate) fn paint_inside(
        &self,
        painter: &Painter,
        rect: Rect,
        outer: &RecipeBox,
        outer_rect: Rect,
    ) {
        painter.extend(self.shapes_inside(
            painter.ctx(),
            rect,
            outer,
            outer_rect,
            painter.opacity(),
        ));
    }

    /// [`RecipeBox::paint_inside`]'s shapes: meshes cut to the outer box.
    pub(crate) fn shapes_inside(
        &self,
        ctx: &egui::Context,
        rect: Rect,
        outer: &RecipeBox,
        outer_rect: Rect,
        opacity: f32,
    ) -> Vec<Shape> {
        // egui puts a painted rect on whole pixels, as the browser snaps a
        // box; the cut follows the boxes where they are painted.
        let snap = |r: Rect, own: Option<bool>| {
            if own.unwrap_or_else(|| ctx.tessellation_options(|o| o.round_rects_to_pixels)) {
                egui::emath::GuiRounding::round_to_pixels(r, ctx.pixels_per_point())
            } else {
                r
            }
        };
        let outline = outer.padding_outline(ctx, outer_rect);
        let shapes = self.shapes_at(ctx, rect, opacity);
        // A plain fill (every style's indicator and range): the outer box's
        // outline cut to the rect, a convex polygon egui anti-aliases as it
        // does the outer box itself.
        if let [Shape::Rect(r)] = shapes.as_slice() {
            if r.stroke.width <= 0.0 && r.corner_radius == CornerRadius::ZERO {
                let points = clip_to_rect(&outline, snap(r.rect, r.round_to_pixels));
                return if points.len() < 3 {
                    Vec::new()
                } else {
                    vec![Shape::convex_polygon(points, r.fill, Stroke::NONE)]
                };
            }
        }
        let clip = RoundedClip::new(ctx, outline);
        let mut out = Vec::new();
        for shape in shapes {
            cut(ctx, shape, &|m| clip.apply(m), &mut out);
        }
        out
    }

    /// `shapes`, a part painted under this box at `rect`, less what the
    /// box hides of them while it is faded (`opacity`, the box's painter's
    /// opacity) and opaque: CSS fades a component's root as one group, so
    /// an opaque part (a slider's or a switch's thumb) hides what lies
    /// under it, where per-shape fading would let it show through. The
    /// cut follows the box's outline as egui paints it, the hidden part
    /// fading out across the outline's feathering band as the box fades
    /// in. Unfaded or see-through, the shapes come back as they are.
    pub(crate) fn hiding(
        &self,
        ctx: &egui::Context,
        rect: Rect,
        opacity: f32,
        shapes: Vec<Shape>,
    ) -> Vec<Shape> {
        if !self.hides(opacity) {
            return shapes;
        }
        let outline = rounded_polygon(on_pixels(ctx, rect), self.radius_on(rect));
        hide(ctx, outline, shapes)
    }

    /// [`RecipeBox::hiding`] for this box painted inside `outer` at
    /// `outer_rect` ([`RecipeBox::shapes_inside`]): the slider's range
    /// hides the track under it. The cut follows the range as it is
    /// painted, the outer box's rounding included.
    pub(crate) fn hiding_inside(
        &self,
        ctx: &egui::Context,
        rect: Rect,
        outer: &RecipeBox,
        outer_rect: Rect,
        opacity: f32,
        shapes: Vec<Shape>,
    ) -> Vec<Shape> {
        if !self.hides(opacity) {
            return shapes;
        }
        let own = rounded_polygon(on_pixels(ctx, rect), self.radius_on(rect));
        let outline = clip_convex(&own, &outer.padding_outline(ctx, outer_rect));
        if outline.len() < 3 {
            return shapes;
        }
        hide(ctx, outline, shapes)
    }

    /// Whether this box, painted at `opacity`, is faded and opaque: then
    /// it hides what lies under it (see [`RecipeBox::hiding`]).
    fn hides(&self, opacity: f32) -> bool {
        // The fill's own alpha, before `faded`'s fade.
        let opaque = self.fill.is_some_and(|f| {
            f.to_normalized_gamma_f32()[3] >= self.opacity - 0.5 / 255.0 // behaviour-ok: one 8-bit alpha step
        });
        opaque && self.opacity * opacity < INITIAL_OPACITY
    }

    /// The padding box's outline at `rect` as egui paints the box, on
    /// whole pixels where egui puts it: what `overflow: hidden` cuts a
    /// child to.
    fn padding_outline(&self, ctx: &egui::Context, rect: Rect) -> Vec<egui::Pos2> {
        rounded_polygon(
            on_pixels(ctx, padding_box(rect, self.borders)),
            inner_radius(self.radius_on(rect), self.borders),
        )
    }

    /// The fill colour, if the recipe sets one that is not transparent.
    #[inline]
    pub fn fill(&self) -> Option<Color32> {
        self.fill
    }

    /// The border widths: top, right, bottom, left.
    #[inline]
    pub fn border_widths(&self) -> [f32; 4] {
        self.borders
    }

    /// The border as one stroke, if all four sides are as wide and not 0.
    fn uniform_border(&self) -> Option<Stroke> {
        let [t, r, b, l] = self.borders;
        (t > 0.0 && t == r && t == b && t == l).then(|| Stroke::new(t, self.border_color))
    }

    /// The box as shapes in paint order on `rect`: shadow (outside the
    /// border box only), ring, fill (inside the border under
    /// `bg-clip-padding`; to the middle of an opaque border on every side,
    /// as Chrome paints it), border inside `rect` (`box-sizing:
    /// border-box`), outline. The shapes are for a painter at full
    /// opacity; [`RecipeBox::paint`] reads the painter's.
    pub fn shapes(&self, ctx: &egui::Context, rect: Rect) -> Vec<Shape> {
        self.shapes_at(ctx, rect, INITIAL_OPACITY)
    }

    /// [`RecipeBox::shapes`] for a painter at `opacity`. A faded box (this
    /// `opacity` or [`RecipeBox::faded`] below 1) paints as CSS paints a
    /// part under `opacity`, as one group: fill and border do not overlap.
    /// The fill ends at the border's inner edge, the border takes its
    /// colour over the fill (a translucent border pre-composited, as the
    /// browser shows it), and the shadow is already outside the border
    /// box. Both are cut from one outline along the same edge, so no
    /// seam opens between them. See [`part_painter`] for what stays
    /// per shape.
    pub(crate) fn shapes_at(&self, ctx: &egui::Context, rect: Rect, opacity: f32) -> Vec<Shape> {
        let radius = self.radius_on(rect);
        let mut out = Vec::new();
        out.extend(self.shadow_shape(ctx, rect, radius));
        out.extend(outside_stroke(self.ring, rect, radius));
        let [t, r, b, l] = self.borders;
        let faded = self.opacity * opacity < INITIAL_OPACITY;
        let mut border_done = false;
        if let Some(fill) = self.fill {
            if self.clip_padding && t + r + b + l > 0.0 {
                let inner = padding_box(rect, self.borders);
                out.push(Shape::rect_filled(
                    inner,
                    inner_radius(radius, self.borders),
                    fill,
                ));
            } else if faded && t + r + b + l > 0.0 && self.border_color.a() > 0 {
                let placed = on_pixels(ctx, rect);
                let hole = rounded_polygon(
                    padding_box(placed, self.borders),
                    inner_radius(radius, self.borders),
                );
                let border = over(self.border_color, fill, self.opacity);
                let outline = rounded_polygon(placed, radius);
                let mesh = match two_tone(ctx, &outline, hole.clone(), self.borders, fill, border) {
                    Some(mesh) => Arc::new(mesh),
                    None => {
                        let key = CutKey::new(ctx, placed, self.borders, radius, fill, border);
                        cut_mesh(ctx, key, || {
                            // The box's own coverage, its outer edge feathered.
                            let cover = tessellate(
                                ctx,
                                // token-ok: a coverage mask, not a colour
                                &egui::epaint::RectShape::filled(placed, radius, Color32::WHITE) // behaviour-ok: coverage mask
                                    .with_round_to_pixels(false),
                            );
                            RoundedClip::new(ctx, hole).two_tone(&cover, fill, border)
                        })
                    }
                };
                out.push(Shape::Mesh(mesh));
                border_done = true;
            } else if t > 0.0 && r > 0.0 && b > 0.0 && l > 0.0 && self.border_color.a() == 255 {
                // Blink's background bleed avoidance: under an opaque
                // border on every side the fill ends half a border width
                // in, so no fill shows in the border's anti-aliased outer
                // edge. The cut is made on the box as egui puts it on the
                // pixels, and stays where it is cut.
                let half = self.borders.map(|w| w / 2.0);
                let placed = on_pixels(ctx, rect);
                out.push(Shape::Rect(
                    egui::epaint::RectShape::filled(
                        padding_box(placed, half),
                        inner_radius(radius, half),
                        fill,
                    )
                    .with_round_to_pixels(false),
                ));
            } else {
                out.push(Shape::rect_filled(rect, radius, fill));
            }
        }
        // A faded box's border went in with its fill.
        let border = (!border_done).then(|| self.uniform_border());
        if let Some(Some(stroke)) = border {
            out.push(Shape::rect_stroke(
                rect,
                radius,
                stroke,
                egui::StrokeKind::Inside,
            ));
        } else if border.is_some() && t + r + b + l > 0.0 && self.border_color.a() > 0 {
            // Sides of unequal width: the border box minus the padding box.
            let mesh = tessellate(
                ctx,
                &egui::epaint::RectShape::filled(rect, radius, self.border_color),
            );
            let hole = rounded_polygon(
                padding_box(rect, self.borders),
                inner_radius(radius, self.borders),
            );
            out.push(Shape::mesh(clip_outside(&mesh, &hole)));
        }
        out.extend(outside_stroke(self.outline, rect, radius));
        out
    }

    /// Paint [`RecipeBox::shapes`] with `painter`.
    pub fn paint(&self, painter: &Painter, rect: Rect) {
        painter.extend(self.shapes_at(painter.ctx(), rect, painter.opacity()));
    }

    /// An egui `Frame` that paints nothing, its inner margin `padding`
    /// (top, right, bottom, left) plus the border widths, for a panel whose
    /// size egui learns only while it runs (a popup, a modal). Its first
    /// call in the panel's closure is [`RecipeBox::reserve`]; after `show`,
    /// [`RecipeBox::paint_reserved`] paints the box under the content.
    ///
    /// ```
    /// use shadcn_egui::components::shared::RecipeBox;
    /// use shadcn_egui::style::State;
    /// use shadcn_egui::Theme;
    ///
    /// fn panel(ctx: &egui::Context) {
    ///     let theme = Theme::current(ctx);
    ///     let popover = [&*theme.style.popover_content];
    ///     let state = theme.state(ctx, State::NONE);
    ///     let panel = RecipeBox::resolve(ctx, &theme, &popover, state);
    ///     let shown = egui::Area::new(egui::Id::new("panel")).show(ctx, |ui| {
    ///         panel
    ///             .frame([16.0; 4])
    ///             .show(ui, |ui| {
    ///                 let slot = RecipeBox::reserve(ui);
    ///                 ui.label("Content");
    ///                 slot
    ///             })
    ///             .inner
    ///     });
    ///     panel.paint_reserved(ctx, shown.response.layer_id, shown.inner, shown.response.rect);
    /// }
    /// ```
    pub fn frame(&self, padding: [f32; 4]) -> egui::Frame {
        inset_frame(std::array::from_fn(|i| padding[i] + self.borders[i]))
    }

    /// Hold the box's place in `ui`'s layer, under everything the panel's
    /// content paints after it.
    pub fn reserve(ui: &Ui) -> egui::layers::ShapeIdx {
        ui.painter().add(Shape::Noop)
    }

    /// Paint the box on `rect` into the place [`RecipeBox::reserve`] held
    /// in `layer`: shadow, ring, fill, border and outline under the content,
    /// as CSS paints a box under its children.
    pub fn paint_reserved(
        &self,
        ctx: &egui::Context,
        layer: egui::LayerId,
        slot: egui::layers::ShapeIdx,
        rect: Rect,
    ) {
        ctx.layer_painter(layer)
            .set(slot, Shape::Vec(self.shapes(ctx, rect)));
    }

    /// Every shadow layer as the browser blurs it, cut to the outside of
    /// the border box: an outer `box-shadow` never shows through the box
    /// (conformance 2.5).
    fn shadow_shape(&self, ctx: &egui::Context, rect: Rect, radius: CornerRadius) -> Option<Shape> {
        (!self.shadow.as_slice().is_empty() && self.shadow_opacity > 0.0).then(|| {
            // The fill's rect as egui's tessellator places it.
            let rect = on_pixels(ctx, rect);
            Shape::Mesh(crate::box_shadow::shadow_mesh(
                ctx,
                rect,
                clamp_radii(self.radii, rect),
                radius,
                self.shadow,
                self.shadow_opacity,
            ))
        })
    }
}

/// `rect` on the physical pixel grid where egui rounds rects before it
/// fills them, so a shadow's cut meets the fill's edge.
fn on_pixels(ctx: &egui::Context, rect: Rect) -> Rect {
    use egui::emath::GuiRounding as _;
    if ctx.tessellation_options(|o| o.round_rects_to_pixels) {
        rect.round_to_pixels(ctx.pixels_per_point())
    } else {
        rect
    }
}

/// `shapes` less what lies inside the convex `outline`, faded in across
/// its feathering band ([`RoundedClip::remove`]).
fn hide(ctx: &egui::Context, outline: Vec<egui::Pos2>, shapes: Vec<Shape>) -> Vec<Shape> {
    let clip = RoundedClip::new(ctx, outline);
    let mut out = Vec::new();
    for shape in shapes {
        cut(ctx, shape, &|m| clip.remove(m), &mut out);
    }
    out
}

/// Everything [`two_tone`]'s fallback mesh depends on: the box as egui
/// puts it on the pixels, its borders, radius and colours, and the
/// context's scale and feathering.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct CutKey {
    rect: [u32; 4],
    borders: [u32; 4],
    radius: CornerRadius,
    fill: Color32,
    border: Color32,
    ppp: u32,
    feathering: Option<u32>,
}

impl CutKey {
    fn new(
        ctx: &egui::Context,
        rect: Rect,
        borders: [f32; 4],
        radius: CornerRadius,
        fill: Color32,
        border: Color32,
    ) -> Self {
        let (on, px) = ctx.tessellation_options(|o| (o.feathering, o.feathering_size_in_pixels));
        Self {
            rect: [rect.min.x, rect.min.y, rect.max.x, rect.max.y].map(f32::to_bits),
            borders: borders.map(f32::to_bits),
            radius,
            fill,
            border,
            ppp: ctx.pixels_per_point().to_bits(),
            feathering: on.then_some(px.to_bits()),
        }
    }
}

/// The fallback meshes of faded boxes with sides of unequal width, which
/// cost 1-4k vertices and milliseconds to cut: the ones the last pass
/// used and the ones this pass used so far. A pass keeps only what it
/// uses, as egui's `FrameCache` does, so an animated box cannot grow it.
#[derive(Clone, Default)]
struct CutCache {
    pass: u64,
    current: std::collections::HashMap<CutKey, Arc<egui::Mesh>>,
    previous: std::collections::HashMap<CutKey, Arc<egui::Mesh>>,
    /// Meshes built, for the tests.
    builds: u64,
}

fn cut_cache_id() -> Id {
    Id::new("shadcn_egui::cut_cache")
}

/// The mesh for `key`, from the cache or from `build`.
fn cut_mesh(
    ctx: &egui::Context,
    key: CutKey,
    build: impl FnOnce() -> egui::Mesh,
) -> Arc<egui::Mesh> {
    let pass = ctx.cumulative_pass_nr();
    let cached = ctx.data_mut(|d| {
        let cache = d.get_temp_mut_or_default::<CutCache>(cut_cache_id());
        if cache.pass != pass {
            cache.pass = pass;
            cache.previous = std::mem::take(&mut cache.current);
        }
        let hit = cache
            .current
            .get(&key)
            .cloned()
            .or_else(|| cache.previous.remove(&key));
        if let Some(mesh) = &hit {
            cache.current.insert(key, mesh.clone());
        }
        hit
    });
    if let Some(mesh) = cached {
        return mesh;
    }
    let mesh = Arc::new(build());
    ctx.data_mut(|d| {
        let cache = d.get_temp_mut_or_default::<CutCache>(cut_cache_id());
        cache.current.insert(key, mesh.clone());
        cache.builds += 1;
    });
    mesh
}

/// `top` composited over `under` (premultiplied, as the browser blends),
/// both already faded by `opacity`: `top + under * (1 - top's own alpha)`.
fn over(top: Color32, under: Color32, opacity: f32) -> Color32 {
    // `top`'s own alpha, before the fade.
    let alpha = top.to_normalized_gamma_f32()[3] / opacity.max(f32::EPSILON);
    let keep = (1.0 - alpha).max(0.0); // behaviour-ok: CSS source-over keeps 1 - alpha of what is under
    under.gamma_multiply(keep) + top
}

/// A `ring-*` or `outline-*` just outside the border box.
fn outside_stroke(stroke: Option<Stroke>, rect: Rect, radius: CornerRadius) -> Option<Shape> {
    stroke.map(|s| Shape::rect_stroke(rect, radius, s, egui::StrokeKind::Outside))
}

/// The padding box: the border box minus each side's border.
fn padding_box(rect: Rect, [t, r, b, l]: [f32; 4]) -> Rect {
    Rect::from_min_max(rect.min + egui::vec2(l, t), rect.max - egui::vec2(r, b))
}

/// The padding box's corner radii: the outer radius minus the wider
/// border that meets at the corner.
fn inner_radius(radius: CornerRadius, [t, r, b, l]: [f32; 4]) -> CornerRadius {
    let inset = |v: u8, a: f32, b: f32| (f32::from(v) - a.max(b)).max(0.0).round() as u8;
    CornerRadius {
        nw: inset(radius.nw, t, l),
        ne: inset(radius.ne, t, r),
        sw: inset(radius.sw, b, l),
        se: inset(radius.se, b, r),
    }
}

/// `shape` as the mesh egui's tessellator makes of it in this context.
fn tessellate(ctx: &egui::Context, shape: &egui::epaint::RectShape) -> egui::Mesh {
    let mut mesh = egui::Mesh::default();
    tessellator(ctx).tessellate_rect(shape, &mut mesh);
    mesh
}

/// egui's tessellator as this context sets it up, for box shapes (no text).
fn tessellator(ctx: &egui::Context) -> egui::epaint::Tessellator {
    egui::epaint::Tessellator::new(
        ctx.pixels_per_point(),
        ctx.tessellation_options(|o| *o),
        [1, 1], // behaviour-ok: no text in a box, the font atlas size is unused
        Vec::new(),
    )
}

/// `shape` as meshes in this context, each passed through `cut`.
fn cut(
    ctx: &egui::Context,
    shape: Shape,
    cut_mesh: &dyn Fn(&egui::Mesh) -> egui::Mesh,
    out: &mut Vec<Shape>,
) {
    match shape {
        Shape::Noop => {}
        Shape::Vec(shapes) => {
            for s in shapes {
                cut(ctx, s, cut_mesh, out);
            }
        }
        Shape::Mesh(mesh) => out.push(Shape::mesh(cut_mesh(&mesh))),
        shape => {
            let mut mesh = egui::Mesh::default();
            tessellator(ctx).tessellate_shape(shape, &mut mesh);
            out.push(Shape::mesh(cut_mesh(&mesh)));
        }
    }
}

/// The convex outline of a rounded rect, as egui's tessellator traces it.
pub(crate) fn rounded_polygon(rect: Rect, radius: CornerRadius) -> Vec<egui::Pos2> {
    let mut points = Vec::new();
    egui::epaint::tessellator::path::rounded_rectangle(&mut points, rect, radius.into());
    points.dedup_by(|a, b| (*a - *b).length_sq() < 1e-6);
    points
}

/// A faded box's fill and border as one mesh built on shared vertices,
/// no triangle cut: the fill inside `hole` (the padding box's outline),
/// blending into `border` across egui's feathering band, the border out
/// to `outline` (the border box's), feathered there as egui feathers a
/// fill. Cut meshes meet at T-junctions, whose cracks a pixel centre can
/// fall into; this mesh has none. `None` for sides of unequal width (a
/// side without a border): the caller cuts instead.
fn two_tone(
    ctx: &egui::Context,
    outline: &[egui::Pos2],
    hole: Vec<egui::Pos2>,
    borders: [f32; 4],
    fill: Color32,
    border: Color32,
) -> Option<egui::Mesh> {
    let hole = RoundedClip::new(ctx, hole);
    let rim = RoundedClip::new(ctx, outline.to_vec());
    let n = hole.exact.len();
    let uniform = borders.iter().all(|w| *w == borders[0]);
    if !uniform || n < 3 || rim.exact.len() < 3 || hole.folded() || rim.folded() {
        return None;
    }
    // A border one band wide: the band's outer edge and the outline's
    // inner one are one curve, traced by different arcs. The band then
    // meets the outline's feathering directly.
    let thin = borders[0] <= rim.half * 2.0 + 1e-3; // behaviour-ok: float tolerance in points
                                                    // Every outer band vertex on or inside the outline's inner band edge:
                                                    // a border at least as wide as the band on every side. A 1px border
                                                    // puts both on one curve, traced by different arcs: a quarter band
                                                    // of slack takes the chords' sag.
    let inside = |p: egui::Pos2| {
        let m = rim.exact.len();
        (0..m).all(|i| (p - rim.exact[i]).dot(rim.normals[i]) <= -rim.half / 2.0)
        // behaviour-ok: half the half band
    };
    if !hole.outer.iter().all(|p| inside(*p)) {
        return None;
    }
    let mut mesh = egui::Mesh::default();
    let mut add = |points: &[egui::Pos2], color: Color32| -> Vec<u32> {
        points
            .iter()
            .map(|p| {
                mesh.colored_vertex(*p, color);
                mesh.vertices.len() as u32 - 1
            })
            .collect()
    };
    let (fill_in, band_out) = (add(&hole.inner, fill), add(&hole.outer, border));
    let (rim_in, rim_out) = (
        add(&rim.inner, border),
        add(&rim.outer, Color32::TRANSPARENT),
    );
    for k in 1..n - 1 {
        mesh.add_triangle(fill_in[0], fill_in[k], fill_in[k + 1]);
    }
    let quads = |mesh: &mut egui::Mesh, a: &[u32], b: &[u32]| {
        for i in 0..a.len() {
            let j = (i + 1) % a.len();
            mesh.add_triangle(a[i], a[j], b[j]);
            mesh.add_triangle(a[i], b[j], b[i]);
        }
    };
    quads(&mut mesh, &fill_in, &band_out);
    if thin {
        zipper(&mut mesh, &band_out, &rim_out);
    } else {
        zipper(&mut mesh, &band_out, &rim_in);
        quads(&mut mesh, &rim_in, &rim_out);
    }
    Some(mesh)
}

/// Triangles between two convex outlines `a` and `b` (vertex indices into
/// `mesh`), one inside the other around the same centre, walked together
/// by angle so each vertex is shared by its neighbours.
fn zipper(mesh: &mut egui::Mesh, a: &[u32], b: &[u32]) {
    let pos = |i: u32| mesh.vertices[i as usize].pos;
    let centre = a
        .iter()
        .fold(egui::Vec2::ZERO, |c, i| c + pos(*i).to_vec2())
        .to_pos2()
        / a.len() as f32;
    let area: f32 = (0..a.len())
        .map(|i| {
            let (p, q) = (pos(a[i]), pos(a[(i + 1) % a.len()]));
            p.x * q.y - q.x * p.y
        })
        .sum();
    let turn = area.signum();
    let tau = std::f32::consts::TAU;
    let start = (pos(a[0]) - centre).angle() * turn;
    // Each outline from its vertex nearest `start`, angles unwrapped and
    // closed back to that vertex one turn on.
    let walk = |list: &[u32]| -> Vec<(u32, f32)> {
        let rel = |i: u32| ((pos(i) - centre).angle() * turn - start).rem_euclid(tau);
        let first = (0..list.len())
            .min_by(|x, y| rel(list[*x]).total_cmp(&rel(list[*y])))
            .unwrap_or(0);
        let mut out: Vec<(u32, f32)> = Vec::with_capacity(list.len() + 1);
        for k in 0..=list.len() {
            let i = list[(first + k) % list.len()];
            let mut t = rel(i);
            if let Some(&(_, last)) = out.last() {
                while t < last - 1e-4 {
                    t += tau;
                }
            }
            out.push((i, t));
        }
        let closed = out[0].1 + tau;
        if let Some(end) = out.last_mut() {
            end.1 = closed;
        }
        out
    };
    let (a, b) = (walk(a), walk(b));
    let (mut i, mut j) = (0, 0);
    while i + 1 < a.len() || j + 1 < b.len() {
        if j + 1 >= b.len() || (i + 1 < a.len() && a[i + 1].1 <= b[j + 1].1) {
            mesh.add_triangle(a[i].0, b[j].0, a[i + 1].0);
            i += 1;
        } else {
            mesh.add_triangle(a[i].0, b[j].0, b[j + 1].0);
            j += 1;
        }
    }
}

/// All of a pixel inside a [`RoundedClip`]'s outline.
const INSIDE: f32 = 1.0; // behaviour-ok: full coverage

/// A convex outline as an anti-aliased clip, as the browser clips to a
/// part with `overflow: hidden` and rounded corners: inside the outline a
/// mesh keeps its colour, and across the band egui feathers the outline
/// over it fades out exactly as egui fades the outline's own fill.
struct RoundedClip {
    /// The outline as egui paints the box.
    exact: Vec<egui::Pos2>,
    /// Each edge's outward unit normal (edge `i` runs from point `i`).
    normals: Vec<egui::Vec2>,
    /// The outline moved half the feathering width out and in along
    /// egui's miter normals, where egui puts its feathering's vertices.
    outer: Vec<egui::Pos2>,
    inner: Vec<egui::Pos2>,
    /// Half the feathering width in points; 0 without feathering.
    half: f32,
}

impl RoundedClip {
    fn new(ctx: &egui::Context, mut exact: Vec<egui::Pos2>) -> Self {
        let (on, px) = ctx.tessellation_options(|o| (o.feathering, o.feathering_size_in_pixels));
        let half = if on {
            px / ctx.pixels_per_point() / 2.0
        } else {
            0.0
        };
        while exact.len() > 1 && (exact[0] - exact[exact.len() - 1]).length_sq() < 1e-6 {
            exact.pop();
        }
        let n = exact.len();
        let centre = exact.iter().fold(egui::Vec2::ZERO, |c, p| c + p.to_vec2()) / n.max(1) as f32;
        let mut normals: Vec<egui::Vec2> = (0..n)
            .map(|i| (exact[(i + 1) % n] - exact[i]).normalized().rot90())
            .collect();
        // Outward: away from the centre of the convex outline.
        if n >= 3 && normals[0].dot(exact[0].to_vec2() - centre) < 0.0 {
            normals.iter_mut().for_each(|v| *v = -*v);
        }
        // egui's miter join (`PathBuilder::add_line_loop`): the mean of
        // the two edges' normals over its squared length, so a vertex moved
        // along it by `half` lies `half` off both edges.
        let miter = |i: usize| {
            let m = (normals[(i + n - 1) % n] + normals[i]) / 2.0;
            m / m.length_sq()
        };
        let outer = (0..n).map(|i| exact[i] + miter(i) * half).collect();
        let inner = (0..n).map(|i| exact[i] - miter(i) * half).collect();
        Self {
            exact,
            normals,
            outer,
            inner,
            half,
        }
    }

    /// `cover` (white, its alpha a coverage) painted `inside` within the
    /// outline and `outside` beyond it, one colour blending into the other
    /// across the band as egui feathers an edge: a border's inner edge
    /// over its fill, in one mesh with no seam.
    fn two_tone(&self, cover: &egui::Mesh, inside: Color32, outside: Color32) -> egui::Mesh {
        let tint = |mut m: egui::Mesh, share: &dyn Fn(egui::Pos2) -> f32| {
            for v in &mut m.vertices {
                let c = outside.lerp_to_gamma(inside, share(v.pos));
                v.color = c.gamma_multiply_u8(v.color.a());
            }
            m
        };
        // Each cut splits one mesh in two, so the pieces share their cut
        // points ([`split_convex`]).
        if self.half <= 0.0 || self.exact.len() < 3 || self.folded() {
            let (inner, outer) = split_convex(cover, &self.exact);
            let mut out = tint(inner, &|_| INSIDE);
            out.append(tint(outer, &|_| 0.0));
            return out;
        }
        let (kept, outer) = split_convex(cover, &self.outer);
        let (inner, mut band) = split_convex(&kept, &self.inner);
        let mut out = tint(inner, &|_| INSIDE);
        out.append(tint(outer, &|_| 0.0));
        let n = self.exact.len();
        for i in 0..n {
            let j = (i + 1) % n;
            let quad = [self.outer[i], self.outer[j], self.inner[j], self.inner[i]];
            let (piece, rest) = split_convex(&band, &quad);
            out.append(tint(piece, &|p| self.cover(i, p)));
            band = rest;
        }
        out
    }

    /// How much of a pixel at `p` in edge `i`'s band lies inside: 1 half a
    /// band in, 0 half a band out, linear between, as egui feathers.
    fn cover(&self, i: usize, p: egui::Pos2) -> f32 {
        let d = (p - self.exact[i]).dot(self.normals[i]);
        (0.5 - d / (self.half * 2.0)).clamp(0.0, 1.0) // behaviour-ok: full cover half a band inside the edge
    }

    /// An outline thinner than the band turns its inner outline inside
    /// out; it is cut hard, as egui paints without feathering.
    fn folded(&self) -> bool {
        let n = self.exact.len();
        (0..n).any(|i| {
            let j = (i + 1) % n;
            (self.inner[j] - self.inner[i]).dot(self.exact[j] - self.exact[i]) <= 0.0
        })
    }

    /// The part of `mesh` inside the box, faded across the edge's band.
    fn apply(&self, mesh: &egui::Mesh) -> egui::Mesh {
        let n = self.exact.len();
        if self.half <= 0.0 || n < 3 || self.folded() {
            return clip_inside(mesh, &self.exact);
        }
        let kept = clip_inside(mesh, &self.outer);
        let mut out = clip_inside(&kept, &self.inner);
        let band = clip_outside(&kept, &self.inner);
        // egui feathers edge `i` over the quad between its inner and outer
        // offsets, its colour falling linearly with the distance to the
        // edge: cut the band into those quads, and every piece's colour is
        // linear in it, so the vertex colours blend to egui's exactly.
        for i in 0..n {
            let j = (i + 1) % n;
            let quad = [self.outer[i], self.outer[j], self.inner[j], self.inner[i]];
            let mut piece = clip_inside(&band, &quad);
            for v in &mut piece.vertices {
                v.color = v.color.gamma_multiply(self.cover(i, v.pos));
            }
            out.append(piece);
        }
        out
    }

    /// The part of `mesh` outside the box, the complement of
    /// [`RoundedClip::apply`]: faded in across the edge's band as egui
    /// fades the outline's own fill out there.
    fn remove(&self, mesh: &egui::Mesh) -> egui::Mesh {
        let n = self.exact.len();
        if self.half <= 0.0 || n < 3 || self.folded() {
            return clip_outside(mesh, &self.exact);
        }
        let (kept, mut out) = split_convex(mesh, &self.outer);
        let band = clip_outside(&kept, &self.inner);
        for i in 0..n {
            let j = (i + 1) % n;
            let quad = [self.outer[i], self.outer[j], self.inner[j], self.inner[i]];
            let mut piece = clip_inside(&band, &quad);
            for v in &mut piece.vertices {
                v.color = v.color.gamma_multiply(INSIDE - self.cover(i, v.pos));
            }
            out.append(piece);
        }
        out
    }
}

/// The convex `polygon` cut to `rect` (Sutherland-Hodgman against the
/// rect's four sides): their intersection, also convex.
fn clip_to_rect(polygon: &[egui::Pos2], rect: Rect) -> Vec<egui::Pos2> {
    let mut points = polygon.to_vec();
    // (axis, position, keep the side above it): x for left and right,
    // y for top and bottom.
    let sides = [
        (0, rect.left(), true),
        (0, rect.right(), false),
        (1, rect.top(), true),
        (1, rect.bottom(), false),
    ];
    for (i, at, keep_above) in sides {
        let axis = |p: egui::Pos2| if i == 0 { p.x } else { p.y };
        let inside = |p: egui::Pos2| (axis(p) >= at) == keep_above || axis(p) == at;
        let mut next = Vec::with_capacity(points.len() + 2);
        for k in 0..points.len() {
            let (a, b) = (points[k], points[(k + 1) % points.len()]);
            if inside(a) {
                next.push(a);
            }
            if inside(a) != inside(b) {
                let t = (at - axis(a)) / (axis(b) - axis(a));
                next.push(a + (b - a) * t);
            }
        }
        points = next;
        if points.len() < 3 {
            return Vec::new();
        }
    }
    points
}

/// The convex `polygon` cut to the convex `clip` (Sutherland-Hodgman
/// against each of the clip's edges): their intersection, also convex.
fn clip_convex(polygon: &[egui::Pos2], clip: &[egui::Pos2]) -> Vec<egui::Pos2> {
    let n = clip.len();
    let turn: f32 = (0..n)
        .map(|i| {
            let (p, q) = (clip[i], clip[(i + 1) % n]);
            p.x * q.y - q.x * p.y
        })
        .sum::<f32>()
        .signum();
    let mut points = polygon.to_vec();
    for i in 0..n {
        let (a, b) = (clip[i], clip[(i + 1) % n]);
        let side = |p: egui::Pos2| (b - a).x * (p - a).y - (b - a).y * (p - a).x;
        let inside = |p: egui::Pos2| side(p) * turn >= 0.0;
        let mut next = Vec::with_capacity(points.len() + 2);
        for k in 0..points.len() {
            let (p, q) = (points[k], points[(k + 1) % points.len()]);
            if inside(p) {
                next.push(p);
            }
            if inside(p) != inside(q) {
                let t = side(p) / (side(p) - side(q));
                next.push(p + (q - p) * t);
            }
        }
        points = next;
        if points.len() < 3 {
            return Vec::new();
        }
    }
    // A clip edge through a vertex puts that vertex in twice.
    points.dedup_by(|a, b| (*a - *b).length_sq() < 1e-6);
    while points.len() > 1 && (points[0] - points[points.len() - 1]).length_sq() < 1e-6 {
        points.pop();
    }
    if points.len() < 3 {
        return Vec::new();
    }
    points
}

/// The part of `mesh` that lies outside the convex `polygon`. A triangle
/// that straddles an edge is cut along it, its vertex colours and texture
/// coordinates interpolated; one wholly inside is dropped at once.
pub(crate) fn clip_outside(mesh: &egui::Mesh, polygon: &[egui::Pos2]) -> egui::Mesh {
    split_convex(mesh, polygon).1
}

/// The part of `mesh` that lies inside the convex `polygon`, as CSS's
/// `overflow: hidden` cuts a child to its parent's rounded box. A triangle
/// that straddles an edge is cut along it as in [`clip_outside`]; one
/// wholly outside is dropped at once.
pub(crate) fn clip_inside(mesh: &egui::Mesh, polygon: &[egui::Pos2]) -> egui::Mesh {
    split_convex(mesh, polygon).0
}

/// One Sutherland-Hodgman pass for [`clip_outside`] and [`clip_inside`]:
/// each triangle is cut by the polygon's edges in turn; the pieces cut off
/// lie outside, the rest inside. Returns (inside, outside).
///
/// A cut point is computed from the edge's end points in a fixed order,
/// whichever way a triangle or polygon runs along it: two triangles that
/// share an edge, and two polygons that share a side, cut it at the same
/// point, so the pieces meet without a crack a pixel centre could fall in.
fn split_convex(mesh: &egui::Mesh, polygon: &[egui::Pos2]) -> (egui::Mesh, egui::Mesh) {
    use egui::epaint::Vertex;
    let n = polygon.len();
    let mut out = egui::Mesh::with_texture(mesh.texture_id);
    let mut rest = egui::Mesh::with_texture(mesh.texture_id);
    if n < 3 {
        rest.append_ref(mesh);
        return (out, rest);
    }
    let first = |a: egui::Pos2, b: egui::Pos2| (a.x, a.y) <= (b.x, b.y);
    let area: f32 = (0..n)
        .map(|i| {
            let (a, b) = (polygon[i], polygon[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum();
    let orient = area.signum();
    // > 0 inside edge `i`, < 0 outside it.
    let side = |i: usize, p: egui::Pos2| {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        let (a, b, sign) = if first(a, b) {
            (a, b, orient)
        } else {
            (b, a, -orient)
        };
        sign * (b - a).x.mul_add(p.y - a.y, -(b - a).y * (p.x - a.x))
    };
    let lerp = |a: &Vertex, b: &Vertex, da: f32, db: f32| {
        let (a, b, da, db) = if first(a.pos, b.pos) {
            (a, b, da, db)
        } else {
            (b, a, db, da)
        };
        let t = da / (da - db);
        Vertex {
            pos: a.pos + (b.pos - a.pos) * t,
            uv: a.uv + (b.uv - a.uv) * t,
            color: a.color.lerp_to_gamma(b.color, t),
        }
    };
    let emit = |out: &mut egui::Mesh, poly: &[Vertex]| {
        if poly.len() < 3 {
            return;
        }
        let base = out.vertices.len() as u32;
        out.vertices.extend_from_slice(poly);
        for k in 1..poly.len() as u32 - 1 {
            out.add_triangle(base, base + k, base + k + 1);
        }
    };
    // Buffers shared by every triangle: each cut adds at most one vertex.
    let (mut inside, mut next, mut cut_off) = (
        Vec::with_capacity(n + 3),
        Vec::with_capacity(n + 3),
        Vec::with_capacity(n + 3),
    );
    for tri in mesh.indices.as_chunks::<3>().0 {
        let tri = tri.map(|i| mesh.vertices[i as usize]);
        let (mut all_inside, mut outside_one) = (true, false);
        for e in 0..n {
            let d = tri.map(|v| side(e, v.pos));
            if d.iter().all(|&d| d <= 0.0) {
                outside_one = true;
                break;
            }
            all_inside &= d.iter().all(|&d| d >= 0.0);
        }
        if outside_one || all_inside {
            emit(if outside_one { &mut rest } else { &mut out }, &tri);
            continue;
        }
        inside.clear();
        inside.extend_from_slice(&tri);
        for e in 0..n {
            next.clear();
            cut_off.clear();
            for k in 0..inside.len() {
                let (a, b) = (&inside[k], &inside[(k + 1) % inside.len()]);
                let (da, db) = (side(e, a.pos), side(e, b.pos));
                if da >= 0.0 {
                    next.push(*a);
                } else {
                    cut_off.push(*a);
                }
                if (da >= 0.0) != (db >= 0.0) {
                    let v = lerp(a, b, da, db);
                    next.push(v);
                    cut_off.push(v);
                }
            }
            emit(&mut rest, &cut_off);
            std::mem::swap(&mut inside, &mut next);
            if inside.len() < 3 {
                break;
            }
        }
        emit(&mut out, &inside);
    }
    (out, rest)
}

/// Apply `translate-x/y` to a part's rect. `Calc(f, n)` is `calc(f × 100% +
/// n)` of the part's own size.
pub fn shift(rect: Rect, x: Option<Shift>, y: Option<Shift>) -> Rect {
    let d = |s: Option<Shift>, own: f32| match s {
        Some(Shift::Px(v)) => v,
        Some(Shift::Calc(f, v)) => own * f + v,
        None => 0.0,
    };
    rect.translate(egui::vec2(d(x, rect.width()), d(y, rect.height())))
}

/// Which keys activate a focused widget: those of upstream's element
/// (conformance 4.1). egui fakes a click for Space and Enter on any focused
/// clickable widget; [`pressable`] replaces that with these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivationKeys {
    /// A native `<button>`: Enter clicks on press, Space on release, and
    /// the held Space shows `ACTIVE`.
    Button,
    /// Enter and Space both click on press: bits switch and tab trigger.
    EnterAndSpace,
    /// Space clicks on press, Enter does nothing: bits checkbox.
    Space,
    /// Enter clicks on press: an `<a>`, a link look.
    Enter,
    /// No key clicks: a slider thumb, whose keys are its own.
    None,
}

fn modality_id() -> Id {
    Id::new("shadcn_egui::focus_modality")
}

fn space_id() -> Id {
    Id::new("shadcn_egui::space_down_on")
}

/// The crate-wide focus modality: the focused widget as last seen, whether
/// `:focus-visible` holds for it, and what this pass's input was.
#[derive(Clone, Copy, Debug)]
struct Modality {
    focused: Option<Id>,
    keyboard: bool,
    pass: u64,
    /// The last key press (`true`) or pointer button event (`false`) of
    /// `pass`, if any.
    input: Option<bool>,
}

/// The ids [`hit_area`] registered in one pass of one viewport. Only the
/// current pass's are kept, so the set does not grow with every control
/// ever drawn. Each viewport has its own set: an immediate viewport runs
/// its passes inside a pass of its parent, with pass numbers of its own.
#[derive(Clone, Debug, Default)]
struct HitAreas {
    pass: u64,
    ids: egui::IdSet,
}

fn hit_areas_id(ctx: &Context) -> Id {
    Id::new("shadcn_egui::hit_areas").with(ctx.viewport_id())
}

/// A control's hit area, upstream's `after:absolute after:-inset-*`: a
/// widget at `rect` that takes the pointer with `sense`, but no part in
/// the layout of a crate row (its height, a `self-stretch` item's
/// height), as an absolute box takes none in a flex row (conformance
/// 3.3). Allocate the control's box first (with
/// [`layout::allocate_item`](crate::layout::allocate_item)), then take
/// the hit area around it and read clicks, drags and hover from the
/// returned response. An `egui::Ui::interact` on the same rect would
/// make a crate row as high as the hit area.
///
/// ```
/// # egui::__run_test_ui(|ui| {
/// use shadcn_egui::components::shared::hit_area;
/// shadcn_egui::layout::row(ui, 8.0, |ui| {
///     ui.label("Accept");
///     // A 16 px box with `after:-inset-x-3 after:-inset-y-2`.
///     let (rect, base) =
///         shadcn_egui::layout::allocate_item(ui, egui::vec2(16.0, 16.0), egui::Sense::hover());
///     let hit = rect.expand2(egui::vec2(12.0, 8.0));
///     let resp = hit_area(ui, hit, base.id.with("hit"), egui::Sense::click());
///     if resp.clicked() {
///         // toggle the app's value
///     }
/// });
/// # });
/// ```
#[must_use]
pub fn hit_area(ui: &Ui, rect: Rect, id: Id, sense: egui::Sense) -> Response {
    let pass = ui.ctx().cumulative_pass_nr();
    let key = hit_areas_id(ui.ctx());
    ui.data_mut(|d| {
        let areas = d.get_temp_mut_or_default::<HitAreas>(key);
        if areas.pass != pass {
            *areas = HitAreas {
                pass,
                ids: egui::IdSet::default(),
            };
        }
        areas.ids.insert(id);
    });
    ui.interact(rect, id, sense)
}

/// The ids among `ids` that name a widget [`hit_area`] registered in this
/// pass of this viewport. The pass's set is read in place, not cloned:
/// every crate row asks once per pass.
pub(crate) fn hit_areas_among(ctx: &Context, ids: impl IntoIterator<Item = Id>) -> Vec<Id> {
    let pass = ctx.cumulative_pass_nr();
    let key = egui::util::id_type_map::RawKey::new::<HitAreas>(hit_areas_id(ctx));
    ctx.data(|d| {
        match d
            .get_temp_raw(key)
            .and_then(|a| a.downcast_ref::<HitAreas>())
        {
            Some(areas) if areas.pass == pass => ids
                .into_iter()
                .filter(|id| areas.ids.contains(id))
                .collect(),
            _ => Vec::new(),
        }
    })
}

/// Whether `:focus-visible` holds for the focused widget, as the browser
/// tracks it (conformance 2.6), crate-wide. The modality is taken when
/// focus changes: by the pass's last key press or pointer button event,
/// and a focus move with no input in its pass (an overlay giving focus
/// back, a scripted focus) keeps the one before, as the browser keeps it
/// for a script's `focus()`. A key press while focus stays makes the
/// focused widget focus-visible; a pointer press that moves no focus (a
/// drag on a scroll bar, a press on the focused widget) changes nothing.
/// A key pressed with Ctrl, Alt or Command is a shortcut and counts as no
/// key, as in Chrome. It starts as keyboard, as a page that has seen no
/// input shows a ring on a scripted focus.
///
/// [`Theme::apply`] runs it at the start and end of every pass, so a
/// focus change in a pass where no crate widget asks is taken with that
/// pass's input; without the hooks, the first crate call of a pass does
/// the work (conformance 6).
pub(crate) fn focus_modality_keyboard(ctx: &Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    let focused = ctx.memory(|m| m.focused());
    let mut m = ctx
        .data(|d| d.get_temp::<Modality>(modality_id()))
        .unwrap_or(Modality {
            focused,
            keyboard: true,
            pass: u64::MAX,
            input: None,
        });
    if m.pass != pass {
        // The raw events: a widget may have consumed the key already.
        m.input = ctx.input(|i| {
            i.raw.events.iter().rev().find_map(|e| match e {
                egui::Event::Key {
                    pressed: true,
                    modifiers,
                    ..
                } => (!(modifiers.alt || modifiers.ctrl || modifiers.command || modifiers.mac_cmd))
                    .then_some(true),
                egui::Event::PointerButton { .. } => Some(false),
                _ => None,
            })
        });
        m.pass = pass;
        if m.input == Some(true) {
            m.keyboard = true;
        }
    }
    if focused != m.focused {
        m.focused = focused;
        m.keyboard = m.input.unwrap_or(m.keyboard);
    }
    ctx.data_mut(|d| d.insert_temp(modality_id(), m));
    m.keyboard
}

/// Run [`focus_modality_keyboard`] at the start and end of every pass of
/// `ctx`; installed once.
pub(crate) fn install_focus_modality(ctx: &Context) {
    let id = Id::new("shadcn_egui::focus_modality_hooks");
    if ctx.data(|d| d.get_temp::<bool>(id).unwrap_or(false)) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(id, true));
    let track = |ui: &mut Ui| {
        focus_modality_keyboard(ui.ctx());
    };
    ctx.on_begin_pass("shadcn_egui::focus_modality", std::sync::Arc::new(track));
    ctx.on_end_pass("shadcn_egui::focus_modality", std::sync::Arc::new(track));
}

/// Make `resp` act on press and keys as upstream's element: a pointer
/// press focuses it, as the browser focuses a button, a tab trigger or a
/// slider thumb on press, without a ring (conformance 2.6, 4.1); and
/// `clicked()` fires for `keys` only. Call it right after the interaction,
/// before reading `clicked()`. A disabled or non-clicking `resp` is left
/// as it is.
pub fn pressable(resp: &mut Response, keys: ActivationKeys) {
    if !resp.enabled() || !resp.sense.senses_click() {
        return;
    }
    let ctx = resp.ctx.clone();
    // A press and its release in one pass leave no press down, only a click.
    let pressed_on = resp.is_pointer_button_down_on() || resp.clicked_by(PointerButton::Primary);
    if pressed_on && ctx.input(|i| i.pointer.any_pressed()) && !resp.has_focus() {
        resp.request_focus();
    }
    let focused = resp.has_focus();
    let (enter, space_down, space_up, access) = ctx.input(|i| {
        (
            i.key_pressed(Key::Enter),
            i.key_pressed(Key::Space),
            i.key_released(Key::Space),
            i.has_accesskit_action_request(resp.id, egui::accesskit::Action::Click),
        )
    });
    let held = ctx.data(|d| d.get_temp::<Id>(space_id())) == Some(resp.id);
    let mut click = false;
    if focused {
        click = match keys {
            ActivationKeys::Button => enter || (space_up && (held || space_down)),
            ActivationKeys::EnterAndSpace => enter || space_down,
            ActivationKeys::Space => space_down,
            ActivationKeys::Enter => enter,
            ActivationKeys::None => false,
        };
    }
    // The held Space of a `<button>`: `ACTIVE` until it comes up or focus
    // leaves, a click only if it comes up on the focused button.
    if keys == ActivationKeys::Button && focused && space_down && !space_up {
        ctx.data_mut(|d| d.insert_temp(space_id(), resp.id));
    } else if held && (space_up || !focused) {
        ctx.data_mut(|d| d.remove::<Id>(space_id()));
    }
    resp.flags
        .set(egui::response::Flags::FAKE_PRIMARY_CLICKED, click || access);
}

/// A widget's [`State`] for one paint: `context` (variant, icon side, …),
/// what pointer and keyboard do to `resp`, and the theme's `DARK`. `FOCUS`
/// is set while `resp` has focus; `FOCUS_VISIBLE` too where the browser's
/// `:focus-visible` would match: focus moved by keyboard, not by a pointer
/// press (crate-wide modality, conformance 2.6). A held Space on a
/// [`pressable`] button is `ACTIVE`. A disabled widget (a disabled `Ui`, or
/// `DISABLED` in `context` for a widget's own `enabled` flag) gets
/// `DISABLED` and no `HOVER`, `ACTIVE`, `FOCUS` or `FOCUS_VISIBLE`, even if
/// `context` has them: shadcn's `disabled:pointer-events-none`. A
/// composite widget passes the response of its focusable inner part (a
/// select's trigger, a slider's thumb), so focus and press are read where
/// they happen. For a widget that takes typed text use [`text_entry_state`].
pub fn interaction_state(theme: &Theme, resp: &Response, context: State) -> State {
    state_of(theme, resp, context, focus_modality_keyboard(&resp.ctx))
}

/// [`interaction_state`] for a widget that takes typed text (input,
/// textarea): the browser's `:focus-visible` matches it whenever it has
/// focus, after a pointer press too.
pub fn text_entry_state(theme: &Theme, resp: &Response, context: State) -> State {
    state_of(theme, resp, context, true)
}

/// Reports `make_info` for `resp` as [`Response::widget_info`] does, then
/// gives the widget an accessible `role` that egui's `WidgetType` lacks
/// (upstream's `role="alert"`, `alertdialog`, `navigation`, `group`).
/// `widget_info` sets a role from the `WidgetType`, so the role is set
/// after it; the name, value and state stay those of `make_info`.
pub fn widget_info_with_role(
    resp: &Response,
    role: egui::accesskit::Role,
    make_info: impl Fn() -> egui::WidgetInfo,
) {
    resp.widget_info(make_info);
    resp.ctx
        .accesskit_node_builder(resp.id, |node| node.set_role(role));
}

/// Upstream's `aria-invalid`: while `invalid`, the accessibility node of
/// `resp` says its value is invalid, as the browser reports the attribute.
/// `widget_info` sets no invalid state, so the call may come before or
/// after it.
pub(crate) fn report_invalid(resp: &Response, invalid: bool) {
    if invalid {
        resp.ctx.accesskit_node_builder(resp.id, |node| {
            node.set_invalid(egui::accesskit::Invalid::True)
        });
    }
}

/// Upstream's `disabled:cursor-not-allowed` (and `data-disabled:`,
/// `peer-disabled:`): while `disabled`, the pointer over `resp` shows the
/// not-allowed cursor. A disabled `Ui` hovers nothing in egui, so the test
/// is that the pointer is on `resp`, not that `resp` is hovered. A part
/// whose upstream classes also say `disabled:pointer-events-none` lets the
/// pointer through to what lies under it, so the browser never shows its
/// cursor: such a part does not call this.
pub(crate) fn disabled_cursor(resp: &Response, disabled: bool) {
    if disabled && resp.contains_pointer() {
        resp.ctx.set_cursor_icon(egui::CursorIcon::NotAllowed);
    }
}

fn state_of(theme: &Theme, resp: &Response, context: State, keyboard: bool) -> State {
    let live = resp.enabled() && !context.contains(State::DISABLED);
    let pointer_and_focus = State::HOVER | State::ACTIVE | State::FOCUS | State::FOCUS_VISIBLE;
    let context = if live {
        context
    } else {
        context.without(pointer_and_focus)
    };
    let focused = live && resp.has_focus();
    let space_held = resp.ctx.data(|d| d.get_temp::<Id>(space_id())) == Some(resp.id);
    theme.state(
        &resp.ctx,
        context
            .with(State::HOVER, live && resp.hovered())
            .with(
                State::ACTIVE,
                live && (resp.is_pointer_button_down_on() || focused && space_held),
            )
            .with(State::FOCUS, context.contains(State::FOCUS) || focused)
            .with(State::FOCUS_VISIBLE, focused && keyboard)
            .with(State::DISABLED, !live),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Rung;

    #[test]
    fn opacity_multiplies_down_the_part_tree() {
        let mut half = Recipe::default();
        half.opacity.base = Some(0.5);
        let part: &[&Recipe] = &[&half];
        let parent: &[&Recipe] = &[&half];
        assert_eq!(opacity(&[part, parent], State::default()), 0.25);
        assert_eq!(opacity(&[], State::default()), 1.0);
    }

    fn area(mesh: &egui::Mesh) -> f32 {
        mesh.indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let [a, b, c] = [0, 1, 2].map(|k| mesh.vertices[t[k] as usize].pos);
                ((b - a).x * (c - a).y - (b - a).y * (c - a).x).abs() / 2.0
            })
            .sum()
    }

    #[test]
    fn clip_outside_removes_the_polygon() {
        let mut quad = egui::Mesh::default();
        quad.add_colored_rect(
            Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(10.0, 10.0)),
            Color32::BLACK,
        );
        let hole = rounded_polygon(
            Rect::from_min_size(egui::pos2(3.0, 3.0), egui::vec2(4.0, 4.0)),
            CornerRadius::ZERO,
        );
        assert!((area(&clip_outside(&quad, &hole)) - 84.0).abs() < 1e-3);
        // A polygon that covers the mesh leaves nothing; one beside it all.
        let all = rounded_polygon(
            Rect::from_min_size(egui::pos2(-1.0, -1.0), egui::vec2(12.0, 12.0)),
            CornerRadius::ZERO,
        );
        assert_eq!(area(&clip_outside(&quad, &all)), 0.0);
        let beside = rounded_polygon(
            Rect::from_min_size(egui::pos2(20.0, 0.0), egui::vec2(4.0, 4.0)),
            CornerRadius::ZERO,
        );
        assert!((area(&clip_outside(&quad, &beside)) - 100.0).abs() < 1e-3);
    }

    #[test]
    fn clip_inside_keeps_the_polygon() {
        let mut quad = egui::Mesh::default();
        quad.add_colored_rect(
            Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(10.0, 10.0)),
            Color32::BLACK,
        );
        let window = rounded_polygon(
            Rect::from_min_size(egui::pos2(3.0, 3.0), egui::vec2(4.0, 4.0)),
            CornerRadius::ZERO,
        );
        assert!((area(&clip_inside(&quad, &window)) - 16.0).abs() < 1e-3);
        // Inside and outside part the mesh between them.
        let round = rounded_polygon(
            Rect::from_min_size(egui::pos2(-5.0, 2.0), egui::vec2(10.0, 6.0)),
            CornerRadius::same(3),
        );
        let total = area(&clip_inside(&quad, &round)) + area(&clip_outside(&quad, &round));
        assert!((total - 100.0).abs() < 1e-3, "{total}");
        let all = rounded_polygon(
            Rect::from_min_size(egui::pos2(-1.0, -1.0), egui::vec2(12.0, 12.0)),
            CornerRadius::ZERO,
        );
        assert!((area(&clip_inside(&quad, &all)) - 100.0).abs() < 1e-3);
        let beside = rounded_polygon(
            Rect::from_min_size(egui::pos2(20.0, 0.0), egui::vec2(4.0, 4.0)),
            CornerRadius::ZERO,
        );
        assert_eq!(area(&clip_inside(&quad, &beside)), 0.0);
    }

    /// A box narrower than the outer box's rounding keeps only the part
    /// inside the outer round cap, as `overflow: hidden` cuts it.
    #[test]
    fn paint_inside_follows_the_outer_rounding() {
        let ctx = egui::Context::default();
        let theme = Theme::vega().in_mode(egui::Theme::Light);
        let mut full = Recipe::default();
        full.radius.base = Some(Radius::Rung(Rung::Full));
        let outer = RecipeBox::resolve(&ctx, &theme, &[&full], State::NONE);
        let mut fill = Recipe::default();
        fill.bg.base = Some(crate::style::Paint::Role(crate::style::Role::Primary, 1.0));
        let track = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 12.0));
        let sliver = Rect::from_min_max(track.min, egui::pos2(2.0, 12.0));
        let centre = egui::pos2(6.0, 6.0);
        // A plain fill: the cap's outline cut to the sliver, one polygon.
        let plain = RecipeBox::resolve(&ctx, &theme, &[&fill], State::NONE);
        let cut_plain = plain.shapes_inside(&ctx, sliver, &outer, track, 1.0);
        let [Shape::Path(path)] = cut_plain.as_slice() else {
            panic!("a plain fill is one convex polygon")
        };
        for p in &path.points {
            assert!((*p - centre).length() <= 6.0 + 1e-3, "{p:?}");
            assert!(p.x <= 2.0 + 1e-3, "{p:?}");
        }
        // A box with a border goes through the mesh clip.
        fill.border_width.base = Some(1.0);
        let inner = RecipeBox::resolve(&ctx, &theme, &[&fill], State::NONE);
        let mut cut = egui::Mesh::default();
        for shape in inner.shapes_inside(&ctx, sliver, &outer, track, 1.0) {
            let egui::Shape::Mesh(m) = shape else {
                panic!("a cut box is meshes")
            };
            cut.append_ref(&m);
        }
        // Inside the cap's circle (centre 6, 6; radius 6) and the sliver,
        // plus the half pixel over which the clip fades out: faded there.
        // egui's miter puts the fade's outer vertices half a pixel off each
        // chord of the circle, a hair past 6.5 from its centre.
        for v in &cut.vertices {
            let d = (v.pos - centre).length();
            assert!(d <= 6.55, "{:?}", v.pos);
            if d > 6.0 + 1e-3 {
                assert!(v.color.a() < 128, "{:?} {:?}", v.pos, v.color);
            }
            // egui's anti-aliasing fringe reaches half a pixel further.
            assert!(v.pos.x <= 2.5 + 1e-3, "{:?}", v.pos);
        }
    }

    fn rounded(base: Option<Rung>, left: Option<Rung>, left_rank: u128) -> RecipeBox {
        let mut r = Recipe::default();
        r.radius.base = base.map(Radius::Rung);
        if let Some(l) = left {
            r.radius_left.rules.push(crate::style::Rule {
                when: crate::style::When::ALWAYS,
                rank: left_rank,
                value: Radius::Rung(l),
            });
        }
        let theme = Theme::vega().in_mode(egui::Theme::Light);
        RecipeBox::resolve(&egui::Context::default(), &theme, &[&r], State::NONE)
    }

    #[test]
    fn corner_radii_resolve_per_side_and_clamp_as_css() {
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(40.0, 20.0));
        // `rounded-full` is half the short side.
        let full = rounded(Some(Rung::Full), None, 0).radius_on(rect);
        assert_eq!(full, CornerRadius::same(10));
        // `rounded-l-none` beside `rounded-md` squares the left corners only.
        let md = Theme::vega()
            .in_mode(egui::Theme::Light)
            .rung(Rung::Md)
            .round() as u8;
        let left_none = rounded(Some(Rung::Md), Some(Rung::None), 0).radius_on(rect);
        assert_eq!(
            left_none,
            CornerRadius {
                nw: 0,
                ne: md,
                sw: 0,
                se: md
            }
        );
        // A side ranked below the shorthand loses to it.
        let mut base = Recipe::default();
        base.radius.rules.push(crate::style::Rule {
            when: crate::style::When::ALWAYS,
            rank: 2,
            value: Radius::Rung(Rung::Md),
        });
        base.radius_left.rules.push(crate::style::Rule {
            when: crate::style::When::ALWAYS,
            rank: 1,
            value: Radius::Rung(Rung::None),
        });
        let b = RecipeBox::resolve(
            &egui::Context::default(),
            &Theme::vega().in_mode(egui::Theme::Light),
            &[&base],
            State::NONE,
        );
        assert_eq!(b.radius_on(rect), CornerRadius::same(md));
    }

    #[test]
    fn unequal_borders_paint_the_border_box_minus_the_padding_box() {
        let mut r = Recipe::default();
        r.border_width.base = Some(1.0);
        r.border_left_width.base = Some(0.0);
        let b = RecipeBox::resolve(
            &egui::Context::default(),
            &Theme::vega().in_mode(egui::Theme::Light),
            &[&r],
            State::NONE,
        );
        assert_eq!(b.border_widths(), [1.0, 1.0, 1.0, 0.0]);
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(20.0, 10.0));
        let shapes = b.shapes(&ctx, rect);
        let meshes: Vec<_> = shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Mesh(m) => Some(m),
                _ => None,
            })
            .collect();
        assert_eq!(meshes.len(), 1);
        let padding_box = Rect::from_min_max(egui::pos2(0.0, 1.0), egui::pos2(19.0, 9.0));
        for t in meshes[0].indices.as_chunks::<3>().0 {
            let c = t
                .iter()
                .map(|&i| meshes[0].vertices[i as usize].pos.to_vec2())
                .fold(egui::Vec2::ZERO, |a, b| a + b)
                / 3.0;
            assert!(!padding_box.shrink(0.01).contains(c.to_pos2()), "{c:?}");
        }
        // Top 20 + bottom 20 + right 8, plus egui's feathering band.
        let border = area(meshes[0]);
        assert!((48.0..48.0 + 60.0).contains(&border), "{border}");
    }

    /// A faded box with sides of unequal width, as the button group's
    /// `NOT_FIRST` rule makes it: no left border.
    fn one_side_off() -> RecipeBox {
        let mut r = Recipe::default();
        r.border_width.base = Some(1.0);
        r.border_left_width.base = Some(0.0);
        r.bg.base = Some(Paint::Role(crate::style::Role::Background, 1.0));
        RecipeBox::resolve(
            &egui::Context::default(),
            &Theme::vega().in_mode(egui::Theme::Light),
            &[&r],
            State::NONE,
        )
    }

    /// `(builds, cached meshes)` of the context's fallback cache.
    fn cut_cache(ctx: &egui::Context) -> (u64, usize) {
        ctx.data_mut(|d| {
            let c = d.get_temp_mut_or_default::<CutCache>(cut_cache_id());
            (c.builds, c.current.len() + c.previous.len())
        })
    }

    /// A still frame cuts the fallback mesh of a faded box with unequal
    /// sides once: later frames take it from the cache.
    #[test]
    fn a_still_faded_box_with_unequal_sides_is_cut_once() {
        let b = one_side_off();
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(40.0, 20.0));
        let mut vertices = 0;
        for _ in 0..4 {
            let _ = crate::test_ui::run(&ctx, egui::RawInput::default(), |_| {
                for shape in b.shapes_at(&ctx, rect, 0.5) {
                    if let Shape::Mesh(m) = shape {
                        vertices = m.vertices.len();
                    }
                }
            });
        }
        assert!(vertices > 0, "the box is one mesh");
        assert_eq!(cut_cache(&ctx).0, 1, "builds over 4 frames");
    }

    /// A faded box with unequal sides that moves every frame keeps the
    /// cache at the meshes of two passes.
    #[test]
    fn the_cut_cache_stays_bounded_while_a_box_moves() {
        let b = one_side_off();
        let ctx = egui::Context::default();
        for frame in 0..20 {
            let _ = crate::test_ui::run(&ctx, egui::RawInput::default(), |_| {
                let at = egui::pos2(10.0 + frame as f32, 10.0);
                let rect = Rect::from_min_size(at, egui::vec2(40.0, 20.0));
                let _ = b.shapes_at(&ctx, rect, 0.5);
            });
        }
        let (builds, size) = cut_cache(&ctx);
        assert_eq!(builds, 20);
        assert!(size <= 2, "cached meshes {size}");
    }
}
