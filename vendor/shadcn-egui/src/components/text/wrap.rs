//! `text-wrap: balance` and `text-wrap: pretty` as Blink breaks them
//! (Chromium 156, `third_party/blink/renderer/core/layout/inline/`):
//! `ScoreLineBreaker` (Knuth-Plass scores with Minikin's penalties) for
//! both, `ParagraphLineBreaker`'s bisection of the available width as the
//! fallback for `balance`. Positions, widths and font sizes are in device
//! px, as Blink holds them (zoomed px), so that the float sums and the
//! `LayoutUnit` snapping fall as in Blink; `zoom` is the device pixels per
//! point.

use super::line_break;

/// U+00AD: a break opportunity that draws a hyphen where it ends a line
/// (`hyphens: manual`, the CSS default), and nothing elsewhere.
pub(super) const SOFT_HYPHEN: char = '\u{AD}';

/// A break opportunity: the next line starts at char `start` at
/// `pos_no_break` (after the spaces). The line before it ends at
/// `pos_if_break` for the score breaker: its last glyph shaped alone, as
/// Blink's `AppendCandidates` reshapes a word end that is not safe to
/// break, trailing spaces hanging. The greedy breaker measures a line as
/// the shaped width of its own text instead, rounded up to a `LayoutUnit`
/// (`ShapeResult::SnappedWidth`): from `x_start`, where the next line
/// starts, to `x_end`, where this one ends, both unsnapped. It breaks
/// after a run of spaces and keeps the kerning against the first space
/// inside the line (elsewhere it reshapes the end too). A soft-hyphen
/// break (`hyphenated`) adds the hyphen to both ends: its snapped width
/// `hyphen` for the greedy breaker. Blink's `LineBreakCandidate`.
///
/// The greedy breaker snaps each inline item (a section of the job) on
/// the line on its own and sums them. `next_item` is the first item start
/// after `x_start` (infinite where none follows) and `next_items` the
/// paragraph's snapped item widths up to it; `prev_item` and `prev_items`
/// the same for the last item start before `x_end`.
///
/// An `emergency` candidate is no break opportunity: it sits between two
/// chars inside a word, where `overflow-wrap: break-word` breaks a word
/// too wide for a line of its own ([`break_word`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Candidate {
    pub start: usize,
    pub pos_no_break: f32,
    pub pos_if_break: f32,
    pub x_start: f64,
    pub x_end: f64,
    pub hyphen: f32,
    pub hyphenated: bool,
    pub next_item: f64,
    pub next_items: f64,
    pub prev_item: f64,
    pub prev_items: f64,
    pub emergency: bool,
}

/// One char of a paragraph as the breakers see it, in device px: its
/// glyph's left edge (after the kerning against the char before), its
/// right edge (before the kerning against the next), as Blink's
/// `ShapeResult` gives them (`LayoutUnit`s, a kerned right edge shaped
/// alone), the same two edges unsnapped (`x`, `x_end`: the 16.16 sums),
/// the width of the hyphen Blink draws where a soft hyphen ends a line,
/// and whether the char starts a new inline item (a section of the job).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Glyph {
    pub c: char,
    pub left: f32,
    pub right: f32,
    pub x: f64,
    pub x_end: f64,
    pub hyphen: f32,
    pub item: bool,
}

/// `kMaxLinesForBalance`, `kMaxLinesForOptimal`.
const MAX_LINES_BALANCE: usize = 6;
const MAX_LINES_PRETTY: usize = 4;
const SCORE_INFINITY: f32 = f32::MAX;
const SCORE_OVERFULL: f32 = 1e12;
const LAST_LINE_PENALTY_MULTIPLIER: f32 = 4.0; // behaviour-ok: kLastLinePenaltyMultiplier
const ORPHANS_PENALTY: f32 = 10000.0; // behaviour-ok: kOrphansPenalty
/// `ShouldOptimize`: a last line under a third of the width is short.
const SHORT_LINE_DENOMINATOR: f32 = 3.0; // behaviour-ok: kShortLineDenominator
/// `AttemptParagraphBalancing` starts bisecting at 80% of the average line
/// width ("20% buffer for potential edge cases").
const BALANCE_START: f32 = 0.8; // behaviour-ok: Blink's bisection start

/// `LayoutUnit`s per device px.
const LAYOUT_UNITS: f32 = 64.0; // behaviour-ok: LayoutUnit::kFixedPointDenominator
/// Float noise allowed when a box width snaps to the `LayoutUnit` grid, in
/// `LayoutUnit`s: the caller's width times the zoom may land a hair under
/// a grid line that Blink's own length lands on.
const SNAP_TOLERANCE: f32 = 1e-3;

/// One `LayoutUnit` in device px.
fn unit() -> f32 {
    LAYOUT_UNITS.recip()
}

/// The box width `width` as Blink holds it: truncated to the grid.
fn snap_width(width: f32) -> f32 {
    (width * LAYOUT_UNITS + SNAP_TOLERANCE).floor() / LAYOUT_UNITS
}

/// A line's shaped width as Blink holds it: rounded up to the grid
/// (`ShapeResult::SnappedWidth`).
fn snap_len(len: f32) -> f32 {
    (len * LAYOUT_UNITS).ceil() / LAYOUT_UNITS
}

/// Whether a line `len` long fits `width`: its snapped width may pass the
/// box width by one unit (`AvailableWidthToFit`).
fn fits(len: f32, width: f32) -> bool {
    snap_len(len) <= snap_width(width) + unit()
}

/// `len` exact device px rounded up to a `LayoutUnit`.
fn snap_exact(len: f64) -> f64 {
    (len * f64::from(LAYOUT_UNITS)).ceil() / f64::from(LAYOUT_UNITS)
}

/// The greedy breaker's width of the line from candidate `s` to `e`: the
/// shaped width of each item's text on it snapped (`SnappedWidth` per
/// item result), summed, plus the hyphen.
fn greedy_len(s: &Candidate, e: &Candidate) -> f32 {
    let len = if s.next_item >= e.x_end {
        snap_exact(e.x_end - s.x_start)
    } else {
        snap_exact(s.next_item - s.x_start)
            + (e.prev_items - s.next_items)
            + snap_exact(e.x_end - e.prev_item)
    };
    len as f32 + e.hyphen
}

/// Whether Blink breaks spaces off a line (`IsBreakableSpace`).
fn is_space(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// The break candidates of one paragraph (no forced break inside): a
/// sentinel at the start, one after each run of spaces followed by text,
/// one between two other chars where [`line_break::can_break`] allows it,
/// and a sentinel at the end of the last char that is not a space.
pub(super) fn candidates(chars: &[Glyph]) -> Vec<Candidate> {
    candidates_with(chars, false)
}

/// [`candidates`], and with `emergency` an emergency candidate between
/// any two other chars inside a word that [`cluster_boundary`] separates.
pub(super) fn candidates_with(chars: &[Glyph], emergency: bool) -> Vec<Candidate> {
    let (x0, x0_exact) = chars.first().map_or((0.0, 0.0), |c| (c.left, c.x));
    let mut out = vec![Candidate {
        start: 0,
        pos_no_break: x0,
        pos_if_break: x0,
        x_start: x0_exact,
        x_end: x0_exact,
        hyphen: 0.0,
        hyphenated: false,
        next_item: f64::INFINITY,
        next_items: 0.0,
        prev_item: 0.0,
        prev_items: 0.0,
        emergency: false,
    }];
    let (mut end, mut end_exact) = (x0, x0_exact);
    let mut i = 0;
    while i < chars.len() {
        let g = chars[i];
        if is_space(g.c) {
            let mut j = i;
            while j < chars.len() && is_space(chars[j].c) {
                j += 1;
            }
            if i > 0 && j < chars.len() {
                out.push(Candidate {
                    start: j,
                    pos_no_break: chars[j].left,
                    pos_if_break: chars[i - 1].right,
                    x_start: chars[j].x,
                    x_end: g.x,
                    hyphen: 0.0,
                    hyphenated: false,
                    ..out[0]
                });
            }
            i = j;
            continue;
        }
        end = g.right;
        end_exact = g.x_end;
        let before2 = i.checked_sub(1).map(|k| chars[k].c);
        if let Some(next) = chars.get(i + 1).filter(|n| !is_space(n.c)) {
            let opportunity = line_break::can_break(before2, g.c, next.c);
            if opportunity || (emergency && cluster_boundary(g.c, next.c)) {
                let hyphenated = opportunity && g.c == SOFT_HYPHEN;
                let hyphen = if hyphenated { g.hyphen } else { 0.0 };
                out.push(Candidate {
                    start: i + 1,
                    pos_no_break: next.left,
                    pos_if_break: g.right + hyphen,
                    x_start: next.x,
                    x_end: g.x_end,
                    hyphen,
                    hyphenated,
                    emergency: !opportunity,
                    ..out[0]
                });
            }
        }
        i += 1;
    }
    out.push(Candidate {
        start: chars.len(),
        pos_no_break: end,
        pos_if_break: end,
        x_start: end_exact,
        x_end: end_exact,
        hyphen: 0.0,
        hyphenated: false,
        ..out[0]
    });
    // Where each item after the first starts, and the snapped widths of
    // the items before it.
    let mut starts: Vec<(f64, f64)> = Vec::new();
    let mut from = x0_exact;
    let mut sum = 0.0;
    for g in chars.iter().skip(1).filter(|g| g.item) {
        sum += snap_exact(g.x - from);
        starts.push((g.x, sum));
        from = g.x;
    }
    if !starts.is_empty() {
        for c in &mut out {
            let next = starts.partition_point(|b| b.0 <= c.x_start);
            (c.next_item, c.next_items) = starts.get(next).copied().unwrap_or((f64::INFINITY, 0.0));
            let prev = starts.partition_point(|b| b.0 < c.x_end);
            (c.prev_item, c.prev_items) =
                prev.checked_sub(1).map_or((x0_exact, 0.0), |k| starts[k]);
        }
    }
    out
}

/// Blink's greedy `LineBreaker` over `cands` from candidate `from`: each
/// line ends at the last candidate that fits `width` (`fits`). The candidate index ending each line, or `None` where a
/// line overflows (Blink then disables both the score and the bisect
/// breaker) or the paragraph needs more than `max_lines`.
fn greedy(cands: &[Candidate], from: usize, width: f32, max_lines: usize) -> Option<Vec<usize>> {
    let mut ends = Vec::new();
    let mut s = from;
    while s + 1 < cands.len() {
        let e = (s + 1..cands.len())
            .take_while(|&e| fits(greedy_len(&cands[s], &cands[e]), width))
            .last()?;
        ends.push(e);
        if ends.len() > max_lines {
            return None;
        }
        s = e;
    }
    Some(ends)
}

/// Blink's normal line breaking (`text-wrap: wrap`) over `cands` at
/// `width` (points): each line ends at the last candidate that fits, its
/// trailing spaces hanging; a line where none fits takes the first, its
/// word overflowing (`overflow-wrap: normal`). The char index where each
/// line after the first starts.
pub(super) fn normal(cands: &[Candidate], width: f32) -> Vec<usize> {
    let mut out = Vec::new();
    let mut s = 0;
    while s + 1 < cands.len() {
        let e = (s + 1..cands.len())
            .take_while(|&e| fits(greedy_len(&cands[s], &cands[e]), width))
            .last()
            .unwrap_or(s + 1);
        if e + 1 < cands.len() {
            out.push(cands[e].start);
        }
        s = e;
    }
    out
}

/// Whether a word may break between `a` and `b` under
/// `overflow-wrap: break-word`, which breaks at grapheme cluster
/// boundaries: not around a zero-width joiner, nor before a combining
/// mark, a variation selector or an emoji skin-tone modifier.
fn cluster_boundary(a: char, b: char) -> bool {
    !(a == '\u{200D}'
        || matches!(
            b,
            '\u{200D}'
                | '\u{0300}'..='\u{036F}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{1F3FB}'..='\u{1F3FF}'
        ))
}

/// Blink's normal line breaking under `overflow-wrap: break-word` over
/// `cands` ([`candidates_with`] with emergency candidates) at `width`:
/// as [`normal`], except that a line where no break opportunity fits
/// (a word too wide for a line of its own) ends at the last emergency
/// candidate inside the word that fits, after one char at least
/// (`LineBreaker::HandleOverflow` breaking anywhere). A word that fits
/// a line of its own moves down whole. The char index where each line
/// after the first starts.
pub(super) fn break_word(cands: &[Candidate], width: f32) -> Vec<usize> {
    let mut out = Vec::new();
    let mut s = 0;
    while s + 1 < cands.len() {
        let fit = |e: &usize| fits(greedy_len(&cands[s], &cands[*e]), width);
        let regular = (s + 1..cands.len()).filter(|&e| !cands[e].emergency);
        // The end sentinel is a regular candidate.
        let first = regular.clone().next().unwrap_or(cands.len() - 1);
        let e = if fit(&first) {
            regular.take_while(fit).last().unwrap_or(first)
        } else {
            (s + 1..first).take_while(fit).last().unwrap_or(s + 1)
        };
        if e + 1 < cands.len() {
            out.push(cands[e].start);
        }
        s = e;
    }
    out
}

/// `ScoreLineBreaker::Optimize` for the paragraph part `cands` (its first
/// candidate is the start of its first line): the candidate indices ending
/// each line, or `None` where Blink gives up (fewer than three break
/// opportunities, or another line count than `lines`).
// `active` moves for the next `end`, as in Blink; the running loop keeps
// its range.
#[allow(clippy::mut_range_bound)]
fn score(
    cands: &[Candidate],
    width: f32,
    zoom: f32,
    font_size: f32,
    balanced: bool,
    lines: usize,
) -> Option<Vec<usize>> {
    if cands.len() < 3 + 2 {
        return None;
    }
    // `SetupParameters`, not justified.
    let width = snap_width(width.max(0.0));
    let hyphen_penalty = width * font_size / zoom * 2.0;
    let line_penalty = hyphen_penalty * 2.0;
    let mut penalty: Vec<f32> = cands
        .iter()
        .map(|c| if c.hyphenated { hyphen_penalty } else { 0.0 })
        .collect();
    // Typographic orphans: the last break opportunity costs more, and so
    // does each hyphenated one right before it.
    for k in (0..cands.len() - 1).rev() {
        penalty[k] += ORPHANS_PENALTY * zoom;
        if !cands[k].hyphenated {
            break;
        }
    }
    let fit = width + unit();
    // (score, previous index, line index) per candidate.
    let mut scores: Vec<(f32, usize, usize)> = vec![(0.0, 0, 0)];
    let mut active = 0;
    for end in 1..cands.len() {
        let end_c = cands[end];
        let is_last = end == cands.len() - 1;
        let mut best = SCORE_INFINITY;
        let mut best_prev = 0;
        let start_edge = end_c.pos_if_break - fit;
        let mut best_hope = 0.0;
        for start in active..end {
            let start_score = scores[start].0;
            if start_score + best_hope >= best {
                continue;
            }
            let delta = cands[start].pos_no_break - start_edge;
            let mut width_score = 0.0;
            let mut additional = 0.0;
            if delta < 0.0 {
                width_score = SCORE_OVERFULL;
            } else if is_last && !balanced {
                additional = LAST_LINE_PENALTY_MULTIPLIER * penalty[start];
            } else {
                width_score = delta * delta / zoom;
            }
            if delta < 0.0 {
                active = start + 1;
            } else {
                best_hope = width_score;
            }
            let s = start_score + width_score + additional;
            if s <= best {
                best = s;
                best_prev = start;
            }
        }
        scores.push((
            best + penalty[end] + line_penalty,
            best_prev,
            scores[best_prev].2 + 1,
        ));
    }
    let mut ends = Vec::new();
    let mut i = cands.len() - 1;
    while i > 0 {
        ends.push(i);
        i = scores[i].1;
    }
    ends.reverse();
    (ends.len() == lines).then_some(ends)
}

/// `ParagraphLineBreaker::AttemptParagraphBalancing`: the narrowest width
/// (bisected to `epsilon`, in `LayoutUnit` steps) at which the
/// greedy breaker still needs the lines it needs at `width`, and the line
/// ends there. `estimated` is Blink's `EstimateNumLines` guess.
fn bisect(cands: &[Candidate], width: f32, epsilon: f32, estimated: usize) -> Option<Vec<usize>> {
    if estimated > MAX_LINES_BALANCE * 2 || width <= 0.0 {
        return None;
    }
    let normal = greedy(cands, 0, width, MAX_LINES_BALANCE)?;
    let lines = normal.len();
    if lines <= 1 {
        return None;
    }
    // `LineWidthSum() / num_lines` in `LayoutUnit`s, then rounded.
    let sum: f32 = line_widths(cands, 0, &normal).sum();
    let average = (sum * LAYOUT_UNITS / lines as f32).floor() / LAYOUT_UNITS;
    let mut lower = (average * BALANCE_START * LAYOUT_UNITS).round() / LAYOUT_UNITS;
    let mut upper = snap_width(width);
    while lower + epsilon < upper {
        let middle = snap_width((upper + lower) / 2.0);
        match greedy(cands, 0, middle, lines) {
            Some(ends) if ends.len() <= lines => upper = middle,
            _ => lower = middle,
        }
    }
    greedy(cands, 0, upper, lines)
}

/// The width of each greedy line ending at `ends`, the first starting at
/// `from` (Blink's `LineInfo` widths).
fn line_widths<'a>(
    cands: &'a [Candidate],
    from: usize,
    ends: &'a [usize],
) -> impl Iterator<Item = f32> + 'a {
    std::iter::once(from)
        .chain(ends.iter().copied())
        .zip(ends.iter().copied())
        .map(|(s, e)| greedy_len(&cands[s], &cands[e]))
}

/// How a paragraph is broken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Style {
    Balance,
    Pretty,
}

/// The char index where each line after the first starts, for one
/// paragraph under `style` at `width` (device px, as `font_size`);
/// `None` where Blink keeps its greedy breaks. `zoom` is the device pixels
/// per point: Blink unzooms its scores once, and the bisection stops at
/// one CSS pixel, rounded up to a `LayoutUnit`. `space_width` is the
/// primary font's space advance and `text_len` the block's text length in
/// chars, for Blink's line estimate.
#[allow(clippy::too_many_arguments)]
pub(super) fn breaks(
    cands: &[Candidate],
    style: Style,
    width: f32,
    font_size: f32,
    zoom: f32,
    space_width: f32,
    text_len: usize,
) -> Option<Vec<usize>> {
    let starts = |ends: Vec<usize>, from: usize| {
        let n = ends.len();
        ends[..n - 1]
            .iter()
            .map(|&e| cands[from + e].start)
            .collect::<Vec<_>>()
    };
    match style {
        Style::Balance => {
            let normal = greedy(cands, 0, width, MAX_LINES_BALANCE)?;
            if normal.len() <= 1 {
                return None;
            }
            let ends = score(cands, width, zoom, font_size, true, normal.len()).or_else(|| {
                let per_line = if space_width > 0.0 {
                    (width / space_width) as usize
                } else {
                    0
                };
                let estimated = match per_line {
                    _ if space_width <= 0.0 => 0,
                    0 => usize::MAX,
                    n => text_len.div_ceil(n),
                };
                // `LayoutUnit::FromFloatCeil(EffectiveZoom())`: one CSS px.
                let epsilon = (zoom * LAYOUT_UNITS).ceil() / LAYOUT_UNITS;
                bisect(cands, width, epsilon, estimated)
            })?;
            Some(starts(ends, 0))
        }
        Style::Pretty => {
            let normal = greedy(cands, 0, width, usize::MAX)?;
            let n = normal.len();
            if n <= 1 {
                return None;
            }
            // Blink scores the last lines that fit its line cache.
            let k = n.saturating_sub(MAX_LINES_PRETTY);
            let from = if k == 0 { 0 } else { normal[k - 1] };
            let last_start = normal.get(n.wrapping_sub(2)).copied().unwrap_or(0);
            let last_width = line_widths(cands, from, &normal[k..]).last()?;
            // `ShouldOptimize`: a short last line holding one word; the
            // third of the width in whole `LayoutUnit`s.
            let third =
                (snap_width(width) * LAYOUT_UNITS / SHORT_LINE_DENOMINATOR).floor() / LAYOUT_UNITS;
            if !(last_width < third && last_start == cands.len() - 2) {
                return None;
            }
            let window = &cands[from..];
            let ends = score(window, width, zoom, font_size, false, n - k)?;
            let mut out: Vec<usize> = normal[..k].iter().map(|&e| cands[e].start).collect();
            out.extend(starts(ends, from));
            Some(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monospace text: every char 1 wide.
    fn mono(text: &str) -> Vec<Glyph> {
        text.chars()
            .enumerate()
            .map(|(i, c)| Glyph {
                c,
                left: i as f32,
                right: i as f32 + 1.0,
                x: i as f64,
                x_end: i as f64 + 1.0,
                hyphen: 1.0,
                item: false,
            })
            .collect()
    }

    fn lines(text: &str, style: Style, width: f32) -> Vec<String> {
        let chars = mono(text);
        let cands = candidates(&chars);
        let starts = breaks(&cands, style, width, 14.0, 1.0, 1.0, chars.len()).unwrap_or_default();
        let mut out = Vec::new();
        let mut s = 0;
        for e in starts.into_iter().chain(std::iter::once(chars.len())) {
            out.push(
                text.chars()
                    .skip(s)
                    .take(e - s)
                    .collect::<String>()
                    .trim_end()
                    .to_owned(),
            );
            s = e;
        }
        out
    }

    /// `text` broken under `overflow-wrap: break-word` at `width`.
    fn break_word_lines(text: &str, width: f32) -> Vec<String> {
        let chars = mono(text);
        let starts = break_word(&candidates_with(&chars, true), width);
        let mut out = Vec::new();
        let mut s = 0;
        for e in starts.into_iter().chain(std::iter::once(chars.len())) {
            out.push(
                text.chars()
                    .skip(s)
                    .take(e - s)
                    .collect::<String>()
                    .trim_end()
                    .to_owned(),
            );
            s = e;
        }
        out
    }

    #[test]
    fn break_word_breaks_only_a_word_too_wide_for_its_own_line() {
        assert_eq!(
            break_word_lines("see abcdefghij", 4.0),
            ["see", "abcd", "efgh", "ij"]
        );
        assert_eq!(break_word_lines("ab cdef", 4.0), ["ab", "cdef"]);
        assert_eq!(break_word_lines("ab cd", 8.0), ["ab cd"]);
        // One char a line at least, even where none fits.
        assert_eq!(break_word_lines("abc", 0.5), ["a", "b", "c"]);
        // A combining mark stays with its base.
        assert_eq!(
            break_word_lines("e\u{301}e\u{301}", 1.0),
            ["e\u{301}", "e\u{301}"]
        );
        // Emergency candidates never reach the other breakers.
        assert!(candidates(&mono("abcd")).iter().all(|c| !c.emergency));
    }

    #[test]
    fn candidates_sit_at_spaces_and_hyphens() {
        let c = candidates(&mono("ab cd-ef  g-1 -2"));
        let starts: Vec<usize> = c.iter().map(|c| c.start).collect();
        assert_eq!(starts, [0, 3, 6, 10, 12, 14, 16]);
        assert_eq!((c[1].pos_if_break, c[1].pos_no_break), (2.0, 3.0));
        assert_eq!((c[2].pos_if_break, c[2].pos_no_break), (6.0, 6.0));
        assert_eq!(c[6].pos_if_break, 16.0);
    }

    #[test]
    fn a_soft_hyphen_break_ends_its_line_after_the_hyphen() {
        let c = candidates(&mono("ab\u{AD}cd"));
        let starts: Vec<usize> = c.iter().map(|c| c.start).collect();
        assert_eq!(starts, [0, 3, 5]);
        assert!(c[1].hyphenated);
        assert_eq!(
            (c[1].pos_if_break, c[1].x_end, c[1].hyphen),
            (4.0, 3.0, 1.0)
        );
        assert_eq!(c[1].pos_no_break, 3.0);
    }

    #[test]
    fn greedy_lines_keep_the_kerning_against_the_space_the_score_breaker_not() {
        // "ab cd ef" with the second space kerned 0.25 closer to "d".
        let mut chars = mono("ab cd ef");
        for g in &mut chars[5..] {
            g.left -= 0.25;
            g.right -= 0.25;
            g.x -= 0.25;
            g.x_end -= 0.25;
        }
        let c = candidates(&chars);
        assert_eq!((c[2].x_end, c[2].pos_if_break), (4.75, 5.0));
        // "ab cd" ends at 4.75 kerned: it fits 4.8 for the greedy breaker.
        assert_eq!(normal(&c, 4.8), [6]);
        assert_eq!(normal(&c, 4.7), [3, 6]);
    }

    #[test]
    fn a_greedy_line_is_its_own_text_snapped_not_its_snapped_edges() {
        // "abcd cd ef": the line "cd ef" starts 0.02 before 5 and ends at
        // 10, 5 apart once each edge is snapped, 5.02 long unsnapped.
        let mut chars = mono("abcd cd ef");
        chars[5].x = 4.98;
        let c = candidates(&chars);
        assert_eq!(greedy_len(&c[1], &c[3]), 5.03125);
        assert_eq!(normal(&c, 5.0), [5, 8]);
        assert_eq!(normal(&c, 5.02), [5]);
    }

    #[test]
    fn a_greedy_line_snaps_each_item_on_its_own() {
        // "ab c|d ef", an item (section) starting at "d" 4.005 in, "d"
        // 1.005 wide: the line "ab cd" is 4.015625 + 1.015625 long, its
        // items snapped each, not 5.01 snapped (5.015625).
        let mut chars = mono("ab cd ef");
        chars[4].x = 4.005;
        chars[4].item = true;
        for g in &mut chars[4..] {
            g.x_end += 0.01;
        }
        for g in &mut chars[5..] {
            g.x += 0.01;
        }
        let c = candidates(&chars);
        assert_eq!(greedy_len(&c[0], &c[2]), 5.03125);
        // A line within one item is snapped once.
        assert_eq!(greedy_len(&c[2], &c[3]), 2.0);
        assert_eq!(normal(&c, 5.0), [3, 6]);
    }

    #[test]
    fn balance_evens_the_lines_and_keeps_their_count() {
        let text = "aaaa bbbb cccc dddd eeee ffff gg";
        assert_eq!(
            lines(text, Style::Balance, 25.0),
            ["aaaa bbbb cccc", "dddd eeee ffff gg"]
        );
    }

    #[test]
    fn balance_gives_up_past_six_lines_and_on_one_line() {
        let text = "aa bb cc dd ee ff gg hh ii jj kk ll mm nn";
        assert_eq!(lines(text, Style::Balance, 5.0).len(), 1);
        assert_eq!(lines("aa bb", Style::Balance, 50.0).len(), 1);
    }

    #[test]
    fn pretty_moves_a_word_down_to_a_lone_last_word() {
        let text = "aaaa bbbb cccc dddd e";
        assert_eq!(
            lines(text, Style::Pretty, 19.0),
            ["aaaa bbbb cccc", "dddd e"]
        );
        // A last line of two words is left alone.
        assert_eq!(lines("aaaa bbbb cccc dd ee", Style::Pretty, 15.0).len(), 1);
    }

    #[test]
    fn an_overflowing_word_keeps_greedy_breaks() {
        let text = "aaaaaaaaaaaaaaaaaaaa b c d e";
        assert_eq!(lines(text, Style::Balance, 10.0).len(), 1);
    }
}
