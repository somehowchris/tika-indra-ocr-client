//! Reading-order features and decoding for the pinned Indra 1.1 models.
//!
//! Port of `indra_ocr/order_post.py` at
//! `0e96b93c6810f17cf8352acbb4487cc9e45b66d5` (Apache-2.0).
//! Scratch matrices are allocated once at worker startup. Per-page work uses
//! bounded slices and small fixed arrays. Equal sort keys use the original line
//! index, because NumPy's default unstable argsort does not promise tie order.

use std::time::Instant;

use opencv::{
    core::{self, Mat, Scalar},
    prelude::MatTraitManual,
};

use crate::{
    EngineError, MAX_LINES, Result,
    unicode_tables::{DECIMAL, EAST_ASIAN_WIDE, UPPER, contains},
};

pub(crate) const N_FEATURES: usize = 16;
const N_CLASSES: usize = 10;
const ORDER_SWEEPS: usize = 4;

/// Three fixed-capacity native allocations, reused across every page.
///
/// Two f64 matrices retain the reference's clustering sums and means; the f32
/// matrix is reused for block logits and then log-probabilities. This is 20 MiB
/// at the 1,024-line limit. Large Rust arrays are never constructed on the stack.
pub(crate) struct Workspace {
    sums: Mat,
    means: Mat,
    logits: Mat,
}

impl Workspace {
    pub(crate) fn new() -> Result<Self> {
        let side = i32::try_from(MAX_LINES)
            .map_err(|_| EngineError::Config("reading-order capacity exceeds OpenCV dimensions"))?;
        Ok(Self {
            sums: Mat::new_rows_cols_with_default(side, side, core::CV_64FC1, Scalar::all(0.0))?,
            means: Mat::new_rows_cols_with_default(side, side, core::CV_64FC1, Scalar::all(0.0))?,
            logits: Mat::new_rows_cols_with_default(side, side, core::CV_32FC1, Scalar::all(0.0))?,
        })
    }
}

/// Populate the row-major `[N, 16]` order-model features.
pub(crate) fn features(
    boxes: &[[f32; 4]],
    texts: &[&str],
    wh: [f32; 2],
    out: &mut [f32],
) -> Result<()> {
    validate_boxes(boxes)?;
    if texts.len() != boxes.len()
        || out.len() < boxes.len() * N_FEATURES
        || wh.iter().any(|v| !v.is_finite() || *v <= 0.0)
    {
        return Err(EngineError::Shape("reading-order feature inputs"));
    }
    if boxes.is_empty() {
        return Ok(());
    }
    let mut heights = [0.0; MAX_LINES];
    for (slot, b) in heights.iter_mut().zip(boxes) {
        *slot = (b[3] - b[1]).max(1e-3);
    }
    let median_height = median(&mut heights[..boxes.len()]);
    for ((b, text), row) in boxes
        .iter()
        .zip(texts)
        .zip(out.as_chunks_mut::<N_FEATURES>().0)
    {
        let width = (b[2] - b[0]).max(1e-3);
        let height = (b[3] - b[1]).max(1e-3);
        row[0] = b[0] / wh[0];
        row[1] = b[1] / wh[1];
        row[2] = b[2] / wh[0];
        row[3] = b[3] / wh[1];
        row[4] = (b[0] + b[2]) / (2.0 * wh[0]);
        row[5] = (b[1] + b[3]) / (2.0 * wh[1]);
        row[6] = width / wh[0];
        row[7] = (height / median_height).ln();
        row[8] = (width / height).ln() / 4.0;
        // Upstream np.full receives a Python float for this one component.
        row[9] = (f64::from(median_height) / f64::from(wh[1]) * 20.0) as f32;
        text_features(text, &mut row[10..]);
    }
    Ok(())
}

/// Decode layout blocks, then order their lines and the blocks themselves.
///
/// Empty `same` selects the pairwise-only model; empty `classes` selects XY-cut
/// for every block. `out[..boxes.len()]` receives the complete permutation.
pub(crate) fn decode(
    pairwise: &[f32],
    same: &[f32],
    classes: &[f32],
    boxes: &[[f32; 4]],
    workspace: &mut Workspace,
    out: &mut [usize],
    deadline: Instant,
) -> Result<()> {
    decode_with_scratch(
        pairwise,
        same,
        classes,
        boxes,
        Scratch {
            sums: workspace.sums.data_typed_mut::<f64>()?,
            means: workspace.means.data_typed_mut::<f64>()?,
            logits: workspace.logits.data_typed_mut::<f32>()?,
        },
        out,
        deadline,
    )
}

// The pure core accepts bounded slices so algorithm checks do not require ONNX.
struct Scratch<'a> {
    sums: &'a mut [f64],
    means: &'a mut [f64],
    logits: &'a mut [f32],
}

fn decode_with_scratch(
    pairwise: &[f32],
    same: &[f32],
    classes: &[f32],
    boxes: &[[f32; 4]],
    scratch: Scratch<'_>,
    out: &mut [usize],
    deadline: Instant,
) -> Result<()> {
    check_deadline(deadline)?;
    validate_boxes(boxes)?;
    let n = boxes.len();
    if pairwise.len() != n * n
        || (!same.is_empty() && same.len() != n * n)
        || (!classes.is_empty() && classes.len() != n * N_CLASSES)
        || out.len() < n
        || scratch.sums.len() < n * n
        || scratch.means.len() < n * n
        || scratch.logits.len() < n * n
    {
        return Err(EngineError::Shape("reading-order tensors or workspace"));
    }
    if pairwise
        .iter()
        .chain(same)
        .chain(classes)
        .any(|v| !v.is_finite())
    {
        return Err(EngineError::Shape("non-finite reading-order logits"));
    }
    if n <= 1 {
        if n == 1 {
            out[0] = 0;
        }
        return Ok(());
    }
    if same.is_empty() {
        for (dst, src) in scratch.logits.iter_mut().zip(pairwise) {
            *dst = log_sigmoid(*src);
        }
        return decode_log_probabilities(&scratch.logits[..n * n], &mut out[..n], deadline);
    }

    let mut owner = [0; MAX_LINES];
    let mut sizes = [0; MAX_LINES];
    cluster(
        same,
        &mut scratch.sums[..n * n],
        &mut scratch.means[..n * n],
        &mut owner[..n],
        &mut sizes[..n],
        deadline,
    )?;

    // Clusters are emitted by ascending surviving root, and each member list
    // starts in input order, matching sorted(members[root]) in the reference.
    let mut roots = [0; MAX_LINES];
    let mut starts = [0; MAX_LINES];
    let mut members = [0; MAX_LINES];
    let mut inner = [0; MAX_LINES];
    let mut block_count = 0;
    let mut written = 0;
    for (root, &size) in sizes[..n].iter().enumerate() {
        if size == 0 {
            continue;
        }
        check_deadline(deadline)?;
        roots[block_count] = root;
        starts[block_count] = written;
        for (line, &group) in owner[..n].iter().enumerate() {
            if group == root {
                members[written] = line;
                inner[written] = line;
                written += 1;
            }
        }
        let ids = &mut inner[starts[block_count]..written];
        if is_grid(classes, ids) {
            top_down(boxes, ids);
        } else {
            xycut(boxes, ids, deadline)?;
        }
        block_count += 1;
    }

    // G @ A @ G.T in f64, taking only the upper triangle, then making the
    // result antisymmetric. Multiplication follows the reference's two stages.
    // BLAS reduction order may differ; stage parity checks retain tolerances.
    let mut weighted_row = [0.0_f64; MAX_LINES];
    let ab = &mut scratch.logits[..block_count * block_count];
    ab.fill(0.0);
    for p in 0..block_count {
        check_deadline(deadline)?;
        weighted_row[..n].fill(0.0);
        let p_weight = 1.0 / sizes[roots[p]] as f64;
        for &i in &members[starts[p]..starts[p] + sizes[roots[p]]] {
            for (accumulator, &value) in weighted_row[..n]
                .iter_mut()
                .zip(&pairwise[i * n..(i + 1) * n])
            {
                *accumulator += p_weight * f64::from(value);
            }
        }
        for q in p + 1..block_count {
            let q_weight = 1.0 / sizes[roots[q]] as f64;
            let mut mean = 0.0_f64;
            for &j in &members[starts[q]..starts[q] + sizes[roots[q]]] {
                mean += weighted_row[j] * q_weight;
            }
            ab[p * block_count + q] = mean as f32;
            ab[q * block_count + p] = -mean as f32;
        }
    }
    for value in ab.iter_mut() {
        *value = log_sigmoid(*value);
    }
    let mut block_order = [0; MAX_LINES];
    decode_log_probabilities(ab, &mut block_order[..block_count], deadline)?;
    written = 0;
    for &block in &block_order[..block_count] {
        let count = sizes[roots[block]];
        // Explicitly gather the final permutation; only small integer indexes
        // move here, never image pixels or model tensors.
        out[written..written + count].copy_from_slice(&inner[starts[block]..starts[block] + count]);
        written += count;
    }
    Ok(())
}

fn cluster(
    same: &[f32],
    sums: &mut [f64],
    means: &mut [f64],
    owner: &mut [usize],
    sizes: &mut [usize],
    deadline: Instant,
) -> Result<()> {
    let n = owner.len();
    let mut best = [0; MAX_LINES];
    let mut best_value = [f64::NEG_INFINITY; MAX_LINES];
    for i in 0..n {
        owner[i] = i;
        sizes[i] = 1;
        for j in 0..n {
            sums[i * n + j] = if i == j {
                0.0
            } else {
                f64::from(same[i * n + j])
            };
            means[i * n + j] = if i == j {
                f64::NEG_INFINITY
            } else {
                sums[i * n + j]
            };
        }
        best[i] = argmax(&means[i * n..(i + 1) * n]);
        best_value[i] = means[i * n + best[i]];
    }
    for _ in 1..n {
        check_deadline(deadline)?;
        let selected = argmax(&best_value[..n]);
        if best_value[selected] <= 0.0 {
            break;
        }
        let partner = best[selected];
        let a = selected.min(partner);
        let b = selected.max(partner);
        for j in 0..n {
            sums[a * n + j] += sums[b * n + j];
        }
        for i in 0..n {
            sums[i * n + a] += sums[i * n + b];
        }
        sums[a * n + a] = 0.0;
        sizes[a] += sizes[b];
        sizes[b] = 0;
        for group in owner.iter_mut() {
            if *group == b {
                *group = a;
            }
        }
        for j in 0..n {
            let value = if j == a || sizes[j] == 0 {
                f64::NEG_INFINITY
            } else {
                sums[a * n + j] / (sizes[a] as f64 * sizes[j] as f64)
            };
            means[a * n + j] = value;
            means[j * n + a] = value;
            means[b * n + j] = f64::NEG_INFINITY;
            means[j * n + b] = f64::NEG_INFINITY;
        }
        best_value[b] = f64::NEG_INFINITY;
        best[a] = argmax(&means[a * n..(a + 1) * n]);
        best_value[a] = means[a * n + best[a]];
        for j in 0..n {
            if sizes[j] != 0 && j != a && (best[j] == a || best[j] == b) {
                best[j] = argmax(&means[j * n..(j + 1) * n]);
                best_value[j] = means[j * n + best[j]];
            }
        }
        for j in 0..n {
            let value = means[a * n + j];
            if sizes[j] != 0 && j != a && value > best_value[j] {
                best[j] = a;
                best_value[j] = value;
            }
        }
    }
    Ok(())
}

fn decode_log_probabilities(lp: &[f32], out: &mut [usize], deadline: Instant) -> Result<()> {
    let n = out.len();
    let mut scores = [0.0; MAX_LINES];
    let mut snapshot = [0; MAX_LINES];
    let mut rest = [0; MAX_LINES];
    let mut suffix = [0.0; MAX_LINES];
    let mut gains = [0.0_f64; MAX_LINES];
    for i in 0..n {
        let row_sum = sum32(&lp[i * n..(i + 1) * n]);
        let mut column_sum = 0.0_f32;
        for j in 0..n {
            column_sum += lp[j * n + i];
        }
        scores[i] = -row_sum + column_sum;
        out[i] = i;
    }
    out.sort_unstable_by(|&a, &b| scores[a].total_cmp(&scores[b]).then(a.cmp(&b)));
    if n <= 1 {
        return Ok(());
    }
    for _ in 0..ORDER_SWEEPS {
        let mut improved = false;
        // One sweep visits the order at its start, even when insertion moves
        // change the current order. The reference explicitly copies this list.
        snapshot[..n].copy_from_slice(out);
        for &x in &snapshot[..n] {
            check_deadline(deadline)?;
            let mut count = 0;
            let mut current = 0;
            for (position, &line) in out.iter().enumerate() {
                if line == x {
                    current = position;
                } else {
                    rest[count] = line;
                    count += 1;
                }
            }
            let mut sum = 0.0_f32;
            suffix[n - 1] = 0.0;
            for position in (0..n - 1).rev() {
                sum += lp[x * n + rest[position]];
                suffix[position] = sum;
            }
            let mut prefix = 0.0_f32;
            for position in 0..n {
                // NumPy concatenates its f32 cumsums with Python [0.0],
                // promoting each final gain addition to f64.
                gains[position] = f64::from(prefix) + f64::from(suffix[position]);
                if position < n - 1 {
                    prefix += lp[rest[position] * n + x];
                }
            }
            let insert_at = argmax(&gains[..n]);
            if gains[insert_at] > gains[current] + 1e-6 {
                out[..insert_at].copy_from_slice(&rest[..insert_at]);
                out[insert_at] = x;
                out[insert_at + 1..].copy_from_slice(&rest[insert_at..n - 1]);
                improved = true;
            }
        }
        if !improved {
            break;
        }
    }
    Ok(())
}

fn is_grid(classes: &[f32], ids: &[usize]) -> bool {
    if classes.is_empty() {
        return false;
    }
    let mut totals = [0.0_f32; N_CLASSES];
    for &line in ids {
        for (total, &value) in totals
            .iter_mut()
            .zip(&classes[line * N_CLASSES..(line + 1) * N_CLASSES])
        {
            *total += value;
        }
    }
    let mut best = 0;
    for (index, value) in totals.iter().enumerate().skip(1) {
        if *value > totals[best] {
            best = index;
        }
    }
    matches!(best, 5 | 8 | 9)
}

fn top_down(boxes: &[[f32; 4]], ids: &mut [usize]) {
    if ids.is_empty() {
        return;
    }
    let mut heights = [0.0; MAX_LINES];
    for (slot, &id) in heights.iter_mut().zip(ids.iter()) {
        *slot = boxes[id][3] - boxes[id][1];
    }
    let row_height = (median(&mut heights[..ids.len()]) * 0.5).max(1e-3);
    // Python's sorted is stable: equal (row, x0) keys keep the incoming order,
    // which is the parent's cut-axis order inside an XY-cut sub-range.
    ids.sort_by(|&a, &b| {
        let row_a = (((boxes[a][1] + boxes[a][3]) / 2.0) / row_height).round_ties_even();
        let row_b = (((boxes[b][1] + boxes[b][3]) / 2.0) / row_height).round_ties_even();
        row_a
            .total_cmp(&row_b)
            .then(boxes[a][0].total_cmp(&boxes[b][0]))
    });
}

fn xycut(boxes: &[[f32; 4]], ids: &mut [usize], deadline: Instant) -> Result<()> {
    // An explicit bounded work stack avoids recursion depth depending on input.
    let mut pending = [(0_usize, 0_usize); MAX_LINES];
    let mut horizontal = [0; MAX_LINES];
    let mut vertical = [0; MAX_LINES];
    let mut count = 1;
    pending[0] = (0, ids.len());
    while count != 0 {
        check_deadline(deadline)?;
        count -= 1;
        let (start, end) = pending[count];
        let len = end - start;
        if len <= 1 {
            continue;
        }
        horizontal[..len].copy_from_slice(&ids[start..end]);
        vertical[..len].copy_from_slice(&ids[start..end]);
        let (gap_y, cut_y) = best_cut(boxes, &mut horizontal[..len], 1);
        let (gap_x, cut_x) = best_cut(boxes, &mut vertical[..len], 0);
        if cut_y == 0 && cut_x == 0 {
            top_down(boxes, &mut ids[start..end]);
            continue;
        }
        let cut = if gap_y >= gap_x {
            ids[start..end].copy_from_slice(&horizontal[..len]);
            cut_y
        } else {
            ids[start..end].copy_from_slice(&vertical[..len]);
            cut_x
        };
        pending[count] = (start + cut, end);
        pending[count + 1] = (start, start + cut);
        count += 2;
    }
    Ok(())
}

/// `xycut_blocks` flattened: reading order for pages the order model does not
/// know (vertical pages, read turned). A cut needs a whitespace gap of at least
/// one median line height; the widest gap cuts first, the upper or left part
/// first. Lines inside a final block go by `order_xycut`.
pub(crate) fn xycut_blocks(boxes: &[[f32; 4]], ids: &mut [usize], deadline: Instant) -> Result<()> {
    validate_boxes(boxes)?;
    if ids.is_empty() {
        return Ok(());
    }
    let mut heights = [0.0; MAX_LINES];
    for (slot, b) in heights.iter_mut().zip(boxes) {
        *slot = b[3] - b[1];
    }
    let min_gap = median(&mut heights[..boxes.len()]);
    let mut pending = [(0_usize, 0_usize); MAX_LINES];
    let mut horizontal = [0; MAX_LINES];
    let mut vertical = [0; MAX_LINES];
    let mut count = 1;
    pending[0] = (0, ids.len());
    while count != 0 {
        check_deadline(deadline)?;
        count -= 1;
        let (start, end) = pending[count];
        let len = end - start;
        if len > 1 {
            horizontal[..len].copy_from_slice(&ids[start..end]);
            vertical[..len].copy_from_slice(&ids[start..end]);
            let (gap_y, cut_y) = best_cut(boxes, &mut horizontal[..len], 1);
            let (gap_x, cut_x) = best_cut(boxes, &mut vertical[..len], 0);
            let (gap, cut, sorted) = if gap_y >= gap_x {
                (gap_y, cut_y, &horizontal[..len])
            } else {
                (gap_x, cut_x, &vertical[..len])
            };
            if cut != 0 && gap >= min_gap {
                ids[start..end].copy_from_slice(sorted);
                pending[count] = (start + cut, end);
                pending[count + 1] = (start, start + cut);
                count += 2;
                continue;
            }
        }
        xycut(boxes, &mut ids[start..end], deadline)?;
    }
    Ok(())
}

fn best_cut(boxes: &[[f32; 4]], ids: &mut [usize], axis: usize) -> (f32, usize) {
    ids.sort_unstable_by(|&a, &b| boxes[a][axis].total_cmp(&boxes[b][axis]).then(a.cmp(&b)));
    let mut end = boxes[ids[0]][axis + 2];
    let mut best_gap = 0.0;
    let mut best_position = 0;
    for (position, &id) in ids.iter().enumerate().skip(1) {
        let gap = boxes[id][axis] - end;
        if gap > best_gap {
            best_gap = gap;
            best_position = position;
        }
        end = end.max(boxes[id][axis + 2]);
    }
    (best_gap, best_position)
}

fn median(values: &mut [f32]) -> f32 {
    values.sort_unstable_by(f32::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

fn argmax(values: &[f64]) -> usize {
    let mut best = 0;
    for (index, &value) in values.iter().enumerate().skip(1) {
        if value > values[best] {
            best = index;
        }
    }
    best
}

fn log_sigmoid(value: f32) -> f32 {
    if value >= 0.0 {
        -(-value).exp().ln_1p()
    } else {
        value - value.exp().ln_1p()
    }
}

/// NumPy's contiguous float64 reduction: the same pairwise scheme as `sum32`.
pub(crate) fn sum64(values: &[f64]) -> f64 {
    0.0 + pairwise_sum64(values)
}

fn pairwise_sum64(values: &[f64]) -> f64 {
    if values.len() < 8 {
        let mut sum = -0.0_f64;
        for value in values {
            sum += value;
        }
        return sum;
    }
    if values.len() <= 128 {
        let mut accumulators = [0.0; 8];
        accumulators.copy_from_slice(&values[..8]);
        let aligned = values.len() - values.len() % 8;
        for chunk in values[8..aligned].as_chunks::<8>().0 {
            for (accumulator, value) in accumulators.iter_mut().zip(chunk) {
                *accumulator += value;
            }
        }
        let mut sum = ((accumulators[0] + accumulators[1]) + (accumulators[2] + accumulators[3]))
            + ((accumulators[4] + accumulators[5]) + (accumulators[6] + accumulators[7]));
        for value in &values[aligned..] {
            sum += value;
        }
        return sum;
    }
    let middle = values.len() / 2 / 8 * 8;
    pairwise_sum64(&values[..middle]) + pairwise_sum64(&values[middle..])
}

/// NumPy's contiguous float32 reduction uses eight accumulators below its
/// 128-element block cutoff, then recursively splits on eight-element borders.
pub(crate) fn sum32(values: &[f32]) -> f32 {
    // np.sum adds its reduction identity after the inner pairwise loop. This
    // makes both the empty sum and a sum of negative zero return positive zero.
    0.0 + pairwise_sum(values)
}

fn pairwise_sum(values: &[f32]) -> f32 {
    if values.len() < 8 {
        let mut sum = -0.0_f32;
        for value in values {
            sum += value;
        }
        return sum;
    }
    if values.len() <= 128 {
        let mut accumulators = [0.0; 8];
        accumulators.copy_from_slice(&values[..8]);
        let aligned = values.len() - values.len() % 8;
        for chunk in values[8..aligned].as_chunks::<8>().0 {
            for (accumulator, value) in accumulators.iter_mut().zip(chunk) {
                *accumulator += value;
            }
        }
        let mut sum = ((accumulators[0] + accumulators[1]) + (accumulators[2] + accumulators[3]))
            + ((accumulators[4] + accumulators[5]) + (accumulators[6] + accumulators[7]));
        for value in &values[aligned..] {
            sum += value;
        }
        return sum;
    }
    let middle = values.len() / 2 / 8 * 8;
    pairwise_sum(&values[..middle]) + pairwise_sum(&values[middle..])
}

/// `display_len`: characters, with East Asian wide ones counted twice.
fn display_len(text: &str) -> usize {
    text.chars()
        .map(|c| if contains(&EAST_ASIAN_WIDE, c) { 2 } else { 1 })
        .sum()
}

fn text_features(text: &str, out: &mut [f32]) {
    out.fill(0.0);
    let text = text.trim_matches(python_space);
    let mut chars = text.chars();
    let Some(first) = chars.next() else { return };
    let length = text.chars().count();
    let digits = text.chars().filter(|c| python_digit(*c)).count();
    let mut prefix = ['\0'; 3];
    let mut prefix_len = 0;
    for c in text.chars().take(3) {
        prefix[prefix_len] = c;
        prefix_len += 1;
    }
    while prefix_len != 0 && matches!(prefix[prefix_len - 1], '.' | ')') {
        prefix_len -= 1;
    }
    let number = prefix_len != 0 && prefix[..prefix_len].iter().all(|c| python_digit(*c));
    let bullet = matches!(first, '•' | '-' | '–' | '▪' | '◦' | '*' | '·' | '→');
    let last = text.chars().next_back().unwrap_or(first);
    out[0] = ((display_len(text) as f64).ln_1p() / 5.0) as f32;
    out[1] = (digits as f64 / length as f64) as f32;
    out[2] = u8::from(bullet || (number && length > 3)) as f32;
    out[3] = u8::from(last == '-') as f32;
    out[4] = u8::from(SENTENCE_END.contains(last)) as f32;
    out[5] = u8::from(contains(&UPPER, first)) as f32;
}

const SENTENCE_END: &str = ".!?:;。！？：；．";

/// Python `\d` in a str pattern: one decimal digit of any script.
pub(crate) fn python_decimal(c: char) -> bool {
    contains(&DECIMAL, c)
}

fn python_space(c: char) -> bool {
    // CPython also strips the four ASCII information separators; Unicode's
    // White_Space property, used by Rust, excludes these.
    c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}')
}

fn python_digit(c: char) -> bool {
    let code = u32::from(c);
    let index = DIGIT_RANGES.partition_point(|&(_, last)| last < code);
    DIGIT_RANGES
        .get(index)
        .is_some_and(|&(first, _)| first <= code)
}

// Numeric_Type=Decimal or Digit, Unicode 15.1 (CPython 3.13's database).
// A numeric fraction or a Han numeral is not a Python str.isdigit() character.
// Data: https://www.unicode.org/Public/15.1.0/ucd/extracted/DerivedNumericType.txt
static DIGIT_RANGES: [(u32, u32); 83] = [
    (0x30, 0x39),
    (0xB2, 0xB3),
    (0xB9, 0xB9),
    (0x660, 0x669),
    (0x6F0, 0x6F9),
    (0x7C0, 0x7C9),
    (0x966, 0x96F),
    (0x9E6, 0x9EF),
    (0xA66, 0xA6F),
    (0xAE6, 0xAEF),
    (0xB66, 0xB6F),
    (0xBE6, 0xBEF),
    (0xC66, 0xC6F),
    (0xCE6, 0xCEF),
    (0xD66, 0xD6F),
    (0xDE6, 0xDEF),
    (0xE50, 0xE59),
    (0xED0, 0xED9),
    (0xF20, 0xF29),
    (0x1040, 0x1049),
    (0x1090, 0x1099),
    (0x1369, 0x1371),
    (0x17E0, 0x17E9),
    (0x1810, 0x1819),
    (0x1946, 0x194F),
    (0x19D0, 0x19DA),
    (0x1A80, 0x1A89),
    (0x1A90, 0x1A99),
    (0x1B50, 0x1B59),
    (0x1BB0, 0x1BB9),
    (0x1C40, 0x1C49),
    (0x1C50, 0x1C59),
    (0x2070, 0x2070),
    (0x2074, 0x2079),
    (0x2080, 0x2089),
    (0x2460, 0x2468),
    (0x2474, 0x247C),
    (0x2488, 0x2490),
    (0x24EA, 0x24EA),
    (0x24F5, 0x24FD),
    (0x24FF, 0x24FF),
    (0x2776, 0x277E),
    (0x2780, 0x2788),
    (0x278A, 0x2792),
    (0xA620, 0xA629),
    (0xA8D0, 0xA8D9),
    (0xA900, 0xA909),
    (0xA9D0, 0xA9D9),
    (0xA9F0, 0xA9F9),
    (0xAA50, 0xAA59),
    (0xABF0, 0xABF9),
    (0xFF10, 0xFF19),
    (0x104A0, 0x104A9),
    (0x10A40, 0x10A43),
    (0x10D30, 0x10D39),
    (0x10E60, 0x10E68),
    (0x11052, 0x1105A),
    (0x11066, 0x1106F),
    (0x110F0, 0x110F9),
    (0x11136, 0x1113F),
    (0x111D0, 0x111D9),
    (0x112F0, 0x112F9),
    (0x11450, 0x11459),
    (0x114D0, 0x114D9),
    (0x11650, 0x11659),
    (0x116C0, 0x116C9),
    (0x11730, 0x11739),
    (0x118E0, 0x118E9),
    (0x11950, 0x11959),
    (0x11C50, 0x11C59),
    (0x11D50, 0x11D59),
    (0x11DA0, 0x11DA9),
    (0x11F50, 0x11F59),
    (0x16A60, 0x16A69),
    (0x16AC0, 0x16AC9),
    (0x16B50, 0x16B59),
    (0x1D7CE, 0x1D7FF),
    (0x1E140, 0x1E149),
    (0x1E2F0, 0x1E2F9),
    (0x1E4F0, 0x1E4F9),
    (0x1E950, 0x1E959),
    (0x1F100, 0x1F10A),
    (0x1FBF0, 0x1FBF9),
];

fn validate_boxes(boxes: &[[f32; 4]]) -> Result<()> {
    if boxes.len() > MAX_LINES {
        return Err(EngineError::Limit("too many lines for reading order"));
    }
    if boxes.iter().flatten().any(|value| !value.is_finite()) {
        return Err(EngineError::Shape("non-finite reading-order box"));
    }
    Ok(())
}

fn check_deadline(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err(EngineError::Deadline)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(30)
    }

    #[test]
    fn unicode_text_features_count_characters_and_only_digits() {
        let mut out = [0.0; 6];
        text_features("\u{1c} É৯²½中- \u{1f}", &mut out);
        // Six characters; the wide Han character counts twice: display length 7.
        assert!((out[0] - (8.0_f64.ln() / 5.0) as f32).abs() < 1e-7);
        assert_eq!(out[1], (2.0 / 6.0) as f32);
        assert_eq!(&out[2..], &[0.0, 1.0, 0.0, 1.0]);
        text_features("१२. texte", &mut out);
        assert_eq!(out[2], 1.0);
        text_features("½.) texte", &mut out);
        assert_eq!(out[2], 0.0);
    }

    #[test]
    fn sentence_ends_and_decimals_follow_the_reference_sets() {
        let mut out = [0.0; 6];
        text_features("終わり。", &mut out);
        assert_eq!(out[4], 1.0);
        assert!((out[0] - (9.0_f64.ln() / 5.0) as f32).abs() < 1e-7);
        text_features("Ende", &mut out);
        assert_eq!((out[4], out[5]), (0.0, 1.0));
        assert!(python_decimal('٣') && python_decimal('7') && !python_decimal('²'));
    }

    #[test]
    fn xycut_blocks_cuts_only_at_wide_gaps_and_orders_the_rest_by_xycut() {
        // Two columns of short lines, a wide gap between them (one median height
        // is 5): both columns are blocks, each read top down.
        let boxes = [
            [0.0, 0.0, 10.0, 5.0],
            [100.0, 0.0, 110.0, 5.0],
            [0.0, 7.0, 10.0, 12.0],
            [100.0, 7.0, 110.0, 12.0],
        ];
        let mut ids = [3, 2, 1, 0];
        xycut_blocks(&boxes, &mut ids, deadline()).unwrap();
        assert_eq!(ids, [0, 2, 1, 3]);
        // A narrow page gap below the median height makes one block.
        let boxes = [
            [0.0, 0.0, 10.0, 5.0],
            [12.0, 0.0, 22.0, 5.0],
            [0.0, 7.0, 22.0, 12.0],
        ];
        let mut ids = [2, 1, 0];
        xycut_blocks(&boxes, &mut ids, deadline()).unwrap();
        assert_eq!(ids, [0, 1, 2]);
    }

    #[test]
    fn feature_shapes_and_blank_text_are_explicit() {
        let boxes = [[10.0, 20.0, 110.0, 40.0], [10.0, 80.0, 210.0, 120.0]];
        let mut out = [99.0; 32];
        features(&boxes, &["", "Été."], [400.0, 600.0], &mut out).unwrap();
        assert_eq!(&out[10..16], &[0.0; 6]);
        assert_eq!(out[9], 1.0);
        assert_eq!(out[25], 1.0);
        assert_eq!(out[30], 1.0);
        assert_eq!(out[31], 1.0);
        assert!(features(&boxes, &[""], [400.0, 600.0], &mut out).is_err());
        assert!(features(&boxes, &["", ""], [0.0, 600.0], &mut out).is_err());
    }

    #[test]
    fn xycut_reads_columns_and_grid_reads_rows() {
        let boxes = [
            [0.0, 0.0, 10.0, 5.0],
            [100.0, 0.0, 110.0, 5.0],
            [0.0, 10.0, 10.0, 15.0],
            [100.0, 10.0, 110.0, 15.0],
        ];
        let mut ids = [0, 1, 2, 3];
        xycut(&boxes, &mut ids, deadline()).unwrap();
        assert_eq!(ids, [0, 2, 1, 3]);
        top_down(&boxes, &mut ids);
        assert_eq!(ids, [0, 1, 2, 3]);
    }

    #[test]
    fn average_linkage_does_not_join_groups_from_one_positive_edge() {
        let same = [0.0, 2.0, -3.0, 2.0, 0.0, 1.0, -3.0, 1.0, 0.0];
        let mut sums = [0.0; 9];
        let mut means = [0.0; 9];
        let mut owner = [0; 3];
        let mut sizes = [0; 3];
        cluster(
            &same,
            &mut sums,
            &mut means,
            &mut owner,
            &mut sizes,
            deadline(),
        )
        .unwrap();
        assert_eq!(owner, [0, 0, 2]);
        assert_eq!(sizes, [2, 0, 1]);
    }

    #[test]
    fn insertion_search_corrects_the_initial_row_sum_ranking() {
        // Checked against decode_order in the pinned upstream. The initial
        // ranking [2, 3, 4, 0, 1] must move line 0 before line 4.
        let pairwise = [
            0.0, 0.0, -7.0, -4.0, 4.0, 0.0, 0.0, -1.0, -8.0, -4.0, 7.0, 1.0, 0.0, 7.0, 0.0, 4.0,
            8.0, -7.0, 0.0, -2.0, -4.0, 4.0, 0.0, 2.0, 0.0,
        ];
        let mut lp = [0.0; 25];
        for (dst, value) in lp.iter_mut().zip(pairwise) {
            *dst = log_sigmoid(value);
        }
        let mut out = [0; 5];
        decode_log_probabilities(&lp, &mut out, deadline()).unwrap();
        assert_eq!(out, [2, 3, 0, 4, 1]);
    }

    #[test]
    fn float64_reduction_is_pairwise_like_numpy() {
        // 30 values of 0.927. NumPy 2.3.5: np.add.reduce gives 27.81 and np.mean
        // 0.9269999999999999. A left-to-right sum, like np.cumsum, gives
        // 27.80999999999999 (CPython's own sum() is compensated since 3.12).
        let values = [0.927_f64; 30];
        assert_eq!(sum64(&values), 27.81);
        assert_eq!(sum64(&values) / 30.0, 0.926_999_999_999_999_9);
        assert_eq!(values.iter().sum::<f64>(), 27.809_999_999_999_99);
        assert_eq!(sum64(&[]).to_bits(), 0.0_f64.to_bits());
    }

    #[test]
    fn float_reduction_retains_the_numpy_identity_sign() {
        assert_eq!(sum32(&[]).to_bits(), 0.0_f32.to_bits());
        assert_eq!(sum32(&[-0.0; 8]).to_bits(), 0.0_f32.to_bits());
        assert_eq!(sum32(&[1.0, -1.0, -0.0]).to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    fn layout_class_changes_only_the_order_inside_its_block() {
        let boxes = [
            [0.0, 0.0, 10.0, 5.0],
            [100.0, 0.0, 110.0, 5.0],
            [0.0, 10.0, 10.0, 15.0],
            [100.0, 10.0, 110.0, 15.0],
        ];
        let mut sums = [0.0; 16];
        let mut means = [0.0; 16];
        let mut logits = [0.0; 16];
        let mut out = [0; 4];
        let mut classes = [0.0; 40];
        for row in classes.as_chunks_mut::<10>().0 {
            row[5] = 1.0;
        }
        decode_with_scratch(
            &[0.0; 16],
            &[1.0; 16],
            &classes,
            &boxes,
            Scratch {
                sums: &mut sums,
                means: &mut means,
                logits: &mut logits,
            },
            &mut out,
            deadline(),
        )
        .unwrap();
        assert_eq!(out, [0, 1, 2, 3]);
        decode_with_scratch(
            &[0.0; 16],
            &[1.0; 16],
            &[],
            &boxes,
            Scratch {
                sums: &mut sums,
                means: &mut means,
                logits: &mut logits,
            },
            &mut out,
            deadline(),
        )
        .unwrap();
        assert_eq!(out, [0, 2, 1, 3]);
    }

    #[test]
    fn expired_deadline_and_nonfinite_tensors_fail() {
        let boxes = [[0.0, 0.0, 1.0, 1.0]; 2];
        let mut sums = [0.0; 4];
        let mut means = [0.0; 4];
        let mut logits = [0.0; 4];
        let mut out = [0; 2];
        assert!(matches!(
            decode_with_scratch(
                &[0.0; 4],
                &[],
                &[],
                &boxes,
                Scratch {
                    sums: &mut sums,
                    means: &mut means,
                    logits: &mut logits
                },
                &mut out,
                Instant::now(),
            ),
            Err(EngineError::Deadline)
        ));
        assert!(
            decode_with_scratch(
                &[f32::NAN; 4],
                &[],
                &[],
                &boxes,
                Scratch {
                    sums: &mut sums,
                    means: &mut means,
                    logits: &mut logits
                },
                &mut out,
                deadline(),
            )
            .is_err()
        );
    }
}
