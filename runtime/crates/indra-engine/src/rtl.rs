//! Right-to-left text for recognisers that read every line left to right.
//!
//! Port of `indra_ocr/rtl.py` at upstream commit
//! `0e96b93c6810f17cf8352acbb4487cc9e45b66d5` (Apache-2.0). A CTC recogniser
//! emits characters in visual order; `flip` turns that into logical order with
//! a small subset of UAX #9: clusters keep their combining marks, left-to-right
//! runs keep their own order, and mirrored brackets change to their mirror
//! image. Character classes come from the CPython tables the reference uses.

use crate::{
    EngineError, Result,
    recognition::MAX_TOKENS,
    unicode_tables::{BIDI_L, BIDI_LTR, BIDI_RTL, MARK, contains},
};

/// Clusters a line can hold: one per token at most, plus one for a combining
/// mark that NFC did not attach to anything.
const MAX_CLUSTERS: usize = MAX_TOKENS + 1;

pub fn is_rtl(c: char) -> bool {
    contains(&BIDI_RTL, c)
}

fn is_ltr(c: char) -> bool {
    contains(&BIDI_LTR, c)
}

fn is_mark(c: char) -> bool {
    contains(&MARK, c)
}

pub fn has_rtl(text: &str) -> bool {
    text.chars().any(is_rtl)
}

/// `rtl_base`: more right-to-left letters than left-to-right letters.
pub fn rtl_base(text: &str) -> bool {
    let mut rtl = 0_usize;
    let mut ltr = 0_usize;
    for c in text.chars() {
        if is_rtl(c) {
            rtl += 1;
        } else if contains(&BIDI_L, c) {
            ltr += 1;
        }
    }
    rtl > ltr
}

const fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        '‹' => '›',
        '›' => '‹',
        other => other,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Rtl,
    Ltr,
    Neutral,
}

/// Fixed scratch for one line: cluster byte ranges and the unit list.
pub struct Scratch {
    clusters: [(u32, u32); MAX_CLUSTERS],
    kinds: [Kind; MAX_CLUSTERS],
    /// Spans of clusters that form one unit, and whether their first
    /// character is mirrored when written.
    units: [(u32, u32, bool); MAX_CLUSTERS],
}

impl Default for Scratch {
    fn default() -> Self {
        Self::new()
    }
}

impl Scratch {
    pub const fn new() -> Self {
        Self {
            clusters: [(0, 0); MAX_CLUSTERS],
            kinds: [Kind::Neutral; MAX_CLUSTERS],
            units: [(0, 0, false); MAX_CLUSTERS],
        }
    }

    fn cluster<'t>(&self, text: &'t str, index: usize) -> &'t str {
        let (start, end) = self.clusters[index];
        &text[start as usize..end as usize]
    }

    fn write(&self, text: &str, index: usize, mirrored: bool, out: &mut String) -> Result<()> {
        let cluster = self.cluster(text, index);
        let mut chars = cluster.chars();
        let first = chars
            .next()
            .ok_or(EngineError::Shape("empty bidi cluster"))?;
        let mut bytes = [0; 4];
        crate::recognition::append(
            out,
            if mirrored { mirror(first) } else { first }.encode_utf8(&mut bytes),
        )?;
        crate::recognition::append(out, chars.as_str())
    }
}

/// `_runs`: `[start, end)` cluster spans from a `strong` cluster to the last
/// `strong` cluster before a cluster of the `stop` kind or the end.
fn runs(kinds: &[Kind], strong: Kind, stop: Kind, mut emit: impl FnMut(usize, usize)) {
    let mut start = None;
    let mut last = 0;
    for (index, kind) in kinds.iter().enumerate() {
        if *kind == strong {
            if start.is_none() {
                start = Some(index);
            }
            last = index;
        } else if *kind == stop
            && let Some(first) = start.take()
        {
            emit(first, last + 1);
        }
    }
    if let Some(first) = start {
        emit(first, last + 1);
    }
}

/// `flip`: logical order to visual order and back. `base_rtl` is the page
/// direction when known; otherwise `rtl_base` decides per line. Appends the
/// flipped text to `out`; a line without right-to-left characters is copied.
pub fn flip(
    text: &str,
    base_rtl: Option<bool>,
    scratch: &mut Scratch,
    out: &mut String,
) -> Result<()> {
    if !has_rtl(text) {
        return crate::recognition::append(out, text);
    }
    let mut count = 0_usize;
    for (offset, c) in text.char_indices() {
        let end = (offset + c.len_utf8()) as u32;
        if count != 0 && is_mark(c) {
            scratch.clusters[count - 1].1 = end;
            continue;
        }
        if count == MAX_CLUSTERS {
            return Err(EngineError::Limit("bidi clusters per line"));
        }
        scratch.clusters[count] = (offset as u32, end);
        scratch.kinds[count] = if is_rtl(c) {
            Kind::Rtl
        } else if is_ltr(c) {
            Kind::Ltr
        } else {
            Kind::Neutral
        };
        count += 1;
    }
    let kinds = &scratch.kinds[..count];
    if base_rtl.unwrap_or_else(|| rtl_base(text)) {
        // Units: left-to-right runs kept whole, every other cluster alone,
        // mirrored; the units are written in reverse.
        let mut units = 0_usize;
        let mut position = 0_usize;
        let mut push = |from: usize, to: usize, mirrored: bool| {
            scratch.units[units] = (from as u32, to as u32, mirrored);
            units += 1;
        };
        runs(kinds, Kind::Ltr, Kind::Rtl, |a, b| {
            for index in position..a {
                push(index, index + 1, true);
            }
            push(a, b, false);
            position = b;
        });
        for index in position..count {
            push(index, index + 1, true);
        }
        for (from, to, mirrored) in scratch.units[..units].iter().rev() {
            for index in *from as usize..*to as usize {
                scratch.write(text, index, *mirrored, out)?;
            }
        }
        return Ok(());
    }
    // Left-to-right base: each right-to-left run is reversed in place, mirrored.
    let mut reversed = 0_usize;
    runs(kinds, Kind::Rtl, Kind::Ltr, |a, b| {
        scratch.units[reversed] = (a as u32, b as u32, true);
        reversed += 1;
    });
    let mut next_run = 0_usize;
    let mut index = 0_usize;
    while index < count {
        if next_run < reversed && scratch.units[next_run].0 as usize == index {
            let (a, b, _) = scratch.units[next_run];
            for member in (a as usize..b as usize).rev() {
                scratch.write(text, member, true, out)?;
            }
            index = b as usize;
            next_run += 1;
        } else {
            scratch.write(text, index, false, out)?;
            index += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flipped(text: &str, base: Option<bool>) -> String {
        let mut scratch = Box::new(Scratch::new());
        let mut out = String::new();
        flip(text, base, &mut scratch, &mut out).expect("flip");
        out
    }

    #[test]
    fn classes_follow_the_reference_tables() {
        assert!(is_rtl('ا') && is_rtl('א') && !is_rtl('a') && !is_rtl('1'));
        assert!(is_ltr('a') && is_ltr('1') && is_ltr('٣') && !is_ltr('ا') && !is_ltr(' '));
        assert!(is_mark('\u{64e}') && !is_mark('a'));
        assert!(has_rtl("abc א") && !has_rtl("abc 123"));
        assert!(rtl_base("םולש abc") && !rtl_base("see םולש here"));
        // Digits are not L letters: an Arabic word with digits reads right to left.
        assert!(rtl_base("مرحبا 123") && rtl_base("[a] אב") && !rtl_base(")בא( abc"));
    }

    #[test]
    fn flip_matches_rtl_py_and_is_its_own_inverse() {
        // Expected values were produced by indra_ocr.rtl.flip (1.1.2) on the same inputs.
        let cases: [(&str, Option<bool>, &str); 10] = [
            (")בא( abc", None, ")אב( abc"),
            ("بَا", Some(true), "ابَ"),
            ("see םולש here", Some(false), "see שלום here"),
            ("see םולש here", None, "see שלום here"),
            ("x ב-א y", Some(false), "x א-ב y"),
            ("םולש abc", None, "abc שלום"),
            ("مرحبا 123", None, "123 ابحرم"),
            ("[a] אב", Some(false), "[a] בא"),
            ("12 אב", None, "בא 12"),
            ("hello", None, "hello"),
        ];
        for (text, base, expected) in cases {
            let out = flipped(text, base);
            assert_eq!(out, expected, "flip of {text:?}");
            assert_eq!(
                flipped(&out, base),
                text,
                "flip is an involution for {text:?}"
            );
        }
        // Harakat stay with their consonant: two clusters, three characters.
        assert_eq!(flipped("بَا", Some(true)).chars().count(), 3);
    }

    #[test]
    fn mirrored_brackets_and_cluster_limits_are_explicit() {
        assert_eq!(mirror('('), ')');
        assert_eq!(mirror('«'), '»');
        assert_eq!(mirror('a'), 'a');
        let mut scratch = Box::new(Scratch::new());
        let mut out = String::new();
        let long = "א".repeat(MAX_CLUSTERS + 1);
        assert!(matches!(
            flip(&long, None, &mut scratch, &mut out),
            Err(EngineError::Limit(_))
        ));
    }
}
