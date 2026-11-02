//! Vector icons, no image loader. Icon data is generated ahead of time by
//! `tools/icongen` (strokes as path commands, fills as pre-triangulated
//! meshes); [`paint_icon`] rasterises it on the CPU into a cached coverage
//! texture and draws that tinted, so a translucent colour applies once to
//! the whole icon.
//!
//! Lucide icons are 24x24 outlines drawn with a 2-unit round-capped stroke.
//! Round caps matter: some icons draw dots as zero-length lines, which vanish
//! without them. Every stroke is painted with round caps and round joins.

use super::icon_raster;
use crate::Theme;
use egui::{pos2, Color32, Painter, Pos2, Rect, Response, Sense, Ui, Vec2, Widget};
pub use shadcn_egui_core::{Icon, IconLayer, IconShape, IconSlots, Seg};

/// Paint `icon` scaled into the square `rect`: the fill (if any) and the
/// strokes of each layer at the layer's alpha, from one coverage texture
/// tinted `color`. `stroke_width` is in icon units (Lucide's own is 2.0)
/// and scales with the icon.
///
/// The texture is rasterised once per icon, physical pixel size and stroke
/// width, and cached in the context; `rect` snaps to whole physical pixels.
/// An icon larger than 1024 physical pixels is drawn from a 1024-pixel
/// texture scaled up. A curve with no start point is skipped.
pub fn paint_icon(painter: &Painter, rect: Rect, icon: &Icon, color: Color32, stroke_width: f32) {
    let ppp = painter.pixels_per_point();
    let (x0, y0) = ((rect.min.x * ppp).round(), (rect.min.y * ppp).round());
    let w = (rect.width() * ppp).round().max(1.0); // behaviour-ok: at least one device pixel
    let h = (rect.height() * ppp).round().max(1.0); // behaviour-ok: at least one device pixel
    let finite = [x0, y0, w, h].iter().all(|v| v.is_finite());
    if !finite || icon.size.is_nan() || icon.size <= 0.0 {
        return;
    }
    let drawn = Rect::from_min_size(pos2(x0 / ppp, y0 / ppp), Vec2::new(w / ppp, h / ppp));
    let ctx = painter.ctx();
    let side = icon_raster::MAX_SIDE.min(ctx.input(|i| i.max_texture_side)) as f32;
    let k = (side / w.max(h)).min(1.0); // behaviour-ok: never rasterise larger than drawn
    let (tw, th) = ((w * k).round().max(1.0), (h * k).round().max(1.0)); // behaviour-ok: at least one device pixel
    let tex = icon_raster::texture(ctx, icon, tw as usize, th as usize, stroke_width);
    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)); // behaviour-ok: the whole texture (UV unit square)
    painter.image(tex.id(), drawn, uv, color);
}

/// An icon as a widget: `ui.add(IconView::new(&icons::MIC))`. Sized
/// `Metrics::icon`, colored `foreground`, stroked `Metrics::icon_stroke`.
/// It has no accessible name; give it one with `.labelled_by(label.id)`.
pub struct IconView<'a> {
    icon: &'a Icon,
    size: Option<f32>,
    color: Option<Color32>,
}

impl<'a> IconView<'a> {
    pub fn new(icon: &'a Icon) -> Self {
        Self {
            icon,
            size: None,
            color: None,
        }
    }
    pub fn size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }
    pub fn color(mut self, color: Color32) -> Self {
        self.color = Some(color);
        self
    }
}

impl Widget for IconView<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = Theme::current(ui.ctx());
        let side = self.size.unwrap_or(t.metrics.icon);
        let (rect, resp) = crate::layout::allocate_item(ui, Vec2::splat(side), Sense::hover());
        // No label of its own (the icon's name is an id, not text a reader
        // should hear): name it with `labelled_by(label.id)`, which an own
        // label would override.
        let enabled = ui.is_enabled();
        resp.widget_info(|| egui::WidgetInfo {
            enabled,
            ..egui::WidgetInfo::new(egui::WidgetType::Image)
        });
        if ui.is_rect_visible(rect) {
            let color = self.color.unwrap_or(t.palette().foreground);
            paint_icon(ui.painter(), rect, self.icon, color, t.metrics.icon_stroke);
        }
        resp
    }
}
