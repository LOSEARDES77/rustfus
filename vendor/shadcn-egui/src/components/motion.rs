//! Upstream's CSS transitions (rules 7, D-88): a part's value that changes
//! moves to its new value over the recipe's `duration-*`, along its
//! `ease-*` curve, when its `transition-*` names the property; otherwise it
//! jumps. `style.animation_time == 0` counts as reduced motion: every
//! transition jumps at once. [`transition`] moves a length or a fraction,
//! [`transition_color`] a colour, mixed as the browser mixes it
//! ([`mix_oklab`](crate::color::mix_oklab)).

use crate::box_shadow::Layers;
use crate::color::Tone;
use crate::style::{pick, Easing, Recipe, State, Transition};
use crate::theme::theme_stamp;
use egui::{Color32, Context, Id};

/// Tailwind 4's `--default-transition-duration`, in seconds: the duration
/// of a transition whose part sets no `duration-*`.
const DEFAULT_DURATION: f32 = 0.15; // behaviour-ok: Tailwind --default-transition-duration

impl Easing {
    /// The eased progress at `t` in `0..=1`, as CSS computes the timing
    /// function: 0 at the start, 1 at the end.
    pub fn at(self, t: f32) -> f32 {
        let (x1, y1, x2, y2) = match self {
            Easing::Linear => return t.clamp(0.0, 1.0), // behaviour-ok: the unit interval
            Easing::Ease => (0.25, 0.1, 0.25, 1.0), // behaviour-ok: CSS initial timing function ease
            Easing::In => (0.4, 0.0, 1.0, 1.0),     // behaviour-ok: Tailwind --ease-in
            Easing::Out => (0.0, 0.0, 0.2, 1.0),    // behaviour-ok: Tailwind --ease-out
            Easing::InOut => (0.4, 0.0, 0.2, 1.0),  // behaviour-ok: Tailwind --ease-in-out
        };
        cubic_bezier(x1, y1, x2, y2, t)
    }
}

/// CSS `cubic-bezier(x1, y1, x2, y2)` at `t`: the curve's y where its x is
/// `t`. x rises monotonically for x1, x2 in `0..=1`, so bisection finds
/// the curve parameter.
fn cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64, t: f32) -> f32 {
    let at_end = t <= 0.0 || t >= 1.0; // behaviour-ok: the unit interval
    if at_end {
        // The curve's fixed ends, exactly.
        return t.clamp(0.0, 1.0); // behaviour-ok: the unit interval
    }
    // One coordinate of the Bernstein form with ends 0 and 1.
    let at = |a: f64, b: f64, s: f64| {
        let u = 1.0 - s; // behaviour-ok: Bernstein polynomial
        3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s // behaviour-ok: Bernstein polynomial
    };
    let t = f64::from(t);
    let (mut lo, mut hi) = (0.0, 1.0); // behaviour-ok: the unit interval
    for _ in 0..40 {
        // behaviour-ok: bisection steps, below f32 precision
        let mid = (lo + hi) / 2.0;
        if at(x1, x2, mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    at(y1, y2, (lo + hi) / 2.0) as f32
}

/// A value a transition moves: the value at eased progress `t` between
/// `a` and `b`.
trait Tween: Copy + PartialEq + Send + Sync + 'static {
    fn tween(a: Self, b: Self, t: f32) -> Self;
    /// Whether a transition can move from or to this value.
    fn finite(self) -> bool {
        true
    }
}

impl Tween for f32 {
    fn tween(a: f32, b: f32, t: f32) -> f32 {
        a + (b - a) * t
    }
    fn finite(self) -> bool {
        self.is_finite()
    }
}

impl Tween for Tone {
    fn tween(a: Tone, b: Tone, t: f32) -> Tone {
        Tone::mix_oklab(a, b, t)
    }
}

impl Tween for Layers {
    fn tween(a: Layers, b: Layers, t: f32) -> Layers {
        Layers::mix(a, b, t)
    }
}

/// One running (or finished) transition, kept per id in the context's
/// temporary data.
#[derive(Clone, Copy, Debug)]
struct Run<T> {
    from: T,
    to: T,
    start: f64,
    duration: f32,
    easing: Easing,
    /// CSS's reversing-adjusted start value and reversing shortening
    /// factor: a transition back to where the last one came from takes
    /// only as long as the way already covered.
    reverse_to: T,
    shortening: f32,
    /// The pass and time the widget last asked for this value: a change
    /// found after skipped passes happened while the widget was not shown.
    seen_pass: u64,
    seen_time: f64,
    /// The theme and mode the value was seen under ([`theme_stamp`]).
    stamp: u64,
}

impl<T: Tween> Run<T> {
    fn settled(value: T) -> Run<T> {
        Run {
            from: value,
            to: value,
            start: 0.0,
            duration: 0.0,
            easing: Easing::Linear,
            reverse_to: value,
            shortening: 1.0, // behaviour-ok: CSS's initial shortening factor
            seen_pass: 0,
            seen_time: 0.0,
            stamp: 0,
        }
    }

    /// The value and eased progress at `now`, and whether it still runs.
    fn at(&self, now: f64) -> (T, f32, bool) {
        let elapsed = (now - self.start) as f32;
        if self.duration <= 0.0 || elapsed >= self.duration {
            return (self.to, 1.0, false); // behaviour-ok: the finished progress
        }
        let eased = self.easing.at(elapsed.max(0.0) / self.duration);
        (T::tween(self.from, self.to, eased), eased, true)
    }
}

/// The value to paint this pass for a property of a part that changes to
/// `target`, timed by the part's recipe stack `layers` under state `s`:
/// when the picked `transition` names `property` (any of its properties,
/// when it is a set), the value moves from where
/// it is to `target` over the picked `duration` (Tailwind's 150 ms without
/// one) along the picked `easing` ([`Easing::InOut`] without one), as a CSS
/// transition does; else it jumps. A picked `duration` without a
/// `transition` moves every property along [`Easing::Ease`], CSS's
/// initial values. Under reduced motion
/// (`style.animation_time == 0`) it jumps too. `id` names the property of
/// this widget; the first pass that sees it shows `target`. Asks for a
/// repaint only while the value moves. A non-finite value has no way to
/// move along: to or from one, the value jumps. A change of the applied
/// theme or of the mode jumps too.
///
/// ```
/// use shadcn_egui::components::motion::transition;
/// use shadcn_egui::style::{Prop, Recipe, State, Transition};
///
/// let indicator = Recipe {
///     transition: Prop::fixed(Transition::ALL),
///     ..Recipe::default()
/// };
/// let ctx = egui::Context::default();
/// let mut output = ctx.run_ui(Default::default(), |ui| {
///     let width = transition(
///         ui.ctx(),
///         ui.id().with("width"),
///         120.0,
///         &[&indicator],
///         Transition::OTHER,
///         State::NONE,
///     );
///     assert_eq!(width, 120.0);
/// });
/// output.textures_delta.clear();
/// ```
pub fn transition(
    ctx: &Context,
    id: Id,
    target: f32,
    layers: &[&Recipe],
    property: Transition,
    s: State,
) -> f32 {
    run(ctx, id, target, layers, property, s)
}

/// [`transition`] for a colour: `transition-colors`, `transition-all` or
/// `transition-[color,box-shadow]` fade a part's state colours from where
/// they are to `target`, mixed in OKLab as the browser mixes them
/// ([`mix_oklab`](crate::color::mix_oklab)). `property` is the colour's property:
/// [`Transition::BACKGROUND`] for the fill, [`Transition::COLOR`] for text
/// and icons, [`Transition::BORDER_COLOR`], [`Transition::SHADOW`] for a
/// ring. `id` names the property of this widget, as for [`transition`].
pub fn transition_color(
    ctx: &Context,
    id: Id,
    target: Color32,
    layers: &[&Recipe],
    property: Transition,
    s: State,
) -> Color32 {
    transition_tone(ctx, id, Tone::of(target), layers, property, s).color
}

/// [`transition_color`] for a colour that keeps the unclipped value it
/// stands for ([`Theme::tone`](crate::Theme)): a role at an alpha fades
/// as its token does.
pub(crate) fn transition_tone(
    ctx: &Context,
    id: Id,
    target: Tone,
    layers: &[&Recipe],
    property: Transition,
    s: State,
) -> Tone {
    run(ctx, id, target, layers, property, s)
}

/// [`transition`] for a part's `box-shadow` elevation (`shadow-*`), under
/// [`Transition::SHADOW`], mixed layer by layer as CSS mixes shadow lists.
pub(crate) fn transition_shadow(
    ctx: &Context,
    id: Id,
    target: Layers,
    layers: &[&Recipe],
    s: State,
) -> Layers {
    run(ctx, id, target, layers, Transition::SHADOW, s)
}

fn run<T: Tween>(
    ctx: &Context,
    id: Id,
    target: T,
    layers: &[&Recipe],
    property: Transition,
    s: State,
) -> T {
    // A `duration-*` without a `transition-*` is CSS's initial
    // `transition-property: all` along its initial `ease`.
    let picked = pick(layers, |r| &r.transition, s);
    let duration = pick(layers, |r| &r.duration, s);
    let names = picked.or(duration.map(|_| Transition::ALL));
    let animates =
        names.is_some_and(|t| t.intersects(property)) && ctx.global_style().animation_time > 0.0;
    let now = ctx.input(|i| i.time);
    let pass = ctx.cumulative_pass_nr();
    let stamp = theme_stamp(ctx);
    // Updated in place: a settled value allocates nothing per pass.
    let (value, running) = ctx.data_mut(|d| {
        let slot = d.get_temp_mut_or_insert_with(id, || Run::settled(target));
        let mut run = match Some(*slot) {
            // A theme, palette or mode change jumps, as the shadcn pages turn
            // transitions off for it (`disableTransitions`).
            Some(run) if animates && target.finite() && run.to.finite() && run.stamp == stamp => {
                // A CSS transition runs on wall time whether the element paints
                // or not. After passes without the widget (scrolled away, not
                // shown) the change is dated to the last pass that showed it,
                // so a change older than the duration shows its target at once.
                let start = if run.seen_pass + 1 < pass {
                    run.seen_time
                } else {
                    now
                };
                let (current, progress, running) = run.at(start);
                if target == run.to {
                    run
                } else {
                    let duration = duration.unwrap_or(DEFAULT_DURATION);
                    let default_easing = if picked.is_some() {
                        Easing::InOut
                    } else {
                        Easing::Ease
                    };
                    let easing = pick(layers, |r| &r.easing, s).unwrap_or(default_easing);
                    // CSS Transitions 1, "starting of transitions", step 4.
                    let (reverse_to, shortening) = if running && target == run.reverse_to {
                        let f = progress * run.shortening + 1.0 - run.shortening; // behaviour-ok: CSS reversing shortening factor
                        (run.to, f.abs().clamp(0.0, 1.0)) // behaviour-ok: the unit interval
                    } else {
                        (current, 1.0) // behaviour-ok: CSS's initial shortening factor
                    };
                    Run {
                        from: current,
                        to: target,
                        start,
                        duration: duration * shortening,
                        easing,
                        reverse_to,
                        shortening,
                        seen_pass: 0,
                        seen_time: 0.0,
                        stamp: 0,
                    }
                }
            }
            _ => Run::settled(target),
        };
        let (value, _, running) = run.at(now);
        if !running {
            run = Run::settled(target);
        }
        (run.seen_pass, run.seen_time, run.stamp) = (pass, now, stamp);
        *slot = run;
        (value, running)
    });
    if running {
        ctx.request_repaint();
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_control_points_give_linear_timing() {
        for t in [0.1, 0.3, 0.5, 0.9] {
            assert!((cubic_bezier(0.25, 0.25, 0.75, 0.75, t) - t).abs() < 1e-5);
        }
    }

    #[test]
    fn every_easing_starts_at_zero_and_ends_at_one() {
        for e in [
            Easing::Linear,
            Easing::Ease,
            Easing::In,
            Easing::Out,
            Easing::InOut,
        ] {
            assert_eq!(e.at(0.0), 0.0, "{e:?}");
            assert_eq!(e.at(1.0), 1.0, "{e:?}");
        }
        assert!(Easing::Out.at(0.5) > 0.5);
        assert!(Easing::In.at(0.5) < 0.5);
    }
}
