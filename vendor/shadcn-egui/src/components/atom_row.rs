//! The atom row: one row of `IntoAtoms` content between the typed icon
//! slots (`data-icon="inline-start"`, `"inline-end"`), as upstream's
//! `inline-flex items-center gap-*` parts lay out their children. The one
//! path for every part that lays out its content in one row with icon
//! slots (button, badge; later kbd, toggle, tab trigger).
//!
//! Text atoms go through the text path ([`TextLook`]): Chrome's advances,
//! kerning and line box. Typed icon slots and [`IconAtom`]s without a
//! caller colour draw in the colour the row paints in, the part's state
//! colour (rules, Media and icons); other images draw as given. Typed icon
//! slots and [`IconAtom`]s without a caller side are the part's icon size
//! ([`AtomRow::icon_size`]), as upstream sizes an svg without a `size-*`
//! class.
//!
//! Use it in two steps, as the size never depends on the interaction
//! state: [`AtomRow::lay_out`] gives the row's border-box size to allocate;
//! with the allocation's `Response`, resolve the state, paint the box, then
//! [`LaidRow::paint`] in the state's text colour.
//!
//! [`IconAtom`]: crate::components::icon_atom::IconAtom

use crate::components::icon::{paint_icon, Icon};
use crate::components::icon_atom::{resize, retint};
use crate::components::text::{TextBlock, TextLook};
use egui::{
    Align2, AtomKind, Atoms, Color32, Image, IntoAtoms, Painter, Rect, SizedAtomKind, TextWrapMode,
    Ui, Vec2, WidgetText,
};

/// One row of atoms with optional typed icon slots, before layout.
#[must_use = "Call .lay_out(ui, look)"]
pub(crate) struct AtomRow<'a> {
    atoms: Atoms<'a>,
    start: Option<Icon>,
    end: Option<Icon>,
    icon_size: f32,
    icon_stroke: f32,
    gap: f32,
    /// Left, right, top and bottom: padding plus border.
    inset: [f32; 4],
}

impl<'a> AtomRow<'a> {
    /// `atoms` with no icon slots, no gap and no inset.
    pub(crate) fn new(ui: &Ui, atoms: impl IntoAtoms<'a>) -> Self {
        let metrics = crate::Theme::current(ui.ctx()).metrics;
        Self {
            atoms: atoms.into_atoms(),
            start: None,
            end: None,
            icon_size: metrics.icon,
            icon_stroke: metrics.icon_stroke,
            gap: 0.0,
            inset: [0.0; 4],
        }
    }

    /// The typed icon slots before and after the content.
    pub(crate) fn icons(mut self, start: Option<Icon>, end: Option<Icon>) -> Self {
        self.start = start;
        self.end = end;
        self
    }

    /// The part's icon size (`[&_svg:not([class*='size-'])]:size-*`): the
    /// side of the typed icon slots and of icon atoms without a caller
    /// side; `Metrics::icon` by default.
    pub(crate) fn icon_size(mut self, size: f32) -> Self {
        self.icon_size = size;
        self
    }

    /// The gap between items (`gap-*`).
    pub(crate) fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// The room between the border box's edges and the content: padding
    /// plus border, per side.
    pub(crate) fn inset(mut self, left: f32, right: f32, top: f32, bottom: f32) -> Self {
        self.inset = [left, right, top, bottom];
        self
    }

    /// Size every item: text in `look`, unwrapped (one row that does not
    /// wrap); an empty text makes no item, as an empty text run makes no
    /// flex item.
    pub(crate) fn lay_out(self, ui: &Ui, look: &TextLook) -> LaidRow<'a> {
        let font = egui::FontSelection::FontId(look.font_id(ui.ctx()));
        let mut items = Vec::new();
        let icon = |i: Icon| Item {
            kind: ItemKind::Icon(i),
            size: Vec2::splat(self.icon_size),
            align: Align2::CENTER_CENTER,
            grow: false,
        };
        items.extend(self.start.map(icon));
        for atom in self.atoms {
            let (align, grow, fixed) = (atom.align, atom.grow, atom.size);
            let (kind, natural) = match atom.kind {
                AtomKind::Text(text) if text.is_empty() => continue,
                AtomKind::Text(text) => {
                    let block = lay_out_text(ui, look, text.clone());
                    let size = block.size();
                    (ItemKind::Text { text, block }, size)
                }
                kind => {
                    let kind = match kind {
                        AtomKind::Image(img) => AtomKind::Image(resize(img, self.icon_size)),
                        kind => kind,
                    };
                    let sized = egui::Atom {
                        kind,
                        ..Default::default()
                    }
                    .into_sized(
                        ui,
                        Vec2::INFINITY,
                        Some(TextWrapMode::Extend),
                        font.clone(),
                    );
                    (ItemKind::Sized(sized.kind), sized.size)
                }
            };
            items.push(Item {
                kind,
                size: fixed.unwrap_or(natural),
                align,
                grow,
            });
        }
        items.extend(self.end.map(icon));
        let gaps = items.len().saturating_sub(1) as f32 * self.gap;
        let content = Vec2::new(
            items.iter().map(|i| i.size.x).sum::<f32>() + gaps,
            items.iter().map(|i| i.size.y).fold(0.0, f32::max),
        );
        LaidRow {
            items,
            content,
            underlined: look.underlined(),
            gap: self.gap,
            inset: self.inset,
            icon_stroke: self.icon_stroke,
        }
    }
}

fn lay_out_text(ui: &Ui, look: &TextLook, text: WidgetText) -> TextBlock {
    look.layout(ui, text, TextWrapMode::Extend, f32::INFINITY)
}

struct Item<'a> {
    kind: ItemKind<'a>,
    size: Vec2,
    align: Align2,
    grow: bool,
}

enum ItemKind<'a> {
    Icon(Icon),
    /// The source text, to lay out again in a look of the paint state.
    Text {
        text: WidgetText,
        block: TextBlock,
    },
    /// An image, a closure's result or a custom atom's space.
    Sized(SizedAtomKind<'a>),
}

/// An [`AtomRow`] with every item sized.
pub(crate) struct LaidRow<'a> {
    items: Vec<Item<'a>>,
    content: Vec2,
    underlined: bool,
    gap: f32,
    inset: [f32; 4],
    icon_stroke: f32,
}

impl LaidRow<'_> {
    /// The border-box size: the content plus the inset.
    pub(crate) fn size(&self) -> Vec2 {
        let [l, r, t, b] = self.inset;
        self.content + Vec2::new(l + r, t + b)
    }

    /// Paint the items in `rect`, a border box: the content centred in the
    /// box less the inset (`justify-center items-center`), a `grow` atom
    /// taking the width left over. `look` is the text's look in the paint
    /// state: where its underline differs from the layout's (a link on
    /// hover), the text lays out again in it. Text without a caller colour,
    /// typed icons and untinted icon atoms take `color`.
    pub(crate) fn paint(
        self,
        ui: &Ui,
        painter: &Painter,
        rect: Rect,
        look: &TextLook,
        color: Color32,
    ) {
        let [l, r, t, b] = self.inset;
        let inner = Rect::from_min_max(rect.min + Vec2::new(l, t), rect.max - Vec2::new(r, b));
        let grow_count = self.items.iter().filter(|i| i.grow).count();
        let spare = (inner.width() - self.content.x).max(0.0);
        let (width, grow) = if grow_count > 0 {
            (inner.width(), spare / grow_count as f32)
        } else {
            (self.content.x, 0.0)
        };
        let row =
            Align2::CENTER_CENTER.align_size_within_rect(Vec2::new(width, self.content.y), inner);
        let relayout = look.underlined() != self.underlined;
        let mut x = row.left();
        for (index, item) in self.items.into_iter().enumerate() {
            let w = item.size.x + if item.grow { grow } else { 0.0 };
            let cell = Rect::from_x_y_ranges(x..=x + w, row.y_range());
            x += w + self.gap;
            let at = item.align.align_size_within_rect(item.size, cell);
            match item.kind {
                ItemKind::Icon(icon) => paint_icon(painter, at, &icon, color, self.icon_stroke),
                ItemKind::Text { text, block } => {
                    let block = if relayout {
                        lay_out_text(ui, look, text)
                    } else {
                        block
                    };
                    block.paint(painter, at.min, color);
                }
                ItemKind::Sized(SizedAtomKind::Text(galley)) => {
                    painter.galley(at.min, galley, color);
                }
                ItemKind::Sized(SizedAtomKind::Image { image, .. }) => {
                    paint_image(painter, &retint(image, color), at);
                }
                ItemKind::Sized(SizedAtomKind::Empty { .. }) => {}
                ItemKind::Sized(SizedAtomKind::Layout(mut layout)) => {
                    layout.fallback_text_color = color;
                    let child = &mut Ui::new(
                        ui.ctx().clone(),
                        ui.id().with(("shadcn_atom_layout", index)),
                        egui::UiBuilder::new().max_rect(at).layer_id(painter.layer_id()).style(ui.style().clone()),
                    );
                    child.set_clip_rect(painter.clip_rect());
                    child.set_opacity(painter.opacity());
                    let response = child.interact(at, child.id(), egui::Sense::hover());
                    layout.paint_at(&child, at, response);
                }
            }
        }
    }
}

/// `image` in `rect` with `painter`, so it takes the painter's opacity
/// (egui's `Image::paint_at` paints with the `Ui`'s). A texture still
/// loading paints nothing.
fn paint_image(painter: &Painter, image: &Image<'_>, rect: Rect) {
    use egui::emath::GuiRounding as _;
    let rect = rect.round_to_pixels(painter.pixels_per_point());
    if let Ok(egui::load::TexturePoll::Ready { texture }) =
        image.load_for_size(painter.ctx(), rect.size())
    {
        egui::widgets::paint_texture_at(painter, rect, image.image_options(), &texture);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::icon_atom::IconAtom;
    use crate::style::{Recipe, State, TextSize};

    fn look(ui: &Ui) -> TextLook {
        let mut r = Recipe::default();
        r.text_size.base = Some(TextSize::Px(10.0));
        r.line_height.base = Some(2.0);
        TextLook::resolve(ui, &[&r], State::NONE)
    }

    #[test]
    fn row_sizes_from_inset_line_icons_and_gaps() {
        let ctx = egui::Context::default();
        crate::components::icon_atom::install(&ctx);
        let mut sizes = Vec::new();
        let _ = crate::test_ui::run(&ctx, egui::RawInput::default(), |ui| {
            let look = look(ui);
            let icons = crate::icons::lucide::SLOTS;
            let size = |row: AtomRow<'_>| row.lay_out(ui, &look).size();
            let text = size(AtomRow::new(ui, "Save"));
            sizes.push(text);
            sizes.push(size(AtomRow::new(ui, "Save").inset(8.0, 6.0, 3.0, 3.0)));
            sizes.push(size(
                AtomRow::new(ui, "Save")
                    .icons(Some(icons.check), Some(icons.check))
                    .icon_size(12.0)
                    .gap(4.0),
            ));
            // An empty text is no item: one icon, no gap.
            sizes.push(size(
                AtomRow::new(ui, "")
                    .icons(Some(icons.check), None)
                    .icon_size(24.0)
                    .gap(4.0),
            ));
            sizes.push(size(
                AtomRow::new(ui, (IconAtom::new(icons.check).size(16.0), "Save")).gap(4.0),
            ));
        });
        // The line box: 10 px text at line height 2.
        assert_eq!(sizes[0].y, 20.0);
        assert_eq!(sizes[1], sizes[0] + Vec2::new(14.0, 6.0));
        assert_eq!(sizes[2], Vec2::new(sizes[0].x + 2.0 * (12.0 + 4.0), 20.0));
        assert_eq!(sizes[3], Vec2::splat(24.0));
        assert_eq!(sizes[4], Vec2::new(sizes[0].x + 16.0 + 4.0, 20.0));
    }

    /// An icon atom without a colour takes the row's, one with a colour
    /// keeps it, and both take the painter's opacity.
    #[test]
    fn icon_atoms_take_the_row_colour_unless_tinted() {
        let ctx = egui::Context::default();
        crate::components::icon_atom::install(&ctx);
        let output = crate::test_ui::run(&ctx, egui::RawInput::default(), |ui| {
            let look = look(ui);
            let check = crate::icons::lucide::SLOTS.check;
            let atoms = (
                IconAtom::new(check),
                IconAtom::new(check).tint(Color32::BLUE),
            );
            let row = AtomRow::new(ui, atoms).lay_out(ui, &look);
            let mut painter = ui.painter().clone();
            painter.set_opacity(0.5);
            let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), row.size());
            row.paint(ui, &painter, rect, &look, Color32::RED);
        });
        let fills: Vec<Color32> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) => Some(r.fill),
                _ => None,
            })
            .collect();
        assert_eq!(
            fills,
            [
                Color32::RED.gamma_multiply(0.5),
                Color32::BLUE.gamma_multiply(0.5)
            ]
        );
    }

    /// An icon atom without a caller side measures and paints at the row's
    /// icon size (`[&_svg:not([class*='size-'])]:size-*`); one with a side
    /// keeps it.
    #[test]
    fn icon_atoms_take_the_row_icon_size_unless_sized() {
        let ctx = egui::Context::default();
        crate::components::icon_atom::install(&ctx);
        let mut sizes = Vec::new();
        let output = crate::test_ui::run(&ctx, egui::RawInput::default(), |ui| {
            let look = look(ui);
            let check = crate::icons::lucide::SLOTS.check;
            for atom in [IconAtom::new(check), IconAtom::new(check).size(20.0)] {
                let row = AtomRow::new(ui, atom).icon_size(12.0).lay_out(ui, &look);
                sizes.push(row.size());
                let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), row.size());
                row.paint(ui, ui.painter(), rect, &look, Color32::RED);
            }
        });
        assert_eq!(sizes, [Vec2::splat(12.0), Vec2::splat(20.0)]);
        let painted: Vec<Vec2> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) => Some(r.rect.size()),
                _ => None,
            })
            .collect();
        assert_eq!(painted, [Vec2::splat(12.0), Vec2::splat(20.0)]);
    }

    #[test]
    fn row_paints_its_content_centred_in_the_inset_box() {
        let ctx = egui::Context::default();
        let output = crate::test_ui::run(&ctx, egui::RawInput::default(), |ui| {
            let look = look(ui);
            let row = AtomRow::new(ui, "Save")
                .inset(10.0, 2.0, 0.0, 0.0)
                .lay_out(ui, &look);
            let content = row.size() - Vec2::new(12.0, 0.0);
            let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(100.0, 40.0));
            row.paint(ui, ui.painter(), rect, &look, Color32::RED);
            // Centred in 10..98: starts at 54 - w/2.
            ui.data_mut(|d| d.insert_temp(egui::Id::new("left"), 54.0 - content.x / 2.0));
        });
        let left: f32 = ctx
            .data(|d| d.get_temp(egui::Id::new("left")))
            .expect("stored");
        let text = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.pos),
                _ => None,
            })
            .expect("text painted");
        assert_eq!(text.x, left);
        // A 20 px line box centred in 40.
        assert_eq!(text.y, 10.0);
    }
}
