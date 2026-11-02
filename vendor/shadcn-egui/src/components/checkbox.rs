//! shadcn Checkbox, ported from shadcn-svelte `ui/checkbox/checkbox.svelte`
//! at `tools/stylegen/PIN`. Every value, the hit area and the disabled
//! fade included, comes from the style's `cn-checkbox` and
//! `cn-checkbox-indicator` recipes. The tick is the theme's `check` icon
//! ([`crate::Theme::icons`]).
//!
//! The indeterminate state of the upstream component is not offered: the
//! value is a plain `bool`.

use crate::components::icon::paint_icon;
use crate::components::shared::{
    disabled_cursor, interaction_state, line_width, part_painter, pressable, report_invalid,
    ActivationKeys, RecipeBox,
};
use crate::style::app::default_variant;
use crate::style::{pick, pick_len, Prop, Recipe, State};
use crate::Theme;
use egui::{vec2, Rect, Sense, Ui, Vec2, Widget};

default_variant!(
    /// The checkbox's variant: upstream has none, so only app variants
    /// ([`CheckboxVariant::app`]) differ from `Default`.
    CheckboxVariant,
    /// The checkbox's recipes an app variant's edit changes.
    CheckboxRecipes {
        /// `data-slot="checkbox"`: the box.
        checkbox,
        /// `data-slot="checkbox-indicator"`: the tick.
        checkbox_indicator,
    }
);

/// A checkbox. `checkbox(ui, &mut on)` is shorthand for
/// `ui.add(Checkbox::new(&mut on))`.
///
/// The returned response's rect is the visible box. Its click target is
/// larger (the recipe's `hit_inset_x` and `hit_inset_y`), and the
/// accessibility node carries that larger rect.
pub struct Checkbox<'a> {
    checked: &'a mut bool,
    enabled: bool,
    invalid: bool,
    variant: CheckboxVariant,
}

impl<'a> Checkbox<'a> {
    pub fn new(checked: &'a mut bool) -> Self {
        Self {
            checked,
            enabled: true,
            invalid: false,
            variant: CheckboxVariant::Default,
        }
    }
    /// `false` draws the box disabled: no hover, no focus, no toggling.
    pub fn enabled(mut self, yes: bool) -> Self {
        self.enabled = yes;
        self
    }
    /// shadcn `aria-invalid`: destructive border and ring.
    pub fn invalid(mut self, yes: bool) -> Self {
        self.invalid = yes;
        self
    }
    /// An app variant ([`CheckboxVariant::app`]).
    pub fn variant(mut self, v: impl Into<CheckboxVariant>) -> Self {
        self.variant = v.into();
        self
    }
}

impl Widget for Checkbox<'_> {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        let t = Theme::current(ui.ctx());
        let v = t.resolve(self.variant);
        let l = [v.part(&t.style, |_, s| &s.checkbox, |r| &r.checkbox)];
        let enabled = self.enabled && ui.is_enabled();
        // Geometry depends on neither the value nor interaction.
        let g = t.state(ui.ctx(), v.bits().with(State::INVALID, self.invalid));
        let get = |f: fn(&Recipe) -> &Prop<f32>| pick_len(&l, f, g);

        // No size: CSS's initial `auto`, the indicator's tick inside the
        // border. The box keeps that size unticked too, so it does not jump.
        let ind = [v.part(
            &t.style,
            |_, s| &s.checkbox_indicator,
            |r| &r.checkbox_indicator,
        )];
        let side = pick(&ind, |r| &r.icon_size, g).unwrap_or(t.metrics.icon);
        let auto = side + line_width(get(|r| &r.border_width), ui.ctx().pixels_per_point()) * 2.0;
        let size = vec2(
            pick(&l, |r| &r.width, g).unwrap_or(auto),
            pick(&l, |r| &r.height, g).unwrap_or(auto),
        );
        let (rect, base) = crate::layout::allocate_item(ui, size, Sense::hover());
        let hit = rect.expand2(vec2(get(|r| &r.hit_inset_x), get(|r| &r.hit_inset_y)));
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let mut resp = crate::components::shared::hit_area(ui, hit, base.id.with("hit"), sense);
        pressable(&mut resp, ActivationKeys::Space);
        if enabled && resp.clicked() {
            *self.checked = !*self.checked;
            resp.mark_changed();
        }
        let checked = *self.checked;
        let context = (if checked {
            State::CHECKED
        } else {
            State::UNCHECKED
        })
        .with(State::INVALID, self.invalid)
        .or(v.bits());
        let st = interaction_state(&t, &resp, context.with(State::DISABLED, !enabled));

        if ui.is_rect_visible(rect) {
            let painter = part_painter(ui, resp.id, &[&l[..]], st);
            RecipeBox::resolve(ui.ctx(), &t, &l, st)
                .transitioned(ui.ctx(), resp.id, &l, st)
                .paint(&painter, rect);
            if checked {
                let side = pick(&ind, |r| &r.icon_size, st).unwrap_or(t.metrics.icon);
                // `text-current`: the tick inherits the box's text colour.
                let tick = pick(&ind, |r| &r.text, st).or_else(|| pick(&l, |r| &r.text, st));
                if let Some(color) = tick.map(|p| t.paint(p)) {
                    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(side));
                    // The indicator sits in the box: both opacities apply.
                    paint_icon(
                        &part_painter(ui, resp.id.with("indicator"), &[&ind[..], &l[..]], st),
                        icon_rect,
                        &t.icons.check,
                        color,
                        t.metrics.icon_stroke,
                    );
                }
            }
        }
        // No label of its own: name it with `labelled_by(label.id)`, which
        // an own label, even an empty one, would override.
        resp.widget_info(|| egui::WidgetInfo {
            enabled,
            selected: Some(checked),
            ..egui::WidgetInfo::new(egui::WidgetType::Checkbox)
        });
        // `disabled:cursor-not-allowed`, on the hit area as on its `::after`.
        disabled_cursor(&resp, !enabled);
        report_invalid(&resp, self.invalid);
        resp.rect = rect;
        resp
    }
}

/// Shorthand for `ui.add(Checkbox::new(checked))`.
pub fn checkbox(ui: &mut Ui, checked: &mut bool) -> egui::Response {
    ui.add(Checkbox::new(checked))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{AppVariant, Paint, Role};

    /// An app variant's edit gets `CheckboxRecipes` and changes only what
    /// it names.
    #[test]
    fn edit_compiles_and_changes_its_part() {
        const DANGER: AppVariant<CheckboxVariant> = CheckboxVariant::app("danger");
        let t = Theme::vega().variant(
            DANGER,
            CheckboxVariant::Default,
            |r: &mut CheckboxRecipes, _style| {
                r.checkbox_indicator
                    .text
                    .set(Paint::Role(Role::Destructive, 1.0))
            },
        );
        let v = t.resolve(CheckboxVariant::from(DANGER));
        let ind = v.part(
            &t.style,
            |_, s| &s.checkbox_indicator,
            |r| &r.checkbox_indicator,
        );
        assert_eq!(
            ind.text.get(State::NONE),
            Some(Paint::Role(Role::Destructive, 1.0))
        );
        let b = v.part(&t.style, |_, s| &s.checkbox, |r| &r.checkbox);
        assert_eq!(b, &*t.style.checkbox);
    }
}
