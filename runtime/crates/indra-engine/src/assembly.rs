//! Split detector merges at token gaps, then expose borrowed line text to reading order.

use std::{ops::Range, time::Instant};

use opencv::prelude::*;

use crate::{
    EngineError, MAX_LINES, MAX_OUTPUT_BYTES, Result, deadline,
    recognition::{self, Lines, MAX_TOKENS, Script},
};

pub(crate) struct Page {
    pub boxes: [[f32; 4]; MAX_LINES],
    pub ranges: [Range<usize>; MAX_LINES],
    pub text: String,
    pub count: usize,
}

impl Page {
    pub fn new() -> Self {
        Self {
            boxes: [[0.0; 4]; MAX_LINES],
            ranges: [const { 0..0 }; MAX_LINES],
            text: String::with_capacity(MAX_OUTPUT_BYTES),
            count: 0,
        }
    }

    pub fn assemble(&mut self, boxes: &[[f32; 4]], lines: &Lines, due: Instant) -> Result<()> {
        self.count = 0;
        self.text.clear();
        let ids = lines.ids.data_typed::<i32>()?;
        let positions = lines.positions.data_typed::<f64>()?;
        for (index, b) in boxes.iter().enumerate() {
            deadline(due)?;
            let line = &lines.metadata[index];
            if line.script == Script::Math {
                let start = self.text.len();
                recognition::append(
                    &mut self.text,
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
                if recognition::token(&line.script, *id)? == " " {
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
                recognition::decode_tokens(&line.script, &ids[first..last], &mut self.text)?;
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
                    if recognition::token(&line.script, ids[token])? != " " {
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
        Ok(())
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
}
