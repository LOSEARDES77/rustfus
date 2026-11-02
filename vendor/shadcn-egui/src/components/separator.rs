//! shadcn Separator, ported from shadcn-svelte `ui/separator/separator.svelte`
//! at `tools/stylegen/PIN`. It has no style marker; its `data-slot="separator"`
//! classes are the `separator` recipe: a `bg-border` line, `h-px` / `w-px`
//! thick, or `metrics.border` thick where the recipe sets no thickness.

use crate::components::shared::part_painter;
use crate::layout::{self, capped_width};
use crate::style::app::default_variant;
use crate::style::{pick, State};
use crate::Theme;
use egui::{Response, Sense, Ui, Vec2, Widget};

/// shadcn `orientation` of the separator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SeparatorOrientation {
    /// A rule across the available width (`w-full`). It has no content
    /// width, so it never widens what it sits in: where the width is
    /// unbounded (an `allocate_ui` of infinite width) it has no length,
    /// and in a [`vstack`](crate::layout::vstack) that fits its row it
    /// spans the width the stack's other items give it.
    #[default]
    Horizontal,
    /// Upstream's `data-vertical:self-stretch`: it stretches in a flex
    /// row and has no height (0) in a block or a column. In a crate
    /// [`row`](crate::layout::row) it is as tall as the row's tallest
    /// other item. A plain egui horizontal `Ui` counts as a flex row: the
    /// rule is as tall as that row, or as its content where the height
    /// is unbounded. In a column (a [`vstack`](crate::layout::vstack), a
    /// vertical `Ui`) it is 0.
    Vertical,
}

default_variant!(
    /// The separator's variant: upstream has none, so only app variants
    /// ([`SeparatorVariant::app`]) differ from `Default`.
    SeparatorVariant,
    /// The separator's recipes an app variant's edit changes.
    SeparatorRecipes {
        /// `data-slot="separator"`: the line.
        separator,
    }
);

/// A separator: `ui.add(Separator::new())`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Separator {
    orientation: SeparatorOrientation,
    variant: SeparatorVariant,
}

impl Separator {
    pub fn new() -> Self {
        Self::default()
    }
    /// shadcn `orientation`: `Horizontal` (the default) or `Vertical`.
    pub fn orientation(mut self, o: SeparatorOrientation) -> Self {
        self.orientation = o;
        self
    }
    /// An app variant ([`SeparatorVariant::app`]).
    pub fn variant(mut self, v: impl Into<SeparatorVariant>) -> Self {
        self.variant = v.into();
        self
    }
}

impl Widget for Separator {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = Theme::current(ui.ctx());
        let v = t.resolve(self.variant);
        let l = [v.part(&t.style, |_, s| &s.separator, |r| &r.separator)];
        let vertical = self.orientation == SeparatorOrientation::Vertical;
        let st = t.state(
            ui.ctx(),
            v.bits().or(if vertical {
                State::VERTICAL
            } else {
                State::HORIZONTAL
            }),
        );
        let size = if vertical {
            let w = pick(&l, |r| &r.width, st).unwrap_or(t.metrics.border);
            // `self-stretch`: the row's tallest other item. Elsewhere the
            // separator has no height of its own (no `h-full`), so in a
            // column it takes its minimum, 0; a plain egui row keeps its
            // idiom and gives its height, or its content's where unbounded.
            let h = layout::stretch_height(ui).unwrap_or_else(|| {
                if !ui.layout().is_horizontal() {
                    0.0
                } else if ui.available_height().is_finite() {
                    ui.available_height()
                } else {
                    Some(ui.min_rect().height())
                        .filter(|h| h.is_finite())
                        .unwrap_or(0.0)
                }
            });
            Vec2::new(w, h)
        } else {
            let h = pick(&l, |r| &r.height, st).unwrap_or(t.metrics.border);
            // `w-full`; a rule has no content width (conformance 3.1).
            Vec2::new(capped_width(ui, f32::INFINITY, 0.0), h)
        };
        let (rect, resp) = layout::allocate_item(ui, size, Sense::hover());
        if ui.is_rect_visible(rect) {
            if let Some(c) = pick(&l, |r| &r.bg, st).map(|p| t.paint(p)) {
                part_painter(ui, resp.id, &[&l[..]], st).rect_filled(rect, 0.0, c);
            }
        }
        resp
    }
}

/// A horizontal rule across the available width: shorthand for
/// `ui.add(Separator::new())`.
pub fn separator(ui: &mut Ui) {
    ui.add(Separator::new());
}

/// A vertical rule that stretches in a row and is 0 in a column (see
/// [`SeparatorOrientation::Vertical`]): shorthand for
/// `ui.add(Separator::new().orientation(SeparatorOrientation::Vertical))`.
pub fn separator_vertical(ui: &mut Ui) {
    ui.add(Separator::new().orientation(SeparatorOrientation::Vertical));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{AppVariant, Paint, Role};

    /// An app variant's edit gets `SeparatorRecipes` and changes only what
    /// it names.
    #[test]
    fn edit_compiles_and_changes_its_part() {
        const ACCENT: AppVariant<SeparatorVariant> = SeparatorVariant::app("accent");
        let t = Theme::vega().variant(
            ACCENT,
            SeparatorVariant::Default,
            |r: &mut SeparatorRecipes, _style| r.separator.recolor(Role::Border, Role::Primary),
        );
        let v = t.resolve(SeparatorVariant::from(ACCENT));
        let l = v.part(&t.style, |_, s| &s.separator, |r| &r.separator);
        assert_eq!(l.bg.get(State::NONE), Some(Paint::Role(Role::Primary, 1.0)));
        assert_eq!(l.height, t.style.separator.height);
    }
}
