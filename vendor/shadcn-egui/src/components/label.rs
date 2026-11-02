//! shadcn Label, ported from shadcn-svelte `ui/label/label.svelte` at
//! `tools/stylegen/PIN`. Of the label's base classes only
//! `peer-disabled:cursor-not-allowed` matters in egui: the label of a
//! disabled control shows the not-allowed cursor. Its look comes from the
//! style's `cn-label` recipe. `peer-disabled:opacity-50` arrives as the
//! DISABLED state of that recipe.
//!
//! `description` has no marker of its own in shadcn: it is the muted `text-sm`
//! paragraph that `cn-card-description` and `cn-dialog-description` share, so
//! it reads the `card_description` recipe.

use crate::components::shared::{disabled_cursor, part_painter};
use crate::components::text::TextLook;
use crate::style::app::default_variant;
use crate::style::{pick, Role, State};
use crate::Theme;
use egui::{Response, Sense, TextWrapMode, Ui, Widget};

default_variant!(
    /// The label's variant: upstream has none, so only app variants
    /// ([`LabelVariant::app`]) differ from `Default`.
    LabelVariant,
    /// The label's recipes an app variant's edit changes.
    LabelRecipes {
        /// `data-slot="label"`: the label.
        label,
    }
);

pub struct Label {
    text: String,
    enabled: bool,
    variant: LabelVariant,
}

impl Label {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            enabled: true,
            variant: LabelVariant::Default,
        }
    }
    /// `false` draws the label as the label of a disabled peer control.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    /// An app variant ([`LabelVariant::app`]).
    pub fn variant(mut self, v: impl Into<LabelVariant>) -> Self {
        self.variant = v.into();
        self
    }
}

pub fn label(ui: &mut Ui, text: &str) -> Response {
    ui.add(Label::new(text))
}

impl Widget for Label {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = Theme::current(ui.ctx());
        let v = t.resolve(self.variant);
        let l = [v.part(&t.style, |_, s| &s.label, |r| &r.label)];
        let enabled = self.enabled && ui.is_enabled();
        let st = t.state(ui.ctx(), v.bits().with(State::DISABLED, !enabled));
        let look = TextLook::resolve(ui, &l, st);
        let inherited = t.role(Role::Foreground); // behaviour-ok: inherits Foreground; reads label first
        let fg = pick(&l, |r| &r.text, st).map_or(inherited, |p| t.paint(p));
        let painter = part_painter(ui, ui.next_auto_id(), &[&l[..]], st);

        // The label is as tall as its line box (`leading-none`: the font
        // size).
        let mut block = look.layout(ui, self.text.as_str(), TextWrapMode::Extend, f32::INFINITY);
        let mut size = block.size();
        // In a `wrap_row` a label longer than a whole line is a flex item
        // shrunk to the line (`min-width: auto`, its longest word): it
        // takes the line's width and its text wraps inside.
        let text = self.text.as_str();
        if crate::layout::in_flex_row(ui) {
            // An item of a crate row is a flex item that shrinks to the
            // room the row's other items leave (`flex-shrink: 1`), not
            // below its longest word (`min-width: auto`), as a `vstack`
            // does there; its text wraps at the width the row gives it.
            let min = || look.layout(ui, text, TextWrapMode::Wrap, 0.0).size().x;
            if let Some(w) = crate::layout::flex_leaf(ui, size.x, min).filter(|&w| size.x > w) {
                block = look.layout(ui, text, TextWrapMode::Wrap, w);
                size = egui::vec2(block.size().x.max(w), block.size().y);
            }
        } else if let Some(line) = crate::layout::wrap_line_width(ui).filter(|&w| size.x > w) {
            block = look.layout(ui, text, TextWrapMode::Wrap, line);
            size = egui::vec2(block.size().x.max(line), block.size().y);
        } else if let Some(width) = crate::layout::text_wrap_width(ui).filter(|&w| size.x > w) {
            // Elsewhere it wraps where egui's `Label` wraps (conformance
            // 3.4): at the room left, as the `Ui`'s wrap mode says, and
            // takes that room, as the block it is (a flex item's width).
            block = look.layout(ui, text, ui.wrap_mode(), width);
            size = egui::vec2(block.size().x.max(width), block.size().y);
        }
        let (rect, resp) = crate::layout::allocate_item(ui, size, Sense::hover());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, enabled, &self.text)
        });
        // `peer-disabled:cursor-not-allowed`.
        disabled_cursor(&resp, !enabled);
        if ui.is_rect_visible(rect) {
            block.paint(&painter, rect.min, fg);
        }
        resp
    }
}

/// Muted `text-sm` helper text, wrapping like a paragraph.
pub fn description(ui: &mut Ui, text: &str) -> Response {
    let t = Theme::current(ui.ctx());
    let l = [&*t.style.card_description];
    let st = t.state(ui.ctx(), State::NONE);
    let look = TextLook::resolve(ui, &l, st);
    let muted = t.role(Role::MutedForeground); // behaviour-ok: inherits; reads card_description first
    let look = match look.recipe_color() {
        Some(_) => look,
        None => look.color(muted),
    };
    look.label(ui, text, TextWrapMode::Wrap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{pick, AppVariant, Paint, Role, Style};

    #[test]
    fn disabled_label_recipe_is_half_opacity() {
        for make in [Style::vega as fn() -> Style, Style::nova] {
            let s = make();
            let l = [&*s.label];
            assert_eq!(pick(&l, |r| &r.opacity, State::DISABLED), Some(0.5));
            assert_eq!(pick(&l, |r| &r.opacity, State::NONE), None);
        }
    }

    /// An app variant's edit gets `LabelRecipes` and changes only what it
    /// names.
    #[test]
    fn edit_compiles_and_changes_its_part() {
        const DANGER: AppVariant<LabelVariant> = LabelVariant::app("danger");
        let t = Theme::vega().variant(
            DANGER,
            LabelVariant::Default,
            |r: &mut LabelRecipes, _style| r.label.text.set(Paint::Role(Role::Destructive, 1.0)),
        );
        let v = t.resolve(LabelVariant::from(DANGER));
        let l = v.part(&t.style, |_, s| &s.label, |r| &r.label);
        assert_eq!(
            l.text.get(State::NONE),
            Some(Paint::Role(Role::Destructive, 1.0))
        );
        assert_eq!(l.bg, t.style.label.bg);
    }
}
