//! Horizontal ink profiles of the skew-search thumbnails; independent of OpenCV.

/// The longest slice `sum` adds exactly: each byte adds at most 255 to a `u32`.
pub(super) const MAX_SUM_BYTES: usize = u32::MAX as usize / 255;

/// The exact sum of at most `MAX_SUM_BYTES` bytes. No 16-byte block and no
/// running total can reach 2^32, so the wrapping additions never wrap. Out of
/// line, LLVM vectorises this form (`psadbw` on the x86-64 baseline); inlined
/// into the row loop of `sharpness` under the release profile's thin LTO it
/// stays sixteen scalar adds per block. The checked `u8` to `i64` sum it
/// replaces is one byte, one add and one overflow branch per step under the
/// release profile's overflow checks.
#[inline(never)]
pub(super) fn sum(bytes: &[u8]) -> u32 {
    debug_assert!(bytes.len() <= MAX_SUM_BYTES);
    let block = |block: &[u8]| {
        block
            .iter()
            .fold(0_u32, |total, byte| total.wrapping_add(u32::from(*byte)))
    };
    let (blocks, tail) = bytes.as_chunks::<16>();
    blocks
        .iter()
        .fold(block(tail), |total, chunk| total.wrapping_add(block(chunk)))
}

/// `_row_sharpness` after the warp: the squared row-to-row change of the
/// horizontal ink profile of `pixels`, rows of `width` bytes. Row sums are
/// exact integers, so the f64 accumulation equals NumPy's pairwise sum.
pub(super) fn sharpness(pixels: &[u8], width: usize) -> f64 {
    let mut total = 0.0_f64;
    let mut prior = 0_i64;
    for (index, row) in pixels.chunks_exact(width).enumerate() {
        let value = i64::from(sum(row));
        if index != 0 {
            let difference = (value - prior) as f64;
            total += difference * difference;
        }
        prior = value;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The row sum this module replaced: a checked `u8` to `i64` sum.
    fn legacy_sum(row: &[u8]) -> i64 {
        row.iter().map(|pixel| i64::from(*pixel)).sum::<i64>()
    }

    fn legacy_sharpness(pixels: &[u8], width: usize) -> f64 {
        let mut total = 0.0_f64;
        let mut prior = 0_i64;
        for (index, row) in pixels.chunks_exact(width).enumerate() {
            let value = legacy_sum(row);
            if index != 0 {
                let difference = (value - prior) as f64;
                total += difference * difference;
            }
            prior = value;
        }
        total
    }

    /// xorshift64: reproducible pixels without a random-number crate.
    struct Random(u64);

    impl Random {
        fn byte(&mut self) -> u8 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 24) as u8
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum Fill {
        Ink,
        Random,
        AlternatingPixels,
        AlternatingRows,
    }

    static FILLS: [Fill; 4] = [
        Fill::Ink,
        Fill::Random,
        Fill::AlternatingPixels,
        Fill::AlternatingRows,
    ];

    fn fill(pixels: &mut [u8], width: usize, kind: Fill, random: &mut Random) {
        for (index, pixel) in pixels.iter_mut().enumerate() {
            *pixel = match kind {
                Fill::Ink => 255,
                Fill::Random => random.byte(),
                Fill::AlternatingPixels => [255, 0][index % 2],
                Fill::AlternatingRows => [255, 0][index / width % 2],
            };
        }
    }

    #[test]
    fn block_sums_and_sharpness_equal_the_checked_formula_at_every_width() {
        // Widths 1..=800 cover every 16-byte tail of rows up to the fine
        // thumbnail's width; three rows give the profile row-to-row changes.
        let mut random = Random(0x9e37_79b9_7f4a_7c15);
        let mut rows = [0_u8; 3 * 800];
        for width in 1..=800 {
            for kind in FILLS {
                let rows = &mut rows[..3 * width];
                fill(rows, width, kind, &mut random);
                for row in rows.chunks_exact(width) {
                    assert_eq!(
                        i64::from(sum(row)),
                        legacy_sum(row),
                        "{kind:?} width {width}"
                    );
                }
                assert_eq!(
                    sharpness(rows, width).to_bits(),
                    legacy_sharpness(rows, width).to_bits(),
                    "{kind:?} width {width}"
                );
            }
        }
    }

    #[test]
    fn sharpness_and_ink_totals_are_bit_identical_on_the_thumbnail_shapes() {
        // Width × height of the fine and coarse thumbnails of the math (preview
        // 1128 × 1600) and TIFF (1100 × 220) fixtures, and a square page whose
        // all-ink total of 163,200,000 is the largest the skew search can sum.
        let mut random = Random(0x2545_f491_4f6c_dd1d);
        let mut page = [0_u8; 800 * 800];
        for (width, height) in [(564, 800), (282, 400), (800, 160), (400, 80), (800, 800)] {
            for kind in FILLS {
                let pixels = &mut page[..width * height];
                fill(pixels, width, kind, &mut random);
                assert_eq!(
                    sharpness(pixels, width).to_bits(),
                    legacy_sharpness(pixels, width).to_bits(),
                    "{kind:?} {width}x{height}"
                );
                assert_eq!(
                    i64::from(sum(pixels)),
                    legacy_sum(pixels),
                    "{kind:?} {width}x{height}"
                );
            }
        }
    }
}
