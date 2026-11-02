//! The font crates this build knows: [`files`] maps an upstream font id to
//! the crate behind its `font-<id>` feature.

mod registry;

use crate::preset::FontName;
use registry::UNLICENSED;

pub use registry::files;

/// The warning `Preset::theme` logs when [`files`] has nothing for `name`
/// (`heading`: the preset's heading font).
pub(crate) fn fallback_warning(name: FontName, heading: bool) -> String {
    let id = name.id();
    let what = if heading { "heading font" } else { "font" };
    if UNLICENSED.contains(&name) {
        format!("shadcn-egui: {what} \"{id}\" is not available (licence); using \"inter\"")
    } else {
        format!("shadcn-egui: {what} \"{id}\" needs the \"font-{id}\" feature; using \"inter\"")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `UNLICENSED` lists exactly the FONTS.tsv rows fontgen skips.
    #[test]
    #[ignore = "requires upstream tools/fontgen/FONTS.tsv, absent from the published crate"]
    fn unlicensed_matches_fonts_tsv() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/fontgen/FONTS.tsv");
        let tsv = std::fs::read_to_string(path).expect("FONTS.tsv");
        let mut expected: Vec<FontName> = tsv
            .lines()
            .skip(1)
            .filter_map(|l| {
                let cols: Vec<&str> = l.split('\t').collect();
                (!matches!(cols[3], "OFL-1.1" | "Apache-2.0"))
                    .then(|| FontName::from_id(cols[0]).expect(cols[0]))
            })
            .collect();
        expected.sort_by_key(|f| f.id());
        let mut listed = UNLICENSED.to_vec();
        listed.sort_by_key(|f| f.id());
        assert_eq!(listed, expected);
    }
}
