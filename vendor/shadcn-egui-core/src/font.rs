//! Font data a font crate exports and `shadcn_egui::Theme` installs.

use std::ops::RangeInclusive;

/// The font's upstream `type`; a mono body font also becomes egui's
/// `Monospace`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontKind {
    Sans,
    Serif,
    Mono,
}

/// The CSS weights a theme installs: regular, medium, semibold, bold.
pub const WEIGHTS: [u16; 4] = [400, 500, 600, 700];

/// The font files of one family.
#[derive(Clone)]
pub enum FontSource {
    /// One variable font; `wght` is its weight axis range, `start <= end`
    /// (see [`FontFiles::face`] for other ranges).
    Variable {
        data: &'static [u8],
        wght: RangeInclusive<f32>,
    },
    /// Static files, only for fonts without a variable version. A missing
    /// weight uses the nearest one that exists.
    Static {
        regular: &'static [u8],
        medium: Option<&'static [u8]>,
        semibold: Option<&'static [u8]>,
        bold: Option<&'static [u8]>,
    },
}

/// One font family as a font crate ships it.
#[derive(Clone)]
pub struct FontFiles {
    /// The upstream font id (`"inter"`, `"jetbrains-mono"`).
    pub name: &'static str,
    pub kind: FontKind,
    pub source: FontSource,
    /// egui `FontTweak::y_offset_factor`: a correction, in em, on top of the
    /// browser's baseline placement that the crate's text path gives every
    /// font. 0 by default; a font gets a value only where its glyphs, on
    /// that baseline, still sit visibly off Chrome's.
    pub y_offset_factor: f32,
    /// The font's licence text (OFL-1.1 or Apache-2.0).
    pub license: &'static str,
}

/// The bytes and variation coordinate to register for one CSS weight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Face {
    pub data: &'static [u8],
    /// `Some` for a variable font: the `wght` coordinate.
    pub wght: Option<f32>,
}

impl FontFiles {
    /// The face for a CSS weight (see [`WEIGHTS`]). A variable font's
    /// weight is raised to the axis start, then lowered to its end, so an
    /// inverted range yields its end and a NaN bound is ignored.
    pub fn face(&self, weight: u16) -> Face {
        match &self.source {
            FontSource::Variable { data, wght } => Face {
                data,
                wght: Some(f32::from(weight).max(*wght.start()).min(*wght.end())),
            },
            FontSource::Static {
                regular,
                medium,
                semibold,
                bold,
            } => {
                let have = [
                    (400u16, Some(*regular)),
                    (500, *medium),
                    (600, *semibold),
                    (700, *bold),
                ];
                // CSS font matching: on a tie, lighter for <= 500, heavier above.
                let tie = |w: u16| if weight <= 500 { w } else { u16::MAX - w };
                let data = have
                    .iter()
                    .filter_map(|&(w, d)| d.map(|d| (w, d)))
                    .min_by_key(|&(w, _)| (w.abs_diff(weight), tie(w)))
                    .map(|(_, d)| d)
                    .unwrap_or(*regular);
                Face { data, wght: None }
            }
        }
    }
}

impl std::fmt::Debug for FontFiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("FontFiles").field(&self.name).finish()
    }
}

impl PartialEq for FontFiles {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static VAR: FontFiles = FontFiles {
        name: "var",
        kind: FontKind::Sans,
        source: FontSource::Variable {
            data: b"var",
            wght: 200.0..=650.0,
        },
        y_offset_factor: 0.0,
        license: "",
    };
    static ONLY_REGULAR: FontFiles = FontFiles {
        name: "only-regular",
        kind: FontKind::Serif,
        source: FontSource::Static {
            regular: b"r",
            medium: None,
            semibold: None,
            bold: None,
        },
        y_offset_factor: 0.0,
        license: "",
    };
    static REGULAR_AND_BOLD: FontFiles = FontFiles {
        name: "regular-bold",
        kind: FontKind::Sans,
        source: FontSource::Static {
            regular: b"r",
            medium: None,
            semibold: None,
            bold: Some(b"b"),
        },
        y_offset_factor: 0.0,
        license: "",
    };

    #[test]
    fn variable_weights_are_clamped_into_the_axis() {
        assert_eq!(
            VAR.face(400),
            Face {
                data: b"var",
                wght: Some(400.0)
            }
        );
        assert_eq!(
            VAR.face(700).wght,
            Some(650.0),
            "above the axis clamps to its max"
        );
        assert_eq!(
            VAR.face(100).wght,
            Some(200.0),
            "below the axis clamps to its min"
        );
    }

    #[test]
    fn an_inverted_or_nan_axis_does_not_panic() {
        let var = |wght| FontFiles {
            source: FontSource::Variable { data: b"var", wght },
            ..VAR.clone()
        };
        assert_eq!(
            var(900.0..=100.0).face(400).wght,
            Some(100.0),
            "inverted: the end wins"
        );
        assert_eq!(
            var(f32::NAN..=900.0).face(400).wght,
            Some(400.0),
            "a NaN bound is ignored"
        );
        assert_eq!(var(f32::NAN..=300.0).face(400).wght, Some(300.0));
        assert_eq!(var(500.0..=f32::NAN).face(400).wght, Some(500.0));
    }

    #[test]
    fn static_fonts_use_the_nearest_weight() {
        for w in WEIGHTS {
            assert_eq!(
                ONLY_REGULAR.face(w),
                Face {
                    data: b"r",
                    wght: None
                },
                "{w}"
            );
        }
        assert_eq!(
            REGULAR_AND_BOLD.face(500).data,
            b"r",
            "500: tie-free, 400 is nearer"
        );
        assert_eq!(REGULAR_AND_BOLD.face(600).data, b"b", "600: 700 is nearer");
        assert_eq!(REGULAR_AND_BOLD.face(700).data, b"b");
    }

    #[test]
    fn font_files_compare_and_print_by_name() {
        assert_eq!(VAR, VAR.clone());
        assert_ne!(VAR, ONLY_REGULAR);
        assert_eq!(format!("{VAR:?}"), "FontFiles(\"var\")");
    }
}
