//! Exact tensor-width accounting. All crops in a batch use the same padded
//! width. Count and aligned work ceilings both apply before a crop is admitted.

use creusot_std::prelude::{bitwise_proof, ensures, logic, DeepModel, Invariant};

use thiserror::Error;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum BatchError {
    #[error("crop width must be positive and alignment must fit")]
    InvalidWidth,
    #[error("recognition batch count or padded-width budget would be exceeded")]
    Full,
}

#[derive(Debug)]
pub struct BatchPlan {
    count: u16,
    width: u32,
    max_count: u16,
    max_work: u32,
}

impl BatchPlan {
    #[ensures(result.count == 0u16 && result.width == 0u32)]
    #[ensures(result.max_count == max_count && result.max_work == max_work)]
    pub const fn new(max_count: u16, max_work: u32) -> Self {
        Self {
            count: 0,
            width: 0,
            max_count,
            max_work,
        }
    }

    #[ensures(result == self.count)]
    pub const fn count(&self) -> u16 {
        self.count
    }

    #[ensures(result == self.width)]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[ensures(result@ == self.width@ * self.count@)]
    #[ensures(result@ <= self.max_work@)]
    pub const fn work(&self) -> u32 {
        self.width * self.count as u32
    }

    #[bitwise_proof]
    #[ensures(match result {
        Some(aligned) => width@ > 0 && width@ <= u32::MAX@ - 7
            && aligned@ == ((width@ + 7) / 8) * 8
            && width@ <= aligned@ && aligned@ - width@ < 8,
        None => width == 0u32 || width@ > u32::MAX@ - 7,
    })]
    pub const fn aligned_width(width: u32) -> Option<u32> {
        if width == 0 {
            return None;
        }
        match width.checked_add(7) {
            Some(padded) => Some(padded & !7),
            None => None,
        }
    }

    /// Failure leaves the previous plan unchanged.
    #[ensures((^self).max_count == self.max_count && (^self).max_work == self.max_work)]
    #[ensures(match result {
        Ok(()) => (^self).count@ == self.count@ + 1
            && (^self).width@ >= self.width@ && (^self).width@ >= width@,
        Err(_) => (^self).count == self.count && (^self).width == self.width,
    })]
    pub fn try_add(&mut self, width: u32) -> Result<(), BatchError> {
        let aligned = Self::aligned_width(width).ok_or(BatchError::InvalidWidth)?;
        let widest = self.width.max(aligned);
        let count = self.count.checked_add(1).ok_or(BatchError::Full)?;
        if count > self.max_count || widest > self.max_work / u32::from(count) {
            return Err(BatchError::Full);
        }
        self.count = count;
        self.width = widest;
        Ok(())
    }

    #[ensures((^self).count == 0u16 && (^self).width == 0u32)]
    #[ensures((^self).max_count == self.max_count && (^self).max_work == self.max_work)]
    pub fn clear(&mut self) {
        self.count = 0;
        self.width = 0;
    }
}

// BEGIN proof-only BatchPlan invariant. Erased from ordinary compilation.
impl DeepModel for BatchError {
    type DeepModelTy = Self;

    #[logic(open)]
    fn deep_model(self) -> Self {
        self
    }
}

impl Invariant for BatchPlan {
    #[logic]
    fn invariant(self) -> bool {
        pearlite! {
            self.count@ <= self.max_count@
            && self.width@ * self.count@ <= self.max_work@
            && self.width@ % 8 == 0
            && (self.count == 0u16) == (self.width == 0u32)
        }
    }
}
// END proof-only BatchPlan invariant.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_final_alignment_and_preserves_plan_on_failure() {
        let mut batch = BatchPlan::new(64, 16);
        batch.try_add(1).expect("first aligned crop");
        batch.try_add(8).expect("second aligned crop");
        assert_eq!(batch.work(), 16);
        assert_eq!(batch.try_add(9), Err(BatchError::Full));
        assert_eq!(batch.count(), 2);
        assert_eq!(batch.width(), 8);
        assert_eq!(batch.try_add(u32::MAX), Err(BatchError::InvalidWidth));
    }

    #[test]
    fn rejects_zero_capacity_and_zero_width() {
        assert_eq!(BatchPlan::new(0, 40960).try_add(8), Err(BatchError::Full));
        assert_eq!(BatchPlan::new(64, 0).try_add(8), Err(BatchError::Full));
        assert_eq!(
            BatchPlan::new(64, 40960).try_add(0),
            Err(BatchError::InvalidWidth)
        );
    }

    #[test]
    fn count_and_work_never_exceed_their_independent_bounds() {
        for max_count in [1, 2, 16, 64] {
            for max_work in [8, 64, 40960, u32::MAX] {
                let mut batch = BatchPlan::new(max_count, max_work);
                for width in 1..2048 {
                    if batch.try_add(width).is_err() {
                        batch.clear();
                    }
                    assert!(batch.count() <= max_count);
                    assert!(batch.work() <= max_work);
                    assert_eq!(batch.width() % 8, 0);
                }
            }
        }
    }

    #[test]
    fn largest_valid_width_and_count_stay_bounded() {
        let mut wide = BatchPlan::new(64, u32::MAX);
        wide.try_add(u32::MAX - 7).expect("largest aligned width");
        assert_eq!(wide.work(), u32::MAX - 7);
        assert_eq!(wide.try_add(8), Err(BatchError::Full));
        let mut count = BatchPlan::new(u16::MAX, u32::MAX);
        for _ in 0..u16::MAX {
            count.try_add(8).expect("count within u16");
        }
        assert_eq!(count.try_add(8), Err(BatchError::Full));
        assert_eq!(count.work(), u32::from(u16::MAX) * 8);
    }
}
