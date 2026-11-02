//! shadcn base colours and colour themes, generated from shadcn/ui
//! `themes.ts` (`generated.rs`, by `tools/stylegen`), and the merge upstream
//! `buildRegistryTheme` applies to them.

#[rustfmt::skip]
#[allow(clippy::type_complexity)]
mod generated;

use crate::color::oklch_to_srgb_a;
use crate::theme::Palette;
use egui::Color32;

/// An upstream `oklch(l c h / alpha)`; alpha 1.0 is opaque.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Oklch(pub f32, pub f32, pub f32, pub f32);

impl Oklch {
    fn color(self) -> Color32 {
        oklch_to_srgb_a(self.0, self.1, self.2, self.3)
    }
}

/// The `oklch(l c h / alpha)` a theme defines for `c`, before it was clipped to sRGB:
/// `Some` when `c` is exactly the `Color32` generated for an upstream
/// token, as long as the token is the only one that clips to it. The
/// browser mixes and fades the token unclipped and clips the result; a
/// colour an app sets itself has no entry and mixes as it is. Keyed by the
/// colour itself, so it cannot go stale when an app writes a palette field.
pub(crate) fn unclipped(c: Color32) -> Option<[f32; 4]> {
    use std::collections::HashMap;
    static TABLE: std::sync::LazyLock<HashMap<Color32, Option<[f32; 4]>>> =
        std::sync::LazyLock::new(|| {
            let bases = generated::BASES.iter().flat_map(|b| [b.2, b.3]);
            let overlays = generated::OVERLAYS.iter().flat_map(|o| [o.1, o.2]);
            let mut table: HashMap<Color32, Option<[f32; 4]>> = HashMap::new();
            for &(_, o) in bases.chain(overlays).flatten() {
                let lch = [o.0, o.1, o.2, o.3];
                table
                    .entry(o.color())
                    .and_modify(|seen| {
                        // Two tokens clip to one colour: no single source.
                        if seen.is_some_and(|s| s != lch) {
                            *seen = None;
                        }
                    })
                    .or_insert(Some(lch));
            }
            table
        });
    TABLE.get(&c).copied().flatten()
}

/// The upstream CSS vars a [`Palette`] holds. `sidebar-*` and `radius` are
/// not palette colours and are not generated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Var {
    Background,
    Foreground,
    Card,
    CardForeground,
    Popover,
    PopoverForeground,
    Primary,
    PrimaryForeground,
    Secondary,
    SecondaryForeground,
    Muted,
    MutedForeground,
    Accent,
    AccentForeground,
    Destructive,
    Border,
    Input,
    Ring,
    Chart1,
    Chart2,
    Chart3,
    Chart4,
    Chart5,
}

type Vars = &'static [(Var, Oklch)];

/// A base colour: (radius in rem, light vars, dark vars).
fn base(id: &str) -> Option<(f32, Vars, Vars)> {
    generated::BASES
        .iter()
        .find(|b| b.0 == id)
        .map(|b| (b.1, b.2, b.3))
}

/// A theme's vars: a base name overlays that whole base (upstream `THEMES`
/// holds the bases too), else a colour overlay.
fn theme_vars(id: &str, dark: bool) -> Option<Vars> {
    let pick = |light: Vars, d: Vars| if dark { d } else { light };
    base(id).map(|(_, l, d)| pick(l, d)).or_else(|| {
        generated::OVERLAYS
            .iter()
            .find(|o| o.0 == id)
            .map(|o| pick(o.1, o.2))
    })
}

/// Every field transparent except `destructive_foreground`, which upstream
/// dropped and the components still read.
fn blank() -> Palette {
    let t = Color32::TRANSPARENT;
    Palette {
        background: t,
        foreground: t,
        card: t,
        card_foreground: t,
        popover: t,
        popover_foreground: t,
        primary: t,
        primary_foreground: t,
        secondary: t,
        secondary_foreground: t,
        muted: t,
        muted_foreground: t,
        accent: t,
        accent_foreground: t,
        destructive: t,
        destructive_foreground: Oklch(0.985, 0.0, 0.0, 1.0).color(),
        border: t,
        input: t,
        ring: t,
        chart: [t; 5],
    }
}

fn set(p: &mut Palette, v: Var, c: Color32) {
    match v {
        Var::Background => p.background = c,
        Var::Foreground => p.foreground = c,
        Var::Card => p.card = c,
        Var::CardForeground => p.card_foreground = c,
        Var::Popover => p.popover = c,
        Var::PopoverForeground => p.popover_foreground = c,
        Var::Primary => p.primary = c,
        Var::PrimaryForeground => p.primary_foreground = c,
        Var::Secondary => p.secondary = c,
        Var::SecondaryForeground => p.secondary_foreground = c,
        Var::Muted => p.muted = c,
        Var::MutedForeground => p.muted_foreground = c,
        Var::Accent => p.accent = c,
        Var::AccentForeground => p.accent_foreground = c,
        Var::Destructive => p.destructive = c,
        Var::Border => p.border = c,
        Var::Input => p.input = c,
        Var::Ring => p.ring = c,
        Var::Chart1 => p.chart[0] = c,
        Var::Chart2 => p.chart[1] = c,
        Var::Chart3 => p.chart[2] = c,
        Var::Chart4 => p.chart[3] = c,
        Var::Chart5 => p.chart[4] = c,
    }
}

/// Upstream `buildRegistryTheme` (config.ts): base vars, then the theme on
/// top, then `chart-1..5` from the chart theme. `None` for an unknown id.
pub(crate) fn merge(base_id: &str, theme: &str, chart: &str, dark: bool) -> Option<Palette> {
    let (_, light_vars, dark_vars) = base(base_id)?;
    let theme = theme_vars(theme, dark)?;
    let chart = theme_vars(chart, dark)?;
    let mut p = blank();
    for &(v, c) in if dark { dark_vars } else { light_vars } {
        set(&mut p, v, c.color());
    }
    for &(v, c) in theme {
        set(&mut p, v, c.color());
    }
    for &(v, c) in chart {
        if matches!(
            v,
            Var::Chart1 | Var::Chart2 | Var::Chart3 | Var::Chart4 | Var::Chart5
        ) {
            set(&mut p, v, c.color());
        }
    }
    Some(p)
}

/// A base colour's own radius (`radius: "0.625rem"` upstream) in px;
/// `None` when `base_id` has no generated palette (`gray`, index §J).
pub(crate) fn base_radius_px(base_id: &str) -> Option<f32> {
    base(base_id).map(|(rem, _, _)| rem * 16.0)
}

/// A base or a colour overlay has this id (`false` for `gray`, index §J).
pub(crate) fn is_theme(id: &str) -> bool {
    theme_vars(id, false).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where generated tokens with different `oklch(l c h)` clip to one
    /// `Color32`, [`unclipped`] has no entry and the colour mixes as itself:
    /// each token must lie so close to that colour that no mix moves by
    /// 1/255 (hue error times a mix's chroma, at most 0.4, under 1e-3).
    /// Today one: stone and taupe's `oklch(0.147 0.004 49.25)` and
    /// `oklch(0.147 0.004 49.3)`, both `#0C0A09`.
    #[test]
    fn tokens_that_clip_to_one_colour_mix_as_that_colour() {
        use std::collections::BTreeMap;
        let bases = generated::BASES.iter().flat_map(|b| [b.2, b.3]);
        let overlays = generated::OVERLAYS.iter().flat_map(|o| [o.1, o.2]);
        let mut seen: BTreeMap<[u8; 4], Vec<[f32; 3]>> = BTreeMap::new();
        for &(_, o) in bases.chain(overlays).flatten() {
            let lch = [o.0, o.1, o.2];
            let sources = seen.entry(o.color().to_array()).or_default();
            if !sources.contains(&lch) {
                sources.push(lch);
            }
        }
        let collisions: Vec<_> = seen.iter().filter(|(_, s)| s.len() > 1).collect();
        assert_eq!(collisions.len(), 1, "{collisions:#?}");
        for (&[r, g, b, a], sources) in collisions {
            let own = Color32::from_rgba_unmultiplied(r, g, b, a);
            assert_eq!(unclipped(own), None);
            let (l, c, h) = crate::color::srgb_to_oklch(own);
            for &[sl, sc, sh] in sources {
                let dh = ((sh - h + 180.0).rem_euclid(360.0) - 180.0).to_radians();
                let off = (sl - l).abs().max((sc - c).abs()).max(dh.abs() * 0.4);
                assert!(off < 1e-3, "{own:?}: oklch({sl} {sc} {sh}) is {off} off");
            }
        }
    }
}
