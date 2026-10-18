//! Width-aware, area-bounded line batches; never pads an entire page to its widest line.

use std::ops::Range;

use crate::{EngineError, MAX_BATCH, MAX_LINES, Result};

pub(crate) const TEXT_COLUMNS: usize = MAX_BATCH * (640 + 8);
pub(crate) const MATH_COLUMNS: usize = MAX_BATCH * 4096;

pub(crate) enum Policy {
    Text,
    Math,
}

pub(crate) struct Plan {
    pub indices: [usize; MAX_LINES],
    count: usize,
    position: usize,
    batch: usize,
    policy: Policy,
}

impl Plan {
    pub fn new(
        widths: &[usize],
        batch: usize,
        policy: Policy,
        include: impl Fn(usize) -> bool,
    ) -> Result<Self> {
        if widths.len() > MAX_LINES
            || !(1..=MAX_BATCH).contains(&batch)
            || widths.iter().any(|w| !(8..=4096).contains(w))
        {
            return Err(EngineError::Shape("recognition batch dimensions"));
        }
        let mut result = Self {
            indices: [0; MAX_LINES],
            count: 0,
            position: 0,
            batch,
            policy,
        };
        for i in 0..widths.len() {
            if include(i) {
                result.indices[result.count] = i;
                result.count += 1;
            }
        }
        result.indices[..result.count].sort_unstable_by_key(|i| (widths[*i], *i));
        Ok(result)
    }

    pub fn next(&mut self, widths: &[usize]) -> Option<Range<usize>> {
        if self.position == self.count {
            return None;
        }
        let start = self.position;
        self.position += 1;
        let (ratio, area) = match self.policy {
            Policy::Text => (1.3, 64 * 640),
            Policy::Math => (4.0, 16 * 64 * 640),
        };
        while self.position < self.count && self.position - start < self.batch {
            let width = widths[self.indices[self.position]].max(64);
            let first = widths[self.indices[start]].max(64);
            if width as f64 > ratio * first as f64 || (self.position - start + 1) * width > area {
                break;
            }
            self.position += 1;
        }
        Some(start..self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exactly_reproduces_ratio_area_and_count_boundaries() {
        let widths = [
            4096, 64, 83, 84, 8, 4096, 4096, 4096, 4096, 4096, 4096, 4096, 4096, 4096, 4096,
        ];
        let mut plan = Plan::new(&widths, 64, Policy::Text, |_| true).expect("plan");
        let first = plan.next(&widths).expect("short");
        assert_eq!(&plan.indices[first], &[4, 1, 2]);
        let second = plan.next(&widths).expect("ratio split");
        assert_eq!(&plan.indices[second], &[3]);
        let third = plan.next(&widths).expect("area cap");
        assert_eq!(third.len(), 10);
        assert_eq!(plan.next(&widths).expect("remaining").len(), 1);
        assert!(plan.next(&widths).is_none());
    }
    #[test]
    fn selection_preserves_original_indices_and_small_config() {
        let widths = [100, 100, 100, 100];
        let mut p = Plan::new(&widths, 1, Policy::Text, |i| i % 2 == 1).expect("plan");
        let r = p.next(&widths).expect("first");
        assert_eq!(&p.indices[r], &[1]);
        let r = p.next(&widths).expect("second");
        assert_eq!(&p.indices[r], &[3]);
        assert!(p.next(&widths).is_none());
    }

    #[test]
    fn all_selected_lines_fit_fixed_scratch_at_one_eight_and_sixty_four() {
        let widths: [usize; MAX_LINES] = std::array::from_fn(|index| 8 + (index * 73) % 4089);
        for cap in [1, 8, 64] {
            for policy in [Policy::Text, Policy::Math] {
                let columns = match &policy {
                    Policy::Text => TEXT_COLUMNS,
                    Policy::Math => MATH_COLUMNS,
                };
                let mut plan = Plan::new(&widths, cap, policy, |index| index % 3 != 0)
                    .expect("bounded varied widths");
                let mut seen = [false; MAX_LINES];
                while let Some(range) = plan.next(&widths) {
                    assert!(!range.is_empty() && range.len() <= cap);
                    let indices = &plan.indices[range];
                    let mut widest = 0;
                    for index in indices {
                        assert!(!seen[*index], "a selected line cannot be processed twice");
                        seen[*index] = true;
                        widest = widest.max(widths[*index]);
                    }
                    let padded = widest.div_ceil(8) * 8;
                    assert!(
                        indices.len() * padded <= columns,
                        "fixed tensor scratch fits"
                    );
                }
                for (index, processed) in seen.iter().enumerate() {
                    assert_eq!(
                        *processed,
                        index % 3 != 0,
                        "every selected line is retained"
                    );
                }
            }
        }
    }
}
