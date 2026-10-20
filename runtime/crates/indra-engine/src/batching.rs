//! Width-aware, area-bounded line batches; never pads an entire page to its widest line.

use std::ops::Range;

use indra_core::batch::{BatchError, BatchPlan};

use crate::{EngineError, MAX_BATCH, MAX_LINES, Result};

pub(crate) const TEXT_COLUMNS: usize = MAX_BATCH * (640 + 8);
/// Startup columns of the base recogniser's tall-window arena. Tall widths
/// track the one-band widths that group a batch within a few per cent for
/// ordinary lines; the arena grows at a new high-water mark for the rest.
pub(crate) const WINDOW_COLUMNS: usize = TEXT_COLUMNS + TEXT_COLUMNS / 4;
pub(crate) const MATH_COLUMNS: usize = MAX_BATCH * 4096;

/// Account for the already selected rows against the actual tensor arena.
/// This does not choose batch membership: the upstream ratio/area policy below
/// remains responsible for grouping. Alignment must fit the arena, whose text
/// capacity includes padding beyond the grouping policy's raw-width budget.
pub(crate) fn tensor_widths(
    widths: impl ExactSizeIterator<Item = Result<usize>>,
    height: usize,
    scalar_capacity: usize,
    output: &mut [i64; MAX_BATCH],
) -> Result<BatchPlan> {
    if height == 0 || widths.len() > MAX_BATCH {
        return Err(EngineError::Shape("recognition batch dimensions"));
    }
    let columns = u32::try_from(scalar_capacity / height)
        .map_err(|_| EngineError::Limit("recognition tensor scratch"))?;
    let mut plan = BatchPlan::new(MAX_BATCH as u16, columns);
    for (slot, width) in widths.enumerate() {
        let width =
            u32::try_from(width?).map_err(|_| EngineError::Shape("recognition line width"))?;
        plan.try_add(width).map_err(|error| match error {
            BatchError::InvalidWidth => EngineError::Shape("recognition line width"),
            BatchError::Full => EngineError::Limit("recognition tensor scratch"),
        })?;
        output[slot] = i64::from(width);
    }
    Ok(plan)
}

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
                    let mut metadata = [-1_i64; MAX_BATCH];
                    let account = tensor_widths(
                        indices.iter().map(|index| Ok(widths[*index])),
                        32,
                        columns * 32,
                        &mut metadata,
                    )
                    .expect("the selected batch fits its real arena");
                    assert_eq!(account.width() as usize, padded);
                    assert_eq!(account.work() as usize, indices.len() * padded);
                    for (slot, index) in indices.iter().enumerate() {
                        assert_eq!(metadata[slot], widths[*index] as i64);
                    }
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

    #[test]
    fn raw_width_groups_keep_their_padding_headroom() {
        for (count, width, padded) in [(62, 657, 664), (63, 649, 656)] {
            let all_widths = [width; MAX_BATCH];
            let widths = &all_widths[..count];
            let mut selected = Plan::new(widths, MAX_BATCH, Policy::Text, |_| true).expect("group");
            let range = selected.next(widths).expect("one reference group");
            assert_eq!(range.len(), count);
            let indices = &selected.indices[range];
            let mut metadata = [-1; MAX_BATCH];
            let account = tensor_widths(
                indices.iter().map(|index| Ok(widths[*index])),
                32,
                32 * TEXT_COLUMNS,
                &mut metadata,
            )
            .expect("padded group fits the real arena");
            assert_eq!(account.width() as usize, padded);
            assert_eq!(account.work() as usize, count * padded);
            for (slot, index) in indices.iter().enumerate() {
                assert_eq!(*index, slot);
                assert_eq!(metadata[slot], width as i64);
            }
            assert!(metadata[count..].iter().all(|value| *value == -1));
            assert!(selected.next(widths).is_none());
        }
    }

    #[test]
    fn tensor_accounting_rejects_invalid_dimensions_and_exact_capacity_overflow() {
        let mut metadata = [-1; MAX_BATCH];
        assert!(tensor_widths([Ok(8)].into_iter(), 0, 256, &mut metadata).is_err());
        assert!(
            tensor_widths(
                (0..MAX_BATCH + 1).map(|_| Ok(8)),
                32,
                1_000_000,
                &mut metadata
            )
            .is_err()
        );
        assert_eq!(metadata, [-1; MAX_BATCH]);
        for width in [0, u32::MAX as usize] {
            assert!(tensor_widths([Ok(width)].into_iter(), 32, 256, &mut metadata).is_err());
            assert_eq!(metadata, [-1; MAX_BATCH]);
        }
        assert!(tensor_widths([Ok(8)].into_iter(), 32, 255, &mut metadata).is_err());
        assert_eq!(metadata, [-1; MAX_BATCH]);
        let account =
            tensor_widths([Ok(8)].into_iter(), 32, 256, &mut metadata).expect("exact capacity");
        assert_eq!(account.count(), 1);
        assert_eq!(account.width(), 8);
        assert_eq!(account.work(), 8);
        assert_eq!(metadata[0], 8);
        assert_eq!(metadata[1..], [-1; MAX_BATCH - 1]);
    }
}
