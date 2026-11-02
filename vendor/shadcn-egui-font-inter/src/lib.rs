//! Inter for shadcn-egui. The font is licensed under the SIL Open Font
//! License 1.1 (`LICENSE-FONT.txt`); this crate's code is MIT.

use shadcn_egui_core::{FontFiles, FontKind, FontSource};

/// Inter, variable weight 100–900.
pub static FONT: FontFiles = FontFiles {
    name: "inter",
    kind: FontKind::Sans,
    source: FontSource::Variable {
        data: include_bytes!("../fonts/inter.ttf"),
        wght: 100.0..=900.0,
    },
    y_offset_factor: 0.0,
    license: include_str!("../LICENSE-FONT.txt"),
};
