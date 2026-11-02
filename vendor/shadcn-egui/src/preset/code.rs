//! Preset codes: a port of upstream `decodePreset` / `encodePreset`
//! (shadcn/ui `packages/registry/src/preset/preset.ts`). A code is a version letter (`a`:
//! 40 bits, `b`: 51 bits) and a base62 number whose bit fields are the list
//! indices, low bits first in `FIELDS_V2` order.

use super::generated::{Field, FIELDS_V1, FIELDS_V2, V1_CHART_COLORS};
use super::{
    BaseColor, FontHeading, FontName, IconLibrary, MenuAccent, MenuColor, Preset, RadiusChoice,
    StyleName, ThemeColor,
};

const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Upstream decodes in f64, exact only up to 2^53; above that we refuse.
const MAX_PAYLOAD: u64 = 1 << 53;

/// Why a string is not a preset code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PresetCodeError {
    /// Empty or a single character.
    TooShort,
    /// The first character is not a known version (`a`, `b`).
    UnknownVersion(char),
    /// A character after the version is not base62 (`0-9A-Za-z`).
    InvalidChar(char),
    /// The number is above 2^53, which the shadcn sites never write.
    TooLong,
}

impl std::fmt::Display for PresetCodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => write!(f, "preset code is too short"),
            Self::UnknownVersion(c) => write!(f, "unknown preset code version {c:?}"),
            Self::InvalidChar(c) => write!(f, "invalid character {c:?} in preset code"),
            Self::TooLong => write!(f, "preset code is too long"),
        }
    }
}

impl std::error::Error for PresetCodeError {}

fn digit(c: char) -> Option<u64> {
    BASE62
        .iter()
        .position(|&d| char::from(d) == c)
        .map(|i| i as u64)
}

pub(super) fn to_base62(mut n: u64) -> String {
    let mut out = Vec::new();
    loop {
        out.push(BASE62[(n % 62) as usize]);
        n /= 62;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).expect("base62 is ASCII")
}

/// `all[idx]`, or `all[0]` past the end (upstream behaviour).
fn pick<T: Copy>(all: &[T], idx: usize) -> T {
    all.get(idx).copied().unwrap_or(all[0])
}

impl Preset {
    /// Decode a preset code from ui.shadcn.com (`"b0"`, `"a…"`). An index
    /// past the end of a list decodes to the list's first value, as upstream
    /// does. An `a` code has no chart colour; it gets the one the site gives
    /// it (`V1_CHART_COLOR_MAP[theme]`, else the theme). Codes from
    /// shadcn-svelte.com use other lists and decode to other presets.
    pub fn from_code(code: &str) -> Result<Preset, PresetCodeError> {
        let mut chars = code.chars();
        let version = chars.next().ok_or(PresetCodeError::TooShort)?;
        if code.chars().nth(1).is_none() {
            return Err(PresetCodeError::TooShort);
        }
        let fields: &[(Field, u32)] = match version {
            'a' => &FIELDS_V1,
            'b' => &FIELDS_V2,
            c => return Err(PresetCodeError::UnknownVersion(c)),
        };
        let mut bits: u64 = 0;
        for c in chars {
            let d = digit(c).ok_or(PresetCodeError::InvalidChar(c))?;
            bits = bits
                .checked_mul(62)
                .and_then(|b| b.checked_add(d))
                .filter(|&b| b <= MAX_PAYLOAD)
                .ok_or(PresetCodeError::TooLong)?;
        }
        // Fields an `a` code lacks keep these: heading inherit; chart set below.
        let mut p = Preset {
            style: StyleName::ALL[0],
            base_color: BaseColor::ALL[0],
            theme: ThemeColor::ALL[0],
            chart_color: ThemeColor::ALL[0],
            font: FontName::ALL[0],
            font_heading: FontHeading::Inherit,
            icons: IconLibrary::ALL[0],
            radius: RadiusChoice::ALL[0],
            menu_color: MenuColor::ALL[0],
            menu_accent: MenuAccent::ALL[0],
        };
        let mut offset = 0;
        for &(field, width) in fields {
            let idx = ((bits >> offset) & ((1 << width) - 1)) as usize;
            p.set_index(field, idx);
            offset += width;
        }
        if version == 'a' {
            p.chart_color = V1_CHART_COLORS
                .iter()
                .find(|(t, _)| *t == p.theme)
                .map_or(p.theme, |&(_, c)| c);
        }
        Ok(p)
    }

    /// The `b` code the shadcn sites write for this preset; `None` when the
    /// radius is [`RadiusChoice::Px`], which has no code.
    pub fn to_code(&self) -> Option<String> {
        let mut bits: u64 = 0;
        let mut offset = 0;
        for &(field, width) in &FIELDS_V2 {
            bits |= (self.index(field)? as u64) << offset;
            offset += width;
        }
        Some(format!("b{}", to_base62(bits)))
    }

    fn set_index(&mut self, field: Field, idx: usize) {
        match field {
            Field::MenuColor => self.menu_color = pick(MenuColor::ALL, idx),
            Field::MenuAccent => self.menu_accent = pick(MenuAccent::ALL, idx),
            Field::Radius => self.radius = pick(RadiusChoice::ALL, idx),
            Field::Font => self.font = pick(FontName::ALL, idx),
            Field::IconLibrary => self.icons = pick(IconLibrary::ALL, idx),
            Field::Theme => self.theme = pick(ThemeColor::ALL, idx),
            Field::BaseColor => self.base_color = pick(BaseColor::ALL, idx),
            Field::Style => self.style = pick(StyleName::ALL, idx),
            Field::ChartColor => self.chart_color = pick(ThemeColor::ALL, idx),
            Field::FontHeading => {
                self.font_heading = match idx.checked_sub(1).and_then(|i| FontName::ALL.get(i)) {
                    Some(&f) => FontHeading::Font(f),
                    None => FontHeading::Inherit,
                }
            }
        }
    }

    /// The list index of a field; `None` only for a `Px` radius.
    fn index(&self, field: Field) -> Option<usize> {
        fn pos<T: PartialEq>(all: &[T], v: &T) -> Option<usize> {
            all.iter().position(|x| x == v)
        }
        match field {
            Field::MenuColor => pos(MenuColor::ALL, &self.menu_color),
            Field::MenuAccent => pos(MenuAccent::ALL, &self.menu_accent),
            Field::Radius => pos(RadiusChoice::ALL, &self.radius),
            Field::Font => pos(FontName::ALL, &self.font),
            Field::IconLibrary => pos(IconLibrary::ALL, &self.icons),
            Field::Theme => pos(ThemeColor::ALL, &self.theme),
            Field::BaseColor => pos(BaseColor::ALL, &self.base_color),
            Field::Style => pos(StyleName::ALL, &self.style),
            Field::ChartColor => pos(ThemeColor::ALL, &self.chart_color),
            Field::FontHeading => match self.font_heading {
                FontHeading::Inherit => Some(0),
                FontHeading::Font(f) => pos(FontName::ALL, &f).map(|i| i + 1),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PresetCodeError::*;

    #[test]
    fn each_error() {
        assert_eq!(Preset::from_code(""), Err(TooShort));
        assert_eq!(Preset::from_code("b"), Err(TooShort));
        assert_eq!(Preset::from_code("c0"), Err(UnknownVersion('c')));
        assert_eq!(Preset::from_code("b0-"), Err(InvalidChar('-')));
        assert_eq!(TooLong.to_string(), "preset code is too long");
    }

    #[test]
    fn too_long_starts_above_two_to_the_53() {
        assert!(Preset::from_code(&format!("b{}", to_base62(1 << 53))).is_ok());
        assert_eq!(
            Preset::from_code(&format!("b{}", to_base62((1 << 53) + 1))),
            Err(TooLong)
        );
    }

    #[test]
    fn base62() {
        assert_eq!(
            (to_base62(0), to_base62(61), to_base62(62)),
            ("0".into(), "z".into(), "10".into())
        );
    }

    #[test]
    fn all_zero_is_index_zero_everywhere() {
        let p = Preset::from_code("b0").expect("decodes");
        assert_eq!(p.style, StyleName::Nova);
        assert_eq!(
            (p.base_color, p.theme, p.chart_color),
            (BaseColor::Neutral, ThemeColor::Neutral, ThemeColor::Neutral)
        );
        assert_eq!(
            (p.font, p.font_heading, p.icons),
            (FontName::Inter, FontHeading::Inherit, IconLibrary::Lucide)
        );
        assert_eq!(
            (p.radius, p.menu_color, p.menu_accent),
            (
                RadiusChoice::Default,
                MenuColor::Default,
                MenuAccent::Subtle
            )
        );
    }

    #[test]
    fn an_index_past_the_list_decodes_to_index_zero() {
        // style: 6 bits at offset 34; 63 is past the 8 styles.
        assert_eq!(
            Preset::from_code(&format!("b{}", to_base62(63 << 34))).map(|p| p.style),
            Ok(StyleName::Nova)
        );
        assert_eq!(
            Preset::from_code(&format!("b{}", to_base62(1 << 34))).map(|p| p.style),
            Ok(StyleName::Vega)
        );
        // fontHeading: 5 bits at offset 46; 27 and up are past "inherit" + 26 fonts.
        assert_eq!(
            Preset::from_code(&format!("b{}", to_base62(31 << 46))).map(|p| p.font_heading),
            Ok(FontHeading::Inherit)
        );
        assert_eq!(
            Preset::from_code(&format!("b{}", to_base62(26 << 46))).map(|p| p.font_heading),
            Ok(FontHeading::Font(FontName::InstrumentSerif))
        );
    }

    #[test]
    fn a_codes_ignore_the_v2_fields() {
        let p =
            Preset::from_code(&format!("a{}", to_base62((5 << 40) | (3 << 46)))).expect("decodes");
        // theme neutral (index 0): V1_CHART_COLOR_MAP gives blue.
        assert_eq!(
            (p.chart_color, p.font_heading),
            (ThemeColor::Blue, FontHeading::Inherit)
        );
    }

    #[test]
    fn a_codes_take_the_chart_colour_from_the_theme() {
        // theme: 6 bits at offset 22 (menuColor 3, menuAccent 3, radius 4, font 6, iconLibrary 6).
        let theme = |t: ThemeColor| {
            let i = ThemeColor::ALL
                .iter()
                .position(|&x| x == t)
                .expect("listed") as u64;
            Preset::from_code(&format!("a{}", to_base62(i << 22))).expect("decodes")
        };
        assert_eq!(
            theme(ThemeColor::Stone).chart_color,
            ThemeColor::Lime,
            "keyed by theme"
        );
        assert_eq!(theme(ThemeColor::Taupe).chart_color, ThemeColor::Cyan);
        assert_eq!(
            theme(ThemeColor::Rose).chart_color,
            ThemeColor::Rose,
            "not in the map: the theme itself"
        );
        assert_eq!(theme(ThemeColor::Gray).chart_color, ThemeColor::Gray);
        assert_eq!(
            theme(ThemeColor::Stone).base_color,
            BaseColor::Neutral,
            "the base colour plays no part"
        );
        let b = Preset::from_code(&format!("b{}", to_base62(1 << 22))).expect("decodes");
        assert_eq!(
            (b.theme, b.chart_color),
            (ThemeColor::Stone, ThemeColor::Neutral),
            "b codes carry their own chart"
        );
    }

    #[test]
    fn px_radius_has_no_code() {
        let p = Preset::from_code("b0").expect("decodes");
        assert_eq!(
            Preset {
                radius: RadiusChoice::Px(6.0),
                ..p
            }
            .to_code(),
            None
        );
        assert_eq!(p.to_code().as_deref(), Some("b0"));
    }
}
