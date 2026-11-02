//! shadcn Progress, ported from shadcn-svelte `ui/progress/progress.svelte`
//! at `tools/stylegen/PIN`, which wraps the bits-ui progress root. Every
//! value comes from the style's `cn-progress` (the track) and
//! `cn-progress-indicator` recipes. The indicator is as wide as the track
//! and moved left by `translateX(-{100 - 100 * value / max}%)`; the track's
//! `overflow-x-hidden` cuts it to the track's rounded box.

use crate::components::motion::transition;
use crate::components::shared::{part_painter, RecipeBox};
use crate::layout::{self, capped_width};
use crate::style::app::default_variant;
use crate::style::{pick_len, Transition};
use crate::Theme;
use egui::{Response, Sense, Ui, Vec2, Widget, WidgetInfo, WidgetType};

default_variant!(
    /// The progress bar's variant: upstream has none, so only app variants
    /// ([`ProgressVariant::app`]) differ from `Default`.
    ProgressVariant,
    /// The progress bar's recipes an app variant's edit changes.
    ProgressRecipes {
        /// `data-slot="progress"`: the track.
        progress,
        /// `data-slot="progress-indicator"`: the filled part.
        progress_indicator,
    }
);

/// A progress bar: `ui.add(Progress::new(0.4))`, or
/// `Progress::from_value(value, max)` with upstream's `value` and `max`.
///
/// It fills the width of its `Ui` (`w-full`) at the style's height and
/// shows no text; a label goes next to it. A new value slides to its place
/// as upstream's `transition-all` does, and jumps under reduced motion
/// (`style.animation_time == 0`).
///
/// The indicator sits where upstream's formula puts it, also outside
/// `0..=max`: a negative value hides it, and a value past `max`, a `max`
/// of 0 or a NaN value fills the track, as the browser drops a `transform`
/// it cannot read (`translateX(--50%)`, `translateX(-NaN%)`). Upstream's
/// indeterminate state (`value={null}`) draws as 0 and is not offered.
///
/// ```
/// use shadcn_egui::components::progress::{progress, Progress};
///
/// fn upload(ui: &mut egui::Ui, sent: f32, size: f32) {
///     ui.add(Progress::from_value(sent, size).desired_width(240.0));
///     progress(ui, 0.66);
/// }
/// ```
#[derive(Clone, Copy, Debug)]
#[must_use = "You should put this widget in a ui with `ui.add(widget);`"]
pub struct Progress {
    value: f32,
    max: f32,
    desired_width: f32,
    variant: ProgressVariant,
}

impl Progress {
    /// A bar filled to `fraction` (`0.0..=1.0`), as egui's
    /// `ProgressBar::new`: upstream's `value` over a `max` of 1.
    #[inline]
    pub fn new(fraction: f32) -> Self {
        Self::from_value(fraction, 1.0) // behaviour-ok: a fraction is a value over a max of 1
    }

    /// Upstream's `value` and `max` (upstream's default `max` is 100).
    #[inline]
    pub fn from_value(value: f32, max: f32) -> Self {
        Self {
            value,
            max,
            desired_width: f32::INFINITY,
            variant: ProgressVariant::Default,
        }
    }

    /// The width in points, capped by the room the `Ui` has, as egui's
    /// `TextEdit::desired_width`. `f32::INFINITY` (the default) fills it.
    #[inline]
    pub fn desired_width(mut self, width: f32) -> Self {
        self.desired_width = width;
        self
    }

    /// An app variant ([`ProgressVariant::app`]).
    #[inline]
    pub fn variant(mut self, v: impl Into<ProgressVariant>) -> Self {
        self.variant = v.into();
        self
    }
}

impl Widget for Progress {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = Theme::current(ui.ctx());
        let v = t.resolve(self.variant);
        let track = [v.part(&t.style, |_, s| &s.progress, |r| &r.progress)];
        let indicator = [v.part(
            &t.style,
            |_, s| &s.progress_indicator,
            |r| &r.progress_indicator,
        )];
        let st = t.state(ui.ctx(), v.bits());
        // `w-full`; a bar has no content width (conformance 3.1). The
        // indicator's `size-full` gives the track no height of its own.
        let height = pick_len(&track, |r| &r.height, st);
        let width = capped_width(ui, self.desired_width, 0.0);
        let (rect, resp) = layout::allocate_item(ui, Vec2::new(width, height), Sense::hover());

        // `translateX(-{100 - 100 * value / max}%)` as a share of the
        // indicator's own width. A negative offset prints as `--50%` and a
        // non-finite one as `-NaN%` or `--Infinity%`: the browser drops a
        // transform it cannot read, which leaves the indicator in place.
        let offset = 1.0 - self.value / self.max; // behaviour-ok: upstream's 100% less the filled share
        let offset = if offset.is_finite() && offset >= 0.0 {
            offset
        } else {
            0.0
        };
        let offset = transition(
            ui.ctx(),
            resp.id.with("offset"),
            offset,
            &indicator,
            Transition::TRANSFORM,
            st,
        );

        if ui.is_rect_visible(rect) {
            let track_box = RecipeBox::resolve(ui.ctx(), &t, &track, st);
            track_box.paint(&part_painter(ui, resp.id, &[&track[..]], st), rect);
            let moved = rect.translate(Vec2::new(-offset * rect.width(), 0.0));
            if moved.intersects(rect) {
                let id = resp.id.with("indicator");
                // The track's `overflow-x-hidden` cuts the indicator to
                // the track's rounded box.
                RecipeBox::resolve(ui.ctx(), &t, &indicator, st)
                    .transitioned(ui.ctx(), id, &indicator, st)
                    .paint_inside(
                        &part_painter(ui, id, &[&indicator[..], &track[..]], st),
                        moved,
                        &track_box,
                        rect,
                    );
            }
        }

        let (value, max) = (self.value, self.max);
        resp.widget_info(|| {
            let mut info = WidgetInfo::new(WidgetType::ProgressIndicator);
            info.enabled = ui.is_enabled();
            info.value = value.is_finite().then_some(f64::from(value));
            info
        });
        // bits-ui's `aria-valuemin` (its `min` default, which shadcn does
        // not set) and `aria-valuemax`.
        ui.ctx().accesskit_node_builder(resp.id, |node| {
            node.set_min_numeric_value(0.0);
            if max.is_finite() {
                node.set_max_numeric_value(f64::from(max));
            }
        });
        resp
    }
}

/// A bar filled to `fraction` (`0.0..=1.0`): shorthand for
/// `ui.add(Progress::new(fraction))`.
pub fn progress(ui: &mut Ui, fraction: f32) -> Response {
    ui.add(Progress::new(fraction))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{AppVariant, Paint, Role, State};

    /// An app variant's edit gets `ProgressRecipes` and changes only what
    /// it names.
    #[test]
    fn edit_compiles_and_changes_its_part() {
        const DANGER: AppVariant<ProgressVariant> = ProgressVariant::app("danger");
        let t = Theme::vega().variant(
            DANGER,
            ProgressVariant::Default,
            |r: &mut ProgressRecipes, _| {
                r.progress_indicator
                    .recolor(Role::Primary, Role::Destructive)
            },
        );
        let v = t.resolve(ProgressVariant::from(DANGER));
        let ind = v.part(
            &t.style,
            |_, s| &s.progress_indicator,
            |r| &r.progress_indicator,
        );
        assert_eq!(
            ind.bg.get(State::NONE),
            Some(Paint::Role(Role::Destructive, 1.0))
        );
        let track = v.part(&t.style, |_, s| &s.progress, |r| &r.progress);
        assert_eq!(track, &*t.style.progress);
    }
}
