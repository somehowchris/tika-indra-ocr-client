//! Stable in-place batch-axis gathering; independent of ORT and the allocator.

pub(super) trait Bits: Copy {
    fn same_bits(self, other: Self) -> bool;
}
impl Bits for f32 {
    fn same_bits(self, other: Self) -> bool {
        self.to_bits() == other.to_bits()
    }
}
impl Bits for i64 {
    fn same_bits(self, other: Self) -> bool {
        self == other
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
pub(super) struct Movement {
    pub bytes: usize,
    pub verified_elements: usize,
}

/// `data` may retain a larger physical allocation from an earlier gather. All
/// strides use `old_batch`, the CURRENT logical batch. Selected rows must be
/// stable and unique. Forward copies cannot overwrite a later selected source.
/// Each moved block has disjoint source/destination ranges, so the original
/// source remains available for an immediate bit comparison without a buffer.
pub(super) fn gather<T: Bits, const VERIFY: bool>(
    data: &mut [T],
    layers: usize,
    old_batch: usize,
    rows: &[usize],
    block: usize,
) -> Result<Movement, &'static str> {
    if layers == 0 || old_batch == 0 || old_batch > 64 || rows.len() > old_batch {
        return Err("gather dimensions");
    }
    let required = layers
        .checked_mul(old_batch)
        .and_then(|n| n.checked_mul(block))
        .ok_or("gather element count overflow")?;
    if data.len() < required
        || rows.iter().any(|row| *row >= old_batch)
        || rows.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err("gather selection");
    }
    let mut movement = Movement::default();
    for layer in 0..layers {
        for (destination_row, source_row) in rows.iter().enumerate() {
            let source = (layer * old_batch + source_row) * block;
            let destination = (layer * rows.len() + destination_row) * block;
            if source == destination || block == 0 {
                continue;
            }
            if destination > source || destination + block > source {
                return Err("gather overlapping source block");
            }
            data.copy_within(source..source + block, destination);
            if VERIFY
                && !data[destination..destination + block]
                    .iter()
                    .zip(&data[source..source + block])
                    .all(|(actual, expected)| actual.same_bits(*expected))
            {
                return Err("gather bit parity");
            }
            movement.bytes = movement
                .bytes
                .checked_add(
                    block
                        .checked_mul(size_of::<T>())
                        .ok_or("gather byte overflow")?,
                )
                .ok_or("gather byte overflow")?;
            if VERIFY {
                movement.verified_elements = movement
                    .verified_elements
                    .checked_add(block)
                    .ok_or("gather count overflow")?;
            }
        }
    }
    Ok(movement)
}

#[cfg(test)]
mod tests {

    // Build expected values independently from immutable original coordinates;
    // the oracle never uses copy_within or a compacted layer stride.
    #[test]
    fn first_middle_last_multiple_all_none_one() {
        fn check(rows: &[usize]) {
            let mut data = [0_i64; 3 * 6 * 8 * 5 * 24];
            for (index, value) in data.iter_mut().enumerate() {
                *value = index as i64 + 1;
            }
            let original = data;
            let block = 8 * 5 * 24;
            super::gather::<_, true>(&mut data, 3, 6, rows, block).expect("valid gather");
            for layer in 0..3 {
                for (target, source) in rows.iter().enumerate() {
                    for element in 0..block {
                        assert_eq!(
                            data[(layer * rows.len() + target) * block + element],
                            original[(layer * 6 + source) * block + element],
                        );
                    }
                }
            }
        }
        for rows in [
            &[1, 2, 3, 4, 5][..],
            &[0, 1, 3, 4, 5],
            &[0, 1, 2, 3, 4],
            &[1, 3, 4, 5],
            &[],
            &[0, 1, 2, 3, 4, 5],
            &[5],
        ] {
            check(rows);
        }
    }

    #[test]
    fn repeated_gather_uses_logical_not_physical_batch() {
        let mut data = [0_i64; 3 * 6 * 7];
        for (index, value) in data.iter_mut().enumerate() {
            *value = index as i64;
        }
        let original = data;
        super::gather::<_, true>(&mut data, 3, 6, &[1, 3, 4, 5], 7).unwrap();
        super::gather::<_, true>(&mut data, 3, 4, &[0, 1, 2], 7).unwrap();
        super::gather::<_, true>(&mut data, 3, 3, &[0, 2], 7).unwrap();
        super::gather::<_, true>(&mut data, 3, 2, &[1], 7).unwrap();
        for layer in 0..3 {
            assert_eq!(
                &data[layer * 7..(layer + 1) * 7],
                &original[(layer * 6 + 4) * 7..(layer * 6 + 5) * 7],
            );
        }
    }

    #[test]
    fn boundaries_overflow_invalid_and_signed_zero() {
        let mut empty = [] as [i64; 0];
        assert_eq!(
            super::gather::<_, true>(&mut empty, 3, 64, &[63], 0)
                .unwrap()
                .bytes,
            0
        );
        assert!(super::gather::<_, true>(&mut empty, usize::MAX, 64, &[0], 319).is_err());
        assert!(super::gather::<_, true>(&mut [0_i64; 8], 1, 4, &[2, 1], 2).is_err());
        assert!(super::gather::<_, true>(&mut [0_i64; 8], 1, 4, &[1, 1], 2).is_err());
        assert!(super::gather::<_, true>(&mut [0_i64; 8], 1, 4, &[4], 2).is_err());
        let mut data = [0.0, -0.0, f32::from_bits(0x7fc00042)];
        let movement = super::gather::<_, true>(&mut data, 1, 3, &[1, 2], 1).unwrap();
        assert_eq!(data[0].to_bits(), (-0.0_f32).to_bits());
        assert_eq!(data[1].to_bits(), 0x7fc00042);
        assert_eq!(movement.bytes, 8);
        let mut maximal = [0_i64; 64 * 319];
        for (index, value) in maximal.iter_mut().enumerate() {
            *value = index as i64;
        }
        super::gather::<_, true>(&mut maximal, 1, 64, &[63], 319).unwrap();
        for (index, value) in maximal[..319].iter().enumerate() {
            assert_eq!(*value, (63 * 319 + index) as i64);
        }
    }
}
