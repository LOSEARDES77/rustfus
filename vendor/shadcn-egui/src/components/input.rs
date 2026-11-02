//! shadcn Input, ported from shadcn-svelte `ui/input/input.svelte` at
//! `tools/stylegen/PIN`. Every value comes from the style's `cn-input`
//! recipe. A frameless egui `TextEdit` sits inside a box this widget
//! paints.

use crate::components::icon_atom::retint;
use crate::components::text::Leading;
use crate::components::text_entry::{within, TextClip, TextEntry};
use crate::style::app::default_variant;
use crate::style::{pick, State};
use crate::{layout, Theme};
use egui::text::LayoutJob;
use egui::{
    AtomKind, Atoms, IntoAtoms, TextBuffer, TextEdit, TextWrapMode, Ui, Vec2, Widget, WidgetText,
};

default_variant!(
    /// The input's variant: upstream has none, so only app variants
    /// ([`InputVariant::app`]) differ from `Default`.
    InputVariant,
    /// The input's recipes an app variant's edit changes.
    InputRecipes {
        /// `data-slot="input"`: the field.
        input,
    }
);

pub struct Input<'a> {
    text: &'a mut dyn TextBuffer,
    hint: Atoms<'static>,
    password: bool,
    max_width: Option<f32>,
    id_salt: Option<egui::Id>,
    enabled: bool,
    invalid: bool,
    variant: InputVariant,
}

impl<'a> Input<'a> {
    /// A field that edits `text`, any egui [`TextBuffer`] (a `String`, or
    /// a `&str` that shows its text read-only), as `TextEdit::singleline`.
    pub fn new(text: &'a mut dyn TextBuffer) -> Self {
        Self {
            text,
            hint: Atoms::default(),
            password: false,
            max_width: None,
            id_salt: None,
            enabled: true,
            invalid: false,
            variant: InputVariant::Default,
        }
    }
    /// The placeholder, shown while the field is empty, as
    /// `TextEdit::hint_text`: text, rich text or an image, in the style's
    /// placeholder colour where it sets none of its own.
    pub fn hint(mut self, h: impl IntoAtoms<'static>) -> Self {
        self.hint = h.into_atoms();
        self
    }
    pub fn password(mut self, yes: bool) -> Self {
        self.password = yes;
        self
    }
    /// Override the default maximum width (`Metrics::field_max_width`). Pass
    /// `f32::INFINITY` for full-width.
    pub fn max_width(mut self, w: f32) -> Self {
        self.max_width = Some(w);
        self
    }
    /// A stable id, independent of call order, from any hashable salt (a
    /// `&str`, or a `format!("row{i}")` per row). Needed only where a caller
    /// (a test, or `ctx.memory_mut(|m| m.request_focus(..))`) must address
    /// this field from outside the widget tree that builds it; without one,
    /// `TextEdit` falls back to an id keyed on widget position.
    pub fn id_salt(mut self, salt: impl egui::AsId) -> Self {
        self.id_salt = Some(egui::Id::new(salt));
        self
    }
    /// `false` draws the field disabled: no focus, no typing.
    pub fn enabled(mut self, yes: bool) -> Self {
        self.enabled = yes;
        self
    }
    /// shadcn `aria-invalid`: destructive border and ring.
    pub fn invalid(mut self, yes: bool) -> Self {
        self.invalid = yes;
        self
    }
    /// An app variant ([`InputVariant::app`]).
    pub fn variant(mut self, v: impl Into<InputVariant>) -> Self {
        self.variant = v.into();
        self
    }
}

impl<'a> Widget for Input<'a> {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        let t = Theme::current(ui.ctx());
        let v = t.resolve(self.variant);
        let l = [v.part(&t.style, |_, s| &s.input, |r| &r.input)];
        let context = v.bits().with(State::INVALID, self.invalid);
        // The field's text sits in its line box as in a browser's
        // text-entry field: the leading's half above rounds up.
        let mut frame = TextEntry::resolve(
            ui,
            &t,
            &l,
            context,
            self.enabled,
            Leading::Field,
            TextClip::Content,
            "shadcn_egui::input",
        );
        let look = frame.look.clone();
        // No height: CSS's initial `auto`, one line box in the padding box.
        let height = pick(&l, |r| &r.height, frame.geometry)
            .unwrap_or_else(|| look.line_height() + frame.inset[0] + frame.inset[2]);
        // Blink centres the editor line in the content box even when it is
        // taller; egui would clamp it to the content top. Equal cuts from
        // both margins keep the centre and give it the room.
        let content = height - frame.inset[0] - frame.inset[2];
        if let Some(editor) = look.editor_height(ui.ctx()) {
            let cut = ((editor - content) / 2.0).ceil().max(0.0);
            let cut = cut.min(frame.inset[0]).min(frame.inset[2]);
            frame.inset[0] -= cut;
            frame.inset[2] -= cut;
        }

        let password = self.password;
        let mut layouter = |ui: &Ui, text: &dyn TextBuffer, _wrap_width: f32| {
            let text = if password {
                let n = text.as_str().chars().count();
                std::iter::repeat_n(egui::epaint::text::PASSWORD_REPLACEMENT_CHAR, n).collect()
            } else {
                text.as_str().to_owned()
            };
            look.layout(ui, text, TextWrapMode::Extend, f32::INFINITY)
                .galley()
                .clone()
        };
        let max_width = self.max_width.unwrap_or(t.metrics.field_max_width);
        let width = layout::capped_width(ui, max_width, frame.content_min);
        let mut edit = TextEdit::singleline(self.text);
        if !self.hint.is_empty() {
            // The placeholder is laid out in the field's look on one line,
            // its line breaks stripped as the HTML spec says for an input,
            // and not elided (`text-overflow: clip`): the field's clip cuts
            // it at the content box. It is measured in the room the field
            // leaves before the caret's.
            let room = frame.content_width(width) - frame.caret_room(ui.ctx());
            let color = frame.placeholder;
            let mut hint = self.hint;
            hint.map_kind(|kind| match kind {
                AtomKind::Text(text) => {
                    let galley = frame.look.layout_colored_with(
                        ui,
                        text,
                        TextWrapMode::Extend,
                        f32::INFINITY,
                        color,
                        drop_line_breaks,
                    );
                    AtomKind::Text(WidgetText::Galley(within(galley, room)))
                }
                AtomKind::Image(image) => AtomKind::Image(retint(image, color)),
                other => other,
            });
            edit = edit.hint_text(hint);
        }
        let edit = edit
            .layouter(&mut layouter)
            .password(password)
            .vertical_align(egui::Align::Center);
        // No not-allowed cursor while disabled: upstream's
        // `disabled:cursor-not-allowed` comes with
        // `disabled:pointer-events-none`, so the browser shows the cursor
        // of what lies under the field.
        frame
            .show(ui, Vec2::new(width, height), edit, self.id_salt)
            .response
            .response
    }
}

/// An input's placeholder on one line: the HTML spec strips its line
/// breaks.
fn drop_line_breaks(job: &mut LayoutJob) {
    if !job.text.contains(['\r', '\n']) {
        return;
    }
    let mut text = String::with_capacity(job.text.len());
    for section in &mut job.sections {
        let start = text.len();
        text.extend(
            job.text[section.byte_range.start.0..section.byte_range.end.0]
                .chars()
                .filter(|c| !matches!(c, '\r' | '\n')),
        );
        section.byte_range = egui::text::ByteIndex(start)..egui::text::ByteIndex(text.len());
    }
    job.text = text;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{AppVariant, Paint, Role};

    /// An app variant's edit gets `InputRecipes` and changes only what it
    /// names.
    #[test]
    fn edit_compiles_and_changes_its_part() {
        const DANGER: AppVariant<InputVariant> = InputVariant::app("danger");
        let t = Theme::vega().variant(
            DANGER,
            InputVariant::Default,
            |r: &mut InputRecipes, _style| r.input.text.set(Paint::Role(Role::Destructive, 1.0)),
        );
        let v = t.resolve(InputVariant::from(DANGER));
        let l = v.part(&t.style, |_, s| &s.input, |r| &r.input);
        assert_eq!(
            l.text.get(State::NONE),
            Some(Paint::Role(Role::Destructive, 1.0))
        );
        assert_eq!(l.bg, t.style.input.bg);
    }
}
