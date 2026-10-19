//! An independent u64 oracle checks the strengthened ordinary behavior.
use indra_batch_plan_proof::{BatchError, BatchPlan};

fn check_add(plan: &mut BatchPlan, max_count: u16, max_work: u32, width: u32) {
    let previous_count = plan.count();
    let previous_width = plan.width();
    let aligned = u64::from(width).div_ceil(8) * 8;
    let widest = aligned.max(u64::from(previous_width));
    let count = u64::from(previous_count) + 1;
    let expected = if width == 0 || aligned > u64::from(u32::MAX) {
        Err(BatchError::InvalidWidth)
    } else if count > u64::from(max_count) || widest * count > u64::from(max_work) {
        Err(BatchError::Full)
    } else {
        Ok(())
    };
    let actual = plan.try_add(width);
    assert_eq!(actual, expected);
    if actual.is_ok() {
        assert_eq!(u64::from(plan.count()), count);
        assert_eq!(u64::from(plan.width()), widest);
        assert_eq!(u64::from(plan.work()), widest * count);
    } else {
        assert_eq!(plan.count(), previous_count);
        assert_eq!(plan.width(), previous_width);
    }
}

#[test]
fn exact_width_acceptance_iff_and_error_precedence_match_u64_oracle() {
    static WIDTHS: [u32; 16] = [
        0,
        1,
        7,
        8,
        9,
        63,
        64,
        65,
        640,
        641,
        649,
        657,
        4096,
        u32::MAX - 7,
        u32::MAX - 6,
        u32::MAX,
    ];
    for max_count in [0, 1, 2, 64, u16::MAX] {
        for max_work in [0, 1, 7, 8, 16, 40960, 41472, u32::MAX] {
            for offset in 0..WIDTHS.len() {
                let mut plan = BatchPlan::new(max_count, max_work);
                for index in 0..WIDTHS.len() {
                    check_add(
                        &mut plan,
                        max_count,
                        max_work,
                        WIDTHS[(index + offset) % WIDTHS.len()],
                    );
                }
            }
        }
    }
    let mut full = BatchPlan::new(u16::MAX, u32::MAX);
    for _ in 0..=u16::MAX {
        check_add(&mut full, u16::MAX, u32::MAX, 8);
    }
    check_add(&mut full, u16::MAX, u32::MAX, 0);
}
