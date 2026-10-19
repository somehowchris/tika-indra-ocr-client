//! Preprocessing follows the pinned Indra crop, orientation and detector rules.

use std::time::Instant;

use opencv::{
    core::{self, Mat, Point2f, Scalar, Size},
    imgproc,
    prelude::*,
};

use crate::{EngineError, MAX_LINES, MAX_PIXELS, Result, deadline};

pub(crate) const LINE_HEIGHT: usize = 32;
pub(crate) const MAX_WIDTH: usize = 4096;
pub(crate) const DET_SIDE: usize = 1024;
pub(crate) const DET_MAX_SIDE: usize = 2048;
pub(crate) const ORIENT_SIDE: usize = 768;

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
    pub out_width: usize,
}

pub(crate) fn crop_geometry(b: &[f32; 4], height: usize) -> Result<Crop> {
    if !b.iter().all(|v| v.is_finite()) || b[2] < b[0] || b[3] < b[1] {
        return Err(EngineError::Shape("invalid detector box"));
    }
    let [x0, y0, x1, y1] = b.map(f64::from);
    let h = (y1 - y0).max(1.0);
    let x = (x0 - 0.3 * h).floor() as i32;
    let y = (y0 - 0.2 * h).floor() as i32;
    let width = (((x1 + 0.3 * h).ceil() as i32 - x).max(1)) as usize;
    let crop_height = (((y1 + 0.2 * h).ceil() as i32 - y).max(1)) as usize;
    if width > 4 * MAX_PIXELS / crop_height {
        return Err(EngineError::Limit("expanded line crop pixels"));
    }
    Ok(Crop {
        x,
        y,
        width,
        height: crop_height,
        out_width: (width as f64 * height as f64 / crop_height as f64)
            .round_ties_even()
            .clamp(8.0, MAX_WIDTH as f64) as usize,
    })
}

pub(crate) struct Scratch {
    pub resized: Mat,
    crop: Mat,
    skew_ink: Mat,
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
            skew_ink: Mat::default(),
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

    pub fn line_input(
        &mut self,
        gray: &Mat,
        crop: &Crop,
        height: usize,
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
        target.fill(255);
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
        let interpolation = if crop.height > height {
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
            Size::new(crop.out_width as i32, height as i32),
            0.0,
            0.0,
            interpolation,
        )?;
        for (to, from) in out.chunks_exact_mut(row_width).take(height).zip(
            self.resized
                .data_typed::<u8>()?
                .chunks_exact(crop.out_width),
        ) {
            for (a, b) in to[..crop.out_width].iter_mut().zip(from) {
                *a = 1.0 - f32::from(*b) / 255.0;
            }
        }
        Ok(())
    }

    pub fn estimate_skew(&mut self, gray: &Mat, due: Instant) -> Result<f64> {
        let scale = 800.0 / f64::from(gray.rows().max(gray.cols()));
        let size = Size::new(
            (f64::from(gray.cols()) * scale).round_ties_even().max(1.0) as i32,
            (f64::from(gray.rows()) * scale).round_ties_even().max(1.0) as i32,
        );
        imgproc::resize(gray, &mut self.resized, size, 0.0, 0.0, imgproc::INTER_AREA)?;
        self.skew_ink = Mat::new_rows_cols_with_default(
            size.height,
            size.width,
            core::CV_32FC1,
            Scalar::all(0.0),
        )?;
        for (ink, pixel) in self
            .skew_ink
            .data_typed_mut::<f32>()?
            .iter_mut()
            .zip(self.resized.data_typed::<u8>()?)
        {
            *ink = (255.0 - f32::from(*pixel)) / 255.0;
        }
        let ink = self.skew_ink.data_typed::<f32>()?;
        if crate::order::sum32(ink) / (ink.len() as f32) < 0.003 {
            return Ok(0.0);
        }
        let mut best = -5.0;
        let mut score = f32::NEG_INFINITY;
        for step in 0..21 {
            deadline(due)?;
            let angle = -5.0 + f64::from(step) * 0.5;
            let value = self.sharpness(angle, size)?;
            if value > score {
                best = angle;
                score = value;
            }
        }
        let start = best - 0.5;
        let delta = start + 0.1 - start; // NumPy arange forms its stored step from the first two values.
        best = start;
        score = f32::NEG_INFINITY;
        for step in 0..11 {
            deadline(due)?;
            let angle = start + f64::from(step) * delta;
            let value = self.sharpness(angle, size)?;
            if value > score {
                best = angle;
                score = value;
            }
        }
        Ok(if best.abs() >= 0.3 { best } else { 0.0 })
    }

    fn sharpness(&mut self, angle: f64, size: Size) -> Result<f32> {
        let transform = opencv::geometry::get_rotation_matrix_2d(
            Point2f::new(size.width as f32 / 2.0, size.height as f32 / 2.0),
            angle,
            1.0,
        )?;
        imgproc::warp_affine(
            &self.skew_ink,
            &mut self.skew_rotated,
            &transform,
            size,
            imgproc::INTER_LINEAR,
            core::BORDER_CONSTANT,
            Scalar::all(0.0),
            core::AlgorithmHint::ALGO_HINT_DEFAULT,
        )?;
        let mut differences = [0.0_f32; 799];
        let mut prior = 0.0;
        for (index, row) in self
            .skew_rotated
            .data_typed::<f32>()?
            .chunks_exact(size.width as usize)
            .enumerate()
        {
            let value = crate::order::sum32(row);
            if index != 0 {
                differences[index - 1] = (value - prior) * (value - prior);
            }
            prior = value;
        }
        Ok(crate::order::sum32(
            &differences[..size.height as usize - 1],
        ))
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

pub(crate) fn turn(gray: Mat, quarter_turns: usize) -> Result<Mat> {
    if quarter_turns == 0 {
        return Ok(gray);
    }
    let mut upright = Mat::default();
    let code = match quarter_turns {
        1 => core::ROTATE_90_CLOCKWISE,
        2 => core::ROTATE_180,
        3 => core::ROTATE_90_COUNTERCLOCKWISE,
        _ => return Err(EngineError::Shape("orientation class")),
    };
    core::rotate(&gray, &mut upright, code)?;
    Ok(upright)
}

pub(crate) fn deskew(gray: Mat, angle: f64) -> Result<Mat> {
    if angle == 0.0 {
        return Ok(gray);
    }
    let transform = opencv::geometry::get_rotation_matrix_2d(
        Point2f::new(gray.cols() as f32 / 2.0, gray.rows() as f32 / 2.0),
        angle,
        1.0,
    )?;
    let mut rotated = Mat::default();
    imgproc::warp_affine(
        &gray,
        &mut rotated,
        &transform,
        gray.size()?,
        imgproc::INTER_LINEAR,
        core::BORDER_REPLICATE,
        Scalar::all(0.0),
        core::AlgorithmHint::ALGO_HINT_DEFAULT,
    )?;
    Ok(rotated)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crop_padding_and_rounding_are_explicit() {
        let c = crop_geometry(&[0.0, 0.0, 100.0, 10.0], 32).expect("crop");
        assert_eq!(
            (c.x, c.y, c.width, c.height, c.out_width),
            (-3, -2, 106, 14, 242)
        );
        assert!(crop_geometry(&[1.0, 2.0, 0.0, 3.0], 32).is_err());
        assert!(crop_geometry(&[0.0, 0.0, f32::NAN, 3.0], 32).is_err());
    }
}
