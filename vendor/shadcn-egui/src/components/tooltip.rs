//! shadcn Tooltip, ported from shadcn-svelte `ui/tooltip/tooltip-content.svelte`
//! at `tools/stylegen/PIN`. Every value comes from the style's
//! `cn-tooltip-content` and `cn-tooltip-arrow` recipes. The arrow
//! classes `rotate-45` and `translate-y-[calc(-50%-2px)]` belong to the style
//! and stylegen ignores them: the arrow is painted here as a diamond whose
//! centre lies on the content edge, below or above the content, centred on the
//! trigger.

use crate::components::context;
use crate::components::shared::{content_ui, opacity, RecipeBox};
use crate::components::text::{content_scope, TextLook};
use crate::style::app::{default_variant, Resolved};
use crate::style::{pick, pick_len};
use crate::Theme;
use egui::{
    pos2, Color32, Context, InnerResponse, Pos2, RectAlign, Response, Shape, TextWrapMode, Ui,
    UiBuilder,
};
use std::f32::consts::{FRAC_1_SQRT_2, FRAC_PI_2};

/// Corner arc segments of the rounded arrow.
const ARC_STEPS: usize = 4;

default_variant!(
    /// The tooltip's variant: upstream has none, so only app variants
    /// ([`TooltipVariant::app`]) differ from `Default`.
    TooltipVariant,
    /// The tooltip's recipes an app variant's edit changes.
    TooltipRecipes {
        /// `data-slot="tooltip-content"`: the panel.
        tooltip_content,
        /// `cn-tooltip-arrow`: the arrow (no `data-slot` of its own).
        tooltip_arrow,
    }
);

/// The context a tooltip gives its content: its variant.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct TooltipContext {
    variant: TooltipVariant,
}

/// The recipes of the tooltip around `ui`; outside a tooltip, the
/// default's.
fn tooltip_recipes(ui: &Ui, t: &Theme) -> Resolved<TooltipVariant> {
    let cx = context::find::<TooltipContext>(ui).unwrap_or_default();
    t.resolve(cx.variant)
}

/// A shadcn tooltip on a trigger:
/// `Tooltip::new(&resp).show(ui, |ui| ui.label("Mute"))`. It opens as
/// egui's tooltip does, while the pointer rests on an enabled trigger.
pub struct Tooltip<'a> {
    trigger: &'a Response,
    variant: TooltipVariant,
}

impl<'a> Tooltip<'a> {
    /// A tooltip shown while the pointer rests on `trigger`.
    pub fn new(trigger: &'a Response) -> Self {
        Self {
            trigger,
            variant: TooltipVariant::Default,
        }
    }
    /// An app variant ([`TooltipVariant::app`]).
    pub fn variant(mut self, v: impl Into<TooltipVariant>) -> Self {
        self.variant = v.into();
        self
    }

    /// Show `content` in the tooltip while it is open; `None` while closed.
    /// Text inside takes the tooltip's size and colour.
    pub fn show<R>(
        self,
        ui: &mut Ui,
        content: impl FnOnce(&mut Ui) -> R,
    ) -> Option<InnerResponse<R>> {
        self.show_on(ui.ctx(), content)
    }

    fn show_on<R>(
        self,
        ctx: &Context,
        content: impl FnOnce(&mut Ui) -> R,
    ) -> Option<InnerResponse<R>> {
        let resp = self.trigger;
        let t = Theme::current(ctx);
        let v = t.resolve(self.variant);
        let g = t.state(ctx, v.bits());
        let content_l = [v.part(&t.style, |_, s| &s.tooltip_content, |r| &r.tooltip_content)];
        let arrow = [v.part(&t.style, |_, s| &s.tooltip_arrow, |r| &r.tooltip_arrow)];
        let px =
            |f: fn(&crate::style::Recipe) -> &crate::style::Prop<f32>| pick_len(&content_l, f, g);
        let (pad_t, pad_r, pad_b, pad_l) = (
            px(|r| &r.pad_top),
            px(|r| &r.pad_right),
            px(|r| &r.pad_bottom),
            px(|r| &r.pad_left),
        );
        let radius =
            pick(&content_l, |r| &r.radius, g).map_or(0.0, |r| t.radius_px(r, f32::INFINITY));
        // The frame and the arrow paint outside the content's `Ui`: fade their
        // colours here. The arrow sits in the content.
        let alpha = opacity(&[&content_l[..]], g);
        let arrow_alpha = opacity(&[&arrow[..], &content_l[..]], g);
        let panel = RecipeBox::resolve(ctx, &t, &content_l, g).faded(alpha);
        let arrow_fill = pick(&arrow, |r| &r.bg, g).map(|p| t.paint(p).gamma_multiply(arrow_alpha));
        let mut tip = egui::Tooltip::for_enabled(resp);
        let max_w = pick(&content_l, |r| &r.max_width, g).unwrap_or(f32::INFINITY);
        // The arrow's tip, not the content, touches the trigger; `side = "top"` is the base default.
        let size = pick_len(&arrow, |r| &r.width, g);
        let reach = size * FRAC_1_SQRT_2;
        let frame = panel.frame([pad_t, pad_r, pad_b, pad_l]);
        tip.popup = tip.popup.align(RectAlign::TOP).gap(reach).frame(frame);
        let parts = TooltipContext {
            variant: self.variant,
        };
        let shown = tip.show(|ui| {
            let slot = RecipeBox::reserve(ui);
            ui.set_max_width(max_w - pad_l - pad_r);
            let inner = ui.scope_builder(context::push(UiBuilder::new(), parts), |ui| {
                // The content's `text-*` and text colour reach what it holds.
                let look = TextLook::resolve(ui, &content_l, g);
                content_scope(ui, &look, |ui| {
                    content_ui(
                        ui,
                        ui.id().with("shadcn_egui::tooltip::content"),
                        &[&content_l[..]],
                        g,
                        content,
                    )
                })
                .inner
            });
            (inner.inner, slot)
        })?;
        let (inner, slot) = shown.inner;
        panel.paint_reserved(ctx, shown.response.layer_id, slot, shown.response.rect);
        if let Some(fill) = arrow_fill {
            let r = pick(&arrow, |r| &r.radius, g).map_or(0.0, |r| t.radius_px(r, size / 2.0));
            let area = shown.response.rect;
            // The content lies above or below the trigger; the arrow sits on the edge facing it.
            let above = area.center().y < resp.rect.center().y;
            let y = if above { area.max.y } else { area.min.y };
            // Centred on the trigger, kept clear of the content's rounded corners.
            let (lo, hi) = (area.min.x + radius + reach, area.max.x - radius - reach);
            let x = if lo < hi {
                resp.rect.center().x.clamp(lo, hi)
            } else {
                area.center().x
            };
            let painter = ctx.layer_painter(shown.response.layer_id);
            painter.add(arrow_shape(pos2(x, y), size, r, fill));
        }
        Some(InnerResponse::new(inner, shown.response))
    }
}

/// Text in the tooltip around `ui`, as `tooltip-content` sets it.
fn content_text(ui: &mut Ui, text: &str) -> Response {
    let t = Theme::current(ui.ctx());
    let v = tooltip_recipes(ui, &t);
    let g = t.state(ui.ctx(), v.bits());
    let content = [v.part(&t.style, |_, s| &s.tooltip_content, |r| &r.tooltip_content)];
    TextLook::resolve(ui, &content, g).label(ui, text, TextWrapMode::Wrap)
}

/// Attach a shadcn tooltip with `text` to `resp`:
/// `tooltip(ui.add(..), "Mute")`. One [`Tooltip`] call, as egui's
/// `on_hover_text`.
pub fn tooltip(resp: Response, text: &str) -> Response {
    let ctx = resp.ctx.clone();
    Tooltip::new(&resp).show_on(&ctx, |ui| content_text(ui, text));
    resp
}

/// A square of side `size` with corner radius `radius`, turned 45 degrees about `centre`.
fn arrow_shape(centre: Pos2, size: f32, radius: f32, fill: Color32) -> Shape {
    let inner = size / 2.0 - radius;
    let mut points = Vec::with_capacity(4 * (ARC_STEPS + 1));
    // The corner arcs' centres, one per quadrant in turning order.
    for (k, (sx, sy)) in [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)] // behaviour-ok: quadrant signs, not lengths
        .into_iter()
        .enumerate()
    {
        for i in 0..=ARC_STEPS {
            let a = (k as f32 + i as f32 / ARC_STEPS as f32) * FRAC_PI_2;
            let (x, y) = (sx * inner + radius * a.cos(), sy * inner + radius * a.sin());
            points.push(centre + egui::vec2((x - y) * FRAC_1_SQRT_2, (x + y) * FRAC_1_SQRT_2));
        }
    }
    Shape::convex_polygon(points, fill, egui::Stroke::NONE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{AppVariant, Paint, Role, State};
    use egui::Rect;

    /// An app variant's edit gets `TooltipRecipes` and changes only what it
    /// names.
    #[test]
    fn edit_compiles_and_changes_its_part() {
        const LOUD: AppVariant<TooltipVariant> = TooltipVariant::app("loud");
        let t = Theme::vega().variant(
            LOUD,
            TooltipVariant::Default,
            |r: &mut TooltipRecipes, _| {
                r.tooltip_content
                    .bg
                    .set(Paint::Role(Role::Destructive, 1.0))
            },
        );
        let v = t.resolve(TooltipVariant::from(LOUD));
        let content = v.part(&t.style, |_, s| &s.tooltip_content, |r| &r.tooltip_content);
        assert_eq!(
            content.bg.get(State::NONE),
            Some(Paint::Role(Role::Destructive, 1.0))
        );
        let arrow = v.part(&t.style, |_, s| &s.tooltip_arrow, |r| &r.tooltip_arrow);
        assert_eq!(arrow, &*t.style.tooltip_arrow);
    }

    #[test]
    fn arrow_is_a_diamond_with_rounded_tips() {
        let centre = pos2(50.0, 50.0);
        let Shape::Path(p) = arrow_shape(centre, 10.0, 2.0, Color32::PLACEHOLDER) else {
            panic!("path")
        };
        let bounds = Rect::from_points(&p.points);
        // A sharp 10px square turned 45 degrees reaches 7.07px; the rounded tips stop short of that.
        let reach = 10.0 * FRAC_1_SQRT_2;
        assert!(bounds.center().distance(centre) < 0.01);
        assert!(
            bounds.width() / 2.0 < reach && bounds.width() / 2.0 > reach - 1.0,
            "{bounds:?}"
        );
        assert!((bounds.width() - bounds.height()).abs() < 0.01);
    }
}
