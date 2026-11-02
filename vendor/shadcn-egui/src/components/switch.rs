//! shadcn Switch, ported from shadcn-svelte `ui/switch/switch.svelte` at
//! `tools/stylegen/PIN`. Every value, the hit area and the disabled fade
//! included, comes from the style's `cn-switch` (track) and
//! `cn-switch-thumb` recipes. The thumb sits inside the track's 1px transparent border and
//! slides between its two positions with the thumb's `transition-transform`
//! through [`motion::transition`](crate::components::motion::transition):
//! upstream's 150 ms ease-in-out, a jump under reduced motion.

use crate::components::motion::transition;
use crate::components::shared::{
    disabled_cursor, interaction_state, line_width, part_painter, pressable, report_invalid, shift,
    ActivationKeys, RecipeBox,
};
use crate::style::app::{default_variant, state_size};
use crate::style::{pick, pick_len, Prop, Recipe, State, Transition};
use crate::Theme;
use egui::{vec2, Rect, Sense, Ui, Widget};

/// shadcn `data-size` of the switch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SwitchSize {
    #[default]
    Default,
    Sm,
    /// An app size: [`SwitchSize::app`].
    #[doc(hidden)]
    App(crate::style::AppVariant<SwitchSize>),
}

state_size!(SwitchSize, SwitchVariant);

default_variant!(
    /// The switch's variant: upstream has none, so only app variants
    /// ([`SwitchVariant::app`]) differ from `Default`.
    SwitchVariant,
    /// The switch's recipes an app variant's or app size's edit changes.
    SwitchRecipes {
        /// `data-slot="switch"`: the track.
        switch,
        /// `data-slot="switch-thumb"`: the thumb.
        switch_thumb,
    }
);

/// A switch. `toggle(ui, &mut on)` is shorthand for
/// `ui.add(Switch::new(&mut on))`.
///
/// The returned response's rect is the visible track. Its click target is
/// larger (the track recipe's `hit_inset_x` and `hit_inset_y`), and the
/// accessibility node (`widget_info`) carries that larger rect. egui has no
/// switch role, so the node reports as a checkbox.
pub struct Switch<'a> {
    on: &'a mut bool,
    size: SwitchSize,
    enabled: bool,
    invalid: bool,
    variant: SwitchVariant,
}

impl<'a> Switch<'a> {
    pub fn new(on: &'a mut bool) -> Self {
        Self {
            on,
            size: SwitchSize::Default,
            enabled: true,
            invalid: false,
            variant: SwitchVariant::Default,
        }
    }
    /// shadcn `size`, or an app size ([`SwitchSize::app`]).
    pub fn size(mut self, size: impl Into<SwitchSize>) -> Self {
        self.size = size.into();
        self
    }
    /// An app variant ([`SwitchVariant::app`]).
    pub fn variant(mut self, v: impl Into<SwitchVariant>) -> Self {
        self.variant = v.into();
        self
    }
    /// `false` draws the switch disabled: no hover, no focus, no toggling.
    pub fn enabled(mut self, yes: bool) -> Self {
        self.enabled = yes;
        self
    }
    /// shadcn `aria-invalid`: destructive border and ring.
    pub fn invalid(mut self, yes: bool) -> Self {
        self.invalid = yes;
        self
    }
}

impl Widget for Switch<'_> {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        let t = Theme::current(ui.ctx());
        let v = t.resolve_pair(self.variant, self.size);
        let track = [v.part(&t.style, |s| &s.switch, |r| &r.switch)];
        let thumb = [v.part(&t.style, |s| &s.switch_thumb, |r| &r.switch_thumb)];
        let enabled = self.enabled && ui.is_enabled();
        let size_bit = v.bits();
        // Geometry depends on neither the value nor interaction.
        let g = t.state(ui.ctx(), size_bit.with(State::INVALID, self.invalid));
        let get = |f: fn(&Recipe) -> &Prop<f32>| pick_len(&track, f, g);

        // No size: CSS's initial `auto`, the thumb inside the border.
        let border = line_width(get(|r| &r.border_width), ui.ctx().pixels_per_point()) * 2.0;
        let thumb_side = |f: fn(&Recipe) -> &Prop<f32>| pick_len(&thumb, f, g);
        let size = vec2(
            pick(&track, |r| &r.width, g).unwrap_or_else(|| thumb_side(|r| &r.width) + border),
            pick(&track, |r| &r.height, g).unwrap_or_else(|| thumb_side(|r| &r.height) + border),
        );
        let (rect, base) = crate::layout::allocate_item(ui, size, Sense::hover());
        let hit = rect.expand2(vec2(get(|r| &r.hit_inset_x), get(|r| &r.hit_inset_y)));
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let mut resp = crate::components::shared::hit_area(ui, hit, base.id.with("hit"), sense);
        pressable(&mut resp, ActivationKeys::EnterAndSpace);
        if enabled && resp.clicked() {
            *self.on = !*self.on;
            resp.mark_changed();
        }
        let on = *self.on;
        let context = |checked: bool| {
            size_bit
                .or(if checked {
                    State::CHECKED
                } else {
                    State::UNCHECKED
                })
                .with(State::INVALID, self.invalid)
        };
        let st = interaction_state(&t, &resp, context(on).with(State::DISABLED, !enabled));

        if ui.is_rect_visible(rect) {
            let border_w = line_width(
                pick_len(&track, |r| &r.border_width, st),
                ui.ctx().pixels_per_point(),
            );
            // The thumb lies inside the border, vertically centred; its
            // translate differs per value and moves as its transition says.
            let side = |f: fn(&Recipe) -> &Prop<f32>| pick_len(&thumb, f, st);
            let size = vec2(side(|r| &r.width), side(|r| &r.height));
            let home =
                Rect::from_min_size(rect.left_center() + vec2(border_w, -size.y / 2.0), size);
            let at = |checked: bool| {
                let s = t.state(ui.ctx(), context(checked));
                shift(
                    home,
                    pick(&thumb, |r| &r.shift_x, s),
                    pick(&thumb, |r| &r.shift_y, s),
                )
            };
            // The `translate` moves, not the screen position: a switch
            // that moves takes its thumb along.
            let target = at(on);
            let x = transition(
                ui.ctx(),
                base.id.with("thumb-x"),
                target.left() - home.left(),
                &thumb,
                Transition::TRANSFORM,
                st,
            );
            let thumb_rect = Rect::from_min_size(egui::pos2(home.left() + x, target.top()), size);
            let thumb_box = RecipeBox::resolve(ui.ctx(), &t, &thumb, st);
            let thumb_painter =
                part_painter(ui, base.id.with("thumb"), &[&thumb[..], &track[..]], st);
            // The track under the thumb is hidden, as in the root's faded
            // group.
            let track_painter = part_painter(ui, base.id, &[&track[..]], st);
            let track_shapes = RecipeBox::resolve(ui.ctx(), &t, &track, st)
                .transitioned(ui.ctx(), base.id, &track, st)
                .shapes_at(ui.ctx(), rect, track_painter.opacity());
            track_painter.extend(thumb_box.hiding(
                ui.ctx(),
                thumb_rect,
                thumb_painter.opacity(),
                track_shapes,
            ));
            thumb_box.paint(&thumb_painter, thumb_rect);
        }
        // No label of its own: name it with `labelled_by(label.id)`, which
        // an own label, even an empty one, would override.
        resp.widget_info(|| egui::WidgetInfo {
            enabled,
            selected: Some(on),
            ..egui::WidgetInfo::new(egui::WidgetType::Checkbox)
        });
        // `data-disabled:cursor-not-allowed`, on the hit area as on its
        // `::after`.
        disabled_cursor(&resp, !enabled);
        report_invalid(&resp, self.invalid);
        resp.rect = rect;
        resp
    }
}

/// Shorthand for `ui.add(Switch::new(on))`.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> egui::Response {
    ui.add(Switch::new(on))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{AppVariant, Paint, Role};

    const DANGER: AppVariant<SwitchVariant> = SwitchVariant::app("danger");
    const WIDE: AppVariant<SwitchSize> = SwitchSize::app("wide");

    fn danger(r: &mut SwitchRecipes, _: crate::StyleName) {
        r.switch_thumb.bg.set(Paint::Role(Role::Destructive, 1.0));
    }
    fn wide(r: &mut SwitchRecipes, _: crate::StyleName) {
        r.switch.width.set(60.0);
    }

    /// An app variant's and an app size's edits get `SwitchRecipes`; named
    /// together, both hold, and the app size keeps its base's bits.
    #[test]
    fn edit_compiles_and_variant_and_size_combine() {
        let t = Theme::vega()
            .variant(DANGER, SwitchVariant::Default, danger)
            .size(WIDE, SwitchSize::Sm, wide);
        let v = t.resolve_pair(SwitchVariant::from(DANGER), SwitchSize::from(WIDE));
        assert_eq!(v.bits(), State::SIZE_SM);
        let thumb = v.part(&t.style, |s| &s.switch_thumb, |r| &r.switch_thumb);
        assert_eq!(
            thumb.bg.get(State::NONE),
            Some(Paint::Role(Role::Destructive, 1.0))
        );
        // The width is drawn under the base's bits, where `sm`'s own rule
        // gave it before the edit.
        let track = v.part(&t.style, |s| &s.switch, |r| &r.switch);
        assert_eq!(track.width.get(v.bits()), Some(60.0));
        let only_size = t.resolve_pair(SwitchVariant::Default, SwitchSize::from(WIDE));
        let thumb = only_size.part(&t.style, |s| &s.switch_thumb, |r| &r.switch_thumb);
        assert_eq!(
            thumb.bg.get(only_size.bits()),
            t.style.switch_thumb.bg.get(State::SIZE_SM)
        );
        let track = only_size.part(&t.style, |s| &s.switch, |r| &r.switch);
        assert_eq!(track.width.get(only_size.bits()), Some(60.0));
        // The parts it does not edit draw as `sm` does.
        assert_eq!(
            track.height.get(only_size.bits()),
            t.style.switch.height.get(State::SIZE_SM)
        );
    }
}
