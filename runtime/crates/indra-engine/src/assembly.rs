//! Split detector merges at token gaps, tidy leader lines and list markers,
//! decide the page direction, then expose borrowed line text to reading order.
//!
//! Port of `assemble`, `split_line`, `gutter_cuts`, `_decode` and `tidy_lines`
//! in `indra_ocr/pipeline.py` at upstream commit
//! `0e96b93c6810f17cf8352acbb4487cc9e45b66d5` (Apache-2.0).

use std::{ops::Range, time::Instant};

use opencv::prelude::*;
use unicode_normalization::UnicodeNormalization;

use crate::{
    EngineError, MAX_LINES, MAX_OUTPUT_BYTES, Result, deadline,
    recognition::{self, Lines, MAX_TOKENS, Script},
    rtl,
    unicode_tables::{LETTER, contains},
};

/// Marker joins append text; the page buffer therefore allows twice the response bound.
const PAGE_TEXT_BYTES: usize = 2 * MAX_OUTPUT_BYTES;
/// One line's decoded text: at most 512 tokens of at most 13 bytes, or one math
/// line of at most 319 tokens; both stay far below this scratch capacity.
const LINE_SCRATCH_BYTES: usize = 64 * 1024;
/// Dot leaders and fill-in rules.
const LEADER_CHARS: &str = ".·…‥_-–—=";
const DASHES: &str = "-–—";
/// List markers; the reference tests `text in BULLETS`, a substring test.
const BULLETS: &str = "•◦▪▫●○■□‣⁃∙*";
/// A list marker and its item are at most this many line heights apart.
const MARKER_GAP: f32 = 2.5;

pub(crate) struct Page {
    pub boxes: [[f32; 4]; MAX_LINES],
    pub ranges: [Range<usize>; MAX_LINES],
    pub text: String,
    pub count: usize,
    /// The page reads right to left: more right-to-left than left-to-right letters.
    pub rtl: bool,
    collapsed: String,
    normalized: String,
    bidi: Box<rtl::Scratch>,
}

impl Page {
    pub fn new() -> Self {
        Self {
            boxes: [[0.0; 4]; MAX_LINES],
            ranges: [const { 0..0 }; MAX_LINES],
            text: String::with_capacity(PAGE_TEXT_BYTES),
            count: 0,
            rtl: false,
            collapsed: String::with_capacity(LINE_SCRATCH_BYTES),
            normalized: String::with_capacity(LINE_SCRATCH_BYTES),
            bidi: Box::new(rtl::Scratch::new()),
        }
    }

    pub fn assemble(&mut self, boxes: &[[f32; 4]], lines: &Lines, due: Instant) -> Result<()> {
        self.count = 0;
        self.text.clear();
        self.rtl = page_direction(
            boxes.len(),
            lines,
            &mut self.collapsed,
            &mut self.normalized,
        )?;
        let ids = lines.ids.data_typed::<i32>()?;
        let positions = lines.positions.data_typed::<f64>()?;
        for (index, b) in boxes.iter().enumerate() {
            deadline(due)?;
            let line = &lines.metadata[index];
            if line.script == Script::Math {
                let start = self.text.len();
                self.decode_text(
                    Script::Math,
                    &lines.math_text[line.math_start..line.math_end],
                )?;
                self.push(b, start..self.text.len())?;
                continue;
            }
            let ids = &ids[index * MAX_TOKENS..index * MAX_TOKENS + line.count];
            let xs = &positions[index * MAX_TOKENS..index * MAX_TOKENS + line.count];
            let h = (f64::from(b[3]) - f64::from(b[1])).max(1.0);
            let mut boundaries = [0_usize; MAX_TOKENS + 1];
            let mut parts = 1;
            let mut prior = None;
            for (token, id) in ids.iter().enumerate() {
                if recognition::token(line.script, *id)? == " " {
                    continue;
                }
                let x = xs[token];
                if let Some(previous) = prior
                    && (x - previous > 3.0 * h || gutter(previous, x, h, boxes, index))
                {
                    boundaries[parts] = token;
                    parts += 1;
                }
                prior = Some(x);
            }
            boundaries[parts] = ids.len();
            for part in 0..parts {
                let first = boundaries[part];
                let last = boundaries[part + 1];
                let start = self.text.len();
                self.decode_tokens(line.script, &ids[first..last])?;
                if parts == 1 {
                    self.push(b, start..self.text.len())?;
                    continue;
                }
                let emitted = &self.text[start..];
                let trimmed = emitted.trim_matches(python_whitespace);
                if trimmed.is_empty() {
                    self.text.truncate(start);
                    continue;
                }
                let begin =
                    start + emitted.len() - emitted.trim_start_matches(python_whitespace).len();
                let end = begin + trimmed.len();
                let mut left = f64::INFINITY;
                let mut right = f64::NEG_INFINITY;
                for token in first..last {
                    if recognition::token(line.script, ids[token])? != " " {
                        left = left.min(xs[token]);
                        right = right.max(xs[token]);
                    }
                }
                if !left.is_finite() {
                    self.text.truncate(start);
                    continue;
                }
                let tight = [
                    f64::from(b[0]).max(left - 0.35 * h) as f32,
                    b[1],
                    f64::from(b[2]).min(right + 0.35 * h) as f32,
                    b[3],
                ];
                self.push(&tight, begin..end)?;
            }
        }
        self.tidy(due)
    }

    /// `_decode` of recogniser tokens: base ids through the vocabulary; group
    /// characters joined with whitespace collapsed, NFC-composed and, with
    /// right-to-left characters, flipped by the page direction.
    fn decode_tokens(&mut self, script: Script, ids: &[i32]) -> Result<()> {
        if !script.as_chars() {
            return recognition::decode_tokens(script, ids, &mut self.text);
        }
        self.collapsed.clear();
        recognition::decode_tokens(script, ids, &mut self.collapsed)?;
        self.finish_characters()
    }

    /// `_decode` of a text token (a math reading): whitespace collapsed, NFC,
    /// flipped when it holds right-to-left characters.
    fn decode_text(&mut self, script: Script, text: &str) -> Result<()> {
        debug_assert_eq!(script, Script::Math);
        self.collapsed.clear();
        let mut pending_space = false;
        for ch in text.chars() {
            if python_whitespace(ch) {
                pending_space = !self.collapsed.is_empty();
            } else {
                if pending_space {
                    recognition::append(&mut self.collapsed, " ")?;
                    pending_space = false;
                }
                let mut bytes = [0; 4];
                recognition::append(&mut self.collapsed, ch.encode_utf8(&mut bytes))?;
            }
        }
        self.finish_characters()
    }

    fn finish_characters(&mut self) -> Result<()> {
        self.normalized.clear();
        for ch in self.collapsed.nfc() {
            let mut bytes = [0; 4];
            recognition::append(&mut self.normalized, ch.encode_utf8(&mut bytes))?;
        }
        if self.text.len() + self.normalized.len() > PAGE_TEXT_BYTES {
            return Err(EngineError::Limit("page text bytes"));
        }
        if rtl::has_rtl(&self.normalized) {
            rtl::flip(
                &self.normalized,
                if self.rtl { Some(true) } else { None },
                &mut self.bidi,
                &mut self.text,
            )
        } else {
            recognition::append(&mut self.text, &self.normalized)
        }
    }

    fn push(&mut self, b: &[f32; 4], range: Range<usize>) -> Result<()> {
        if self.count == MAX_LINES {
            return Err(EngineError::Limit("lines after column splitting"));
        }
        self.boxes[self.count] = *b;
        self.ranges[self.count] = range;
        self.count += 1;
        Ok(())
    }

    fn line(&self, index: usize) -> &str {
        &self.text[self.ranges[index].start..self.ranges[index].end]
    }

    /// The byte range of a line's text without Python whitespace at either end.
    fn trimmed(&self, index: usize) -> Range<usize> {
        let line = self.line(index);
        let start = self.ranges[index].start + line.len()
            - line.trim_start_matches(python_whitespace).len();
        start..start + line.trim_matches(python_whitespace).len()
    }

    /// `tidy_lines`: drop lines of leader characters only; join a lone list
    /// marker to the line that starts within `MARKER_GAP` line heights after it
    /// on the same row (before it on a right-to-left page).
    fn tidy(&mut self, due: Instant) -> Result<()> {
        let n = self.count;
        let mut keep = [true; MAX_LINES];
        let mut heights = [0.0_f32; MAX_LINES];
        let mut widths = [0.0_f32; MAX_LINES];
        for index in 0..n {
            let b = self.boxes[index];
            heights[index] = b[3] - b[1];
            widths[index] = b[2] - b[0];
            let text = self.line(index);
            let mut core = text.chars().filter(|c| *c != ' ');
            if let Some(first) = core.next()
                && LEADER_CHARS.contains(first)
                && core.clone().all(|c| LEADER_CHARS.contains(c))
            {
                let length = 1 + core.count();
                let wide = widths[index] >= 3.0 * heights[index];
                let dashes = text.chars().any(|c| DASHES.contains(c));
                if length >= 3 || wide || !dashes {
                    keep[index] = false;
                }
            }
        }
        for index in 0..n {
            deadline(due)?;
            let marker = self.line(index).trim_matches(python_whitespace);
            if !keep[index] || !(BULLETS.contains(marker) || enumerator(marker)) {
                continue;
            }
            let mut best = None;
            let mut best_gap = f32::INFINITY;
            for other in 0..n {
                if other == index
                    || !keep[other]
                    || self.line(other).trim_matches(python_whitespace).is_empty()
                {
                    continue;
                }
                let mine = self.boxes[index];
                let theirs = self.boxes[other];
                let overlap = mine[3].min(theirs[3]) - mine[1].max(theirs[1]);
                if overlap < 0.5 * heights[index].min(heights[other]) {
                    continue;
                }
                let gap = if self.rtl {
                    mine[0] - theirs[2]
                } else {
                    theirs[0] - mine[2]
                };
                if -0.3 * heights[index] <= gap
                    && gap <= MARKER_GAP * heights[index].max(heights[other])
                    && (best.is_none() || gap < best_gap)
                {
                    best = Some(other);
                    best_gap = gap;
                }
            }
            let Some(other) = best else {
                continue;
            };
            let marker = self.trimmed(index);
            let item = self.trimmed(other);
            if self.text.len() + marker.len() + 1 + item.len() > PAGE_TEXT_BYTES {
                return Err(EngineError::Limit("page text bytes"));
            }
            // `f"{marker} {item}"` is appended; the old bytes stay unreferenced.
            let start = self.text.len();
            self.text.extend_from_within(marker);
            self.text.push(' ');
            self.text.extend_from_within(item);
            self.ranges[other] = start..self.text.len();
            let mine = self.boxes[index];
            let theirs = self.boxes[other];
            self.boxes[other] = [
                mine[0].min(theirs[0]),
                mine[1].min(theirs[1]),
                mine[2].max(theirs[2]),
                mine[3].max(theirs[3]),
            ];
            keep[index] = false;
        }
        let mut written = 0;
        for (index, kept) in keep[..n].iter().enumerate() {
            if *kept {
                self.boxes[written] = self.boxes[index];
                self.ranges[written] = self.ranges[index].clone();
                written += 1;
            }
        }
        self.count = written;
        Ok(())
    }

    pub fn write(&self, order: &[usize], output: &mut String) -> Result<()> {
        if order.len() != self.count {
            return Err(EngineError::Shape("reading order length"));
        }
        for (position, index) in order.iter().enumerate() {
            if *index >= self.count {
                return Err(EngineError::Shape("reading order index"));
            }
            if position > 0 {
                recognition::append(output, "\n")?;
            }
            let range = &self.ranges[*index];
            recognition::append(output, &self.text[range.start..range.end])?;
        }
        Ok(())
    }
}

/// `_page_rtl`: more right-to-left than left-to-right letters over every
/// recognised line, counted on the raw recogniser text before splitting.
fn page_direction(
    count: usize,
    lines: &Lines,
    joined: &mut String,
    normalized: &mut String,
) -> Result<bool> {
    let ids = lines.ids.data_typed::<i32>()?;
    let mut letters = 0_usize;
    let mut rtl = 0_usize;
    let mut count_text = |text: &str| {
        for c in text.chars() {
            if contains(&LETTER, c) {
                letters += 1;
                if rtl::is_rtl(c) {
                    rtl += 1;
                }
            }
        }
    };
    for index in 0..count {
        let line = &lines.metadata[index];
        match line.script {
            Script::Math => count_text(&lines.math_text[line.math_start..line.math_end]),
            Script::Cjk => {
                // `Vocab.decode` composes jamo into syllables before the count.
                joined.clear();
                for id in &ids[index * MAX_TOKENS..index * MAX_TOKENS + line.count] {
                    recognition::append(joined, recognition::token(Script::Cjk, *id)?)?;
                }
                normalized.clear();
                for ch in joined.nfc() {
                    let mut bytes = [0; 4];
                    recognition::append(normalized, ch.encode_utf8(&mut bytes))?;
                }
                count_text(normalized);
            }
            script => {
                for id in &ids[index * MAX_TOKENS..index * MAX_TOKENS + line.count] {
                    count_text(recognition::token(script, *id)?);
                }
            }
        }
    }
    Ok(rtl > letters - rtl)
}

/// `ENUMERATOR.fullmatch`: `1.` `12)` `a.` `B)` `(3)` `(b)` `(iv)`.
fn enumerator(text: &str) -> bool {
    let decimals = |s: &str| {
        let n = s.chars().count();
        (1..=3).contains(&n) && s.chars().all(crate::order::python_decimal)
    };
    let letter = |s: &str| s.len() == 1 && s.as_bytes()[0].is_ascii_alphabetic();
    let roman = |s: &str| {
        let n = s.chars().count();
        (1..=5).contains(&n) && s.chars().all(|c| "ivxlcIVXLC".contains(c))
    };
    if let Some(head) = text.strip_suffix(['.', ')'])
        && !text.starts_with('(')
        && (decimals(head) || letter(head))
    {
        return true;
    }
    if let Some(inner) = text
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
    {
        return decimals(inner) || letter(inner) || roman(inner);
    }
    false
}

fn python_whitespace(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}')
}

fn gutter(a: f64, c: f64, h: f64, boxes: &[[f32; 4]], mine: usize) -> bool {
    if c - a <= 1.6 * h {
        return false;
    }
    let left = (a + 0.35 * h) as f32;
    let right = (c - 0.35 * h) as f32;
    let threshold = (0.5 * h) as f32;
    boxes
        .iter()
        .enumerate()
        .filter(|(i, b)| {
            *i != mine && ((b[2] - left).abs() < threshold || (b[0] - right).abs() < threshold)
        })
        .take(3)
        .count()
        >= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gutter_needs_three_other_edges_and_strict_distance() {
        let boxes = [
            [0.0, 0.0, 100.0, 10.0],
            [0.0, 20.0, 13.5, 30.0],
            [0.0, 40.0, 13.5, 50.0],
            [0.0, 60.0, 13.5, 70.0],
        ];
        assert!(gutter(10.0, 30.0, 10.0, &boxes, 0));
        assert!(!gutter(10.0, 26.0, 10.0, &boxes, 0));
        assert!(!gutter(10.0, 30.0, 10.0, &boxes[..3], 0));
    }

    #[test]
    fn enumerators_follow_the_reference_pattern() {
        for marker in [
            "1.", "12)", "a.", "B)", "(3)", "(b)", "(iv)", "(IV)", "٣.", "999.",
        ] {
            assert!(enumerator(marker), "{marker}");
        }
        for text in [
            "1", "1234.", "ab.", "(vi", "(1.", "(abc)", "(ivxlcx)", "", ".", "1.)",
        ] {
            assert!(!enumerator(text), "{text}");
        }
        assert!(BULLETS.contains("•") && BULLETS.contains("") && !BULLETS.contains("-"));
    }

    fn page_with(lines: &[(&str, [f32; 4])], rtl: bool) -> Page {
        let mut page = Page::new();
        for (text, b) in lines {
            let start = page.text.len();
            page.text.push_str(text);
            page.push(b, start..page.text.len()).expect("line");
        }
        page.rtl = rtl;
        page
    }

    fn texts(page: &Page) -> Vec<String> {
        (0..page.count)
            .map(|index| page.line(index).to_owned())
            .collect()
    }

    #[test]
    fn tidy_drops_leader_lines_and_keeps_lone_dashes() {
        let due = Instant::now() + std::time::Duration::from_secs(5);
        let mut page = page_with(
            &[
                (". . . .", [0.0, 0.0, 100.0, 10.0]),
                ("_", [0.0, 20.0, 5.0, 30.0]),
                ("-", [0.0, 40.0, 5.0, 50.0]),
                ("- -", [0.0, 60.0, 40.0, 70.0]),
                ("( )", [0.0, 80.0, 20.0, 90.0]),
            ],
            false,
        );
        page.tidy(due).expect("tidy");
        assert_eq!(texts(&page), ["-", "( )"]);
    }

    #[test]
    fn tidy_joins_markers_to_their_item_by_page_direction() {
        let due = Instant::now() + std::time::Duration::from_secs(5);
        let mut page = page_with(
            &[
                ("•", [0.0, 0.0, 8.0, 10.0]),
                ("first item", [12.0, 0.0, 100.0, 10.0]),
                ("2)", [0.0, 20.0, 12.0, 30.0]),
                ("second item", [60.0, 21.0, 160.0, 31.0]),
                ("", [0.0, 40.0, 4.0, 50.0]),
                ("after blank", [6.0, 40.0, 80.0, 50.0]),
            ],
            false,
        );
        page.tidy(due).expect("tidy");
        // The second marker is over 2.5 heights away and stays; an empty line
        // joins as the reference does, leaving a leading space.
        assert_eq!(
            texts(&page),
            ["• first item", "2)", "second item", " after blank"]
        );
        assert_eq!(page.boxes[0], [0.0, 0.0, 100.0, 10.0]);
        let mut page = page_with(
            &[
                ("•", [100.0, 0.0, 108.0, 10.0]),
                ("item", [10.0, 0.0, 96.0, 10.0]),
            ],
            true,
        );
        page.tidy(due).expect("tidy");
        assert_eq!(texts(&page), ["• item"]);
    }
}
