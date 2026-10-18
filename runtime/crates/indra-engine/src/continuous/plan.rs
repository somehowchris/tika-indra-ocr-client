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

/// Preserve complete reference groups and their exact padded width. The pinned
/// models are not invariant to extra right padding: width-compatible rows alone
/// are insufficient for numerical parity. A peer contributes its entire next
/// group only when the padded width and combined row/work budgets agree.
/// Cursors remain transactional until inference and scattering succeed.
struct Group {
    count: usize,
    end: usize,
    padded: usize,
    widest: usize,
}

fn group(view: &View<'_>, cap: usize) -> Result<Group> {
    let mut result = Group {
        count: 0,
        end: view.position,
        padded: 0,
        widest: 0,
    };
    let mut first = 0;
    while result.count < cap {
        let Some((_, width)) = view.candidate(&mut result.end)? else {
            break;
        };
        if result.count == 0 {
            first = width.max(64);
        } else if width.max(64) as f64 > 1.3 * first as f64
            || (result.count + 1) * width.max(64) > 64 * 640
        {
            break;
        }
        result.count += 1;
        result.widest = result.widest.max(width);
        result.end += 1;
    }
    result.padded = result.widest.div_ceil(8) * 8;
    Ok(result)
}

fn append(view: &View<'_>, owner: usize, group: &Group, batch: &mut Batch) -> Result<()> {
    let mut position = view.position;
    for _ in 0..group.count {
        let (line, width) = view
            .candidate(&mut position)?
            .ok_or(EngineError::Shape("continuous reference group changed"))?;
        batch.rows[batch.count] = Row { owner, line, width };
        batch.count += 1;
        batch.owners[owner] += 1;
        position += 1;
    }
    batch.end[owner] = group.end;
    Ok(())
}

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
    let view = views[anchor]
        .as_ref()
        .ok_or(EngineError::Shape("continuous anchor"))?;
    let primary = group(view, cap)?;
    if primary.count == 0 {
        return Err(EngineError::Shape("continuous anchor has no ready rows"));
    }
    append(view, anchor, &primary, batch)?;
    let peer = 1 - anchor;
    if let Some(view) = &views[peer] {
        let candidate = group(view, cap)?;
        let count = primary.count + candidate.count;
        if candidate.count != 0
            && candidate.padded == primary.padded
            && count <= cap
            && count * primary.padded <= crate::batching::TEXT_COLUMNS
            && count * primary.widest.max(candidate.widest).max(64) <= 64 * 640
        {
            append(view, peer, &candidate, batch)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatible_rows_retain_owner_and_original_line() {
        let widths = [[83, 64, 84], [70, 83, 800]];
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
        let expected = [(0, 1, 64), (0, 0, 83), (1, 0, 70), (1, 1, 83)];
        for (row, expected) in batch.rows[..batch.count].iter().zip(expected) {
            assert_eq!((row.owner, row.line, row.width), expected);
        }
    }

    #[test]
    fn different_canonical_padding_never_shares_a_model_call() {
        let widths = [[64, 83, 84], [70, 80, 800]];
        let indices = [0, 1, 2];
        let flags = [false; 3];
        let mut batch = Batch::new();
        let views = std::array::from_fn(|owner| {
            Some(View {
                widths: &widths[owner],
                indices: &indices,
                flags: &flags,
                route: None,
                position: 0,
            })
        });
        select(views, 0, 64, &mut batch).expect("canonical anchor group");
        assert_eq!(batch.owners, [2, 0]);
        assert_eq!(batch.end, [2, 0]);
        assert_eq!(batch.rows[0].width, 64);
        assert_eq!(batch.rows[1].width, 83);
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

    struct ReferenceGroups {
        indices: [usize; MAX_LINES],
        bounds: [[usize; 2]; MAX_LINES],
        count: usize,
    }
    fn reference(
        widths: &[usize],
        flags: &[bool],
        route: Option<bool>,
        cap: usize,
    ) -> ReferenceGroups {
        let mut p = crate::batching::Plan::new(widths, cap, crate::batching::Policy::Text, |i| {
            route.is_none_or(|r| r == flags[i])
        })
        .unwrap();
        let mut r = ReferenceGroups {
            indices: [0; MAX_LINES],
            bounds: [[0; 2]; MAX_LINES],
            count: 0,
        };
        while let Some(range) = p.next(widths) {
            r.bounds[r.count] = [range.start, range.end];
            r.count += 1;
        }
        r.indices = p.indices;
        r
    }
    #[test]
    fn both_owner_drains_preserve_complete_normal_groups_and_padding() {
        let boundary = [8, 63, 64, 65, 83, 84, 649, 657, 664, 4096];
        let sizes = [1, 2, 3, 8, 16, 63, 64, 65, 128, 511, 1024];
        let mut merges = 0usize;
        let mut groups = 0usize;
        for seed in 0..128usize {
            let all_widths: [[_; MAX_LINES]; 2] = std::array::from_fn(|owner| {
                std::array::from_fn(|i| {
                    if seed % 4 == 0 {
                        boundary[(i + owner * 7 + seed) % boundary.len()]
                    } else if seed % 4 == 1 {
                        8 + ((i * 73 + seed * 11 + owner * 37) % 4089)
                    } else if seed % 4 == 2 {
                        64 + (seed % 3) * 8
                    } else {
                        648 + ((i + owner) % 3) * 8
                    }
                })
            });
            let all_flags: [[_; MAX_LINES]; 2] =
                std::array::from_fn(|owner| std::array::from_fn(|i| (i + owner + seed) % 3 == 0));
            let lengths = [
                sizes[seed % sizes.len()],
                sizes[(seed * 3 + 1) % sizes.len()],
            ];
            for route in [None, Some(false), Some(true)] {
                for cap in [1, 2, 3, 8, 16, 32, 64] {
                    let refs: [ReferenceGroups; 2] = std::array::from_fn(|o| {
                        reference(
                            &all_widths[o][..lengths[o]],
                            &all_flags[o][..lengths[o]],
                            route,
                            cap,
                        )
                    });
                    let mut sorted: [[_; MAX_LINES]; 2] =
                        std::array::from_fn(|_| std::array::from_fn(|i| i));
                    for o in 0..2 {
                        sorted[o][..lengths[o]].sort_unstable_by_key(|i| (all_widths[o][*i], *i));
                    }
                    let mut cursors = [0usize; 2];
                    let mut next = [0usize; 2];
                    let mut preferred = seed % 2;
                    let mut seen = [[false; MAX_LINES]; 2];
                    while next[0] < refs[0].count || next[1] < refs[1].count {
                        let anchor = if next[preferred] < refs[preferred].count {
                            preferred
                        } else {
                            1 - preferred
                        };
                        preferred = 1 - anchor;
                        let views = std::array::from_fn(|o| {
                            if next[o] < refs[o].count {
                                Some(View {
                                    widths: &all_widths[o][..lengths[o]],
                                    indices: &sorted[o][..lengths[o]],
                                    flags: &all_flags[o][..lengths[o]],
                                    route,
                                    position: cursors[o],
                                })
                            } else {
                                None
                            }
                        });
                        let mut batch = Batch::new();
                        select(views, anchor, cap, &mut batch).unwrap();
                        assert!(batch.owners[anchor] > 0);
                        assert!(batch.count <= cap);
                        merges += usize::from(batch.owners[0] > 0 && batch.owners[1] > 0);
                        let actual_padding = batch.rows[..batch.count]
                            .iter()
                            .map(|row| row.width)
                            .max()
                            .unwrap()
                            .div_ceil(8)
                            * 8;
                        for o in 0..2 {
                            if batch.owners[o] == 0 {
                                continue;
                            }
                            let [start, end] = refs[o].bounds[next[o]];
                            let expected = &refs[o].indices[start..end];
                            let mut found = 0usize;
                            for row in batch.rows[..batch.count]
                                .iter()
                                .filter(|row| row.owner == o)
                            {
                                assert_eq!(
                                    row.line, expected[found],
                                    "seed={seed} route={route:?} cap={cap} owner={o}"
                                );
                                assert!(!seen[o][row.line]);
                                seen[o][row.line] = true;
                                found += 1;
                            }
                            assert_eq!(found, expected.len());
                            let canonical = expected
                                .iter()
                                .map(|i| all_widths[o][*i])
                                .max()
                                .unwrap()
                                .div_ceil(8)
                                * 8;
                            assert_eq!(
                                actual_padding, canonical,
                                "original owner padding must survive sharing"
                            );
                            cursors[o] = batch.end[o];
                            next[o] += 1;
                            groups += 1;
                        }
                    }
                    for o in 0..2 {
                        for i in 0..lengths[o] {
                            assert_eq!(seen[o][i], route.is_none_or(|r| r == all_flags[o][i]));
                        }
                    }
                }
            }
        }
        println!(
            "independent seeds=128 caps=7 routes=3 owner_groups={groups} actual_two_owner_merges={merges}"
        );
        assert!(merges > 0);
    }
}
