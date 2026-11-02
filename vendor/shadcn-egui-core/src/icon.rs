//! Vector icon data. Painting lives in `shadcn_egui::components::icon`.

use crate::blob::{BlobLayers, BlobRange, BlobSegs, FillMesh};

/// One path command in icon units (the SVG's user space, origin top-left).
/// A `M` starts a new subpath; `Z` closes the current one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    M(f32, f32),
    L(f32, f32),
    Q(f32, f32, f32, f32),
    C(f32, f32, f32, f32, f32, f32),
    Z,
}

/// A generated icon: its name (for screen readers), the side of its square
/// view box, and where its path data lives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Icon {
    pub name: &'static str,
    pub size: f32,
    pub shape: IconShape,
}

/// Where an icon's path data lives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IconShape {
    /// Path commands in a Rust slice (icongen `--out`); strokes only.
    Segs(&'static [Seg]),
    /// A record in an icon crate's blob (icongen `--crate-dir`): layers of
    /// strokes and an optional fill, in the 24-unit box.
    Blob(BlobRange),
}

impl Icon {
    /// The icon's paint layers: the opaque one first, then one per
    /// translucent part of the source SVG (an element with `opacity`,
    /// `stroke-opacity` or `fill-opacity`). Draw each layer on its own at
    /// its `alpha`; strokes that overlap within a layer count once.
    pub fn layers(&self) -> IconLayers {
        match self.shape {
            IconShape::Segs(s) => IconLayers(Layers::Slice(Some(s))),
            IconShape::Blob(r) => IconLayers(Layers::Blob(BlobLayers::new(r))),
        }
    }
}

/// Strokes and fill of an [`Icon`] painted together at one opacity.
/// Non-exhaustive: more paint properties may follow.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct IconLayer {
    /// Opacity in 0..=1, multiplied into the colour the icon is drawn in.
    pub alpha: f32,
    /// The stroke commands, in icon units. A non-empty sequence starts with `Seg::M`.
    pub segs: SegIter,
    /// The pre-triangulated fill; `None` for a stroke-only layer.
    pub fill: Option<FillMesh>,
}

/// The layers of an [`Icon`], from [`Icon::layers`].
#[derive(Clone, Debug)]
pub struct IconLayers(Layers);

#[derive(Clone, Debug)]
enum Layers {
    Slice(Option<&'static [Seg]>),
    Blob(BlobLayers),
}

impl Iterator for IconLayers {
    type Item = IconLayer;

    fn next(&mut self) -> Option<IconLayer> {
        match &mut self.0 {
            Layers::Slice(s) => s.take().map(|s| IconLayer {
                alpha: 1.0,
                segs: SegIter::Slice(s.iter()),
                fill: None,
            }),
            Layers::Blob(b) => b.next(),
        }
    }
}

/// The stroke commands of an [`Icon`], from either storage.
#[derive(Clone, Debug)]
pub enum SegIter {
    Slice(core::slice::Iter<'static, Seg>),
    Words(BlobSegs),
}

impl Iterator for SegIter {
    type Item = Seg;

    fn next(&mut self) -> Option<Seg> {
        match self {
            SegIter::Slice(it) => it.next().copied(),
            SegIter::Words(w) => w.next(),
        }
    }
}

/// The icons the components draw: a slot per shape, plus a slot named
/// after its role where an icon library draws that role with another icon
/// than the shape's. Each icon crate exports its set as `SLOTS`;
/// `shadcn_egui::Theme::icons` holds the active one.
///
/// New slots arrive in minor releases, so the struct cannot be built with
/// a literal: start from [`IconSlots::all`] and set the slots to change.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct IconSlots {
    /// Checkbox tick and the selected Select item.
    pub check: Icon,
    /// Select trigger: a down chevron, or the library's up-and-down
    /// selector (tabler, hugeicons).
    pub select_trigger: Icon,
    /// A plain down chevron (Select's scroll-down button).
    pub chevron_down_plain: Icon,
    /// An up chevron (Select's scroll-up button).
    pub chevron_up: Icon,
    /// A left chevron (pagination's previous link and button).
    pub chevron_left: Icon,
    /// A right chevron (pagination's next link and button).
    pub chevron_right: Icon,
    /// Three dots in a row (pagination's ellipsis).
    pub more_horizontal: Icon,
    /// Dialog close button.
    pub close: Icon,
}

impl IconSlots {
    /// Every slot draws `icon`. Set the slots to change from there:
    ///
    /// ```
    /// # use shadcn_egui_core::{Icon, IconShape, IconSlots};
    /// # const DOT: Icon = Icon { name: "dot", size: 24.0, shape: IconShape::Segs(&[]) };
    /// # const TICK: Icon = DOT;
    /// const SLOTS: IconSlots = {
    ///     let mut s = IconSlots::all(DOT);
    ///     s.check = TICK;
    ///     s
    /// };
    /// assert_eq!(SLOTS.check, TICK);
    /// ```
    pub const fn all(icon: Icon) -> Self {
        Self {
            check: icon,
            select_trigger: icon,
            chevron_down_plain: icon,
            chevron_up: icon,
            chevron_left: icon,
            chevron_right: icon,
            more_horizontal: icon,
            close: icon,
        }
    }
}
