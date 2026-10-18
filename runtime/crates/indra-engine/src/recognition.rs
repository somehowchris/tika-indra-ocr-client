//! Script classification and CTC decoding use the exact FP32 model snapshot.

use std::time::Instant;

use opencv::{
    core::{self, Mat, Scalar},
    prelude::*,
};
use ort::{session::Session, value::TensorRef};

use crate::{
    EngineError, MAX_BATCH, MAX_LINES, OcrStats, Result,
    batching::{Plan, Policy},
    deadline,
    geometry::{self, Scratch},
    vocabulary,
};

pub(crate) const MAX_TOKENS: usize = geometry::MAX_WIDTH / 8;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Script {
    Text,
    Chinese,
    Math,
}

pub(crate) struct Line {
    pub script: Script,
    pub count: usize,
    pub confidence: f32,
    pub math_start: usize,
    pub math_end: usize,
}

impl Line {
    pub const fn empty() -> Self {
        Self {
            script: Script::Text,
            count: 0,
            confidence: 0.0,
            math_start: 0,
            math_end: 0,
        }
    }
}

pub(crate) struct Lines {
    pub metadata: [Line; MAX_LINES],
    pub ids: Mat,
    pub positions: Mat,
    pub math_text: String,
}

impl Lines {
    pub fn new() -> Result<Self> {
        Ok(Self {
            metadata: [const { Line::empty() }; MAX_LINES],
            ids: Mat::new_rows_cols_with_default(
                MAX_LINES as i32,
                MAX_TOKENS as i32,
                core::CV_32SC1,
                Scalar::all(0.0),
            )?,
            positions: Mat::new_rows_cols_with_default(
                MAX_LINES as i32,
                MAX_TOKENS as i32,
                core::CV_64FC1,
                Scalar::all(0.0),
            )?,
            math_text: String::with_capacity(crate::MAX_OUTPUT_BYTES),
        })
    }
    pub fn reset(&mut self) {
        for line in &mut self.metadata {
            *line = Line::empty();
        }
        self.math_text.clear();
    }
}

pub(crate) fn widths(boxes: &[[f32; 4]], height: usize, out: &mut [usize]) -> Result<()> {
    for (b, w) in boxes.iter().zip(out) {
        *w = geometry::crop_geometry(b, height)?.out_width;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn fill_batch(
    gray: &Mat,
    boxes: &[[f32; 4]],
    indices: &[usize],
    height: usize,
    scratch: &mut Scratch,
    batch: &mut Mat,
    widths: &mut [i64; MAX_BATCH],
    due: Instant,
) -> Result<usize> {
    let mut max_width = 0;
    for (slot, index) in indices.iter().enumerate() {
        let crop = geometry::crop_geometry(&boxes[*index], height)?;
        widths[slot] = crop.out_width as i64;
        max_width = max_width.max(crop.out_width);
    }
    let padded = max_width.div_ceil(8) * 8;
    let area = indices.len() * height * padded;
    let data = batch
        .data_typed_mut::<f32>()?
        .get_mut(..area)
        .ok_or(EngineError::Limit("recognition tensor scratch"))?;
    data.fill(0.0);
    for (slot, index) in indices.iter().enumerate() {
        deadline(due)?;
        let crop = geometry::crop_geometry(&boxes[*index], height)?;
        scratch.line_input(
            gray,
            &crop,
            height,
            &mut data[slot * height * padded..(slot + 1) * height * padded],
            padded,
        )?;
    }
    Ok(padded)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn classify(
    session: &mut Session,
    gray: &Mat,
    boxes: &[[f32; 4]],
    line_widths: &[usize],
    max_batch: usize,
    scratch: &mut Scratch,
    batch: &mut Mat,
    flags: &mut [bool],
    due: Instant,
    trace: &mut crate::trace::Recorder,
    model: &'static str,
) -> Result<()> {
    let _span = tracing::info_span!("ocr.classifier", model, lines = boxes.len()).entered();
    let mut plan = Plan::new(line_widths, max_batch, Policy::Text, |_| true)?;
    let mut widths = [0_i64; MAX_BATCH];
    while let Some(range) = plan.next(line_widths) {
        let indices = &plan.indices[range];
        let padded = fill_batch(
            gray,
            boxes,
            indices,
            geometry::LINE_HEIGHT,
            scratch,
            batch,
            &mut widths,
            due,
        )?;
        let _batch_span = tracing::debug_span!(
            "ocr.line_batch",
            model,
            rows = indices.len(),
            columns = padded
        )
        .entered();
        let active = indices.len() * geometry::LINE_HEIGHT * padded;
        trace.tensor(
            match model {
                "script" => "script-input",
                "mathcls" => "math-classifier-input",
                "rec_cjk" => "chinese-input",
                _ => "text-input",
            },
            &[
                indices.len() as i64,
                1,
                geometry::LINE_HEIGHT as i64,
                padded as i64,
            ],
            &batch.data_typed::<f32>()?[..active],
        )?;
        trace.tensor(
            "line-widths",
            &[indices.len() as i64],
            &widths[..indices.len()],
        )?;
        let mut trace_indices = [0_i64; MAX_BATCH];
        for (to, from) in trace_indices[..indices.len()].iter_mut().zip(indices) {
            *to = *from as i64;
        }
        trace.tensor(
            "line-indices",
            &[indices.len() as i64],
            &trace_indices[..indices.len()],
        )?;
        let x = TensorRef::from_array_view((
            [indices.len(), 1, geometry::LINE_HEIGHT, padded],
            &batch.data_typed::<f32>()?[..active],
        ))?;
        let w = TensorRef::from_array_view(([indices.len()], &widths[..indices.len()]))?;
        deadline(due)?;
        let outputs = session.run(ort::inputs![x, w])?;
        let (shape, probabilities) = outputs[0].try_extract_tensor::<f32>()?;
        trace.tensor(
            if model == "script" {
                "script-probabilities"
            } else {
                "math-classifier-probabilities"
            },
            shape,
            probabilities,
        )?;
        if probabilities.len() != indices.len() || !probabilities.iter().all(|v| v.is_finite()) {
            return Err(EngineError::Shape("line classifier output"));
        }
        for (index, value) in indices.iter().zip(probabilities) {
            flags[*index] = *value > 0.5;
        }
        deadline(due)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn text(
    session: &mut Session,
    gray: &Mat,
    boxes: &[[f32; 4]],
    line_widths: &[usize],
    hanzi: &[bool],
    chinese: bool,
    max_batch: usize,
    scratch: &mut Scratch,
    batch: &mut Mat,
    lines: &mut Lines,
    stats: &mut OcrStats,
    due: Instant,
    trace: &mut crate::trace::Recorder,
) -> Result<()> {
    let model = if chinese { "rec_cjk" } else { "rec" };
    let _span = tracing::info_span!("ocr.recognition", model, lines = boxes.len()).entered();
    let mut plan = Plan::new(line_widths, max_batch, Policy::Text, |i| {
        hanzi[i] == chinese
    })?;
    let mut widths = [0_i64; MAX_BATCH];
    while let Some(range) = plan.next(line_widths) {
        let indices = &plan.indices[range];
        let padded = fill_batch(
            gray,
            boxes,
            indices,
            geometry::LINE_HEIGHT,
            scratch,
            batch,
            &mut widths,
            due,
        )?;
        let _batch_span = tracing::debug_span!(
            "ocr.line_batch",
            model,
            rows = indices.len(),
            columns = padded
        )
        .entered();
        let active = indices.len() * geometry::LINE_HEIGHT * padded;
        trace.tensor(
            match model {
                "script" => "script-input",
                "mathcls" => "math-classifier-input",
                "rec_cjk" => "chinese-input",
                _ => "text-input",
            },
            &[
                indices.len() as i64,
                1,
                geometry::LINE_HEIGHT as i64,
                padded as i64,
            ],
            &batch.data_typed::<f32>()?[..active],
        )?;
        trace.tensor(
            "line-widths",
            &[indices.len() as i64],
            &widths[..indices.len()],
        )?;
        let mut trace_indices = [0_i64; MAX_BATCH];
        for (to, from) in trace_indices[..indices.len()].iter_mut().zip(indices) {
            *to = *from as i64;
        }
        trace.tensor(
            "line-indices",
            &[indices.len() as i64],
            &trace_indices[..indices.len()],
        )?;
        let x = TensorRef::from_array_view((
            [indices.len(), 1, geometry::LINE_HEIGHT, padded],
            &batch.data_typed::<f32>()?[..active],
        ))?;
        let w = TensorRef::from_array_view(([indices.len()], &widths[..indices.len()]))?;
        deadline(due)?;
        let outputs = session.run(ort::inputs![x, w])?;
        if outputs.len() != 3 {
            return Err(EngineError::Shape("recognizer output count"));
        }
        let (shape, best) = outputs[0].try_extract_tensor::<i64>()?;
        let (prob_shape, probabilities) = outputs[1].try_extract_tensor::<f32>()?;
        let (length_shape, lengths) = outputs[2].try_extract_tensor::<i64>()?;
        trace.tensor(
            if chinese { "chinese-ids" } else { "text-ids" },
            shape,
            best,
        )?;
        trace.tensor(
            if chinese {
                "chinese-confidences"
            } else {
                "text-confidences"
            },
            prob_shape,
            probabilities,
        )?;
        trace.tensor("ctc-lengths", length_shape, lengths)?;
        if shape.len() != 2
            || shape[0] != indices.len() as i64
            || shape[1] <= 0
            || shape[1] as usize > MAX_TOKENS
            || shape != prob_shape
            || lengths.len() != indices.len()
        {
            return Err(EngineError::Shape("recognizer output dimensions"));
        }
        let timesteps = shape[1] as usize;
        let stride = padded as f64 / timesteps as f64;
        for (slot, index) in indices.iter().enumerate() {
            let n = usize::try_from(lengths[slot])
                .map_err(|_| EngineError::Shape("negative CTC length"))?;
            if n > timesteps {
                return Err(EngineError::Shape("CTC length exceeds output"));
            }
            let ids = &best[slot * timesteps..slot * timesteps + n];
            let probs = &probabilities[slot * timesteps..slot * timesteps + n];
            let crop = geometry::crop_geometry(&boxes[*index], geometry::LINE_HEIGHT)?;
            let factor = crop.width as f64 / line_widths[*index] as f64;
            let target_ids = &mut lines.ids.data_typed_mut::<i32>()?
                [*index * MAX_TOKENS..(*index + 1) * MAX_TOKENS];
            let target_positions = &mut lines.positions.data_typed_mut::<f64>()?
                [*index * MAX_TOKENS..(*index + 1) * MAX_TOKENS];
            let mut peaks = [0.0_f32; MAX_TOKENS];
            let mut count = 0;
            let vocab = if chinese {
                &vocabulary::CHINESE[..]
            } else {
                &vocabulary::TEXT[..]
            };
            for t in 0..n {
                if ids[t] < 0
                    || ids[t] as usize > vocab.len()
                    || !probs[t].is_finite()
                    || !(0.0..=1.0).contains(&probs[t])
                {
                    return Err(EngineError::Shape("invalid CTC token/probability"));
                }
                if ids[t] == 0 || (t > 0 && ids[t] == ids[t - 1]) {
                    continue;
                }
                target_ids[count] = ids[t] as i32;
                target_positions[count] = f64::from(crop.x) + (t as f64 + 0.5) * stride * factor;
                peaks[count] = probs[t];
                count += 1;
            }
            let conf = if count == 0 {
                0.0
            } else {
                crate::order::sum32(&peaks[..count]) / count as f32
            };
            lines.metadata[*index].script = if chinese {
                Script::Chinese
            } else {
                Script::Text
            };
            lines.metadata[*index].confidence = conf;
            lines.metadata[*index].count = if conf >= 0.5 { count } else { 0 };
        }
        stats.batch_counts[indices.len() - 1] += 1;
        stats.valid_columns += widths[..indices.len()]
            .iter()
            .map(|w| *w as u64)
            .sum::<u64>();
        stats.padded_columns += (padded * indices.len()) as u64;
        deadline(due)?;
    }
    Ok(())
}

pub(crate) fn token(script: &Script, id: i32) -> Result<&'static str> {
    let vocabulary = match script {
        Script::Text => &vocabulary::TEXT[..],
        Script::Chinese => &vocabulary::CHINESE[..],
        Script::Math => return Err(EngineError::Shape("math token used as CTC token")),
    };
    usize::try_from(id - 1)
        .ok()
        .and_then(|id| vocabulary.get(id))
        .copied()
        .ok_or(EngineError::Shape("vocabulary token index"))
}

pub(crate) fn append(output: &mut String, text: &str) -> Result<()> {
    if output
        .len()
        .checked_add(text.len())
        .is_none_or(|n| n > crate::MAX_OUTPUT_BYTES)
    {
        return Err(EngineError::Limit("OCR output bytes"));
    }
    output.push_str(text);
    Ok(())
}

pub(crate) fn decode_tokens(script: &Script, ids: &[i32], output: &mut String) -> Result<()> {
    let chinese = *script == Script::Chinese;
    let mut pending_space = false;
    let start = output.len();
    for (index, id) in ids.iter().enumerate() {
        let text = token(script, *id)?;
        if chinese {
            for ch in text.chars() {
                if ch.is_whitespace() || matches!(ch, '\u{1c}'..='\u{1f}') {
                    pending_space = output.len() > start;
                } else {
                    if pending_space {
                        append(output, " ")?;
                        pending_space = false;
                    }
                    let mut bytes = [0; 4];
                    append(output, ch.encode_utf8(&mut bytes))?;
                }
            }
        } else {
            append(output, text)?;
            if text.len() > 2
                && text.starts_with('\\')
                && text.as_bytes()[1..].iter().all(u8::is_ascii_alphabetic)
                && ids.get(index + 1).is_some_and(|next| {
                    usize::try_from(*next - 1)
                        .ok()
                        .and_then(|id| vocabulary::TEXT_STARTS_LETTER.get(id))
                        .copied()
                        .unwrap_or(false)
                })
            {
                append(output, " ")?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_decode_separates_latex_commands_from_letters() {
        let alpha = vocabulary::TEXT
            .iter()
            .position(|s| *s == "\\alpha")
            .expect("alpha") as i32
            + 1;
        let x = vocabulary::TEXT.iter().position(|s| *s == "x").expect("x") as i32 + 1;
        let mut output = String::new();
        decode_tokens(&Script::Text, &[alpha, x], &mut output).expect("decode");
        assert_eq!(output, "\\alpha x");
    }
    #[test]
    fn invalid_tokens_are_errors() {
        assert!(token(&Script::Text, 0).is_err());
        assert!(token(&Script::Chinese, 100_000).is_err());
    }
}
