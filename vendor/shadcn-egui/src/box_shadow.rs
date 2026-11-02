//! CSS outer `box-shadow` as an egui mesh, to the browser's pixels
//! (conformance 2.5).
//!
//! A layer is the border box moved by its offset and grown by its spread,
//! blurred by a Gaussian with a deviation of half the blur radius (CSS
//! Backgrounds 3). egui's own `Shadow` is a linear ramp as wide as the blur
//! radius, with no negative spread and one layer, so the crate builds the
//! mesh itself: rings of vertices along the shape's distance contours, each
//! vertex coloured with the blurred shape's exact coverage, and the whole
//! cut to the outside of the border box. The mesh is cached per frame, so
//! an idle box costs no allocation.

use crate::color::mix_oklab;
use crate::components::shared::{clip_outside, rounded_polygon};
use crate::theme::ShadowLayer;
use egui::cache::{ComputerMut, FrameCache};
use egui::{Color32, CornerRadius, Mesh, Pos2, Rect};
use std::f32::consts::{FRAC_PI_2, PI, SQRT_2};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Arc segments per corner of a ring.
const CORNER_SEGMENTS: usize = 8;
/// The rings reach this many deviations into and out of the shape, where
/// the coverage is within 0.2 % of full and of none.
const REACH: f32 = 3.0;
/// Rings per deviation. Linear colour between rings a quarter deviation
/// apart stays within 0.2 % of the Gaussian edge, well under one level of
/// 255 for Tailwind's alphas.
const RINGS_PER_SIGMA: f32 = 4.0;
/// Points on each straight side next to each corner, a quarter deviation
/// apart: along a side the coverage changes within `REACH` deviations of
/// a corner and is flat between.
const SIDE_POINTS: usize = (REACH * RINGS_PER_SIGMA) as usize;
/// Samples across a corner band in the coverage integral.
const BAND_SAMPLES: usize = 16;

/// Most layers a theme shadow has: Tailwind's `--shadow-*` have two.
const MAX_LAYERS: usize = 2;

/// The layer CSS pads the shorter of two shadow lists with when it mixes
/// them: `0 0 0 0 transparent`.
const NO_LAYER: ShadowLayer =
    ShadowLayer::new(egui::Vec2::ZERO, 0.0, 0.0, Color32::TRANSPARENT, 0.0);

/// A `box-shadow` list held by value, so a transition can mix two of them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Layers {
    list: [ShadowLayer; MAX_LAYERS],
    len: usize,
}

impl Layers {
    /// No shadow: `box-shadow: none`.
    pub(crate) const NONE: Layers = Layers {
        list: [NO_LAYER; MAX_LAYERS],
        len: 0,
    };

    /// The theme's `layers`. Panics past [`MAX_LAYERS`], which no theme
    /// shadow reaches.
    pub(crate) fn new(layers: &[ShadowLayer]) -> Layers {
        assert!(
            layers.len() <= MAX_LAYERS,
            "a theme shadow has at most {MAX_LAYERS} layers"
        );
        let mut list = [NO_LAYER; MAX_LAYERS];
        list[..layers.len()].copy_from_slice(layers);
        Layers {
            list,
            len: layers.len(),
        }
    }

    pub(crate) fn as_slice(&self) -> &[ShadowLayer] {
        &self.list[..self.len]
    }

    /// CSS's mix of two shadow lists at `t`: layer by layer, the shorter
    /// list padded with `0 0 0 0 transparent`; lengths move linearly, the
    /// colour in OKLab with premultiplied alpha ([`mix_oklab`]).
    pub(crate) fn mix(a: Layers, b: Layers, t: f32) -> Layers {
        let len = a.len.max(b.len);
        let mut list = [NO_LAYER; MAX_LAYERS];
        for (i, l) in list.iter_mut().enumerate().take(len) {
            *l = mix_layer(a.list[i], b.list[i], t);
        }
        Layers { list, len }
    }
}

fn mix_layer(a: ShadowLayer, b: ShadowLayer, t: f32) -> ShadowLayer {
    let lerp = |p: f32, q: f32| p + (q - p) * t;
    let alpha = lerp(a.alpha(), b.alpha());
    // Premultiplied: each side's colour weighs by its alpha, so a padding
    // layer lends only its alpha.
    let w = if alpha > 0.0 {
        t * b.alpha() / alpha
    } else {
        t
    };
    let opaque = |c: Color32| Color32::from_rgb(c.r(), c.g(), c.b());
    ShadowLayer::new(
        a.offset() + (b.offset() - a.offset()) * t,
        lerp(a.blur(), b.blur()),
        lerp(a.spread(), b.spread()),
        mix_oklab(opaque(a.color()), opaque(b.color()), w),
        alpha,
    )
}

/// The shadow `layers` of the border box `rect` (already on the pixel grid
/// where egui rounds rects) with CSS corner radii `radii` (nw, ne, sw, se),
/// faded by `opacity` and cut to the outside of the box egui fills with
/// `cut`. First layer on top.
pub(crate) fn shadow_mesh(
    ctx: &egui::Context,
    rect: Rect,
    radii: [f32; 4],
    cut: CornerRadius,
    layers: Layers,
    opacity: f32,
) -> Arc<Mesh> {
    let key = Key {
        rect,
        radii,
        cut,
        layers,
        opacity,
        pixels_per_point: ctx.pixels_per_point(),
    };
    ctx.memory_mut(|m| {
        m.caches
            .cache::<FrameCache<Arc<Mesh>, Builder>>()
            .get(key)
            .clone()
    })
}

#[derive(Clone, Copy)]
struct Key {
    rect: Rect,
    radii: [f32; 4],
    cut: CornerRadius,
    layers: Layers,
    opacity: f32,
    pixels_per_point: f32,
}

impl Hash for Key {
    fn hash<H: Hasher>(&self, h: &mut H) {
        let r = self.rect;
        for v in [
            r.min.x,
            r.min.y,
            r.max.x,
            r.max.y,
            self.opacity,
            self.pixels_per_point,
        ]
        .into_iter()
        .chain(self.radii)
        {
            v.to_bits().hash(h);
        }
        self.cut.hash(h);
        for l in self.layers.as_slice() {
            for v in [l.offset().x, l.offset().y, l.blur(), l.spread(), l.alpha()] {
                v.to_bits().hash(h);
            }
            l.color().hash(h);
        }
    }
}

#[derive(Default)]
struct Builder;

impl ComputerMut<Key, Arc<Mesh>> for Builder {
    fn compute(&mut self, k: Key) -> Arc<Mesh> {
        let mut mesh = Mesh::default();
        for layer in k.layers.as_slice().iter().rev() {
            add_layer(
                &mut mesh,
                k.rect,
                k.radii,
                layer,
                k.opacity,
                k.pixels_per_point,
            );
        }
        Arc::new(clip_outside(&mesh, &rounded_polygon(k.rect, k.cut)))
    }
}

/// CSS's corner clamping: where two radii on one side add up to more than
/// its length, all four scale down by the same factor.
pub(crate) fn clamp_radii([nw, ne, sw, se]: [f32; 4], rect: Rect) -> [f32; 4] {
    let (w, h) = (rect.width().max(0.0), rect.height().max(0.0));
    let f = [(w, nw + ne), (w, sw + se), (h, nw + sw), (h, ne + se)]
        .into_iter()
        .filter(|(_, sum)| *sum > 0.0)
        .map(|(len, sum)| len / sum)
        .fold(1.0_f32, f32::min);
    [nw, ne, sw, se].map(|r| (r * f).max(0.0))
}

/// A shadow corner's radius: the box's radius plus the spread, where a
/// radius under a positive spread grows less (CSS Backgrounds 3, "spread
/// distance"), floored at 0.
fn spread_radius(r: f32, spread: f32) -> f32 {
    if spread > 0.0 && r < spread {
        r + spread * (1.0 + (r / spread - 1.0).powi(3))
    } else {
        (r + spread).max(0.0)
    }
}

/// `rect` grown by `d` on every side; a side that would cross the other
/// stops at the centre.
fn grow(rect: Rect, d: f32) -> Rect {
    let c = rect.center();
    let half = (rect.size() / 2.0 + egui::Vec2::splat(d)).max(egui::Vec2::ZERO);
    Rect::from_center_size(c, half * 2.0)
}

fn add_layer(
    mesh: &mut Mesh,
    rect: Rect,
    radii: [f32; 4],
    layer: &ShadowLayer,
    opacity: f32,
    pixels_per_point: f32,
) {
    let spread = layer.spread();
    let shape = grow(rect.translate(layer.offset()), spread);
    let shape_radii = clamp_radii(radii.map(|r| spread_radius(r, spread)), shape);
    let sigma = layer.blur() / 2.0;
    let [r, g, b, _] = layer.color().to_srgba_unmultiplied();
    let alpha = layer.alpha() * opacity;
    // Ring offsets from the shape's outline and their coverage. A blur of
    // 0 is a hard edge, feathered over one physical pixel as egui does.
    let rings: Vec<(f32, Option<f32>)> = if sigma > 0.0 {
        let n = (2.0 * REACH * RINGS_PER_SIGMA) as usize;
        (0..=n)
            .map(|k| (sigma * (k as f32 / RINGS_PER_SIGMA - REACH), None))
            .collect()
    } else {
        let h = 0.5 / pixels_per_point;
        vec![(-h, Some(1.0)), (h, Some(0.0))]
    };
    let per_ring = 4 * (CORNER_SEGMENTS + 1 + 2 * SIDE_POINTS);
    let first = mesh.vertices.len() as u32;
    for &(d, fixed) in &rings {
        let ring = grow(shape, d);
        let ring_radii = clamp_radii(shape_radii.map(|r| (r + d).max(0.0)), ring);
        contour(ring, ring_radii, sigma / RINGS_PER_SIGMA, |p| {
            let cov = fixed.unwrap_or_else(|| coverage(p, shape, shape_radii, sigma));
            let a = (alpha * cov).clamp(0.0, 1.0) * 255.0;
            mesh.colored_vertex(p, Color32::from_rgba_unmultiplied(r, g, b, a.round() as u8));
        });
    }
    // The innermost ring as a fan, then a strip between each pair of rings.
    let n = per_ring as u32;
    for i in 1..n - 1 {
        mesh.add_triangle(first, first + i, first + i + 1);
    }
    for r in 0..rings.len() as u32 - 1 {
        let (a, b) = (first + r * n, first + (r + 1) * n);
        for i in 0..n {
            let j = (i + 1) % n;
            mesh.add_triangle(a + i, b + i, b + j);
            mesh.add_triangle(a + i, b + j, a + j);
        }
    }
}

/// The outline of a rounded rect from the nw corner clockwise:
/// `CORNER_SEGMENTS + 1` points per corner and `SIDE_POINTS` on each side
/// next to each corner, `step` apart (closer on a short side), so rings of
/// any size pair up point by point.
fn contour(rect: Rect, [nw, ne, sw, se]: [f32; 4], step: f32, mut point: impl FnMut(Pos2)) {
    let corners = [
        (egui::pos2(rect.min.x + nw, rect.min.y + nw), nw, PI),
        (egui::pos2(rect.max.x - ne, rect.min.y + ne), ne, 1.5 * PI),
        (egui::pos2(rect.max.x - se, rect.max.y - se), se, 0.0),
        (egui::pos2(rect.min.x + sw, rect.max.y - sw), sw, FRAC_PI_2),
    ];
    let arc = |(c, r, start): (Pos2, f32, f32), s: usize| {
        let a = start + FRAC_PI_2 * s as f32 / CORNER_SEGMENTS as f32;
        c + r * egui::vec2(a.cos(), a.sin())
    };
    for k in 0..4 {
        for s in 0..=CORNER_SEGMENTS {
            point(arc(corners[k], s));
        }
        let (a, b) = (
            arc(corners[k], CORNER_SEGMENTS),
            arc(corners[(k + 1) % 4], 0),
        );
        let len = (b - a).length();
        let dir = if len > 0.0 {
            (b - a) / len
        } else {
            egui::Vec2::ZERO
        };
        let at = |j: usize| (j as f32 * step).min(len / 2.0);
        for j in 1..=SIDE_POINTS {
            point(a + dir * at(j));
        }
        for j in (1..=SIDE_POINTS).rev() {
            point(b - dir * at(j));
        }
    }
}

/// The share of a Gaussian of deviation `sigma` around `p` that falls in
/// the rounded rect: exact along x (the error function), integrated along
/// y, numerically only across the corner bands.
fn coverage(p: Pos2, rect: Rect, [nw, ne, sw, se]: [f32; 4], sigma: f32) -> f32 {
    let cdf = |t: f32| 0.5 * (1.0 + erf(t / (sigma * SQRT_2)));
    // The shape's span at height y, as a share of the Gaussian along x.
    let row = |y: f32| {
        let inset = |top: f32, bottom: f32| {
            let (r, dy) = if y < rect.min.y + top {
                (top, rect.min.y + top - y)
            } else if y > rect.max.y - bottom {
                (bottom, y - (rect.max.y - bottom))
            } else {
                return 0.0;
            };
            r - (r * r - dy * dy).max(0.0).sqrt()
        };
        let (x0, x1) = (rect.min.x + inset(nw, sw), rect.max.x - inset(ne, se));
        (cdf(x1 - p.x) - cdf(x0 - p.x)).max(0.0)
    };
    let top_band = (rect.min.y + nw.max(ne)).min(rect.max.y);
    let bottom_band = (rect.max.y - sw.max(se)).max(top_band);
    // Straight sides: the span is constant, the y integral exact.
    let mut sum = row(rect.center().y.clamp(top_band, bottom_band))
        * (cdf(bottom_band - p.y) - cdf(top_band - p.y)).max(0.0);
    let norm = 1.0 / (sigma * (2.0 * PI).sqrt());
    let window = 4.0 * sigma;
    for (a, b) in [(rect.min.y, top_band), (bottom_band, rect.max.y)] {
        let (a, b) = (a.max(p.y - window), b.min(p.y + window));
        if b <= a {
            continue;
        }
        let dy = (b - a) / BAND_SAMPLES as f32;
        for i in 0..BAND_SAMPLES {
            let y = a + (i as f32 + 0.5) * dy;
            let t = (y - p.y) / sigma;
            sum += row(y) * norm * (-0.5 * t * t).exp() * dy;
        }
    }
    sum.clamp(0.0, 1.0)
}

/// The error function, Abramowitz and Stegun 7.1.26 (error below 1.5e-7).
fn erf(x: f32) -> f32 {
    let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
    let poly = t
        * (0.254_829_6
            + t * (-0.284_496_74 + t * (1.421_413_8 + t * (-1.453_152 + t * 1.061_405_4))));
    (1.0 - poly * (-x * x).exp()).copysign(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erf_matches_known_values() {
        for (x, want) in [
            (0.0, 0.0),
            (0.5, 0.520_499_9),
            (1.0, 0.842_700_8),
            (2.0, 0.995_322_3),
        ] {
            assert!((erf(x) - want).abs() < 1e-6, "erf({x})");
            assert!((erf(-x) + want).abs() < 1e-6, "erf(-{x})");
        }
    }

    #[test]
    fn coverage_of_a_wide_box_is_the_gaussian_edge() {
        // Far from the corners the blurred edge is the normal CDF.
        let rect = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1000.0, 100.0));
        let r = [8.0; 4];
        let at = |y: f32| coverage(egui::pos2(500.0, y), rect, r, 10.0);
        assert!((at(100.0) - 0.5).abs() < 1e-3);
        assert!((at(110.0) - 0.158_655).abs() < 1e-3);
        assert!((at(50.0) - 1.0).abs() < 1e-3);
        // A corner gives less than either side.
        let corner = coverage(egui::pos2(1000.0, 100.0), rect, r, 10.0);
        assert!(corner < 0.25 && corner > 0.1, "{corner}");
    }

    #[test]
    fn spread_follows_css() {
        assert_eq!(spread_radius(16.0, -12.0), 4.0);
        assert_eq!(spread_radius(4.0, -12.0), 0.0);
        assert_eq!(spread_radius(0.0, 4.0), 0.0);
        assert_eq!(spread_radius(8.0, 4.0), 12.0);
    }
}
