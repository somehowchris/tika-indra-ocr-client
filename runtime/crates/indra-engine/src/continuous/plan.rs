//! Two bounded owner queues. A rotating anchor gets the first row; a compatible
//! peer can contribute subsequent rows to the same native invocation.

use crate::{EngineError, MAX_BATCH, MAX_LINES, Result};

pub(super) struct View<'a> {
    pub widths: &'a [usize],
    pub indices: &'a [usize],
    pub flags: &'a [bool],
    pub route: Option<bool>,
    pub position: usize,
}

impl View<'_> {
    fn candidate(&self, position: &mut usize) -> Result<Option<(usize, usize)>> {
        while let Some(index) = self.indices.get(*position) {
            let width = *self
                .widths
                .get(*index)
                .filter(|width| (8..=4096).contains(*width))
                .ok_or(EngineError::Shape("continuous line width/index"))?;
            let flag = *self
                .flags
                .get(*index)
                .ok_or(EngineError::Shape("continuous script index"))?;
            if self.route.is_none_or(|route| route == flag) {
                return Ok(Some((*index, width)));
            }
            *position += 1;
        }
        Ok(None)
    }
}

pub(super) struct Row {
    pub owner: usize,
    pub line: usize,
    pub width: usize,
}

impl Row {
    const fn empty() -> Self {
        Self {
            owner: 0,
            line: 0,
            width: 0,
        }
    }
}

pub(super) struct Batch {
    pub rows: [Row; MAX_BATCH],
    pub count: usize,
    pub end: [usize; 2],
    pub owners: [usize; 2],
}

impl Batch {
    pub const fn new() -> Self {
        Self {
            rows: [const { Row::empty() }; MAX_BATCH],
            count: 0,
            end: [0; 2],
            owners: [0; 2],
        }
    }
}

/// Preserve the reference text policy's row cap, 1.3 width ratio (with the
/// 64-column proxy minimum), and 40,960-column raw work cap. The existing proved
/// tensor_widths check separately guards actual aligned scratch capacity.
/// Selection is transactional: caller cursors do not change until inference and
/// scattering have succeeded. No page is copied into a merged image.
pub(super) fn select(
    views: [Option<View<'_>>; 2],
    anchor: usize,
    cap: usize,
    batch: &mut Batch,
) -> Result<()> {
    if anchor >= 2 || !(1..=MAX_BATCH).contains(&cap) || views[anchor].is_none() {
        return Err(EngineError::Shape("continuous batch selection"));
    }
    for view in views.iter().flatten() {
        if view.indices.len() > MAX_LINES || view.position > view.indices.len() {
            return Err(EngineError::Shape("continuous queue position"));
        }
    }
    batch.count = 0;
    batch.owners.fill(0);
    for (end, view) in batch.end.iter_mut().zip(&views) {
        *end = view.as_ref().map_or(0, |view| view.position);
    }
    let mut preferred = anchor;
    let mut narrowest = usize::MAX;
    let mut widest = 0;
    while batch.count < cap {
        let mut added = false;
        for attempt in 0..2 {
            let owner = (preferred + attempt) % 2;
            let Some(view) = &views[owner] else { continue };
            let Some((line, width)) = view.candidate(&mut batch.end[owner])? else {
                continue;
            };
            let proxy = width.max(64);
            let low = narrowest.min(proxy);
            let high = widest.max(proxy);
            if high as f64 > 1.3 * low as f64 || (batch.count + 1) * high > 64 * 640 {
                continue;
            }
            batch.rows[batch.count] = Row { owner, line, width };
            batch.count += 1;
            batch.end[owner] += 1;
            batch.owners[owner] += 1;
            narrowest = low;
            widest = high;
            preferred = 1 - owner;
            added = true;
            break;
        }
        if !added {
            break;
        }
    }
    if batch.count == 0 {
        return Err(EngineError::Shape("continuous anchor has no ready rows"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatible_rows_retain_owner_and_original_line() {
        let widths = [[83, 64, 84], [70, 80, 800]];
        let indices = [[1, 0, 2], [0, 1, 2]];
        let flags = [false; 3];
        let views = std::array::from_fn(|owner| {
            Some(View {
                widths: &widths[owner],
                indices: &indices[owner],
                flags: &flags,
                route: None,
                position: 0,
            })
        });
        let mut batch = Batch::new();
        select(views, 0, 64, &mut batch).expect("compatible two-owner batch");
        assert_eq!(batch.count, 4);
        assert_eq!(batch.owners, [2, 2]);
        assert_eq!(batch.end, [2, 2]);
        let expected = [(0, 1, 64), (1, 0, 70), (0, 0, 83), (1, 1, 80)];
        for (row, expected) in batch.rows[..batch.count].iter().zip(expected) {
            assert_eq!((row.owner, row.line, row.width), expected);
        }
    }

    #[test]
    fn anchor_guarantees_wide_owner_progress_without_crossing_ratio() {
        let widths = [64, 4096];
        let indices = [0, 1];
        let flags = [false; 2];
        for anchor in 0..2 {
            let views = [
                Some(View {
                    widths: &widths,
                    indices: &indices[..1],
                    flags: &flags,
                    route: None,
                    position: 0,
                }),
                Some(View {
                    widths: &widths,
                    indices: &indices[1..],
                    flags: &flags,
                    route: None,
                    position: 0,
                }),
            ];
            let mut batch = Batch::new();
            select(views, anchor, 64, &mut batch).expect("bounded fair anchor");
            assert_eq!(batch.count, 1);
            assert_eq!(batch.rows[0].owner, anchor);
            assert_eq!(batch.owners[1 - anchor], 0);
        }
    }

    #[test]
    fn one_owner_keeps_reference_membership_and_padding_headroom() {
        for cap in [1, 8, 64] {
            let widths: [usize; MAX_LINES] = std::array::from_fn(|i| 8 + (i * 73) % 4089);
            let flags: [bool; MAX_LINES] = std::array::from_fn(|i| i % 3 == 0);
            let mut reference =
                crate::batching::Plan::new(&widths, cap, crate::batching::Policy::Text, |i| {
                    flags[i]
                })
                .expect("reference selection");
            let mut sorted: [usize; MAX_LINES] = std::array::from_fn(|i| i);
            sorted.sort_unstable_by_key(|i| (widths[*i], *i));
            let mut cursor = 0;
            while let Some(range) = reference.next(&widths) {
                let mut batch = Batch::new();
                select(
                    [
                        Some(View {
                            widths: &widths,
                            indices: &sorted,
                            flags: &flags,
                            route: Some(true),
                            position: cursor,
                        }),
                        None,
                    ],
                    0,
                    cap,
                    &mut batch,
                )
                .expect("single-owner fast path");
                let expected = &reference.indices[range];
                assert_eq!(batch.count, expected.len());
                for (row, expected) in batch.rows[..batch.count].iter().zip(expected) {
                    assert_eq!(row.owner, 0);
                    assert_eq!(row.line, *expected);
                }
                cursor = batch.end[0];
            }
        }
        let widths = [657; 62];
        let indices: [usize; 62] = std::array::from_fn(|i| i);
        let flags = [false; 62];
        let mut batch = Batch::new();
        select(
            [
                Some(View {
                    widths: &widths,
                    indices: &indices,
                    flags: &flags,
                    route: None,
                    position: 0,
                }),
                None,
            ],
            0,
            64,
            &mut batch,
        )
        .expect("raw budget");
        let mut tensor_widths = [0; MAX_BATCH];
        let account = crate::batching::tensor_widths(
            batch.rows[..batch.count].iter().map(|row| Ok(row.width)),
            32,
            32 * crate::batching::TEXT_COLUMNS,
            &mut tensor_widths,
        )
        .expect("actual aligned arena");
        assert_eq!(batch.count, 62);
        assert_eq!(account.work(), 41_168);
    }

    #[test]
    fn selection_is_transactional_and_route_is_not_confused_with_owner() {
        let widths = [64; 5];
        let indices = [4, 3, 2, 1, 0];
        let flags = [true, false, true, false, true];
        let mut batch = Batch::new();
        select(
            [
                None,
                Some(View {
                    widths: &widths,
                    indices: &indices,
                    flags: &flags,
                    route: Some(false),
                    position: 0,
                }),
            ],
            1,
            64,
            &mut batch,
        )
        .expect("text route");
        assert_eq!(batch.count, 2);
        assert_eq!(batch.owners, [0, 2]);
        assert_eq!(batch.rows[0].line, 3);
        assert_eq!(batch.rows[1].line, 1);
        assert_eq!(indices, [4, 3, 2, 1, 0]);
        assert_eq!(flags, [true, false, true, false, true]);
    }
}
