//! Preprocessing follows the pinned Indra 1.1 crop, orientation, deskew and detector rules.
//!
//! Port of `indra_ocr/rec_preproc.py` (`crop_line`, `is_dark_line`) and the
//! `pipeline.py` thumbnail, skew-search and page-turn helpers at upstream
//! commit `0e96b93c6810f17cf8352acbb4487cc9e45b66d5` (Apache-2.0).

use std::time::Instant;

use opencv::{
    core::{self, Mat, Point2f, Scalar, Size},
    imgproc,
    prelude::*,
};

use crate::{EngineError, MAX_LINES, MAX_PIXELS, Result, deadline};

pub(crate) const LINE_HEIGHT: usize = 32;
/// `meta.json` `rec_window`: the base recogniser reads the line and the page
/// around it, three line bands tall. Group recognisers and classifiers read one band.
pub(crate) const WINDOW: usize = 3;
pub(crate) const WINDOW_HEIGHT: usize = LINE_HEIGHT * WINDOW;
pub(crate) const MAX_WIDTH: usize = 4096;
pub(crate) const DET_SIDE: usize = 1024;
pub(crate) const DET_MAX_SIDE: usize = 2048;
pub(crate) const ORIENT_SIDE: usize = 768;
/// The skew search and orientation model read a page preview of at most this side.
pub(crate) const PREVIEW_SIDE: usize = 1600;
const SKEW_SIDE: usize = 800;
const DESKEW_MAX: f64 = 45.0;
const DESKEW_MIN: f64 = 0.3;
/// `orient.onnx` class 4: the page is set in vertical columns.
pub(crate) const VERTICAL: usize = 4;
const PAD_X: f64 = 0.3;
const PAD_Y: f64 = 0.2;

pub(crate) fn floats(count: usize) -> Result<Mat> {
    Ok(Mat::new_rows_cols_with_default(
        1,
        i32::try_from(count).map_err(|_| EngineError::Limit("float scratch size"))?,
        core::CV_32FC1,
        Scalar::all(0.0),
    )?)
}

#[derive(Debug)]
pub(crate) struct Crop {
    pub x: i32,
    pub y: i32,
    pub width: usize,
    pub height: usize,
    /// Rows of the recogniser input: `height` of the band times `window`.
    pub rows: usize,
    pub out_width: usize,
}

/// `crop_line` geometry: a box padded by 0.3 heights left and right and, for a
/// `window`-band window, by `(window × 1.4 − 1) / 2` heights above and below.
pub(crate) fn crop_geometry(b: &[f32; 4], height: usize, window: usize) -> Result<Crop> {
    if !b.iter().all(|v| v.is_finite()) || b[2] < b[0] || b[3] < b[1] || window == 0 {
        return Err(EngineError::Shape("invalid detector box"));
    }
    let pad_y = (window as f64 * (1.0 + 2.0 * PAD_Y) - 1.0) / 2.0;
    let rows = height * window;
    let [x0, y0, x1, y1] = b.map(f64::from);
    let h = (y1 - y0).max(1.0);
    let x = (x0 - PAD_X * h).floor() as i32;
    let y = (y0 - pad_y * h).floor() as i32;
    let width = (((x1 + PAD_X * h).ceil() as i32 - x).max(1)) as usize;
    let crop_height = (((y1 + pad_y * h).ceil() as i32 - y).max(1)) as usize;
    if width > 4 * MAX_PIXELS / crop_height {
        return Err(EngineError::Limit("expanded line crop pixels"));
    }
    Ok(Crop {
        x,
        y,
        width,
        height: crop_height,
        rows,
        out_width: (width as f64 * rows as f64 / crop_height as f64)
            .round_ties_even()
            .clamp(8.0, MAX_WIDTH as f64) as usize,
    })
}

/// `is_dark_line`: light text on a dark ground. The 2nd, 50th and 98th
/// percentiles of the box pixels come from the histogram exactly as NumPy's
/// linear percentile would compute them; a longer bright tail means the ground
/// is dark and the recognisers read the line inverted.
pub(crate) fn is_dark_line(gray: &Mat, b: &[f32; 4]) -> Result<bool> {
    let rows = gray.rows() as i64;
    let cols = gray.cols() as i64;
    // Python int() truncates toward zero, as `as i64` does.
    let y0 = (b[1] as i64).max(0);
    let y1 = (b[3] as i64 + 1).min(rows);
    let x0 = (b[0] as i64).max(0);
    let x1 = (b[2] as i64 + 1).min(cols);
    if y1 <= y0 || x1 <= x0 {
        return Ok(false);
    }
    let pixels = gray.data_typed::<u8>()?;
    let mut histogram = [0_u64; 256];
    for row in y0..y1 {
        let line = &pixels[(row * cols + x0) as usize..(row * cols + x1) as usize];
        for value in line {
            histogram[usize::from(*value)] += 1;
        }
    }
    let n = ((y1 - y0) * (x1 - x0)) as u64;
    let mut cumulative = [0_u64; 256];
    let mut running = 0;
    for (total, count) in cumulative.iter_mut().zip(&histogram) {
        running += count;
        *total = running;
    }
    // np.searchsorted(cum, i, side="right"): the number of cumulative counts <= i.
    let position = |i: u64| cumulative.iter().take_while(|total| **total <= i).count() as f64;
    let percentile = |q: f64| {
        let x = q * (n - 1) as f64;
        let i = x.floor();
        let g = x - i;
        let lo = position(i as u64);
        let hi = position((i as u64 + 1).min(n - 1));
        let d = hi - lo;
        if g >= 0.5 {
            hi - d * (1.0 - g)
        } else {
            lo + d * g
        }
    };
    let (lo, med, hi) = (percentile(0.02), percentile(0.5), percentile(0.98));
    Ok(hi - med > 2.0 * (med - lo) + 8.0)
}

pub(crate) struct Scratch {
    pub resized: Mat,
    crop: Mat,
    skew_fine: Mat,
    skew_coarse: Mat,
    skew_rotated: Mat,
    mask: Mat,
    labels: Mat,
    stats: Mat,
    centroids: Mat,
    label_map: Mat,
}

impl Scratch {
    pub fn new() -> Result<Self> {
        Ok(Self {
            resized: Mat::default(),
            crop: Mat::default(),
            skew_fine: Mat::default(),
            skew_coarse: Mat::default(),
            skew_rotated: Mat::default(),
            mask: Mat::default(),
            labels: Mat::default(),
            stats: Mat::default(),
            centroids: Mat::default(),
            label_map: Mat::new_rows_cols_with_default(
                1,
                (DET_MAX_SIDE * DET_MAX_SIDE / 4 + 1) as i32,
                core::CV_32SC1,
                Scalar::all(-1.0),
            )?,
        })
    }

    pub fn page_input(
        &mut self,
        gray: &Mat,
        side: usize,
        square: bool,
        out: &mut [f32],
    ) -> Result<(usize, usize, f64)> {
        let height = gray.rows() as usize;
        let width = gray.cols() as usize;
        let scale = side as f64 / height.max(width) as f64;
        let h = (height as f64 * scale).round_ties_even().max(1.0) as usize;
        let w = (width as f64 * scale).round_ties_even().max(1.0) as usize;
        let ph = if square { side } else { h.div_ceil(32) * 32 };
        let pw = if square { side } else { w.div_ceil(32) * 32 };
        let active = out
            .get_mut(..ph * pw)
            .ok_or(EngineError::Limit("page tensor scratch"))?;
        active.fill(0.0);
        let interp = if square || scale < 1.0 {
            imgproc::INTER_AREA
        } else {
            imgproc::INTER_LINEAR
        };
        imgproc::resize(
            gray,
            &mut self.resized,
            Size::new(w as i32, h as i32),
            0.0,
            0.0,
            interp,
        )?;
        let pixels = self.resized.data_typed::<u8>()?;
        for (target, source) in active
            .chunks_exact_mut(pw)
            .take(h)
            .zip(pixels.chunks_exact(w))
        {
            for (to, from) in target[..w].iter_mut().zip(source) {
                *to = 1.0 - f32::from(*from) / 255.0;
            }
        }
        Ok((ph, pw, scale))
    }

    /// One recogniser input row block: the crop filled with paper (ink for a
    /// dark line) outside the page, resized to `crop.rows` rows, as an ink map.
    pub fn line_input(
        &mut self,
        gray: &Mat,
        crop: &Crop,
        dark: bool,
        out: &mut [f32],
        row_width: usize,
    ) -> Result<()> {
        let required = crop.height * crop.width;
        if self.crop.total() < required {
            self.crop = Mat::new_rows_cols_with_default(
                1,
                required as i32,
                core::CV_8UC1,
                Scalar::all(255.0),
            )?;
        }
        let page_width = gray.cols() as usize;
        let page_height = gray.rows() as usize;
        let pixels = gray.data_typed::<u8>()?;
        let target = &mut self.crop.data_typed_mut::<u8>()?[..required];
        target.fill(if dark { 0 } else { 255 });
        let sx0 = crop.x.max(0) as usize;
        let sy0 = crop.y.max(0) as usize;
        let sx1 = (i64::from(crop.x) + crop.width as i64).clamp(0, page_width as i64) as usize;
        let sy1 = (i64::from(crop.y) + crop.height as i64).clamp(0, page_height as i64) as usize;
        if sx1 > sx0 && sy1 > sy0 {
            let dx = (sx0 as i64 - i64::from(crop.x)) as usize;
            let dy = (sy0 as i64 - i64::from(crop.y)) as usize;
            for row in 0..sy1 - sy0 {
                target[(dy + row) * crop.width + dx..(dy + row) * crop.width + dx + sx1 - sx0]
                    .copy_from_slice(
                        &pixels[(sy0 + row) * page_width + sx0..(sy0 + row) * page_width + sx1],
                    );
            }
        }
        let interpolation = if crop.height > crop.rows {
            imgproc::INTER_AREA
        } else {
            imgproc::INTER_LINEAR
        };
        // The crop arena grows only at a new high-water mark. A borrowed header
        // exposes exactly this crop's shape without another pixel allocation.
        let crop_view = Mat::from_slice(&*target)?;
        let crop_view = crop_view.reshape(1, crop.height as i32)?;
        imgproc::resize(
            &crop_view,
            &mut self.resized,
            Size::new(crop.out_width as i32, crop.rows as i32),
            0.0,
            0.0,
            interpolation,
        )?;
        for (to, from) in out.chunks_exact_mut(row_width).take(crop.rows).zip(
            self.resized
                .data_typed::<u8>()?
                .chunks_exact(crop.out_width),
        ) {
            if dark {
                for (a, b) in to[..crop.out_width].iter_mut().zip(from) {
                    *a = f32::from(*b) / 255.0;
                }
            } else {
                for (a, b) in to[..crop.out_width].iter_mut().zip(from) {
                    *a = 1.0 - f32::from(*b) / 255.0;
                }
            }
        }
        Ok(())
    }

    /// `_ink_thumb`: the page shrunk (or stretched) so its long side is `side`,
    /// as an 8-bit ink map (255 = ink).
    fn ink_thumb(gray: &Mat, side: usize, out: &mut Mat) -> Result<Size> {
        let scale = side as f64 / f64::from(gray.rows().max(gray.cols()));
        let size = Size::new(
            (f64::from(gray.cols()) * scale).round_ties_even().max(1.0) as i32,
            (f64::from(gray.rows()) * scale).round_ties_even().max(1.0) as i32,
        );
        imgproc::resize(gray, out, size, 0.0, 0.0, imgproc::INTER_AREA)?;
        for pixel in out.data_typed_mut::<u8>()? {
            *pixel = 255 - *pixel;
        }
        Ok(size)
    }

    /// `estimate_skew`: the counter-clockwise angle whose horizontal ink
    /// profile changes most sharply row to row. ±45° in 2° steps on a 400-pixel
    /// thumbnail; 0° and the two best coarse angles refined in 0.5° steps, then
    /// the best of those in 0.1° steps, on the 800-pixel thumbnail.
    pub fn estimate_skew(&mut self, gray: &Mat, due: Instant) -> Result<f64> {
        let fine = Self::ink_thumb(gray, SKEW_SIDE, &mut self.skew_fine)?;
        let ink = self.skew_fine.data_typed::<u8>()?;
        let total = ink.iter().map(|value| u64::from(*value)).sum::<u64>();
        if (total as f64) / (ink.len() as f64) < 0.003 * 255.0 {
            return Ok(0.0);
        }
        let coarse = Self::ink_thumb(gray, SKEW_SIDE / 2, &mut self.skew_coarse)?;
        let mut scores = [0.0_f64; 46];
        for (step, score) in scores.iter_mut().enumerate() {
            deadline(due)?;
            let angle = -DESKEW_MAX + step as f64 * 2.0;
            *score = Self::row_sharpness(&self.skew_coarse, &mut self.skew_rotated, angle, coarse)?;
        }
        // np.argsort(scores)[-2:]: the two best coarse angles. The reference's
        // unstable sort does not define the order of exactly tied integer
        // scores; a stable sort keeps the later angle, which is used here.
        let mut best = [0_usize, usize::MAX];
        for (index, score) in scores.iter().enumerate().skip(1) {
            if *score >= scores[best[0]] {
                best = [index, best[0]];
            } else if best[1] == usize::MAX || *score >= scores[best[1]] {
                best[1] = index;
            }
        }
        let mut starts = [
            0.0,
            -DESKEW_MAX + best[1] as f64 * 2.0,
            -DESKEW_MAX + best[0] as f64 * 2.0,
        ];
        python_set_order(&mut starts);
        // Each start contributes nine angles in 0.5° steps; the ranges of nearby
        // starts overlap and every angle is scored once, in first-seen order.
        let mut near = [0.0_f64; 27];
        let mut near_scores = [0.0_f64; 27];
        let mut count = 0;
        for start in starts {
            let first = start - 2.0;
            let delta = (first + 0.5) - first;
            for step in 0..9 {
                let angle = first + step as f64 * delta;
                let seen = near[..count].iter().position(|value| *value == angle);
                let score = match seen {
                    Some(index) => near_scores[index],
                    None => {
                        deadline(due)?;
                        Self::row_sharpness(&self.skew_fine, &mut self.skew_rotated, angle, fine)?
                    }
                };
                near[count] = angle;
                near_scores[count] = score;
                count += 1;
            }
        }
        let mut refined = near[0];
        let mut refined_score = near_scores[0];
        for (angle, score) in near[..count].iter().zip(&near_scores[..count]) {
            if *score > refined_score {
                refined = *angle;
                refined_score = *score;
            }
        }
        let first = refined - 0.5;
        let delta = (first + 0.1) - first; // NumPy arange forms its stored step from the first two values.
        let mut best_angle = first;
        let mut best_score = f64::NEG_INFINITY;
        for step in 0..11 {
            deadline(due)?;
            let angle = first + step as f64 * delta;
            let score = Self::row_sharpness(&self.skew_fine, &mut self.skew_rotated, angle, fine)?;
            if score > best_score {
                best_angle = angle;
                best_score = score;
            }
        }
        Ok(best_angle)
    }

    /// `_row_sharpness`: squared row-to-row change of the horizontal ink
    /// profile of the thumbnail turned by `angle`. Row sums are exact integers,
    /// so the f64 accumulation equals NumPy's pairwise sum.
    fn row_sharpness(ink: &Mat, rotated: &mut Mat, angle: f64, size: Size) -> Result<f64> {
        let transform = opencv::geometry::get_rotation_matrix_2d(
            Point2f::new(size.width as f32 / 2.0, size.height as f32 / 2.0),
            angle,
            1.0,
        )?;
        imgproc::warp_affine(
            ink,
            rotated,
            &transform,
            size,
            imgproc::INTER_LINEAR,
            core::BORDER_CONSTANT,
            Scalar::all(0.0),
            core::AlgorithmHint::ALGO_HINT_DEFAULT,
        )?;
        let mut total = 0.0_f64;
        let mut prior = 0_i64;
        for (index, row) in rotated
            .data_typed::<u8>()?
            .chunks_exact(size.width as usize)
            .enumerate()
        {
            let value = row.iter().map(|pixel| i64::from(*pixel)).sum::<i64>();
            if index != 0 {
                let difference = (value - prior) as f64;
                total += difference * difference;
            }
            prior = value;
        }
        Ok(total)
    }

    pub fn decode_detector(
        &mut self,
        out: &[f32],
        height: usize,
        width: usize,
        boxes: &mut [[f32; 4]],
        due: Instant,
    ) -> Result<usize> {
        let plane = height
            .checked_mul(width)
            .ok_or(EngineError::Shape("detector dimensions overflow"))?;
        if plane > DET_MAX_SIDE * DET_MAX_SIDE / 4
            || out.len() != 3 * plane
            || !out.iter().all(|v| v.is_finite())
        {
            return Err(EngineError::Shape("detector output"));
        }
        self.mask = Mat::new_rows_cols_with_default(
            height as i32,
            width as i32,
            core::CV_8UC1,
            Scalar::all(0.0),
        )?;
        for (pixel, logit) in self
            .mask
            .data_typed_mut::<u8>()?
            .iter_mut()
            .zip(&out[..plane])
        {
            *pixel = u8::from(1.0_f32 / (1.0 + (-logit).exp()) > 0.5);
        }
        let components = imgproc::connected_components_with_stats(
            &self.mask,
            &mut self.labels,
            &mut self.stats,
            &mut self.centroids,
            4,
            core::CV_32S,
        )? as usize;
        let stats = self.stats.data_typed::<i32>()?;
        let map = &mut self.label_map.data_typed_mut::<i32>()?[..components];
        map.fill(-1);
        let mut count = 0;
        for label in 1..components {
            if stats[label * 5 + 4] < 2 {
                continue;
            }
            if count == boxes.len() {
                return Err(EngineError::Limit("detected lines"));
            }
            map[label] = count as i32;
            count += 1;
        }
        let mut top = [0.0_f64; MAX_LINES];
        let mut bottom = [0.0_f64; MAX_LINES];
        for (index, label) in self.labels.data_typed::<i32>()?.iter().enumerate() {
            if index % 65_536 == 0 {
                deadline(due)?;
            }
            let slot = map[*label as usize];
            if slot < 0 {
                continue;
            }
            let y = (index / width) as f32 * 2.0 + 1.0;
            top[slot as usize] += f64::from(y - out[plane + index]);
            bottom[slot as usize] += f64::from(y + out[2 * plane + index]);
        }
        for label in 1..components {
            let slot = map[label];
            if slot < 0 {
                continue;
            }
            let slot = slot as usize;
            let area = f64::from(stats[label * 5 + 4]);
            let y0 = top[slot] / area;
            let y1 = bottom[slot] / area;
            let extension = 0.15 * (y1 - y0).max(1.0);
            boxes[slot] = [
                (f64::from(stats[label * 5]) * 2.0 - extension) as f32,
                y0 as f32,
                (f64::from(stats[label * 5] + stats[label * 5 + 2]) * 2.0 + extension) as f32,
                y1 as f32,
            ];
        }
        Ok(count)
    }
}

/// CPython iterates a three-element set of these integer-valued floats in
/// hash-slot order (table of eight slots, one perturbed probe on a collision).
/// The reference builds its refinement candidates in that order.
fn python_set_order(values: &mut [f64; 3]) {
    fn hash(value: f64) -> i64 {
        let integer = value as i64;
        if integer == -1 { -2 } else { integer }
    }
    let mut slots = [usize::MAX; 3];
    let mut taken = [false; 8];
    for (position, value) in values.iter().enumerate() {
        let h = hash(*value);
        let mut i = (h & 7) as usize;
        let mut perturb = h as u64;
        while taken[i] {
            perturb >>= 5;
            i = ((i as u64)
                .wrapping_mul(5)
                .wrapping_add(1)
                .wrapping_add(perturb)
                & 7) as usize;
        }
        taken[i] = true;
        slots[position] = i;
    }
    let mut order = [0, 1, 2];
    order.sort_unstable_by_key(|position| slots[*position]);
    *values = order.map(|position| values[position]);
}

/// `preview`: the page shrunk once to at most `PREVIEW_SIDE` pixels a side.
/// A page already within the bound is used as it is.
pub(crate) fn preview(gray: &Mat, out: &mut Mat) -> Result<bool> {
    let longest = gray.rows().max(gray.cols());
    let scale = PREVIEW_SIDE as f64 / f64::from(longest);
    if scale >= 1.0 {
        return Ok(false);
    }
    imgproc::resize(
        gray,
        out,
        Size::new(
            (f64::from(gray.cols()) * scale).round_ties_even().max(1.0) as i32,
            (f64::from(gray.rows()) * scale).round_ties_even().max(1.0) as i32,
        ),
        0.0,
        0.0,
        imgproc::INTER_AREA,
    )?;
    Ok(true)
}

/// Quarter turns that undo an orientation class: `np.rot90(gray, -k)` for a
/// turned page, and one counter-clockwise turn for a vertical page.
pub(crate) const fn rotation_code(class: usize) -> Result<Option<i32>> {
    Ok(match class {
        0 => None,
        1 => Some(core::ROTATE_90_CLOCKWISE),
        2 => Some(core::ROTATE_180),
        3 | VERTICAL => Some(core::ROTATE_90_COUNTERCLOCKWISE),
        _ => return Err(EngineError::Shape("orientation class")),
    })
}

pub(crate) fn turn(gray: &Mat, class: usize, out: &mut Mat) -> Result<bool> {
    let Some(code) = rotation_code(class)? else {
        return Ok(false);
    };
    core::rotate(gray, out, code)?;
    Ok(true)
}

/// Whether the reference straightens the page by `angle` or leaves it alone.
pub(crate) const fn deskews(angle: f64) -> bool {
    angle.abs() >= DESKEW_MIN
}

/// `rotate_page`: the page turned by `angle` degrees counter-clockwise about its centre.
pub(crate) fn rotate_page(gray: &Mat, angle: f64, out: &mut Mat) -> Result<()> {
    let transform = opencv::geometry::get_rotation_matrix_2d(
        Point2f::new(gray.cols() as f32 / 2.0, gray.rows() as f32 / 2.0),
        angle,
        1.0,
    )?;
    imgproc::warp_affine(
        gray,
        out,
        &transform,
        gray.size()?,
        imgproc::INTER_LINEAR,
        core::BORDER_REPLICATE,
        Scalar::all(0.0),
        core::AlgorithmHint::ALGO_HINT_DEFAULT,
    )?;
    Ok(())
}

/// `unrotate_box`: a box of the deskewed page mapped back to the page before
/// the deskew, as the bounding box of its four mapped corners, clipped and
/// rounded to a tenth as the reference reports it.
pub(crate) fn unrotate_box(b: &[f32; 4], angle: f64, wh: [i32; 2]) -> Result<[f32; 4]> {
    let w = f64::from(wh[0]);
    let h = f64::from(wh[1]);
    let transform = opencv::geometry::get_rotation_matrix_2d(
        Point2f::new(wh[0] as f32 / 2.0, wh[1] as f32 / 2.0),
        -angle,
        1.0,
    )?;
    let m = transform.data_typed::<f64>()?;
    let [x0, y0, x1, y1] = b.map(f64::from);
    let mut xs = [0.0_f64; 4];
    let mut ys = [0.0_f64; 4];
    for (index, (x, y)) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)]
        .into_iter()
        .enumerate()
    {
        xs[index] = m[0] * x + m[1] * y + m[2];
        ys[index] = m[3] * x + m[4] * y + m[5];
    }
    let round = |value: f64, limit: f64| (value.clamp(0.0, limit) * 10.0).round_ties_even() / 10.0;
    Ok([
        round(xs.iter().copied().fold(f64::INFINITY, f64::min), w) as f32,
        round(ys.iter().copied().fold(f64::INFINITY, f64::min), h) as f32,
        round(xs.iter().copied().fold(f64::NEG_INFINITY, f64::max), w) as f32,
        round(ys.iter().copied().fold(f64::NEG_INFINITY, f64::max), h) as f32,
    ])
}

/// A vertical page was read turned a quarter turn counter-clockwise; its boxes
/// go back on the page as it is: `(x, y) → (W − y, x)` with `W` the turned height.
pub(crate) const fn vertical_box_back(b: &[f32; 4], turned_rows: i32) -> [f32; 4] {
    let w = turned_rows as f32;
    [w - b[3], b[0], w - b[1], b[2]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_padding_and_rounding_are_explicit() {
        let c = crop_geometry(&[0.0, 0.0, 100.0, 10.0], 32, 1).expect("crop");
        assert_eq!(
            (c.x, c.y, c.width, c.height, c.rows, c.out_width),
            (-3, -2, 106, 14, 32, 242)
        );
        // A three-band window pads 1.6 heights above and below and keeps the text scale.
        let tall = crop_geometry(&[0.0, 0.0, 100.0, 10.0], 32, 3).expect("tall crop");
        assert_eq!(
            (
                tall.x,
                tall.y,
                tall.width,
                tall.height,
                tall.rows,
                tall.out_width
            ),
            (-3, -16, 106, 42, 96, 242)
        );
        assert!(crop_geometry(&[1.0, 2.0, 0.0, 3.0], 32, 1).is_err());
        assert!(crop_geometry(&[0.0, 0.0, f32::NAN, 3.0], 32, 1).is_err());
        assert!(crop_geometry(&[0.0, 0.0, 1.0, 3.0], 32, 0).is_err());
    }

    #[test]
    fn dark_line_follows_the_histogram_percentile_rule() {
        let mut page = Mat::new_rows_cols_with_default(20, 50, core::CV_8UC1, Scalar::all(255.0))
            .expect("page");
        assert!(!is_dark_line(&page, &[2.0, 2.0, 40.0, 18.0]).expect("white page"));
        // A dark banner with light text: most pixels near 20, a bright tail at 240.
        for (index, pixel) in page
            .data_typed_mut::<u8>()
            .expect("pixels")
            .iter_mut()
            .enumerate()
        {
            *pixel = if index % 7 == 0 { 240 } else { 20 };
        }
        assert!(is_dark_line(&page, &[0.0, 0.0, 49.0, 19.0]).expect("dark line"));
        assert!(!is_dark_line(&page, &[60.0, 0.0, 70.0, 19.0]).expect("empty box"));
    }

    #[test]
    fn set_order_and_turn_classes_match_the_reference_tables() {
        let mut starts = [0.0, 3.0, 11.0];
        python_set_order(&mut starts);
        assert_eq!(starts, [0.0, 3.0, 11.0]);
        let mut starts = [0.0, -45.0, 7.0];
        python_set_order(&mut starts);
        // hash(-45) & 7 == 3, hash(7) & 7 == 7: slot order.
        assert_eq!(starts, [0.0, -45.0, 7.0]);
        let mut starts = [0.0, 9.0, 1.0];
        python_set_order(&mut starts);
        // 9 and 1 share slot 1; the perturbed probe for 1 lands on slot 6.
        assert_eq!(starts, [0.0, 9.0, 1.0]);
        let mut starts = [0.0, -1.0, 5.0];
        python_set_order(&mut starts);
        assert_eq!(starts, [0.0, 5.0, -1.0]);
        assert_eq!(rotation_code(0).expect("upright"), None);
        assert_eq!(
            rotation_code(VERTICAL).expect("vertical"),
            Some(core::ROTATE_90_COUNTERCLOCKWISE)
        );
        assert!(rotation_code(5).is_err());
        assert!(deskews(0.3) && !deskews(0.29));
    }

    #[test]
    fn vertical_boxes_map_back_through_the_quarter_turn() {
        assert_eq!(
            vertical_box_back(&[10.0, 20.0, 110.0, 40.0], 300),
            [260.0, 10.0, 280.0, 110.0]
        );
        let b = unrotate_box(&[10.0, 20.0, 110.0, 40.0], 0.0, [200, 300]).expect("identity");
        assert_eq!(b, [10.0, 20.0, 110.0, 40.0]);
    }
}
