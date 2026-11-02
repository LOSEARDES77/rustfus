//! shadcn Slider, ported from shadcn-svelte `ui/slider/slider.svelte` at
//! `tools/stylegen/PIN`, which wraps the bits-ui slider. Every value comes
//! from the style's `cn-slider` (root), `cn-slider-track`,
//! `cn-slider-range` and `cn-slider-thumb` recipes. Opacity multiplies down
//! the parts as in the DOM: the range sits in the track, the track and the
//! thumb in the root.
//!
//! The thumb stays inside the track: its centre travels from half a thumb
//! after the track start to half a thumb before its end, as in bits-ui.

use crate::components::shared::{
    interaction_state, part_painter, pressable, ActivationKeys, RecipeBox,
};
use crate::style::app::default_variant;
use crate::style::{pick_len, Recipe, State};
use crate::{layout, Theme};
use egui::{pos2, vec2, EventFilter, Key, Modifiers, Rect, Sense, Ui, Widget};

/// shadcn `orientation` of the slider.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SliderOrientation {
    #[default]
    Horizontal,
    Vertical,
}

default_variant!(
    /// The slider's variant: upstream has none, so only app variants
    /// ([`SliderVariant::app`]) differ from `Default`.
    SliderVariant,
    /// The slider's recipes an app variant's edit changes.
    SliderRecipes {
        /// `data-slot="slider"`: the root.
        slider,
        /// `data-slot="slider-track"`.
        slider_track,
        /// `data-slot="slider-range"`: the filled part of the track.
        slider_range,
        /// `data-slot="slider-thumb"`.
        slider_thumb,
    }
);

/// A slider with one thumb (bits-ui also supports several; this port does
/// not). The value is clamped to the range on every frame and snapped to
/// `step` when the user changes it. A vertical slider has its minimum at the
/// bottom.
///
/// Keys while focused: Left/Down and Right/Up move one step (without a
/// step, 1% of the range), PageDown/PageUp ten steps, Home/End jump to the
/// ends. The returned response's rect is the slider's root box; its click
/// target also covers the thumb's hit area.
pub struct Slider<'a> {
    value: &'a mut f32,
    range: std::ops::RangeInclusive<f32>,
    step: f32,
    orientation: SliderOrientation,
    enabled: bool,
    label: String,
    variant: SliderVariant,
}

impl<'a> Slider<'a> {
    pub fn new(value: &'a mut f32, range: std::ops::RangeInclusive<f32>) -> Self {
        Self {
            value,
            range,
            step: 0.0,
            orientation: SliderOrientation::Horizontal,
            enabled: true,
            label: String::new(),
            variant: SliderVariant::Default,
        }
    }
    /// Snap to multiples of `step` from the range start; 0 = continuous.
    pub fn step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }
    pub fn orientation(mut self, o: SliderOrientation) -> Self {
        self.orientation = o;
        self
    }
    /// `false` draws the slider disabled: no hover, no focus, no changes.
    pub fn enabled(mut self, yes: bool) -> Self {
        self.enabled = yes;
        self
    }
    /// Accessible name (shadcn sets it through a label).
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = text.into();
        self
    }
    /// An app variant ([`SliderVariant::app`]).
    pub fn variant(mut self, v: impl Into<SliderVariant>) -> Self {
        self.variant = v.into();
        self
    }
}

/// Snap `v` to the step grid starting at `lo`, then clamp into `lo..=hi`.
fn snap(v: f32, lo: f32, hi: f32, step: f32) -> f32 {
    let v = if v.is_nan() { lo } else { v };
    let v = if step > 0.0 && step.is_finite() {
        lo + ((v - lo) / step).round() * step
    } else {
        v
    };
    v.clamp(lo, hi)
}

impl Widget for Slider<'_> {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        let t = Theme::current(ui.ctx());
        let v = t.resolve(self.variant);
        let root = [v.part(&t.style, |_, s| &s.slider, |r| &r.slider)];
        let track = [v.part(&t.style, |_, s| &s.slider_track, |r| &r.slider_track)];
        let range = [v.part(&t.style, |_, s| &s.slider_range, |r| &r.slider_range)];
        let thumb = [v.part(&t.style, |_, s| &s.slider_thumb, |r| &r.slider_thumb)];
        let enabled = self.enabled && ui.is_enabled();
        let vertical = self.orientation == SliderOrientation::Vertical;
        let orient = if vertical {
            State::VERTICAL
        } else {
            State::HORIZONTAL
        }
        .or(v.bits());

        // A degenerate range is a slider that cannot move.
        let lo = if self.range.start().is_finite() {
            *self.range.start()
        } else {
            0.0
        };
        let hi = if self.range.end().is_finite() {
            self.range.end().max(lo)
        } else {
            lo
        };
        let step = self.step;

        // Geometry depends on neither the value nor interaction.
        let g = t.state(ui.ctx(), orient);
        let size_of =
            |l: &[&Recipe]| vec2(pick_len(l, |r| &r.width, g), pick_len(l, |r| &r.height, g));
        let thumb_size = size_of(&thumb);
        let thickness = if vertical {
            size_of(&track).x
        } else {
            size_of(&track).y
        };
        let (along, cross) = if vertical {
            (thumb_size.y, thumb_size.x)
        } else {
            (thumb_size.x, thumb_size.y)
        };
        let cross = cross.max(thickness);
        let size = if vertical {
            vec2(cross, pick_len(&root, |r| &r.min_height, g).max(along))
        } else {
            vec2(layout::capped_width(ui, f32::INFINITY, along), cross)
        };
        let (rect, base) = layout::allocate_overflowing(ui, size, Sense::hover());

        // Axis helpers: `a` runs along the slider from the minimum end.
        let (start, len) = if vertical {
            (rect.bottom(), rect.height())
        } else {
            (rect.left(), rect.width())
        };
        let dir = if vertical { -1.0 } else { 1.0 }; // behaviour-ok: direction of travel
        let travel = (len - along).max(0.0);
        let centre_of = |frac: f32| start + dir * (along / 2.0 + frac * travel);
        let frac_of = |v: f32| {
            if hi > lo {
                ((v - lo) / (hi - lo)).clamp(0.0, 1.0) // behaviour-ok: fraction of the range
            } else {
                0.0
            }
        };
        let thumb_at = |frac: f32| {
            let c = centre_of(frac);
            let centre = if vertical {
                pos2(rect.center().x, c)
            } else {
                pos2(c, rect.center().y)
            };
            Rect::from_center_size(centre, thumb_size)
        };

        // The value is always clamped; NaN becomes the minimum.
        let mut value = snap(*self.value, lo, hi, 0.0);
        let hit_inset = vec2(
            pick_len(&thumb, |r| &r.hit_inset_x, g),
            pick_len(&thumb, |r| &r.hit_inset_y, g),
        );
        let hit = rect.union(thumb_at(frac_of(value)).expand2(hit_inset));
        let sense = if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        };
        let mut resp = crate::components::shared::hit_area(ui, hit, base.id.with("hit"), sense);
        pressable(&mut resp, ActivationKeys::None);

        let mut moved = false;
        if enabled && hi > lo {
            if resp.is_pointer_button_down_on() || resp.dragged() || resp.clicked() {
                if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                    let a = if vertical { p.y } else { p.x };
                    let frac = if travel > 0.0 {
                        ((a - centre_of(0.0)) / (dir * travel)).clamp(0.0, 1.0) // behaviour-ok: fraction of the range
                    } else {
                        0.0
                    };
                    value = snap(lo + frac * (hi - lo), lo, hi, step);
                    moved = true;
                }
            }
            if resp.has_focus() {
                ui.memory_mut(|m| {
                    m.set_focus_lock_filter(
                        resp.id,
                        EventFilter {
                            horizontal_arrows: true,
                            vertical_arrows: true,
                            ..Default::default()
                        },
                    )
                });
                // Without a step, bits-ui moves by 1% of the range.
                let default_steps = 100.0; // behaviour-ok: bits-ui's 1% default step, not a design token
                let unit = if step > 0.0 && step.is_finite() {
                    step
                } else {
                    (hi - lo) / default_steps
                };
                let presses = |keys: &[Key]| {
                    ui.input_mut(|i| {
                        keys.iter()
                            .map(|k| i.consume_key(Modifiers::NONE, *k) as i32)
                            .sum::<i32>()
                    })
                };
                let page_steps = 10; // behaviour-ok: bits-ui's PageUp/PageDown move ten steps, not a design token
                let steps = presses(&[Key::ArrowRight, Key::ArrowUp])
                    - presses(&[Key::ArrowLeft, Key::ArrowDown])
                    + page_steps * (presses(&[Key::PageUp]) - presses(&[Key::PageDown]));
                if steps != 0 {
                    value = snap(value + steps as f32 * unit, lo, hi, step);
                    moved = true;
                }
                if presses(&[Key::Home]) > 0 {
                    value = lo;
                    moved = true;
                }
                if presses(&[Key::End]) > 0 {
                    value = hi;
                    moved = true;
                }
            }
        }
        let changed = moved && value != *self.value;
        *self.value = value;
        if changed {
            resp.mark_changed();
        }

        let thumb_rect = thumb_at(frac_of(value));
        let pointer_on_thumb = ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|p| thumb_rect.expand2(hit_inset).contains(p));
        let st = interaction_state(&t, &resp, orient.with(State::DISABLED, !enabled));
        let st = st.with(
            State::HOVER,
            st.contains(State::HOVER) && (pointer_on_thumb || resp.dragged()),
        );

        if ui.is_rect_visible(hit) {
            let track_painter =
                part_painter(ui, resp.id.with("track"), &[&track[..], &root[..]], st);
            let range_painter = part_painter(
                ui,
                resp.id.with("range"),
                &[&range[..], &track[..], &root[..]],
                st,
            );
            let thumb_painter =
                part_painter(ui, resp.id.with("thumb"), &[&thumb[..], &root[..]], st);

            let track_rect = if vertical {
                Rect::from_center_size(rect.center(), vec2(thickness, rect.height()))
            } else {
                Rect::from_center_size(rect.center(), vec2(rect.width(), thickness))
            };
            // The thumb hides what lies under it, as in the root's faded
            // group.
            let thumb_box = RecipeBox::resolve(ui.ctx(), &t, &thumb, st).transitioned(
                ui.ctx(),
                resp.id.with("thumb"),
                &thumb,
                st,
            );
            let under_thumb =
                |shapes| thumb_box.hiding(ui.ctx(), thumb_rect, thumb_painter.opacity(), shapes);
            let track_box = RecipeBox::resolve(ui.ctx(), &t, &track, st);
            let range_box = RecipeBox::resolve(ui.ctx(), &t, &range, st);
            let c = centre_of(frac_of(value));
            let range_rect = if vertical {
                Rect::from_min_max(pos2(track_rect.left(), c), track_rect.right_bottom())
            } else {
                Rect::from_min_max(track_rect.left_top(), pos2(c, track_rect.bottom()))
            };
            // The range hides the track under it as the thumb does.
            track_painter.extend(under_thumb(range_box.hiding_inside(
                ui.ctx(),
                range_rect,
                &track_box,
                track_rect,
                range_painter.opacity(),
                track_box.shapes_at(ui.ctx(), track_rect, track_painter.opacity()),
            )));

            // The track's `overflow-hidden` cuts the range to its rounded box.
            range_painter.extend(under_thumb(range_box.shapes_inside(
                ui.ctx(),
                range_rect,
                &track_box,
                track_rect,
                range_painter.opacity(),
            )));

            thumb_box.paint(&thumb_painter, thumb_rect);
        }
        let label = self.label;
        resp.widget_info(|| egui::WidgetInfo::slider(enabled, value as f64, label.as_str()));
        resp.rect = rect;
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{AppVariant, Paint, Role};

    /// An app variant's edit gets `SliderRecipes` and changes only what it
    /// names.
    #[test]
    fn edit_compiles_and_changes_its_part() {
        const DANGER: AppVariant<SliderVariant> = SliderVariant::app("danger");
        let t = Theme::vega().variant(
            DANGER,
            SliderVariant::Default,
            |r: &mut SliderRecipes, _| r.slider_range.recolor(Role::Primary, Role::Destructive),
        );
        let v = t.resolve(SliderVariant::from(DANGER));
        let range = v.part(&t.style, |_, s| &s.slider_range, |r| &r.slider_range);
        assert_eq!(
            range.bg.get(State::NONE),
            Some(Paint::Role(Role::Destructive, 1.0))
        );
        let track = v.part(&t.style, |_, s| &s.slider_track, |r| &r.slider_track);
        assert_eq!(track, &*t.style.slider_track);
    }
}
