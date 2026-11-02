//! shadcn Kbd, ported from shadcn-svelte `ui/kbd/kbd.svelte` and
//! `ui/kbd/kbd-group.svelte` at `tools/stylegen/PIN`. Every value comes
//! from the style's `cn-kbd` and `cn-kbd-group` recipes.
//!
//! A key chip lays out its content in one row, as a button does: text and
//! icons, `gap` apart, `min-w-*` wide at least. The look inside a
//! tooltip (`in-data-[slot=tooltip-content]:`) waits for the tooltip's
//! closure form; a chip draws its own look everywhere.

use crate::color::Tone;
use crate::components::atom_row::AtomRow;
use crate::components::context;
use crate::components::icon::Icon;
use crate::components::shared::{line_width, part_painter, transitioned_text, RecipeBox};
use crate::components::text::TextLook;
use crate::style::app::default_variant;
use crate::style::{pick, Prop, Recipe, Role};
use crate::{layout, Theme};
use egui::{InnerResponse, IntoAtoms, Response, Sense, Ui, UiBuilder, Vec2, Widget};

default_variant!(
    /// The kbd's variant: upstream has none, so only app variants
    /// ([`KbdVariant::app`]) differ from `Default`. A [`KbdGroup`]'s
    /// variant reaches the chips inside it.
    KbdVariant,
    /// The kbd's recipes an app variant's edit changes.
    KbdRecipes {
        /// `data-slot="kbd"`: the key chip.
        kbd,
        /// `data-slot="kbd-group"`: the row of chips.
        kbd_group,
    }
);

/// The context a kbd group gives its chips: its variant.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct KbdContext {
    variant: KbdVariant,
}

/// A key chip: `ui.add(Kbd::new("Ctrl"))`. Several chips that form one
/// shortcut go in a [`KbdGroup`].
///
/// ```
/// # egui::__run_test_ui(|ui| {
/// use shadcn_egui::components::kbd::{Kbd, KbdGroup};
/// KbdGroup::new().show(ui, |ui| {
///     ui.add(Kbd::new("Ctrl"));
///     ui.label("+");
///     ui.add(Kbd::new("B"));
/// });
/// # });
/// ```
#[must_use = "You should put this widget in a ui with `ui.add(widget);`"]
pub struct Kbd<'a> {
    atoms: egui::Atoms<'a>,
    /// The icon of an icon-only chip, with its accessible name.
    icon: Option<(Icon, String)>,
    variant: Option<KbdVariant>,
}

impl<'a> Kbd<'a> {
    /// A chip showing `content`: text, or text and icons in one row
    /// (`IconAtom` icons take the chip's text colour).
    #[inline]
    pub fn new(content: impl IntoAtoms<'a>) -> Self {
        Self {
            atoms: content.into_atoms(),
            icon: None,
            variant: None,
        }
    }

    /// A chip showing `icon` alone; `name` is its accessible name.
    #[inline]
    pub fn icon_only(icon: Icon, name: impl Into<String>) -> Self {
        Self {
            atoms: egui::Atoms::default(),
            icon: Some((icon, name.into())),
            variant: None,
        }
    }

    /// An app variant ([`KbdVariant::app`]). Without one, a chip in a
    /// [`KbdGroup`] takes the group's variant.
    #[inline]
    pub fn variant(mut self, v: impl Into<KbdVariant>) -> Self {
        self.variant = Some(v.into());
        self
    }
}

/// A key chip with `content`: shorthand for `ui.add(Kbd::new(content))`.
pub fn kbd<'a>(ui: &mut Ui, content: impl IntoAtoms<'a>) -> Response {
    ui.add(Kbd::new(content))
}

impl Widget for Kbd<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = Theme::current(ui.ctx());
        let variant = self
            .variant
            .or_else(|| context::find::<KbdContext>(ui).map(|c| c.variant))
            .unwrap_or_default();
        let v = t.resolve(variant);
        let l = [v.part(&t.style, |_, s| &s.kbd, |r| &r.kbd)];
        let context = v.bits();
        // A kbd has no interaction states (`pointer-events-none`): its
        // geometry and paint state are one.
        let st = t.state(ui.ctx(), context);
        let get = |f: fn(&Recipe) -> &Prop<f32>| pick(&l, f, st).unwrap_or(0.0);
        let border_w = line_width(get(|r| &r.border_width), ui.ctx().pixels_per_point());
        let icon_px = pick(&l, |r| &r.icon_size, st).unwrap_or(t.metrics.icon);
        let look = TextLook::resolve(ui, &l, st);
        let (row, name) = match self.icon {
            Some((icon, name)) => (AtomRow::new(ui, self.atoms).icons(Some(icon), None), name),
            None => {
                let name = self
                    .atoms
                    .text()
                    .map(|s| s.into_owned())
                    .unwrap_or_default();
                (AtomRow::new(ui, self.atoms), name)
            }
        };
        let row = row
            .icon_size(icon_px)
            .gap(get(|r| &r.gap))
            .inset(
                get(|r| &r.pad_left) + border_w,
                get(|r| &r.pad_right) + border_w,
                get(|r| &r.pad_top) + border_w,
                get(|r| &r.pad_bottom) + border_w,
            )
            .lay_out(ui, &look);
        // No height: CSS's initial `auto`, the content's height.
        let height = pick(&l, |r| &r.height, st)
            .unwrap_or(row.size().y)
            .max(get(|r| &r.min_height));
        let width = row.size().x.max(get(|r| &r.min_width));
        let (id, rect) = layout::allocate_item_space(ui, Vec2::new(width, height));
        // `pointer-events-none`: egui has no sense without hover, so the
        // chip registers with an empty clip, as a widget scrolled out of
        // view: its hit rect (clip ∩ rect) never holds the pointer, while
        // the response keeps `rect` for the accessible node's bounds.
        let clip = ui.clip_rect();
        ui.set_clip_rect(egui::Rect::NOTHING);
        let resp = ui.interact(rect, id, Sense::hover());
        ui.set_clip_rect(clip);
        let enabled = ui.is_enabled();
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, enabled, &name));
        if !ui.is_rect_visible(rect) {
            return resp;
        }
        let painter = part_painter(ui, id, &[&l[..]], st);
        let inherited = t.role(Role::Foreground); // behaviour-ok: inherits Foreground; upstream sets no text colour
        let fg = pick(&l, |r| &r.text, st).map_or(Tone::of(inherited), |p| t.tone(p));
        let fg = transitioned_text(ui.ctx(), id, fg, &l, st);
        RecipeBox::resolve(ui.ctx(), &t, &l, st)
            .transitioned(ui.ctx(), id, &l, st)
            .paint(&painter, rect);
        row.paint(ui, &painter, rect, &look, fg);
        resp
    }
}

/// A row of key chips that form one shortcut:
/// `KbdGroup::new().show(ui, |ui| ..)`. The chips and any text between
/// them sit in one row, `gap` apart, centred on each other.
#[must_use = "You should call .show()"]
#[derive(Clone, Copy, Debug, Default)]
pub struct KbdGroup {
    variant: KbdVariant,
}

impl KbdGroup {
    /// An empty group.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// An app variant ([`KbdVariant::app`]); the chips inside take it
    /// unless they name their own.
    #[inline]
    pub fn variant(mut self, v: impl Into<KbdVariant>) -> Self {
        self.variant = v.into();
        self
    }

    /// Lay out `add`'s chips and text in one row.
    pub fn show<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
        let t = Theme::current(ui.ctx());
        let v = t.resolve(self.variant);
        let l = [v.part(&t.style, |_, s| &s.kbd_group, |r| &r.kbd_group)];
        let st = t.state(ui.ctx(), v.bits());
        let gap = pick(&l, |r| &r.gap, st).unwrap_or(0.0);
        let parts = KbdContext {
            variant: self.variant,
        };
        // `inline-flex`: a crate row, as high as its content.
        ui.scope_builder(context::push(UiBuilder::new(), parts), |ui| {
            layout::row(ui, gap, add)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{AppVariant, Paint, State};

    /// An app variant's edit gets `KbdRecipes` and changes only what it
    /// names.
    #[test]
    fn edit_compiles_and_changes_its_part() {
        const LOUD: AppVariant<KbdVariant> = KbdVariant::app("loud");
        let t = Theme::vega().variant(LOUD, KbdVariant::Default, |r: &mut KbdRecipes, _| {
            r.kbd.bg.set(Paint::Role(Role::Primary, 1.0))
        });
        let v = t.resolve(KbdVariant::from(LOUD));
        let kbd = v.part(&t.style, |_, s| &s.kbd, |r| &r.kbd);
        assert_eq!(
            kbd.bg.get(State::NONE),
            Some(Paint::Role(Role::Primary, 1.0))
        );
        assert_eq!(kbd.text, t.style.kbd.text);
        let group = v.part(&t.style, |_, s| &s.kbd_group, |r| &r.kbd_group);
        assert_eq!(group, &*t.style.kbd_group);
    }
}
