//! shadcn Textarea, ported from shadcn-svelte `ui/textarea/textarea.svelte`
//! at `tools/stylegen/PIN`. Every value comes from the style's
//! `cn-textarea` recipe. A frameless egui `TextEdit` sits inside a box this
//! widget paints.
//!
//! The field fills the `Ui`'s width up to `Metrics::field_max_width`
//! (`.desired_width` changes the cap) and grows with its text from the
//! recipe's minimum height (`min-h-16`), as upstream's
//! `field-sizing-content` does; an empty field grows with its hint text.
//! The user cannot resize it. Text wraps at the field's width; Enter starts
//! a new line and Tab moves the focus on, as in the browser. A disabled
//! `Ui` (`ui.add_enabled`) disables the field.
//!
//! ```
//! use shadcn_egui::components::textarea::{textarea, Textarea};
//!
//! fn message(ui: &mut egui::Ui, text: &mut String) {
//!     ui.add(Textarea::new(text).hint_text("Type your message here."));
//!     if textarea(ui, text).changed() {
//!         // the text changed
//!     }
//! }
//! ```

use crate::components::icon_atom::retint;
use crate::components::shared::disabled_cursor;
use crate::components::text::Leading;
use crate::components::text_entry::{within, TextClip, TextEntry};
use crate::style::app::default_variant;
use crate::style::{pick, State};
use crate::{layout, Theme};
use egui::text_edit::TextEditOutput;
use egui::{
    AtomKind, Atoms, IntoAtoms, Response, TextBuffer, TextEdit, TextWrapMode, Ui, Vec2, Widget,
    WidgetText,
};

default_variant!(
    /// The textarea's variant: upstream has none, so only app variants
    /// ([`TextareaVariant::app`]) differ from `Default`.
    TextareaVariant,
    /// The textarea's recipes an app variant's edit changes.
    TextareaRecipes {
        /// `data-slot="textarea"`: the field.
        textarea,
    }
);

/// A multi-line text field bound to `text`, as egui's
/// `TextEdit::multiline`. Place it with `ui.add`, or with
/// [`Textarea::show`] for the `TextEditOutput` (cursor, galley).
#[must_use = "You should put this widget in a ui with `ui.add(widget);`"]
pub struct Textarea<'t> {
    text: &'t mut dyn TextBuffer,
    hint_text: Atoms<'static>,
    desired_width: Option<f32>,
    id_salt: Option<egui::Id>,
    invalid: bool,
    variant: TextareaVariant,
}

impl<'t> Textarea<'t> {
    /// A field that edits `text`: a `&mut String`, or a `&mut &str` that
    /// shows its text and takes no typing.
    #[inline]
    pub fn new(text: &'t mut dyn TextBuffer) -> Self {
        Self {
            text,
            hint_text: Atoms::default(),
            desired_width: None,
            id_salt: None,
            invalid: false,
            variant: TextareaVariant::Default,
        }
    }

    /// Shown while the field is empty, in the style's placeholder colour,
    /// wrapped at the field's width; the empty field grows to hold it.
    /// Text atoms keep the attributes the caller set on them.
    #[doc(alias = "placeholder")]
    #[inline]
    pub fn hint_text(mut self, hint_text: impl IntoAtoms<'static>) -> Self {
        self.hint_text = hint_text.into_atoms();
        self
    }

    /// The width the field asks for, in points, capped to the available
    /// width (default `Metrics::field_max_width`). `f32::INFINITY` fills
    /// the `Ui`'s width.
    #[inline]
    pub fn desired_width(mut self, desired_width: f32) -> Self {
        self.desired_width = Some(desired_width);
        self
    }

    /// A stable id, independent of call order, from any hashable salt, as
    /// `TextEdit::id_salt`. Needed only where a caller must address this
    /// field from outside the widget tree that builds it.
    #[inline]
    pub fn id_salt(mut self, id_salt: impl egui::AsId) -> Self {
        self.id_salt = Some(egui::Id::new(id_salt));
        self
    }

    /// shadcn `aria-invalid`: destructive border and ring.
    #[inline]
    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    /// An app variant ([`TextareaVariant::app`]).
    #[inline]
    pub fn variant(mut self, variant: impl Into<TextareaVariant>) -> Self {
        self.variant = variant.into();
        self
    }

    /// Show the field and return egui's `TextEditOutput` (its `response`,
    /// the laid-out text and the cursor).
    pub fn show(self, ui: &mut Ui) -> TextEditOutput {
        let t = Theme::current(ui.ctx());
        let v = t.resolve(self.variant);
        let l = [v.part(&t.style, |_, s| &s.textarea, |r| &r.textarea)];
        let context = v.bits().with(State::INVALID, self.invalid);
        // A `<textarea>`'s rows are line boxes, placed as inline text.
        let frame = TextEntry::resolve(
            ui,
            &t,
            &l,
            context,
            true,
            Leading::Inline,
            TextClip::Padding,
            "shadcn_egui::textarea",
        );
        let g = frame.geometry;
        debug_assert!(
            pick(&l, |r| &r.field_sizing_content, g).unwrap_or(false),
            "Textarea grows with its content: a recipe without field_sizing_content \
             would need a scrolling field, which the crate does not draw"
        );
        // A `<textarea>` breaks a word wider than the field
        // (`overflow-wrap: break-word`, the browser's default for it).
        let look = frame.look.clone().break_word(true);
        let max_width = self.desired_width.unwrap_or(t.metrics.field_max_width);
        let width = layout::capped_width(ui, max_width, frame.content_min);
        let wrap = frame.content_width(width);

        // `field-sizing-content`: the box holds its text, or its hint text
        // while empty, inside the recipe's height bounds.
        let text_height = look
            .layout(ui, self.text.as_str(), TextWrapMode::Wrap, wrap)
            .size()
            .y;
        let mut hint = self.hint_text;
        let mut hint_height = 0.0_f32;
        if self.text.as_str().is_empty() {
            let color = frame.placeholder;
            hint.map_kind(|kind| match kind {
                AtomKind::Text(text) => {
                    let galley = look.layout_colored(ui, text, TextWrapMode::Wrap, wrap, color);
                    hint_height = hint_height.max(galley.size().y);
                    AtomKind::Text(WidgetText::Galley(within(galley, wrap)))
                }
                AtomKind::Image(image) => AtomKind::Image(retint(image, color)),
                other => other,
            });
        }
        let content = text_height.max(hint_height) + frame.inset[0] + frame.inset[2];
        let height = pick(&l, |r| &r.height, g)
            .unwrap_or(content)
            .max(frame.get(|r| &r.min_height));

        let mut layouter = |ui: &Ui, text: &dyn TextBuffer, wrap_width: f32| {
            let block = look.layout(ui, text.as_str(), TextWrapMode::Wrap, wrap_width);
            within(block.galley().clone(), wrap_width)
        };
        let mut edit = TextEdit::multiline(self.text);
        if !hint.is_empty() {
            edit = edit.hint_text(hint);
        }
        let edit = edit
            .layouter(&mut layouter)
            .desired_width(f32::INFINITY)
            .desired_rows(1)
            .vertical_align(egui::Align::Min);
        let disabled = !frame.enabled;
        let out = frame.show(ui, Vec2::new(width, height), edit, self.id_salt);
        // `disabled:cursor-not-allowed`: unlike the input, the textarea
        // has no `disabled:pointer-events-none` that would let the pointer
        // through.
        disabled_cursor(&out.response.response, disabled);
        out
    }
}

impl Widget for Textarea<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui).response.response
    }
}

/// A [`Textarea`] bound to `text`: `ui.add(Textarea::new(text))`.
pub fn textarea(ui: &mut Ui, text: &mut dyn TextBuffer) -> Response {
    ui.add(Textarea::new(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{AppVariant, Paint, Role};

    /// An app variant's edit gets `TextareaRecipes` and changes only what
    /// it names.
    #[test]
    fn edit_compiles_and_changes_its_part() {
        const DANGER: AppVariant<TextareaVariant> = TextareaVariant::app("danger");
        let t = Theme::vega().variant(
            DANGER,
            TextareaVariant::Default,
            |r: &mut TextareaRecipes, _style| {
                r.textarea.text.set(Paint::Role(Role::Destructive, 1.0))
            },
        );
        let v = t.resolve(TextareaVariant::from(DANGER));
        let l = v.part(&t.style, |_, s| &s.textarea, |r| &r.textarea);
        assert_eq!(
            l.text.get(State::NONE),
            Some(Paint::Role(Role::Destructive, 1.0))
        );
        assert_eq!(l.bg, t.style.textarea.bg);
    }
}
