//! The text path: the one way crate-drawn text resolves its look and sits
//! in its line box (conformance 2.3, 2.4).
//!
//! Each attribute of a piece of crate text resolves in one order: what the
//! caller set on it explicitly (a `RichText` size, family, colour, line
//! height, underline), else the part's recipe, else what the enclosing
//! `Ui` gives: for colour its text colour (CSS's `currentColor`), for size
//! its `TextStyle::Body` size, for the line height the factor a crate
//! container set on its content (see [`content_scope`]). egui's `strong`
//! and `weak` are no explicit colours: such text takes the recipe colour.
//!
//! Text sits in its line box as Blink places it: the font's ascent A and
//! descent D, read from the font's own tables, each rounded to whole
//! device pixels, the baseline `floor((line height - (A + D)) / 2) + A`
//! below the line top, the font's line gap unused. A text-entry field
//! places it as Blink places its editor (see [`Leading`]). Every row of
//! wrapped text follows the same rule, and the laid-out text is as tall as
//! its line boxes.
//!
//! A recipe's `underline` is drawn as Blink draws it: its offset below
//! the baseline (the recipe's `underline_offset`, else Blink's `auto` gap)
//! and a tenth of the font size thick, both on whole device pixels.
//!
//! ```
//! use shadcn_egui::components::text::TextLook;
//! use shadcn_egui::style::State;
//! use shadcn_egui::Theme;
//!
//! fn title(ui: &mut egui::Ui, text: &str) -> egui::Response {
//!     let theme = Theme::current(ui.ctx());
//!     let layers = [&*theme.style.card_title];
//!     let look = TextLook::resolve(ui, &layers, theme.state(ui.ctx(), State::NONE));
//!     let block = look.layout(ui, text, egui::TextWrapMode::Wrap, ui.available_width());
//!     let (rect, resp) = ui.allocate_exact_size(block.size(), egui::Sense::hover());
//!     block.paint(ui.painter(), rect.min, look.resolved_color(ui));
//!     resp
//! }
//! ```

use crate::components::context;
use crate::style::{pick, pick_where, Recipe, State, TextSize, TextWrap, Weight};
use crate::theme::{family_for, heading_family_for};
use crate::Theme;
use egui::text::{LayoutJob, TextWrapping};
use egui::FontData;
use egui::{
    Align, Color32, FontFamily, FontId, FontSelection, Galley, Painter, Pos2, Response, Stroke,
    TextWrapMode, Ui, Vec2, WidgetText,
};
use std::sync::Arc;

mod line_break;
mod shape;
mod wrap;

/// The context a crate container puts on its content `Ui` to pass the
/// text attributes egui widgets cannot take to crate text inside: the
/// line-height factor (`leading-*`), the weight, the tracking and
/// `uppercase`, which inherit in CSS.
#[derive(Clone, Copy)]
struct Inherited {
    line_factor: f32,
    weight: Weight,
    /// In px: CSS computes `letter-spacing` to a length on the element
    /// whose `tracking-*` sets it, and text of another size inherits the
    /// length.
    tracking_px: f32,
    uppercase: bool,
}

/// How a line box's leading (its height minus the font's rounded ascent
/// and descent) splits above and below the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Leading {
    /// Inline text, as Blink's `CalculateLeadingSpace`: the half above is
    /// rounded down to a device pixel.
    #[default]
    Inline,
    /// The inner editor of a text `<input>`: Blink lays it out as one line
    /// of `line-height: normal` (ascent, descent and the font's line gap,
    /// each rounded to device pixels), the baseline half the line gap
    /// (rounded down) plus the ascent below its top. Each row is that
    /// line, not the line box, so a field's caret and selection span it;
    /// the field centres it in its content box (egui's
    /// `Align::Center`), and the painter rounds its top to the device
    /// pixel as Blink snaps the editor.
    Field,
}

/// The resolved text attributes of one part: its recipe stack over what
/// the enclosing `Ui` gives. Build it with [`TextLook::resolve`], lay text
/// out with [`TextLook::layout`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TextLook {
    size: f32,
    line_height: f32,
    weight: Weight,
    heading: bool,
    color: Option<Color32>,
    tracking: f32,
    uppercase: bool,
    underline: bool,
    underline_offset: Option<f32>,
    leading: Leading,
    wrap_style: Option<TextWrap>,
    break_word: bool,
}

impl TextLook {
    /// The text look of the part whose recipes are `layers` (in upstream's
    /// class order) under `state`. What the recipes leave out comes from
    /// `ui`: the size from its `TextStyle::Body`, the line height from the
    /// ratio of the recipes' named `text-*` size, else from the factor of an
    /// enclosing [`content_scope`] (else CSS's 1.5 of the base size), the
    /// weight, tracking and `uppercase` from an enclosing [`content_scope`],
    /// the colour from its text colour at paint time.
    pub fn resolve(ui: &Ui, layers: &[&Recipe], state: State) -> Self {
        let theme = Theme::current(ui.ctx());
        let inherited = context::find::<Inherited>(ui);
        let recipe_size = pick(layers, |r| &r.text_size, state);
        let size = recipe_size.map_or_else(|| body_size(ui), |s| theme.text_px(s));
        // A named `text-*` sets a line height too, as a ratio of its own
        // size: an arbitrary `text-[0.8rem]` over it keeps that ratio, and
        // one alone inherits the line height as other text does.
        let named = pick_where(
            layers,
            |r| &r.text_size,
            |s| !matches!(s, TextSize::Px(_)),
            state,
        );
        let line_height = match (pick(layers, |r| &r.line_height, state), named) {
            (Some(k), _) => k * size,
            (None, Some(n)) if Some(n) == recipe_size => theme.line_px(n),
            (None, Some(n)) => theme.line_px(n) / theme.text_px(n) * size,
            (None, None) => inherited.map_or_else(
                || theme.line_px(TextSize::Px(size)),
                |i| i.line_factor * size,
            ),
        };
        Self {
            size,
            line_height,
            weight: pick(layers, |r| &r.weight, state)
                .or(inherited.map(|i| i.weight))
                .unwrap_or(Weight::Normal),
            heading: false,
            color: pick(layers, |r| &r.text, state).map(|p| theme.paint(p)),
            tracking: pick(layers, |r| &r.tracking, state)
                .or(inherited.map(|i| i.tracking_px / size))
                .unwrap_or(0.0),
            uppercase: pick(layers, |r| &r.uppercase, state)
                .or(inherited.map(|i| i.uppercase))
                .unwrap_or(false),
            underline: pick(layers, |r| &r.underline, state) == Some(true),
            underline_offset: pick(layers, |r| &r.underline_offset, state),
            leading: Leading::Inline,
            wrap_style: pick(layers, |r| &r.text_wrap, state),
            break_word: false,
        }
    }

    /// `cn-font-heading`: the theme's heading font instead of the body
    /// font.
    #[inline]
    pub fn heading(mut self, heading: bool) -> Self {
        self.heading = heading;
        self
    }

    /// How the leading splits around the text; [`Leading::Field`] for a
    /// text-entry field.
    #[inline]
    pub fn leading(mut self, leading: Leading) -> Self {
        self.leading = leading;
        self
    }

    /// `overflow-wrap: break-word`, a `<textarea>`'s UA default: wrapped
    /// text breaks a word too wide for a line of its own between two of
    /// its chars, so the text fits its width.
    #[inline]
    pub(crate) fn break_word(mut self, break_word: bool) -> Self {
        self.break_word = break_word;
        self
    }

    /// The colour of text without an explicit colour, in place of the
    /// recipe's (a part whose colour comes from another part's state).
    #[inline]
    pub fn color(mut self, color: Color32) -> Self {
        self.color = Some(color);
        self
    }

    /// Whether the text is underlined (`underline`), as a part whose
    /// underline follows its state lays it out again when it changes.
    #[inline]
    pub fn underlined(&self) -> bool {
        self.underline
    }

    /// How [`place_rows`] places and decorates this look's rows.
    fn place(&self) -> Place {
        Place {
            leading: self.leading,
            underline: self.underline.then_some(Underline {
                offset: self.underline_offset,
            }),
        }
    }

    /// The font size in px.
    #[inline]
    pub fn size(&self) -> f32 {
        self.size
    }

    /// The line box height in px.
    #[inline]
    pub fn line_height(&self) -> f32 {
        self.line_height
    }

    /// The font of text without an explicit size or family.
    pub fn font_id(&self, ctx: &egui::Context) -> FontId {
        let family = if self.heading {
            heading_family_for(ctx, self.weight)
        } else {
            family_for(ctx, self.weight)
        };
        FontId::new(self.size, family)
    }

    /// The height of a field's editor line in points: one line of
    /// `line-height: normal`, the font's ascent, descent and line gap,
    /// each rounded to device pixels.
    pub(crate) fn editor_height(&self, ctx: &egui::Context) -> Option<f32> {
        let ppp = ctx.pixels_per_point();
        let em = family_vmetrics(ctx, &self.font_id(ctx).family)?;
        let [a, d, gap] =
            [em.ascent, em.descent, em.line_gap].map(|v| (v * self.size * ppp).round());
        Some((a + d + gap) / ppp)
    }

    /// Where Blink paints the caret in a row, in points below the row's
    /// top: the font's ascent and descent (each rounded to device pixels)
    /// around the baseline. In a field's editor line ([`Leading::Field`])
    /// the baseline sits half the line gap plus the ascent below the top;
    /// in a line box ([`Leading::Inline`], a textarea's rows) half the
    /// leading, rounded down, plus the ascent, where the text sits.
    pub(crate) fn caret_span(&self, ctx: &egui::Context) -> Option<std::ops::Range<f32>> {
        let ppp = ctx.pixels_per_point();
        let em = family_vmetrics(ctx, &self.font_id(ctx).family)?;
        let [a, d, gap] =
            [em.ascent, em.descent, em.line_gap].map(|v| (v * self.size * ppp).round());
        let top = match self.leading {
            Leading::Field => (gap / 2.0).floor(),
            Leading::Inline => ((self.line_height * ppp - a - d) / 2.0).floor(),
        };
        Some(top / ppp..(top + a + d) / ppp)
    }

    /// The colour of text without an explicit colour: the recipe's, else
    /// `ui`'s text colour.
    pub fn resolved_color(&self, ui: &Ui) -> Color32 {
        self.color.unwrap_or_else(|| ui.visuals().text_color())
    }

    /// The recipe's colour alone, if it sets one.
    #[inline]
    pub fn recipe_color(&self) -> Option<Color32> {
        self.color
    }

    /// `text` as a layout job: the caller's explicit attributes kept, the
    /// recipe's filled in where the caller set none. Text without an
    /// explicit colour (`strong` and `weak` text included) is
    /// `Color32::PLACEHOLDER`, which the painter replaces with the colour
    /// passed at paint time. Sections without an explicit line height get
    /// the look's.
    fn job(&self, ui: &Ui, text: WidgetText) -> LayoutJob {
        let ctx = ui.ctx();
        let font = self.font_id(ctx);
        let mut job = match text {
            WidgetText::Text(s) => {
                LayoutJob::simple_format(s, egui::TextFormat::simple(font, Color32::PLACEHOLDER))
            }
            WidgetText::RichText(rich) => {
                let style = placeholder_style(ctx);
                WidgetText::RichText(rich)
                    .into_layout_job(&style, FontSelection::FontId(font), Align::BOTTOM)
                    .as_ref()
                    .clone()
            }
            WidgetText::LayoutJob(job) => job.as_ref().clone(),
            WidgetText::Galley(galley) => galley.job.as_ref().clone(),
        };
        if self.uppercase {
            uppercase(&mut job);
        }
        for section in &mut job.sections {
            let f = &mut section.format;
            if f.line_height.is_none() {
                f.line_height = Some(self.line_height);
            }
            if f.extra_letter_spacing == 0.0 {
                f.extra_letter_spacing = self.tracking * f.font_id.size;
            }
        }
        job
    }

    /// Lay `text` out in this look. `wrap` and `width` say where it wraps
    /// (`TextWrapMode::Wrap` at the available width is where the browser
    /// wraps). Paint the result with [`TextBlock::paint`]. A laid-out
    /// galley the caller passes is laid out again in this look, its own
    /// attributes kept, so its rows sit in their line boxes too.
    pub fn layout(
        &self,
        ui: &Ui,
        text: impl Into<WidgetText>,
        wrap: TextWrapMode,
        width: f32,
    ) -> TextBlock {
        let job = self.job(ui, text.into());
        let source = self.lay_out(ui.ctx(), job, wrap, width);
        TextBlock {
            galley: place_rows(ui.ctx(), source.clone(), self.place(), 0.0),
            source,
            place: self.place(),
        }
    }

    /// Show `text` in this look as an egui `Label` (selectable as egui's
    /// labels are, with the label's accessibility info): wrapped at the
    /// available width unless `wrap` says otherwise, each row placed as
    /// the browser places it, in the look's colour.
    pub fn label(&self, ui: &mut Ui, text: impl Into<WidgetText>, wrap: TextWrapMode) -> Response {
        let mut job = self.job(ui, text.into());
        fill_color(&mut job, self.resolved_color(ui));
        job.halign = ui.layout().horizontal_placement();
        job.justify = ui.layout().horizontal_justify();
        let source = self.lay_out(ui.ctx(), job, wrap, ui.available_width());
        let galley = place_rows(ui.ctx(), source.clone(), self.place(), 0.0);
        // `egui::Label`'s own `ui`, with the rows placed for the origin the
        // label gets (the placement moves glyphs only, not the size).
        let (pos, galley, mut response) = egui::Label::new(galley).layout_in_ui(ui);
        let galley = if galley.elided {
            galley
        } else {
            place_rows(ui.ctx(), source, self.place(), pos.y)
        };
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), galley.text())
        });
        if ui.is_rect_visible(response.rect) {
            if galley.elided {
                let job = LayoutJob {
                    sections: galley.job.sections.clone(),
                    text: galley.job.text.clone(),
                    ..LayoutJob::default()
                };
                response = response.on_hover_text(job);
            }
            let color = ui.style().visuals.text_color();
            let underline = if response.has_focus() || response.highlighted() {
                Stroke::new(1.0_f32, color) // behaviour-ok: egui Label's focus underline
            } else {
                Stroke::NONE
            };
            if ui.style().interaction.selectable_labels {
                egui::text_selection::LabelSelectionState::label_text_selection(
                    ui, &response, pos, galley, color, underline,
                );
            } else {
                ui.painter().add(
                    egui::epaint::TextShape::new(pos, galley, color).with_underline(underline),
                );
            }
        }
        response
    }
}

impl TextLook {
    /// [`TextLook::layout`] with every colour the caller did not set
    /// replaced by `color` in the galley itself, for a galley egui paints
    /// (a `TextEdit`'s hint text).
    pub(crate) fn layout_colored(
        &self,
        ui: &Ui,
        text: impl Into<WidgetText>,
        wrap: TextWrapMode,
        width: f32,
        color: Color32,
    ) -> Arc<Galley> {
        self.layout_colored_with(ui, text, wrap, width, color, |_| ())
    }

    /// [`TextLook::layout_colored`] with `edit` applied to the job before
    /// it is laid out (an input's hint drops its line breaks).
    pub(crate) fn layout_colored_with(
        &self,
        ui: &Ui,
        text: impl Into<WidgetText>,
        wrap: TextWrapMode,
        width: f32,
        color: Color32,
        edit: impl FnOnce(&mut LayoutJob),
    ) -> Arc<Galley> {
        let mut job = self.job(ui, text.into());
        edit(&mut job);
        fill_color(&mut job, color);
        let galley = self.lay_out(ui.ctx(), job, wrap, width);
        place_rows(ui.ctx(), galley, self.place(), 0.0)
    }
}

impl TextLook {
    /// `job` laid out by egui with Chrome's glyph positions (see
    /// [`shape`]), wrapped as `wrap` and `width` say. Wrapped text breaks
    /// where Blink breaks it (see [`wrap`]): trailing spaces hang, and
    /// under `text-balance` or `text-pretty` the lines follow those
    /// breakers. Balance applies to the first paragraph only (Blink
    /// balances from the block's first line), pretty to each. The breaks
    /// become forced ones for egui; the galley keeps the caller's text.
    /// Results are cached per
    /// job, width, mode and scale (notes §1), the measured advances per job
    /// and scale: a new width breaks the lines again without measuring.
    fn lay_out(
        &self,
        ctx: &egui::Context,
        job: LayoutJob,
        wrap: TextWrapMode,
        width: f32,
    ) -> Arc<Galley> {
        use std::hash::{Hash, Hasher};
        let ppp = ctx.pixels_per_point();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        job.hash(&mut hasher);
        ppp.to_bits().hash(&mut hasher);
        // A theme switch keeps the family names for other faces.
        let faces = shape::section_faces(ctx, &job);
        for face in &faces {
            face.as_ref()
                .map(|f| Arc::as_ptr(f) as usize)
                .hash(&mut hasher);
        }
        let measured_key = hasher.finish();
        let style = self.wrap_style.map(|s| s as u8);
        (wrap as u8, width.to_bits(), style, self.break_word).hash(&mut hasher);
        let key = hasher.finish();
        let id = egui::Id::new("shadcn_egui::text::laid_out");
        let frame = ctx.cumulative_frame_nr();
        let hit = ctx.data_mut(|d| {
            let cache = d.get_temp_mut_or_default::<LayoutCache>(id);
            if cache.frame != frame {
                cache.entries.retain(|_, e| e.1 + 1 >= frame);
                cache.measured.retain(|_, e| e.1 + 1 >= frame);
                cache.frame = frame;
            }
            if let Some(e) = cache.entries.get_mut(&key) {
                e.1 = frame;
                return Ok(e.0.clone());
            }
            Err(cache.measured.get_mut(&measured_key).map(|e| {
                e.1 = frame;
                e.0.clone()
            }))
        });
        let measured = match hit {
            Ok(galley) => return galley,
            Err(measured) => measured,
        };
        let adv = measured.unwrap_or_else(|| Arc::new(shape::advances(ctx, &job, &faces)));
        let out = self.shape_and_lay_out(ctx, job, &faces, &adv, wrap, width);
        ctx.data_mut(|d| {
            let cache = d.get_temp_mut_or_default::<LayoutCache>(id);
            cache.entries.insert(key, (out.clone(), frame));
            cache.measured.insert(measured_key, (adv, frame));
        });
        out
    }

    /// [`TextLook::lay_out`] past its cache: `job` with its advances
    /// `adv` ([`shape::advances`]); `faces` holds each section's face data
    /// ([`shape::section_faces`]).
    fn shape_and_lay_out(
        &self,
        ctx: &egui::Context,
        job: LayoutJob,
        faces: &[Option<Arc<egui::FontData>>],
        adv: &shape::Advances,
        wrap: TextWrapMode,
        width: f32,
    ) -> Arc<Galley> {
        let mut adv = adv.clone();
        let chars: Vec<char> = job.text.chars().collect();
        let n = chars.len();
        let breaking = wrap == TextWrapMode::Wrap && width.is_finite();
        let mut section_of = Vec::with_capacity(n);
        for (si, s) in job.sections.iter().enumerate() {
            section_of.extend(std::iter::repeat_n(
                si,
                job.text[s.byte_range.start.0..s.byte_range.end.0].chars().count(),
            ));
        }
        // `starts[i]`: a line starts at char `i` after a chosen break.
        let mut starts = vec![false; n + 1];
        if breaking {
            let ppp = ctx.pixels_per_point();
            let size = job
                .sections
                .first()
                .map_or(self.size, |s| s.format.font_id.size);
            let space = job.sections.first().map_or(0.0, |s| {
                ctx.fonts_mut(|f| f.glyph_width(&s.format.font_id, ' '))
            });
            let mut first = 0;
            for (p, para) in chars.split(|c| *c == '\n').enumerate() {
                // Each char's left and right edge on the paragraph's line in
                // device px, as Blink's `ShapeResult` holds them: glyph
                // advances (with the kerning against the next glyph) summed
                // in 16.16 fixed point, a char's position rounded up to a
                // `LayoutUnit`. A right edge before a kerned pair is the
                // glyph shaped alone: its rounded left plus its own advance.
                // The greedy breaker takes the same edges unrounded.
                let step = |at: usize| i64::from(adv.fixed[at]) + shape::to_fixed(adv.spacing[at]);
                let lu = shape::ceil_to_layout_unit;
                let exact = |v: i64| v as f64 / f64::from(shape::FIXED_ONE);
                let mut x: i64 = 0;
                let edges: Vec<wrap::Glyph> = para
                    .iter()
                    .enumerate()
                    .map(|(i, &c)| {
                        let at = first + i;
                        let next_kern = if i + 1 < para.len() {
                            adv.fixed_kern[at + 1]
                        } else {
                            0
                        };
                        let left = lu(x);
                        let right = if next_kern == 0 {
                            lu(x + step(at))
                        } else {
                            left + step(at) as f32 / shape::FIXED_ONE
                        };
                        let (x_left, x_end) = (exact(x), exact(x + step(at)));
                        x += step(at) + i64::from(next_kern);
                        let hyphen = adv.hyphen[at].map_or(0.0, |(_, w)| lu(i64::from(w)));
                        wrap::Glyph {
                            c,
                            left,
                            right,
                            x: x_left,
                            x_end,
                            hyphen,
                            item: i > 0 && section_of.get(at) != section_of.get(at - 1),
                        }
                    })
                    .collect();
                let cands = wrap::candidates(&edges);
                let style = match self.wrap_style {
                    Some(TextWrap::Balance) if p == 0 => Some(wrap::Style::Balance),
                    Some(TextWrap::Pretty) => Some(wrap::Style::Pretty),
                    _ => None,
                };
                let found = style
                    .and_then(|s| {
                        wrap::breaks(&cands, s, width * ppp, size * ppp, ppp, space * ppp, n)
                    })
                    .unwrap_or_else(|| {
                        if self.break_word {
                            wrap::break_word(&wrap::candidates_with(&edges, true), width * ppp)
                        } else {
                            wrap::normal(&cands, width * ppp)
                        }
                    });
                for s in found {
                    starts[first + s] = true;
                }
                first += para.len() + 1;
            }
        }
        // A soft hyphen that ends a line is its hyphen there.
        for i in 0..n {
            if let (Some((_, w)), true) = (adv.hyphen[i], starts[i + 1]) {
                adv.chrome[i] = w as f32 / shape::FIXED_ONE / ctx.pixels_per_point();
            }
        }
        // One section per char, each moved by the gap that puts it where
        // Chrome puts it: egui advances the char before by its own width,
        // the section's leading space adds the rest and the kerning.
        // egui draws outlines at the coordinates it is given; Chrome at
        // `opsz` = the font size where the face has the axis.
        let opsz: Vec<bool> = job
            .sections
            .iter()
            .zip(faces)
            .map(|(s, face)| {
                face.as_deref()
                    .is_some_and(|d| shape::needs_opsz(d, s.format.coords.as_ref()))
            })
            .collect();
        let mut out = LayoutJob {
            sections: Vec::with_capacity(n),
            text: String::with_capacity(job.text.len() + 8),
            ..job.clone()
        };
        out.wrap = if breaking {
            let mut w = TextWrapping::no_max_width();
            if job.justify {
                w.max_width = width;
            }
            w
        } else {
            TextWrapping::from_wrap_mode_and_width(wrap, width)
        };
        // The original char behind each char of `out`, for the row widths,
        // and the exact x of its pen on its line, in points.
        let mut origin: Vec<Option<usize>> = Vec::with_capacity(n);
        let mut pens: Vec<f32> = Vec::with_capacity(n);
        let mut pen = 0.0;
        let mut line_start = true;
        let mut prev_section = usize::MAX;
        for (i, &c) in chars.iter().enumerate() {
            let si = section_of.get(i).copied().unwrap_or(0);
            let section = &job.sections[si];
            if starts[i] && !(i > 0 && chars[i - 1] == ' ') {
                // After a hyphen: a forced break goes into egui's job only.
                let at = out.text.len();
                out.text.push('\n');
                out.sections.push(egui::text::LayoutSection {
                    leading_space: 0.0,
                    byte_range: egui::text::ByteIndex(at)..egui::text::ByteIndex(at + 1),
                    format: job.sections[section_of[i - 1]].format.clone(),
                });
                origin.push(None);
                pens.push(0.0);
                line_start = true;
            }
            // egui's job draws a soft hyphen as its hyphen where it ends a
            // line, and every char Chrome draws as nothing (a soft hyphen
            // elsewhere too) as a zero-width space, which egui hides: one
            // char each, so the indices stay the caller's.
            let c = match adv.hyphen[i] {
                Some((hyphen, _)) if starts[i + 1] => hyphen,
                _ if c == ' ' && starts[i + 1] => '\n',
                _ if shape::ignorable(c) => '\u{200B}',
                _ => c,
            };
            let mut gap = if line_start {
                0.0
            } else {
                adv.chrome[i - 1] - adv.egui[i - 1] + adv.kern[i]
            };
            if si != prev_section {
                gap += section.leading_space;
                prev_section = si;
            }
            let mut format = section.format.clone();
            format.extra_letter_spacing = 0.0;
            if opsz[si] {
                format.coords.push(b"opsz", format.font_id.size);
            }
            let at = out.text.len();
            out.text.push(c);
            out.sections.push(egui::text::LayoutSection {
                leading_space: gap,
                byte_range: egui::text::ByteIndex(at)..egui::text::ByteIndex(out.text.len()),
                format,
            });
            origin.push(Some(i));
            // egui's own pen: the advance it gives the char before, then
            // the gap (Chrome's advance and the kerning).
            pen = if line_start {
                gap
            } else {
                pen + adv.egui[i - 1] + gap
            };
            pens.push(pen);
            line_start = c == '\n';
        }
        if out.sections.is_empty() {
            out.sections = job.sections.clone();
        }
        let justify = out.justify;
        let mut galley = ctx.fonts_mut(|f| f.layout_job(out));
        // egui draws each glyph at its pen, a quarter device pixel binned,
        // but keeps the pen cut to the device pixel below as the glyph's x
        // (the caret, the selection and the row width follow it). Unless
        // egui justified the row (and placed it again), each glyph keeps
        // its exact pen, and a row ends at Chrome's right edge of its last
        // char.
        let g = Arc::make_mut(&mut galley);
        let mut at = 0;
        let mut right = f32::NEG_INFINITY;
        for placed in &mut g.rows {
            let glyphs = placed.row.glyphs.len();
            if !justify {
                let row = Arc::make_mut(&mut placed.row);
                for (k, glyph) in row.glyphs.iter_mut().enumerate() {
                    if let Some(&x) = pens.get(at + k) {
                        glyph.pos.x = x;
                    }
                }
                if let Some(&Some(i)) = glyphs.checked_sub(1).and_then(|k| origin.get(at + k)) {
                    row.size.x = pens[at + glyphs - 1] + adv.chrome[i];
                }
            }
            right = right.max(placed.pos.x + placed.row.size.x);
            at += glyphs + usize::from(placed.ends_with_newline);
            // A row ended by an inserted `\n` ends as egui's own wrapped
            // rows do: the caller's text has no char there.
            if placed.ends_with_newline && origin.get(at - 1) == Some(&None) {
                placed.ends_with_newline = false;
            }
        }
        if job.halign == Align::LEFT && right.is_finite() {
            g.rect.max.x = g.rect.min.x + right;
        }
        // The galley's text, char indices, selection and copied text are
        // the caller's.
        g.job = Arc::new(job);
        galley
    }
}

/// The galleys [`TextLook::lay_out`] laid out in this frame and the one
/// before: by the hash of the job, width, mode, scale and faces, the galley
/// and the frame it was last used; by the hash of the job, scale and faces,
/// the job's advances and the frame they were last used.
#[derive(Clone, Default)]
struct LayoutCache {
    frame: u64,
    entries: std::collections::HashMap<u64, (Arc<Galley>, u64)>,
    measured: std::collections::HashMap<u64, (Arc<shape::Advances>, u64)>,
}

/// Every placeholder colour of `job` (text and underline) set to `color`.
fn fill_color(job: &mut LayoutJob, color: Color32) {
    for section in &mut job.sections {
        if section.format.color == Color32::PLACEHOLDER {
            section.format.color = color;
        }
        if section.format.underline.color == Color32::PLACEHOLDER {
            section.format.underline.color = color;
        }
    }
}

/// Text laid out by [`TextLook::layout`].
#[derive(Clone, Debug)]
pub struct TextBlock {
    /// The galley as egui laid it out, before [`place_rows`].
    source: Arc<Galley>,
    place: Place,
    galley: Arc<Galley>,
}

impl TextBlock {
    /// The size of the line boxes: the text's width, the line height times
    /// the rows.
    pub fn size(&self) -> Vec2 {
        self.galley.size()
    }

    /// The laid-out galley, its rows placed in their line boxes for an
    /// origin on a whole device pixel ([`TextBlock::paint`] places them for
    /// its own origin).
    pub fn galley(&self) -> &Arc<Galley> {
        &self.galley
    }

    /// Paint the text with its first line box's top left at `pos`. `color`
    /// replaces every colour the caller did not set (see
    /// [`TextLook::resolved_color`]).
    pub fn paint(&self, painter: &Painter, pos: Pos2, color: Color32) {
        let galley = place_rows(painter.ctx(), self.source.clone(), self.place, pos.y);
        painter.galley(pos, galley, color);
    }
}

/// Run `add` in a child `Ui` whose text a crate container's recipe sets
/// (upstream's text colour, `text-*`, `leading-*`, `font-*`, `tracking-*`
/// and `uppercase` on the container): plain egui widgets inside take the
/// colour and the size, crate text inside takes them all through
/// [`TextLook::resolve`], below its own recipe's values. What the recipe
/// leaves out stays as `ui` has it.
pub fn content_scope<R>(
    ui: &mut Ui,
    look: &TextLook,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let builder = egui::UiBuilder::new().ui_stack_info(context::inherited_info(
        egui::UiStackInfo::default(),
        Inherited {
            line_factor: look.line_height / look.size,
            weight: look.weight,
            tracking_px: look.tracking * look.size,
            uppercase: look.uppercase,
        },
    ));
    ui.scope_builder(builder, |ui| {
        if let Some(c) = look.color {
            ui.visuals_mut().override_text_color = Some(c);
        }
        for style in [egui::TextStyle::Body, egui::TextStyle::Button] {
            if let Some(font) = ui.style_mut().text_styles.get_mut(&style) {
                font.size = look.size;
            }
        }
        add(ui)
    })
}

fn body_size(ui: &Ui) -> f32 {
    egui::TextStyle::Body.resolve(ui.style()).size
}

/// The context's style with every colour egui would resolve for text
/// without an explicit colour (the override, `strong`, `weak`, the
/// underline's line colour) set to `Color32::PLACEHOLDER`, and no font
/// override. Built once per context style, not per frame.
fn placeholder_style(ctx: &egui::Context) -> Arc<egui::Style> {
    let id = egui::Id::new("shadcn_egui::text::placeholder_style");
    let source = ctx.global_style();
    let cached = ctx.data(|d| d.get_temp::<(Arc<egui::Style>, Arc<egui::Style>)>(id));
    if let Some((_, style)) = cached.filter(|(src, _)| Arc::ptr_eq(src, &source)) {
        return style;
    }
    let mut style = (*source).clone();
    style.override_font_id = None;
    style.override_text_style = None;
    let v = &mut style.visuals;
    v.override_text_color = None;
    v.weak_text_color = Some(Color32::PLACEHOLDER);
    v.widgets.active.fg_stroke.color = Color32::PLACEHOLDER;
    v.widgets.noninteractive.fg_stroke.color = Color32::PLACEHOLDER;
    let style = Arc::new(style);
    ctx.data_mut(|d| d.insert_temp(id, (source, style.clone())));
    style
}

/// A face's ascent, descent and line gap in em, as Skia picks them: OS/2
/// typo metrics when `USE_TYPO_METRICS` is set, else hhea. A line box with
/// a set line height leaves the line gap out; only a field's editor, a
/// `line-height: normal` line, uses it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct VMetrics {
    ascent: f32,
    descent: f32,
    line_gap: f32,
}

/// The [`VMetrics`] of face `index` in the font file `data` (TrueType,
/// OpenType or a collection), from its `head`, `hhea` and `OS/2` tables.
/// `None` for a file that is not a font or is cut short. Each read stays
/// inside its table: a short `OS/2` table without the typo fields counts
/// as none. A font whose `hhea` ascender and descender are both 0 takes
/// the typo metrics, else the win metrics, as FreeType does.
fn vmetrics(data: &[u8], index: u32) -> Option<VMetrics> {
    let bytes = |o: usize, n: usize| data.get(o..o.checked_add(n)?);
    let u16_at = |o: usize| bytes(o, 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let u32_at = |o: usize| bytes(o, 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let base = if bytes(0, 4)? == b"ttcf" {
        // numFonts at 8, the offset table of each face from 12.
        if index >= u32_at(8)? {
            return None;
        }
        let rec = usize::try_from(index)
            .ok()?
            .checked_mul(4)?
            .checked_add(12)?;
        usize::try_from(u32_at(rec)?).ok()?
    } else {
        0
    };
    let tables = usize::from(u16_at(base.checked_add(4)?)?);
    // A table as its bytes, from its record: tag, checksum, offset, length.
    let table = |tag: &[u8; 4]| {
        (0..tables).find_map(|i| {
            let rec = base.checked_add(12 + 16 * i)?;
            if bytes(rec, 4)? != tag {
                return None;
            }
            let offset = usize::try_from(u32_at(rec.checked_add(8)?)?).ok()?;
            bytes(offset, usize::try_from(u32_at(rec.checked_add(12)?)?).ok()?)
        })
    };
    let be16 = |t: &[u8], o: usize| t.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let i16_in = |t: &[u8], o: usize| be16(t, o).map(|v| f32::from(v as i16));
    let upem = f32::from(be16(table(b"head")?, 18)?);
    let hhea = table(b"hhea")?;
    let (mut ascent, mut descent, mut gap) =
        (i16_in(hhea, 4)?, -i16_in(hhea, 6)?, i16_in(hhea, 8)?);
    // sTypoAscender, sTypoDescender, sTypoLineGap; a short table lacks them.
    let os2 = table(b"OS/2");
    let typo = os2.and_then(|t| Some((i16_in(t, 68)?, -i16_in(t, 70)?, i16_in(t, 72)?)));
    if ascent == 0.0 && descent == 0.0 {
        let win = os2.and_then(|t| Some((f32::from(be16(t, 74)?), f32::from(be16(t, 76)?))));
        match (typo, win) {
            (Some((a, d, g)), _) if a != 0.0 || d != 0.0 => (ascent, descent, gap) = (a, d, g),
            (_, Some((a, d))) => (ascent, descent, gap) = (a, d, 0.0),
            _ => {}
        }
    }
    // fsSelection bit 7, USE_TYPO_METRICS.
    let use_typo = os2
        .and_then(|t| be16(t, 62))
        .is_some_and(|sel| sel & 0x80 != 0);
    if let (true, Some(t)) = (use_typo, typo) {
        (ascent, descent, gap) = t;
    }
    (upem > 0.0).then_some(VMetrics {
        ascent: ascent / upem,
        descent: descent / upem,
        line_gap: gap.max(0.0) / upem,
    })
}

/// The metrics of the first face of `family`, read from the context's
/// font definitions once per face. The cache keeps the face's data with
/// the metrics: a theme that switches fonts reuses the key names for
/// other faces.
fn family_vmetrics(ctx: &egui::Context, family: &FontFamily) -> Option<VMetrics> {
    let (id, data) = ctx.fonts(|f| {
        let defs = f.definitions();
        let key = defs.families.get(family)?.first()?;
        let id = egui::Id::new("shadcn_egui::text::vmetrics").with(key);
        Some((id, defs.font_data.get(key)?.clone()))
    })?;
    let cached = ctx.data(|d| d.get_temp::<(Arc<FontData>, Option<VMetrics>)>(id));
    if let Some((_, m)) = cached.filter(|(src, _)| Arc::ptr_eq(src, &data)) {
        return m;
    }
    let m = vmetrics(&data.font, data.index);
    ctx.data_mut(|d| d.insert_temp(id, (data, m)));
    m
}

/// What [`place_rows`] places a galley's rows by, besides the galley.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Place {
    leading: Leading,
    /// The look's `underline`, drawn under every section without an
    /// underline of its own.
    underline: Option<Underline>,
}

/// `text-decoration: underline` as Blink draws it, with
/// `text-decoration-thickness: auto` (see [`underline_dev`]).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Underline {
    /// `text-underline-offset` in px; `None` is `auto`.
    offset: Option<f32>,
}

/// Blink's underline under text of `size` px on a screen of `ppp` device
/// pixels per point, in device px: its top below the baseline and its
/// thickness. The thickness is `TextDecorationThickness::Resolve` for
/// `auto` (a tenth of the font size, at least one device pixel), painted
/// on whole device pixels (rounded down, as Chrome 156 paints it). The
/// top is `TextDecorationOffset::ComputeUnderlineOffset` for
/// `text-underline-position: auto`: the offset rounded to the device
/// pixel, or for `auto` a gap of half the unrounded thickness, rounded
/// up, at least one device pixel.
fn underline_dev(size: f32, underline: Underline, ppp: f32) -> (f32, f32) {
    // `auto` thickness: `font_size / 10.0f`.
    const AUTO_THICKNESS_DIVISOR: f32 = 10.0; // behaviour-ok: Blink's auto thickness
    let thickness = (size * ppp / AUTO_THICKNESS_DIVISOR).max(1.0); // behaviour-ok: one device px
    let top = match underline.offset {
        Some(px) => (px * ppp).round(),
        None => (thickness / 2.0).ceil().max(1.0), // behaviour-ok: Blink's minimum gap
    };
    (top, thickness.floor())
}

/// A galley placed by [`place_rows`], with what it was placed from.
#[derive(Clone)]
struct Placed {
    source: Arc<Galley>,
    ppp: f32,
    place: Place,
    placed: Arc<Galley>,
    frame: u64,
}

/// A [`PlacedCache`] key: the source galley's address, the device-pixel
/// fraction of the origin it is painted at, the leading and the
/// underline offset's bits.
type PlacedKey = (usize, u32, Leading, Option<Option<u32>>);

/// The galleys [`place_rows`] placed in this frame and the one before, by
/// the address of the galley egui laid out. egui's galley cache hands out
/// the same galley while the text stays the same, so an idle widget's text
/// is placed once, not once per frame.
#[derive(Clone, Default)]
struct PlacedCache {
    frame: u64,
    entries: std::collections::HashMap<PlacedKey, Placed>,
}

/// [`place_galley_rows`] for a galley painted with its top at `origin_y`
/// (points; 0 where the origin is not known yet), cached per galley egui
/// laid out and origin fraction (notes §1: an idle widget allocates
/// nothing beyond egui's own).
fn place_rows(
    ctx: &egui::Context,
    galley: Arc<Galley>,
    place: Place,
    origin_y: f32,
) -> Arc<Galley> {
    let id = egui::Id::new("shadcn_egui::text::placed");
    let (ppp, frame) = (ctx.pixels_per_point(), ctx.cumulative_frame_nr());
    // The tessellator rounds the galley origin to the pixel; Blink rounds
    // origin and baseline together. The fraction it drops goes into the
    // baselines.
    let frac = origin_y * ppp - (origin_y * ppp).round();
    let key = (
        Arc::as_ptr(&galley) as usize,
        frac.to_bits(),
        place.leading,
        place.underline.map(|u| u.offset.map(f32::to_bits)),
    );
    let hit = ctx.data_mut(|d| {
        let cache = d.get_temp_mut_or_default::<PlacedCache>(id);
        if cache.frame != frame {
            // Text not shown in the last frame is gone from egui's cache too.
            cache.entries.retain(|_, e| e.frame + 1 >= frame);
            cache.frame = frame;
        }
        let e = cache.entries.get_mut(&key)?;
        let same = Arc::ptr_eq(&e.source, &galley) && e.ppp == ppp && e.place == place;
        same.then(|| {
            e.frame = frame;
            e.placed.clone()
        })
    });
    if let Some(placed) = hit {
        return placed;
    }
    // Placing reads the font tables through `ctx.data`: not inside the
    // `data_mut` above.
    let placed = place_galley_rows(ctx, galley.clone(), place, frac);
    ctx.data_mut(|d| {
        let entry = Placed {
            source: galley,
            ppp,
            place,
            placed: placed.clone(),
            frame,
        };
        d.get_temp_mut_or_default::<PlacedCache>(id)
            .entries
            .insert(key, entry);
    });
    placed
}

/// `galley` with each row in its line box as Blink lays it out: the line
/// tops at the unrounded sum of the line heights above, the baseline
/// where Blink paints it, the half leading (split by `leading`) and the
/// ascent of the row's first font added to the line top in device pixels,
/// ascent and descent each rounded to whole device pixels, the sum
/// rounded to the pixel. egui rounds every row's height and top to the
/// pixel and puts the baseline at the font's ascent; here each row keeps
/// its line box as its rect (selection and cursor span it) and its glyphs
/// move inside it. The galley is as tall as its line boxes. `frac` is the
/// device-pixel fraction of the paint origin that the tessellator rounds
/// away: each baseline is rounded with it, as Blink rounds the origin and
/// the baseline once.
fn place_galley_rows(
    ctx: &egui::Context,
    mut galley: Arc<Galley>,
    place: Place,
    frac: f32,
) -> Arc<Galley> {
    let leading = place.leading;
    let ppp = ctx.pixels_per_point();
    let g = Arc::make_mut(&mut galley);
    let job = g.job.clone();
    let mut bounds = egui::Rect::NOTHING;
    // The line top of the row, in points from the galley top.
    let mut top = 0.0;
    // The byte where the row starts in the job's text: rows take its
    // characters in order, a row ended by `\n` one more.
    let mut byte = 0;
    for placed in &mut g.rows {
        let start = byte;
        let chars = placed.row.glyphs.len() + usize::from(placed.ends_with_newline);
        for _ in 0..chars {
            byte += job.text[byte..].chars().next().map_or(0, char::len_utf8);
        }
        let section = job
            .sections
            .iter()
            .find(|s| s.byte_range.contains(&egui::text::ByteIndex(start)))
            .or(job.sections.last());
        // The row's ascent, descent and line gap in device px, each
        // rounded: from its first font's tables, else egui's metrics.
        let glyph = placed.row.glyphs.first();
        let dev = section.and_then(|s| {
            let size = s.format.font_id.size;
            let em = family_vmetrics(ctx, &s.format.font_id.family).or_else(|| {
                glyph.map(|g| VMetrics {
                    ascent: g.font_face_ascent / size,
                    descent: (g.font_face_height - g.font_face_ascent) / size,
                    line_gap: 0.0,
                })
            })?;
            Some([em.ascent, em.descent, em.line_gap].map(|v| (v * size * ppp).round()))
        });
        // The row's line box: its tallest line height, as egui takes it
        // before rounding; an empty row has its section's. A field's row
        // is its editor line.
        let line = match (leading, dev) {
            (Leading::Field, Some([a, d, gap])) => (a + d + gap) / ppp,
            _ => placed
                .row
                .glyphs
                .iter()
                .map(|g| g.line_height)
                .reduce(f32::max)
                .or_else(|| section.and_then(|s| s.format.line_height))
                .unwrap_or(placed.row.size.y),
        };
        placed.pos.y = top;
        let row = Arc::make_mut(&mut placed.row);
        row.size.y = line;
        if let (Some(glyph), Some([ascent, descent, gap])) = (row.glyphs.first(), dev) {
            let half = match leading {
                Leading::Inline => ((line * ppp - ascent - descent) / 2.0).floor(),
                Leading::Field => (gap / 2.0).floor(),
            };
            let baseline = (frac + top * ppp + half + ascent).round() / ppp - top;
            let shift = egui::vec2(0.0, baseline - glyph.pos.y);
            for glyph in &mut row.glyphs {
                glyph.pos.y += shift.y;
            }
            row.visuals.mesh.translate(shift);
            row.visuals.mesh_bounds = row.visuals.mesh_bounds.translate(shift);
            if let Some(underline) = place.underline {
                underline_row(row, &job, start, underline, baseline, ppp);
            }
        }
        bounds |= row.visuals.mesh_bounds.translate(placed.pos.to_vec2());
        top += line;
    }
    g.rect.max.y = g.rect.min.y + top;
    g.mesh_bounds = bounds;
    galley
}

/// Draw `underline` into `row`'s mesh under each run of glyphs whose
/// section has no underline of its own, in the run's colour, its top and
/// thickness from the run's font size by [`underline_dev`], below
/// `baseline` (points from the row top, on a device pixel), from the run's
/// first glyph's left to its last glyph's right edge. The row's text starts
/// at byte `start` of the job's text.
fn underline_row(
    row: &mut egui::epaint::text::Row,
    job: &LayoutJob,
    start: usize,
    underline: Underline,
    baseline: f32,
    ppp: f32,
) {
    let mut runs: Vec<(usize, f32, f32)> = Vec::new();
    let (mut byte, mut prev) = (start, None);
    for glyph in &row.glyphs {
        let at = byte;
        byte += job.text[at..].chars().next().map_or(0, char::len_utf8);
        let section = job.sections.iter().position(|s| s.byte_range.contains(&egui::text::ByteIndex(at)));
        let same = section.is_some() && prev == section;
        prev = section;
        let Some(section) = section else {
            continue;
        };
        if job.sections[section].format.underline != Stroke::NONE {
            continue;
        }
        match runs.last_mut() {
            Some((_, _, right)) if same => *right = glyph.max_x(),
            _ => runs.push((section, glyph.pos.x, glyph.max_x())),
        }
    }
    for (section, left, right) in runs {
        let format = &job.sections[section].format;
        let (top, thickness) = underline_dev(format.font_id.size, underline, ppp);
        let y = baseline + top / ppp;
        let rect =
            egui::Rect::from_min_max(egui::pos2(left, y), egui::pos2(right, y + thickness / ppp));
        row.visuals.mesh.add_colored_rect(rect, format.color);
        row.visuals.mesh_bounds |= rect;
    }
}

/// `uppercase`: every section's text in upper case, byte ranges adjusted
/// (a letter may grow, as `ß` to `SS`).
fn uppercase(job: &mut LayoutJob) {
    let mut text = String::with_capacity(job.text.len());
    for section in &mut job.sections {
        let start = text.len();
        text.push_str(&job.text[section.byte_range.start.0..section.byte_range.end.0].to_uppercase());
        section.byte_range = egui::text::ByteIndex(start)..egui::text::ByteIndex(text.len());
    }
    job.text = text;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_ui(ppp: f32, f: impl FnOnce(&mut Ui)) {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(ppp);
        let mut f = Some(f);
        let _ = crate::test_ui::run(&ctx, Default::default(), |ui| {
            if let Some(f) = f.take() {
                f(ui);
            }
        });
    }

    /// The caret in a line box (a textarea's row) spans the font's ascent
    /// and descent where the row's text sits: half the leading, rounded
    /// down, below the line top, the baseline an ascent below that. Blink
    /// takes the caret's height and top from the text fragment, which is
    /// as tall as the font's ascent plus descent and placed at the half
    /// leading of its line box (`ComputeLocalCaretRectAtTextOffset` in
    /// `third_party/blink/renderer/core/editing/local_caret_rect.cc`, the
    /// fragment from `inline_box_state.cc`'s `ComputeTextMetrics`); a text
    /// `<input>`'s editor line keeps its own span ([`Leading::Field`]).
    #[test]
    fn an_inline_caret_spans_the_text_of_its_line_box() {
        for ppp in [1.0, 1.5] {
            with_ui(ppp, |ui| {
                Theme::vega().apply(ui.ctx());
                let mut r = Recipe::default();
                r.text_size.base = Some(TextSize::Sm);
                let look = TextLook::resolve(ui, &[&r], State::NONE);
                let em = family_vmetrics(ui.ctx(), &look.font_id(ui.ctx()).family)
                    .expect("the theme's font has tables");
                let [a, d, gap] =
                    [em.ascent, em.descent, em.line_gap].map(|v| (v * look.size() * ppp).round());
                let span = look.caret_span(ui.ctx()).unwrap();
                let half = ((look.line_height() * ppp - a - d) / 2.0).floor();
                assert_eq!(span, half / ppp..(half + a + d) / ppp, "ppp {ppp}");
                // The row's baseline sits an ascent below the caret's top.
                let block = look.layout(ui, "Ag", TextWrapMode::Extend, f32::INFINITY);
                let baseline = block.galley().rows[0].row.glyphs[0].pos.y;
                assert!(
                    (baseline - (span.start + a / ppp)).abs() < 1e-3,
                    "ppp {ppp}"
                );
                // A field's editor line keeps the line-gap rule.
                let field = look.clone().leading(Leading::Field);
                let top = (gap / 2.0).floor();
                assert_eq!(
                    field.caret_span(ui.ctx()).unwrap(),
                    top / ppp..(top + a + d) / ppp
                );
            });
        }
    }

    #[test]
    fn recipe_fills_what_the_caller_left_out() {
        with_ui(1.0, |ui| {
            let mut r = Recipe::default();
            r.text_size.base = Some(TextSize::Xs);
            r.text.base = Some(crate::style::Paint::Role(crate::style::Role::Primary, 1.0));
            let look = TextLook::resolve(ui, &[&r], State::NONE);
            let theme = Theme::current(ui.ctx());
            assert_eq!(look.size(), theme.metrics.text_xs);
            assert_eq!(look.line_height(), theme.metrics.line_xs);
            // Plain text: recipe size, placeholder colour, recipe line box.
            let job = look.job(ui, "a".into());
            let f = &job.sections[0].format;
            assert_eq!(f.font_id.size, theme.metrics.text_xs);
            assert_eq!(f.color, Color32::PLACEHOLDER);
            assert_eq!(f.line_height, Some(theme.metrics.line_xs));
            // Explicit attributes win; strong is no explicit colour.
            let rich = egui::RichText::new("b")
                // token-ok: test data, a caller's explicit attribute
                .size(30.0)
                .line_height(Some(40.0))
                .strong();
            let job = look.job(ui, rich.into());
            let f = &job.sections[0].format;
            assert_eq!(f.font_id.size, 30.0);
            assert_eq!(f.line_height, Some(40.0));
            assert_eq!(f.color, Color32::PLACEHOLDER);
            // token-ok: test data, a caller's explicit attribute
            let red = egui::RichText::new("c").color(Color32::RED).weak();
            let job = look.job(ui, red.into());
            // token-ok: test data, a caller's explicit attribute
            assert_eq!(job.sections[0].format.color, Color32::RED);
        });
    }

    /// An arbitrary size over a named one (nova's `cn-button` `text-sm`,
    /// then `cn-button-size-sm` `text-[0.8rem]`) keeps the named size's
    /// line-height ratio, as CSS's unitless `--text-sm--line-height`; an
    /// arbitrary size alone inherits the line height.
    #[test]
    fn arbitrary_size_keeps_the_named_sizes_line_ratio() {
        with_ui(1.0, |ui| {
            let theme = Theme::current(ui.ctx());
            let m = &theme.metrics;
            let mut named = Recipe::default();
            named.text_size.base = Some(TextSize::Sm);
            let mut arbitrary = Recipe::default();
            arbitrary.text_size.base = Some(TextSize::Px(12.8));
            let look = TextLook::resolve(ui, &[&named, &arbitrary], State::NONE);
            assert_eq!(look.size(), 12.8);
            assert_eq!(look.line_height(), m.line_sm / m.text_sm * 12.8);
            // The named size alone keeps its own line height.
            let look = TextLook::resolve(ui, &[&arbitrary, &named], State::NONE);
            assert_eq!(look.line_height(), m.line_sm);
            // Alone: the inherited factor, or CSS's 1.5.
            let look = TextLook::resolve(ui, &[&arbitrary], State::NONE);
            assert_eq!(look.line_height(), 12.8 * 1.5);
            let mut container = Recipe::default();
            container.text_size.base = Some(TextSize::Lg);
            let outer = TextLook::resolve(ui, &[&container], State::NONE);
            content_scope(ui, &outer, |ui| {
                let look = TextLook::resolve(ui, &[&arbitrary], State::NONE);
                assert_eq!(look.line_height(), m.line_lg / m.text_lg * 12.8);
            });
        });
    }

    #[test]
    fn colour_falls_back_to_the_ui_text_colour() {
        with_ui(1.0, |ui| {
            let look = TextLook::resolve(ui, &[], State::NONE);
            ui.visuals_mut().override_text_color = Some(Color32::GREEN);
            assert_eq!(look.resolved_color(ui), Color32::GREEN);
        });
    }

    #[test]
    fn content_scope_passes_line_factor_and_size() {
        with_ui(1.0, |ui| {
            let mut r = Recipe::default();
            r.text_size.base = Some(TextSize::Px(10.0));
            r.line_height.base = Some(2.0);
            let outer = TextLook::resolve(ui, &[&r], State::NONE);
            content_scope(ui, &outer, |ui| {
                let inner = TextLook::resolve(ui, &[], State::NONE);
                assert_eq!(inner.size(), 10.0);
                assert_eq!(inner.line_height(), 20.0);
            });
        });
    }

    #[test]
    fn content_scope_passes_weight_tracking_and_uppercase() {
        with_ui(1.0, |ui| {
            let mut r = Recipe::default();
            r.weight.base = Some(Weight::Semibold);
            r.tracking.base = Some(0.1);
            r.uppercase.base = Some(true);
            let outer = TextLook::resolve(ui, &[&r], State::NONE);
            content_scope(ui, &outer, |ui| {
                let inner = TextLook::resolve(ui, &[], State::NONE);
                assert_eq!(inner.weight, Weight::Semibold);
                // A length that round-trips through px: equal up to float.
                assert!((inner.tracking - 0.1).abs() < 1e-6);
                assert!(inner.uppercase);
                let job = inner.job(ui, "ab".into());
                assert_eq!(job.text, "AB");
                let spacing = job.sections[0].format.extra_letter_spacing;
                assert!((spacing - 0.1 * inner.size()).abs() < 1e-5);
                // The text's own recipe wins over the scope's.
                let mut own = Recipe::default();
                own.weight.base = Some(Weight::Normal);
                own.tracking.base = Some(0.0);
                own.uppercase.base = Some(false);
                let inner = TextLook::resolve(ui, &[&own], State::NONE);
                assert_eq!(inner.weight, Weight::Normal);
                assert_eq!(inner.tracking, 0.0);
                assert!(!inner.uppercase);
                // An explicit caller attribute wins over both.
                let inner = TextLook::resolve(ui, &[], State::NONE);
                let rich = egui::RichText::new("ab").extra_letter_spacing(2.0);
                let job = inner.job(ui, rich.into());
                assert_eq!(job.sections[0].format.extra_letter_spacing, 2.0);
            });
        });
    }

    /// CSS computes `letter-spacing` to a length where the class sets it:
    /// `text-xs tracking-widest` (0.1 em of 12 px) spaces `text-sm` text
    /// inside by 1.2 px, not by 0.1 em of 14 px, also one scope further in.
    #[test]
    fn content_scope_passes_tracking_as_a_length() {
        with_ui(1.0, |ui| {
            let mut outer = Recipe::default();
            outer.text_size.base = Some(TextSize::Px(12.0));
            outer.tracking.base = Some(0.1);
            let outer = TextLook::resolve(ui, &[&outer], State::NONE);
            let mut sm = Recipe::default();
            sm.text_size.base = Some(TextSize::Px(14.0));
            let spacing = |ui: &Ui| {
                let inner = TextLook::resolve(ui, &[&sm], State::NONE);
                inner.job(ui, "ab".into()).sections[0]
                    .format
                    .extra_letter_spacing
            };
            content_scope(ui, &outer, |ui| {
                assert!((spacing(ui) - 1.2).abs() < 1e-5, "{}", spacing(ui));
                let mut lg = Recipe::default();
                lg.text_size.base = Some(TextSize::Px(18.0));
                let middle = TextLook::resolve(ui, &[&lg], State::NONE);
                content_scope(ui, &middle, |ui| {
                    assert!((spacing(ui) - 1.2).abs() < 1e-5, "{}", spacing(ui));
                });
            });
        });
    }

    /// The baseline of every row, in device pixels below the galley top.
    fn baselines(galley: &Galley, ppp: f32) -> Vec<f32> {
        galley
            .rows
            .iter()
            .map(|r| (r.pos.y + r.row.glyphs[0].pos.y) * ppp)
            .collect()
    }

    #[test]
    fn every_row_sits_on_the_blink_baseline() {
        for ppp in [1.0, 1.5, 2.0] {
            with_ui(ppp, |ui| {
                let mut r = Recipe::default();
                r.text_size.base = Some(TextSize::Px(10.0));
                r.line_height.base = Some(3.0);
                let look = TextLook::resolve(ui, &[&r], State::NONE);
                let block = look.layout(ui, "Hg\nHg", TextWrapMode::Extend, f32::INFINITY);
                assert_eq!(block.size().y, 60.0);
                let font = look.font_id(ui.ctx());
                let em = family_vmetrics(ui.ctx(), &font.family).expect("font tables");
                let (a, d) = (
                    (em.ascent * 10.0 * ppp).round(),
                    (em.descent * 10.0 * ppp).round(),
                );
                let first = ((30.0 * ppp - a - d) / 2.0).floor() + a;
                assert_eq!(baselines(block.galley(), ppp), [first, first + 30.0 * ppp]);
                let field = look.clone().leading(Leading::Field);
                let block = field.layout(ui, "Hg", TextWrapMode::Extend, f32::INFINITY);
                // The field's row is its editor line, with the line gap of
                // egui's default font, the baseline half the gap below.
                let g = (em.line_gap * 10.0 * ppp).round();
                assert_eq!(block.size().y * ppp, a + d + g);
                assert_eq!(baselines(block.galley(), ppp), [(g / 2.0).floor() + a]);
            });
        }
    }

    #[test]
    fn fractional_line_boxes_stay_unrounded() {
        // `text-xs/relaxed`: 12 px text on 19.5 px lines. Blink keeps the
        // line tops at k * 19.5 and rounds only the painted baseline.
        for ppp in [1.0, 1.5, 2.0] {
            with_ui(ppp, |ui| {
                let mut r = Recipe::default();
                r.text_size.base = Some(TextSize::Px(12.0));
                r.line_height.base = Some(1.625);
                let look = TextLook::resolve(ui, &[&r], State::NONE);
                let text = "Hg\nHg\nHg";
                let block = look.layout(ui, text, TextWrapMode::Extend, f32::INFINITY);
                let galley = block.galley();
                assert_eq!(block.size().y, 58.5);
                let tops: Vec<_> = galley.rows.iter().map(|r| r.rect()).collect();
                for (k, rect) in tops.iter().enumerate() {
                    assert_eq!((rect.top(), rect.height()), (k as f32 * 19.5, 19.5));
                }
                let font = look.font_id(ui.ctx());
                let em = family_vmetrics(ui.ctx(), &font.family).expect("font tables");
                let (a, d) = (
                    (em.ascent * 12.0 * ppp).round(),
                    (em.descent * 12.0 * ppp).round(),
                );
                let half = ((19.5 * ppp - a - d) / 2.0).floor();
                let want: Vec<_> = (0..3)
                    .map(|k| (k as f32 * 19.5 * ppp + half + a).round())
                    .collect();
                assert_eq!(baselines(galley, ppp), want);
                // The drawn glyphs move with the baseline: each vertex
                // keeps egui's offset from its glyph's baseline.
                let mut job = look.job(ui, text.into());
                job.wrap =
                    TextWrapping::from_wrap_mode_and_width(TextWrapMode::Extend, f32::INFINITY);
                let egui_galley = ui.ctx().fonts_mut(|f| f.layout_job(job));
                for (placed, egui_row) in galley.rows.iter().zip(&egui_galley.rows) {
                    let (y, y0) = (placed.glyphs[0].pos.y, egui_row.glyphs[0].pos.y);
                    let mesh = &placed.visuals.mesh.vertices;
                    for (v, v0) in mesh.iter().zip(&egui_row.visuals.mesh.vertices) {
                        assert!((v.pos.y - y - (v0.pos.y - y0)).abs() < 1e-4);
                    }
                    // and land on the device pixel grid.
                    let top = (placed.pos.y + mesh[0].pos.y) * ppp;
                    assert!((top - top.round()).abs() < 1e-3, "{top}");
                }
            });
        }
    }

    #[test]
    fn cached_measures_equal_a_fresh_measure() {
        // Kerned pairs, a soft hyphen and a char Inter lacks (egui's
        // fallback face).
        let text = "AVAST! To Wa\u{AD}ter, Tokyo \u{4E2D} yo";
        let context = |ppp: f32| {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(ppp);
            crate::Theme::vega().apply(&ctx);
            ctx.set_theme(egui::Theme::Light);
            let _ = crate::test_ui::run(&ctx, Default::default(), |_| {});
            ctx
        };
        let measure = |ctx: &egui::Context, text: &str| {
            let format = egui::TextFormat::simple(FontId::proportional(14.0), Color32::WHITE);
            let job = LayoutJob::simple_format(text.into(), format);
            shape::advances(ctx, &job, &shape::section_faces(ctx, &job))
        };
        let ctx = context(1.5);
        let first = measure(&ctx, text);
        assert!(first.fixed_kern.iter().any(|k| *k != 0));
        assert!(first.hyphen.iter().any(Option::is_some));
        // Another text fills the cache with more chars and pairs.
        measure(&ctx, "Yay AVA Ty Te");
        assert_eq!(measure(&ctx, text), first);
        assert_eq!(measure(&context(1.5), text), first);
        assert_ne!(
            measure(&ctx, "AVAST").fixed,
            measure(&context(1.0), "AVAST").fixed
        );
    }

    #[test]
    fn an_unchanged_text_is_placed_once() {
        let ctx = egui::Context::default();
        let mut galleys = Vec::new();
        for _ in 0..3 {
            let _ = crate::test_ui::run(&ctx, Default::default(), |ui| {
                let look = TextLook::resolve(ui, &[], State::NONE);
                let block = look.layout(ui, "Hg", TextWrapMode::Extend, f32::INFINITY);
                galleys.push(block.galley().clone());
            });
        }
        assert!(Arc::ptr_eq(&galleys[0], &galleys[1]));
        assert!(Arc::ptr_eq(&galleys[1], &galleys[2]));
        // Another scale places it again.
        ctx.set_pixels_per_point(2.0);
        let _ = crate::test_ui::run(&ctx, Default::default(), |ui| {
            let look = TextLook::resolve(ui, &[], State::NONE);
            let block = look.layout(ui, "Hg", TextWrapMode::Extend, f32::INFINITY);
            assert!(!Arc::ptr_eq(&galleys[2], block.galley()));
            assert_eq!(block.galley().pixels_per_point, 2.0);
        });
    }

    #[test]
    fn metrics_follow_a_font_switch_under_the_same_key() {
        // A theme that switches fonts keeps its key names: the metrics
        // must come from the face now under the key.
        let ctx = egui::Context::default();
        let defs = |data: egui::FontData| {
            let mut d = egui::FontDefinitions::empty();
            d.font_data.insert("k".into(), Arc::new(data));
            d.families
                .insert(FontFamily::Proportional, vec!["k".into()]);
            d
        };
        let measure = |ctx: &egui::Context| {
            let mut m = None;
            let _ = crate::test_ui::run(&ctx, Default::default(), |ui| {
                m = family_vmetrics(ui.ctx(), &FontFamily::Proportional);
            });
            m.expect("font tables")
        };
        let other = egui::FontDefinitions::default();
        let ubuntu = other.font_data["Ubuntu-Light"].as_ref().clone();
        let inter = shadcn_egui_font_inter::FONT.face(400);
        ctx.set_fonts(defs(ubuntu.clone()));
        let before = measure(&ctx);
        ctx.set_fonts(defs(egui::FontData::from_static(inter.data)));
        let after = measure(&ctx);
        assert_ne!(before, after);
        assert_eq!(after, vmetrics(inter.data, 0).expect("tables"));
        assert_eq!(before, vmetrics(&ubuntu.font, 0).expect("tables"));
    }

    #[test]
    fn metrics_come_from_the_font_tables() {
        // Inter sets USE_TYPO_METRICS: typo ascender 1984, descender -494,
        // 2048 units per em.
        let inter = shadcn_egui_font_inter::FONT.face(400);
        let m = vmetrics(inter.data, 0).expect("tables");
        assert_eq!(m.ascent, 1984.0 / 2048.0);
        assert_eq!(m.descent, 494.0 / 2048.0);
        assert_eq!(m.line_gap, 0.0);
    }

    /// The `OS/2` fields the metrics read, in a table of `len` bytes.
    struct Os2 {
        len: usize,
        sel: u16,
        typo: (i16, i16, i16),
        win: (u16, u16),
    }

    /// A minimal font file: `head`, `hhea` and, if given, an `OS/2`
    /// table, all values big-endian at their offsets.
    fn font(hhea: (i16, i16, i16), os2: Option<Os2>) -> Vec<u8> {
        let mut head = vec![0u8; 54];
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        let mut hh = vec![0u8; 36];
        for (o, v) in [(4, hhea.0), (6, hhea.1), (8, hhea.2)] {
            hh[o..o + 2].copy_from_slice(&v.to_be_bytes());
        }
        let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![(b"head", head), (b"hhea", hh)];
        if let Some(Os2 {
            len,
            sel,
            typo,
            win,
        }) = os2
        {
            let mut t = vec![0u8; 78];
            t[62..64].copy_from_slice(&sel.to_be_bytes());
            for (o, v) in [(68, typo.0), (70, typo.1), (72, typo.2)] {
                t[o..o + 2].copy_from_slice(&v.to_be_bytes());
            }
            t[74..76].copy_from_slice(&win.0.to_be_bytes());
            t[76..78].copy_from_slice(&win.1.to_be_bytes());
            t.truncate(len);
            tables.push((b"OS/2", t));
        }
        let mut out = vec![0u8; 12 + 16 * tables.len()];
        out[4..6].copy_from_slice(&(tables.len() as u16).to_be_bytes());
        for (i, (tag, t)) in tables.iter().enumerate() {
            let rec = 12 + 16 * i;
            out[rec..rec + 4].copy_from_slice(*tag);
            let offset = out.len() as u32;
            out[rec + 8..rec + 12].copy_from_slice(&offset.to_be_bytes());
            out[rec + 12..rec + 16].copy_from_slice(&(t.len() as u32).to_be_bytes());
            out.extend_from_slice(t);
        }
        out
    }

    #[test]
    fn metrics_survive_broken_and_unusual_tables() {
        let m = |a: f32, d: f32, g: f32| VMetrics {
            ascent: a / 1000.0,
            descent: d / 1000.0,
            line_gap: g / 1000.0,
        };
        // hhea, typo metrics when USE_TYPO_METRICS is set.
        let typo = Some(Os2 {
            len: 78,
            sel: 0x80,
            typo: (900, -300, 50),
            win: (1100, 400),
        });
        assert_eq!(
            vmetrics(&font((800, -200, 10), None), 0),
            Some(m(800.0, 200.0, 10.0))
        );
        assert_eq!(
            vmetrics(&font((800, -200, 10), typo), 0),
            Some(m(900.0, 300.0, 50.0))
        );
        // A version-0 OS/2 cut before the typo fields: hhea.
        let short = Some(Os2 {
            len: 68,
            sel: 0x80,
            typo: (900, -300, 50),
            win: (1100, 400),
        });
        assert_eq!(
            vmetrics(&font((800, -200, 10), short), 0),
            Some(m(800.0, 200.0, 10.0))
        );
        // hhea all zero: typo metrics, else win metrics (FreeType).
        let no_flag = Some(Os2 {
            len: 78,
            sel: 0,
            typo: (900, -300, 50),
            win: (1100, 400),
        });
        assert_eq!(
            vmetrics(&font((0, 0, 0), no_flag), 0),
            Some(m(900.0, 300.0, 50.0))
        );
        let win_only = Some(Os2 {
            len: 78,
            sel: 0,
            typo: (0, 0, 0),
            win: (1100, 400),
        });
        assert_eq!(
            vmetrics(&font((0, 0, 0), win_only), 0),
            Some(m(1100.0, 400.0, 0.0))
        );
        // A table record pointing past the end, a face index past the
        // collection, offsets near the address limit: no metrics, no panic.
        let mut bad = font((800, -200, 10), None);
        bad[12 + 16 + 8..12 + 16 + 12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(vmetrics(&bad, 0), None);
        let mut ttc = b"ttcf".to_vec();
        ttc.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 1]);
        ttc.extend_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(vmetrics(&ttc, 1), None);
        assert_eq!(vmetrics(&ttc, 0), None);
        assert_eq!(vmetrics(b"tt", 0), None);
    }

    #[test]
    fn uppercase_keeps_sections_apart() {
        let mut job = LayoutJob::default();
        job.append("straße", 0.0, Default::default());
        // egui 0.36 merges adjacent sections with identical formats.
        job.append("x", 0.0, egui::TextFormat { color: Color32::RED, ..Default::default() });
        uppercase(&mut job);
        assert_eq!(job.text, "STRASSEX");
        assert_eq!(job.sections[0].byte_range, egui::text::ByteIndex(0)..egui::text::ByteIndex(7));
        assert_eq!(job.sections[1].byte_range, egui::text::ByteIndex(7)..egui::text::ByteIndex(8));
    }
}
