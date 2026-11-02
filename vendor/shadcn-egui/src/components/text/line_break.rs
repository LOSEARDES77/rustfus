//! Break opportunities between two chars that are not spaces, as Blink
//! finds them under `word-break: normal`, `line-break: auto` (Chromium 156,
//! `LazyLineBreakIterator::NextBreakablePosition`; conformance 3.4). Blink
//! breaks after every run of spaces itself; between two other chars it asks,
//! in this order:
//!
//! 1. `-` before an ASCII digit: a break only where the char before the `-`
//!    is an ASCII letter or digit (`x-|1`, `pre-|1990`, `2-|3`; not `-1`).
//! 2. Two chars from `!` to U+00FF: a pair table. ASCII pairs follow
//!    Blink's own rules (`LineBreakData::FillAscii`: after `-` and `?`,
//!    before `(`, `<`, `[`, `{` after some punctuation); the other pairs are
//!    what ICU's line iterator says for the two chars alone.
//! 3. Else ICU's line iterator: the rules of UAX #14 over the line break
//!    classes of the chars.
//!
//! UAX #14 rules are built for the classes [`class`] gives: letters and
//! digits, the ASCII and Latin-1 punctuation, dashes (`BA`, `B2`, `HY`),
//! the zero-width space, the soft hyphen, no-break chars (`GL`, `WJ`),
//! combining marks, CJK ideographs, kana, Hangul syllables (as `ID`) and
//! the CJK punctuation around them. Every other char counts as a letter
//! (`AL`), which never breaks next to a letter: such text keeps its breaks
//! at spaces.

/// A UAX #14 line break class (`AI` resolves to `AL`; `CJ`, the small kana
/// and the prolonged sound mark, to `ID`, as Chrome breaks before them under
/// `line-break: auto`; Hangul syllables count as `ID`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Class {
    Al,
    B2,
    Ba,
    Bb,
    Cl,
    Cm,
    Cp,
    Ex,
    Gl,
    Hy,
    Id,
    In,
    Is,
    Ns,
    Nu,
    Op,
    /// `OP` of East Asian width (wide, fullwidth, halfwidth): LB30 skips it.
    OpWide,
    Po,
    Pr,
    Qu,
    Sy,
    Wj,
    Zw,
}

/// The line break class of `c` (Unicode 16 `LineBreak.txt`) for the classes
/// this module builds; [`Class::Al`] for every other char.
pub(super) fn class(c: char) -> Class {
    use Class::*;
    match c {
        '0'..='9' => Nu,
        '!' | '?' => Ex,
        '"' | '\'' => Qu,
        '$' | '+' | '\\' => Pr,
        '%' => Po,
        '(' | '[' | '{' => Op,
        ')' | ']' => Cp,
        '}' => Cl,
        ',' | '.' | ':' | ';' => Is,
        '-' => Hy,
        '/' => Sy,
        '|' => Ba,
        '\u{A0}' => Gl,
        '\u{A1}' | '\u{BF}' => Op,
        '\u{A2}' | '\u{B0}' => Po,
        '\u{A3}'..='\u{A5}' | '\u{B1}' => Pr,
        '\u{AB}' | '\u{BB}' => Qu,
        '\u{AD}' => Ba,
        '\u{B4}' => Bb,
        '\u{35C}'..='\u{362}' | '\u{1DCD}' | '\u{1DFC}' => Gl,
        '\u{FE20}' | '\u{FE22}' | '\u{FE24}' | '\u{FE26}' | '\u{FE27}' | '\u{FE29}' => Gl,
        '\u{FE2B}' | '\u{FE2D}' | '\u{FE2E}' => Gl,
        '\u{300}'..='\u{34E}' | '\u{350}'..='\u{36F}' => Cm,
        '\u{1AB0}'..='\u{1ACE}' | '\u{1DC0}'..='\u{1DFF}' | '\u{20D0}'..='\u{20F0}' => Cm,
        '\u{FE00}'..='\u{FE0F}' | '\u{FE20}'..='\u{FE2F}' => Cm,
        '\u{200C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' => Cm,
        '\u{2066}'..='\u{206F}' => Cm,
        // ZWJ: no break on either side (LB8a, LB9), as a word joiner.
        '\u{200D}' => Wj,
        '\u{1680}' | '\u{2000}'..='\u{2006}' | '\u{2008}'..='\u{200A}' | '\u{205F}' => Ba,
        '\u{2056}' | '\u{2058}'..='\u{205B}' | '\u{205D}' | '\u{205E}' => Ba,
        '\u{2039}' | '\u{203A}' => Qu,
        '\u{203C}' | '\u{203D}' | '\u{2047}'..='\u{2049}' => Ns,
        '\u{2044}' => Is,
        '\u{2045}' => Op,
        '\u{2046}' => Cl,
        '\u{2057}' | '\u{20C0}' => Po,
        '\u{200B}' => Zw,
        '\u{2010}' | '\u{2012}' | '\u{2013}' | '\u{2027}' | '\u{3000}' => Ba,
        '\u{2011}' | '\u{202F}' | '\u{2007}' => Gl,
        '\u{2014}' | '\u{2E3A}' | '\u{2E3B}' => B2,
        '\u{2018}' | '\u{2019}' | '\u{201B}'..='\u{201D}' | '\u{201F}' => Qu,
        '\u{201A}' | '\u{201E}' => Op,
        '\u{2024}'..='\u{2026}' => In,
        '\u{2030}'..='\u{2037}' | '\u{FFE0}' => Po,
        '\u{20A0}'..='\u{20A6}' | '\u{20A8}'..='\u{20B5}' | '\u{20B7}'..='\u{20BA}' => Pr,
        '\u{20BC}'..='\u{20BD}' | '\u{20BF}'..='\u{20CF}' | '\u{FFE1}' => Pr,
        '\u{2060}' | '\u{FEFF}' => Wj,
        '\u{3001}' | '\u{3002}' | '\u{301E}' | '\u{301F}' => Cl,
        '\u{3012}' | '\u{3013}' => Id,
        '\u{3008}'..='\u{301B}' => {
            if (c as u32 - 0x3008).is_multiple_of(2) {
                OpWide
            } else {
                Cl
            }
        }
        '\u{301D}' => OpWide,
        '\u{3005}' | '\u{301C}' | '\u{303B}' | '\u{303C}' | '\u{309B}'..='\u{309E}' => Ns,
        '\u{30A0}' | '\u{30FB}' | '\u{30FD}' | '\u{30FE}' => Ns,
        '\u{302A}'..='\u{302F}' | '\u{3035}' | '\u{3099}' | '\u{309A}' => Cm,
        '\u{2E80}'..='\u{2FFF}'
        | '\u{3003}'..='\u{303F}'
        | '\u{3041}'..='\u{30FF}'
        | '\u{31F0}'..='\u{31FF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{AC00}'..='\u{D7A3}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{20000}'..='\u{2FFFD}'
        | '\u{30000}'..='\u{3FFFD}' => Id,
        '\u{FF01}' | '\u{FF1F}' => Ex,
        '\u{FF04}' => Pr,
        '\u{FF05}' => Po,
        '\u{FF08}' | '\u{FF3B}' | '\u{FF5B}' | '\u{FF5F}' | '\u{FF62}' => OpWide,
        '\u{FF09}' | '\u{FF0C}' | '\u{FF0E}' | '\u{FF3D}' | '\u{FF5D}' => Cl,
        '\u{FF60}' | '\u{FF61}' | '\u{FF63}' | '\u{FF64}' => Cl,
        '\u{FF1A}' | '\u{FF1B}' | '\u{FF65}' => Ns,
        '\u{FF67}'..='\u{FF70}' => Id,
        '\u{FF02}'..='\u{FF60}' => Id,
        _ => Al,
    }
}

/// Whether a line may break between `before` and `after`, neither a space
/// or tab, `before2` the char before `before` (`None` at the paragraph
/// start).
pub(super) fn can_break(before2: Option<char>, before: char, after: char) -> bool {
    // `ShouldBreakFast`: control chars never break.
    if before < '!' || after < '!' {
        return false;
    }
    if before == '-' && after.is_ascii_digit() {
        // A `-` that may be a minus sign keeps its number.
        return before2.is_some_and(|c| c.is_ascii_alphanumeric());
    }
    if before.is_ascii() && after.is_ascii() {
        return ascii_pair(before, after);
    }
    if before != '-' && before <= '\u{FF}' && after <= '\u{FF}' {
        // The pair table holds what ICU says for the two chars alone.
        return uax14(None, class(before), class(after));
    }
    // A space before counts as the paragraph start (LB20a).
    let before2 = before2.filter(|c| !matches!(c, ' ' | '\t')).map(class);
    uax14(before2, class(before), class(after))
}

/// `LineBreakData::FillAscii`, later rules overriding earlier ones.
fn ascii_pair(a: char, b: char) -> bool {
    let mut out = false;
    if matches!(b, '(' | '<' | '[' | '{') {
        out = true;
    }
    if matches!(a, '-' | '?') {
        out = true;
    }
    if a == '-' && b == '$' {
        out = false;
    }
    if b == '!' || (a == '?' && matches!(b, '"' | '\'')) {
        out = false;
    }
    if matches!(b, ')' | ',' | '.' | '/' | ':' | ';' | '?' | ']' | '}') {
        out = false;
    }
    if a == '-' && b.is_ascii_digit() {
        out = false;
    }
    if matches!(a, '$' | '\'' | '(' | '/' | '0'..='9' | '<' | '@' | 'A'..='Z' | '[')
        || matches!(a, '^'..='`' | 'a'..='z' | '{' | '\u{7F}')
    {
        out = false;
    }
    out
}

/// UAX #14 (rules LB7 to LB31, ICU's default tailoring) for the pair
/// `before`, `after` with no space between; `before2` is the class before
/// `before` where ICU sees it.
fn uax14(before2: Option<Class>, before: Class, after: Class) -> bool {
    use Class::*;
    // LB7, LB8: never before a zero-width space, always after it.
    if after == Zw {
        return false;
    }
    if before == Zw {
        return true;
    }
    // LB9, LB10: a combining mark joins the char before; alone, a letter.
    if after == Cm {
        return false;
    }
    let before = if before == Cm { Al } else { before };
    // LB11, LB12, LB12a: word joiners and no-break chars.
    if before == Wj || after == Wj || before == Gl {
        return false;
    }
    if after == Gl && !matches!(before, Ba | Hy) {
        return false;
    }
    // LB13, LB14: not before closing punctuation, not after opening.
    if matches!(after, Cl | Cp | Ex | Is | Sy) || matches!(before, Op | OpWide) {
        return false;
    }
    // LB15-LB19: quotes keep their neighbours (a CJK neighbour on both
    // sides of a curly quote is not built: LB19a).
    if before == Qu || after == Qu {
        return false;
    }
    // LB17: between two-em dashes.
    if before == B2 && after == B2 {
        return false;
    }
    // LB20a: a hyphen starting a word keeps the letter after it.
    if matches!(before, Hy) && after == Al && matches!(before2, None | Some(Zw | Gl)) {
        return false;
    }
    // LB21, LB22.
    if matches!(after, Ba | Hy | Ns | In) || before == Bb {
        return false;
    }
    // LB23, LB23a, LB24, LB25 (pairs), LB28, LB29, LB30.
    !matches!(
        (before, after),
        (Al, Nu | Pr | Po | Al | Op)
            | (Nu, Al | Nu | Sy | Is | Po | Pr | Op)
            | (Pr, Id | Al | Nu)
            | (Po, Al | Nu)
            | (Id, Po)
            | (Hy, Nu)
            | (Is, Al)
            | (Cp, Al | Nu)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn breaks(text: &str) -> Vec<usize> {
        let c: Vec<char> = text.chars().collect();
        (1..c.len())
            .filter(|&i| {
                c[i - 1] != ' '
                    && c[i] != ' '
                    && can_break(i.checked_sub(2).map(|k| c[k]), c[i - 1], c[i])
            })
            .collect()
    }

    #[test]
    fn hyphens_break_before_letters_and_after_alphanumerics_before_digits() {
        assert_eq!(breaks("well-known"), [5]);
        assert_eq!(breaks("x-1 pre-1990 2-3"), [2, 8, 15]);
        assert_eq!(breaks("a -1 -b"), [6]);
        assert_eq!(breaks("a-$ a-, a?b"), [10]);
    }

    #[test]
    fn dashes_soft_hyphens_and_zero_width_spaces_break() {
        // Em dash (B2): before and after; en dash (BA): after only.
        assert_eq!(breaks("a\u{2014}b"), [1, 2]);
        assert_eq!(breaks("a\u{2014}\u{2014}b"), [1, 3]);
        assert_eq!(breaks("10\u{2013}20"), [3]);
        assert_eq!(breaks("Do\u{AD}nau"), [3]);
        assert_eq!(breaks("ab\u{200B}\u{200B}cd"), [4]);
        assert_eq!(breaks("a\u{2014}, (\u{2014}b"), [1, 6]);
        assert!(breaks("a\u{A0}\u{2014}b").len() == 1);
    }

    #[test]
    fn cjk_breaks_between_ideographs_not_before_closing_punctuation() {
        // 中文，日本語「テスト」です。
        let text = "中文，日本語「テスト」です。";
        assert_eq!(breaks(text), [1, 3, 4, 5, 6, 8, 9, 11, 12]);
        // A small kana and the prolonged sound mark start a line too.
        assert_eq!(breaks("キャラーメ"), [1, 2, 3, 4]);
        assert_eq!(breaks("ab中cd"), [2, 3]);
    }

    #[test]
    fn letters_digits_and_slashes_keep_together() {
        assert!(breaks("and/or path/to/file a.b").is_empty());
        assert!(breaks("Ünïcødé łódź").is_empty());
    }
}
