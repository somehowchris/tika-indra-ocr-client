use serde::{Deserialize, Serialize};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

/// NFC Unicode scalars the workspace holds for one text.
pub const MAX_CODEPOINTS: usize = 32_768;
/// Whitespace tokens the workspace holds for one text. Every token after the
/// first follows a whitespace scalar, so a text within `MAX_CODEPOINTS` never
/// has more.
pub const MAX_WORDS: usize = MAX_CODEPOINTS / 2;
// Word offsets are u16 scalar positions, including the exclusive end.
const _: () = assert!(MAX_CODEPOINTS <= u16::MAX as usize);

#[derive(Debug, Error)]
pub enum MetricError {
    #[error("text exceeds the {MAX_CODEPOINTS}-codepoint evaluation limit")]
    TooLong,
}

#[derive(Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Distance {
    pub reference_units: usize,
    pub hypothesis_units: usize,
    pub errors: usize,
}

impl Distance {
    /// An empty reference has no defined rate; retain its insertion count.
    pub fn rate(&self) -> Option<f64> {
        (self.reference_units != 0).then(|| self.errors as f64 / self.reference_units as f64)
    }
}

#[derive(Default)]
struct Word {
    start: u16,
    end: u16,
}

/// Reused across comparisons; edit distance uses one row and a diagonal scalar.
/// The score is not capped at 100%: insertions can make errors exceed reference length.
pub struct Workspace {
    reference: [char; MAX_CODEPOINTS],
    hypothesis: [char; MAX_CODEPOINTS],
    row: [u32; MAX_CODEPOINTS + 1],
    reference_words: [Word; MAX_WORDS],
    hypothesis_words: [Word; MAX_WORDS],
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            reference: ['\0'; MAX_CODEPOINTS],
            hypothesis: ['\0'; MAX_CODEPOINTS],
            row: [0; MAX_CODEPOINTS + 1],
            reference_words: std::array::from_fn(|_| Word::default()),
            hypothesis_words: std::array::from_fn(|_| Word::default()),
        }
    }
}

impl Workspace {
    pub fn characters(
        &mut self,
        reference: &str,
        hypothesis: &str,
        collapse_whitespace: bool,
    ) -> Result<Distance, MetricError> {
        let n = fill(reference, &mut self.reference, collapse_whitespace)?;
        let m = fill(hypothesis, &mut self.hypothesis, collapse_whitespace)?;
        let errors = edit_distance(n, m, &mut self.row, |i, j| {
            self.reference[i] == self.hypothesis[j]
        });
        Ok(Distance {
            reference_units: n,
            hypothesis_units: m,
            errors,
        })
    }

    /// NFC whitespace-token WER. Call only for cases whose declared tokenizer is
    /// `whitespace`; whitespace splitting is not a Chinese word segmenter.
    pub fn words(&mut self, reference: &str, hypothesis: &str) -> Result<Distance, MetricError> {
        let reference_len = fill(reference, &mut self.reference, false)?;
        let hypothesis_len = fill(hypothesis, &mut self.hypothesis, false)?;
        let n = words(&self.reference[..reference_len], &mut self.reference_words)?;
        let m = words(
            &self.hypothesis[..hypothesis_len],
            &mut self.hypothesis_words,
        )?;
        let errors = edit_distance(n, m, &mut self.row, |i, j| {
            let a = &self.reference_words[i];
            let b = &self.hypothesis_words[j];
            self.reference[a.start as usize..a.end as usize]
                == self.hypothesis[b.start as usize..b.end as usize]
        });
        Ok(Distance {
            reference_units: n,
            hypothesis_units: m,
            errors,
        })
    }
}

fn words(text: &[char], out: &mut [Word; MAX_WORDS]) -> Result<usize, MetricError> {
    let mut count = 0;
    let mut position = 0;
    while position < text.len() {
        if text[position].is_whitespace() {
            position += 1;
            continue;
        }
        let start = position;
        while position < text.len() && !text[position].is_whitespace() {
            position += 1;
        }
        let word = out.get_mut(count).ok_or(MetricError::TooLong)?;
        word.start = start as u16;
        word.end = position as u16;
        count += 1;
    }
    Ok(count)
}

fn edit_distance(
    n: usize,
    m: usize,
    row: &mut [u32; MAX_CODEPOINTS + 1],
    equal: impl Fn(usize, usize) -> bool,
) -> usize {
    // Equal prefixes and suffixes can be removed from unit-cost Levenshtein
    // distance. Exact outputs then take linear comparison time and no DP cells.
    let mut prefix = 0;
    while prefix < n.min(m) && equal(prefix, prefix) {
        prefix += 1;
    }
    let mut end_n = n;
    let mut end_m = m;
    while end_n > prefix && end_m > prefix && equal(end_n - 1, end_m - 1) {
        end_n -= 1;
        end_m -= 1;
    }
    let width = end_m - prefix;
    for (j, cell) in row[..=width].iter_mut().enumerate() {
        *cell = j as u32;
    }
    for i in prefix..end_n {
        let mut diagonal = row[0];
        row[0] = (i - prefix + 1) as u32;
        for j in 0..width {
            let above = row[j + 1];
            row[j + 1] = (above + 1)
                .min(row[j] + 1)
                .min(diagonal + u32::from(!equal(i, prefix + j)));
            diagonal = above;
        }
    }
    row[width] as usize
}

fn fill(
    text: &str,
    out: &mut [char; MAX_CODEPOINTS],
    collapse: bool,
) -> Result<usize, MetricError> {
    let mut count = 0;
    let mut pending_space = false;
    for ch in text.nfc() {
        if collapse && ch.is_whitespace() {
            pending_space = count != 0;
            continue;
        }
        if pending_space {
            *out.get_mut(count).ok_or(MetricError::TooLong)? = ' ';
            count += 1;
            pending_space = false;
        }
        *out.get_mut(count).ok_or(MetricError::TooLong)? = ch;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scores_unicode_scalars_after_nfc_and_preserves_punctuation() {
        let mut w = Workspace::default();
        assert_eq!(
            w.characters("a\u{308} β中", "ä β中", false).unwrap(),
            Distance {
                reference_units: 4,
                hypothesis_units: 4,
                errors: 0
            }
        );
        assert_eq!(w.characters("a-b", "ab", false).unwrap().errors, 1);
        assert_eq!(w.characters("कि", "क", false).unwrap().errors, 1);
    }
    #[test]
    fn empty_reference_has_insertion_count_without_division() {
        let mut w = Workspace::default();
        assert_eq!(
            w.characters("", "hallucination", false).unwrap(),
            Distance {
                reference_units: 0,
                hypothesis_units: 13,
                errors: 13
            }
        );
        assert_eq!(w.characters("a", "aaaa", false).unwrap().errors, 3);
    }
    #[test]
    fn normalised_secondary_score_only_collapses_whitespace() {
        let mut w = Workspace::default();
        assert_eq!(
            w.characters("  Français\n中文\t", "Français 中文", true)
                .unwrap()
                .errors,
            0
        );
        assert_ne!(w.characters("\ntext\n", "text", false).unwrap().errors, 0);
        assert_ne!(w.characters("①", "1", true).unwrap().errors, 0);
    }
    #[test]
    fn retained_workspace_does_not_keep_previous_comparison_state() {
        let mut w = Workspace::default();
        assert_eq!(w.characters("kitten", "sitting", false).unwrap().errors, 3);
        assert_eq!(w.characters("x", "", false).unwrap().errors, 1);
        assert_eq!(w.characters("", "", false).unwrap().errors, 0);
    }
    #[test]
    fn adversarial_unicode_is_scalar_nfc_without_compatibility_or_glyph_folding() {
        let mut w = Workspace::default();
        for (a, b) in [("가", "가"), ("q\u{301}\u{327}", "q\u{327}\u{301}")] {
            assert_eq!(w.characters(a, b, false).unwrap().errors, 0);
        }
        for (a, b) in [
            ("👩\u{200d}🔬", "👩🔬"),
            ("مَ", "م"),
            ("Α", "A"),
            ("①", "1"),
            ("x\u{200b}y", "xy"),
        ] {
            assert_eq!(w.characters(a, b, true).unwrap().errors, 1);
        }
    }
    #[test]
    fn whitespace_words_preserve_punctuation_and_canonical_accents() {
        let mut w = Workspace::default();
        assert_eq!(
            w.words("a\u{308}\u{a0}β\n42.", "ä β 42.").unwrap().errors,
            0
        );
        assert_eq!(
            w.words("coût 42.", "cout 42").unwrap(),
            Distance {
                reference_units: 2,
                hypothesis_units: 2,
                errors: 2
            }
        );
        assert_eq!(w.words("", "x y").unwrap().rate(), None);
        assert_eq!(w.words("one", "one two three").unwrap().rate(), Some(2.0));
    }
    #[test]
    fn boundaries_are_checked_before_comparison_without_truncating_text() {
        let mut w = Workspace::default();
        let mut text = "a ".repeat(MAX_WORDS);
        assert_eq!(text.chars().count(), MAX_CODEPOINTS);
        assert_eq!(w.words(&text, &text).unwrap().reference_units, MAX_WORDS);
        assert_eq!(w.characters(&text, &text, false).unwrap().errors, 0);
        text.push('a');
        assert!(matches!(
            w.characters(&text, "", false),
            Err(MetricError::TooLong)
        ));
    }
}
