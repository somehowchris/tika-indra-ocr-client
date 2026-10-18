//! Optional attention decoder with owned, typed KV-cache tensors and bounded token histories.

use std::{path::Path, time::Instant};

use opencv::{core::Mat, prelude::*};
use ort::{
    environment::Environment,
    session::{Session, SessionInputValue, SessionOutputs},
    value::{Tensor, TensorRef},
};

use crate::{
    EngineConfig, EngineError, MAX_BATCH, MAX_LINES, Result,
    batching::{self, Plan, Policy},
    deadline,
    geometry::{self, Scratch},
    recognition::{self, Lines, Script},
    vocabulary,
};

const HEIGHT: usize = 64;
const MAX_MATH_TOKENS: usize = 320;

pub(crate) struct Reader {
    classifier: Session,
    encoder: Session,
    init: Session,
    step: Session,
    batch: Mat,
    sequences: [[i32; MAX_MATH_TOKENS]; MAX_BATCH],
    probabilities: [[f64; MAX_MATH_TOKENS]; MAX_BATCH],
    raw: String,
    clean: String,
}

impl Reader {
    pub fn load(environment: &Environment, path: &Path, config: &EngineConfig) -> Result<Self> {
        Ok(Self {
            classifier: crate::runtime::session(
                environment,
                path,
                "mathcls.onnx",
                config,
                &["x", "widths"],
                &["p"],
            )?,
            encoder: crate::runtime::session(
                environment,
                path,
                "math_enc.onnx",
                config,
                &["x", "widths"],
                &["memory", "mask"],
            )?,
            init: crate::runtime::session(
                environment,
                path,
                "math_dec_init.onnx",
                config,
                &["memory"],
                &["cross_k", "cross_v"],
            )?,
            step: crate::runtime::session(
                environment,
                path,
                "math_dec_step.onnx",
                config,
                &[
                    "token", "pos", "self_k", "self_v", "cross_k", "cross_v", "mask",
                ],
                &["logits", "new_k", "new_v"],
            )?,
            batch: geometry::floats(batching::MATH_COLUMNS * HEIGHT)?,
            sequences: [[0; MAX_MATH_TOKENS]; MAX_BATCH],
            probabilities: [[0.0; MAX_MATH_TOKENS]; MAX_BATCH],
            raw: String::with_capacity(16 * 1024),
            clean: String::with_capacity(16 * 1024),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn recognize(
        &mut self,
        gray: &Mat,
        boxes: &[[f32; 4]],
        text_widths: &[usize],
        max_batch: usize,
        scratch: &mut Scratch,
        text_batch: &mut Mat,
        lines: &mut Lines,
        due: Instant,
        trace: &mut crate::trace::Recorder,
    ) -> Result<()> {
        let mut flags = [false; MAX_LINES];
        recognition::classify(
            &mut self.classifier,
            gray,
            boxes,
            text_widths,
            max_batch,
            scratch,
            text_batch,
            &mut flags[..boxes.len()],
            due,
            trace,
            "mathcls",
        )?;
        self.read(
            gray,
            boxes,
            &flags[..boxes.len()],
            max_batch,
            scratch,
            lines,
            due,
            trace,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn read(
        &mut self,
        gray: &Mat,
        boxes: &[[f32; 4]],
        flags: &[bool],
        max_batch: usize,
        scratch: &mut Scratch,
        lines: &mut Lines,
        due: Instant,
        trace: &mut crate::trace::Recorder,
    ) -> Result<()> {
        let _span = tracing::info_span!(
            "ocr.math",
            model = "math",
            lines = flags.iter().filter(|v| **v).count()
        )
        .entered();
        let mut line_widths = [0_usize; MAX_LINES];
        recognition::widths(boxes, HEIGHT, &mut line_widths[..boxes.len()])?;
        let line_widths = &line_widths[..boxes.len()];
        let mut plan = Plan::new(line_widths, max_batch, Policy::Math, |i| flags[i])?;
        let mut widths = [0_i64; MAX_BATCH];
        while let Some(range) = plan.next(line_widths) {
            let indices = &plan.indices[range];
            let b = indices.len();
            let _batch_span = tracing::debug_span!("ocr.math_batch", rows = b).entered();
            let padded = recognition::fill_batch(
                gray,
                boxes,
                indices,
                HEIGHT,
                scratch,
                &mut self.batch,
                &mut widths,
                due,
            )?;
            trace.tensor(
                "math-input",
                &[b as i64, 1, HEIGHT as i64, padded as i64],
                &self.batch.data_typed::<f32>()?[..b * HEIGHT * padded],
            )?;
            trace.tensor("math-widths", &[b as i64], &widths[..b])?;
            let x = TensorRef::from_array_view((
                [b, 1, HEIGHT, padded],
                &self.batch.data_typed::<f32>()?[..b * HEIGHT * padded],
            ))?;
            let w = TensorRef::from_array_view(([b], &widths[..b]))?;
            deadline(due)?;
            let [memory, mask] = two(self.encoder.run(ort::inputs![x, w])?)?;
            let (memory_shape, memory_values) = memory.try_extract_tensor::<f32>()?;
            trace.tensor("math-memory", memory_shape, memory_values)?;
            let (mask_shape, mask_values) = mask.try_extract_tensor::<f32>()?;
            trace.tensor("math-memory-mask", mask_shape, mask_values)?;
            if memory_shape.len() != 3
                || memory_shape[0] != b as i64
                || memory_shape[2] != 192
                || memory_shape[1] <= 0
                || memory_shape[1] > 1024
                || mask_shape.as_ref() != [b as i64, memory_shape[1]]
            {
                return Err(EngineError::Shape("math encoder outputs"));
            }
            let memory_steps = memory_shape[1] as usize;
            let [cross_k, cross_v] = two(self.init.run(ort::inputs![&memory])?)?;
            cache_shape(&cross_k, b, memory_steps)?;
            cache_shape(&cross_v, b, memory_steps)?;
            let (s, v) = cross_k.try_extract_tensor::<f32>()?;
            trace.tensor("math-cross-k", s, v)?;
            let (s, v) = cross_v.try_extract_tensor::<f32>()?;
            trace.tensor("math-cross-v", s, v)?;
            let mut cache: Option<[Tensor<f32>; 2]> = None;
            let mut tokens = [1_i64; MAX_BATCH];
            let mut done = [false; MAX_BATCH];
            let mut lengths = [0_usize; MAX_BATCH];
            let mut probability_lengths = [0_usize; MAX_BATCH];
            for step in 0..MAX_MATH_TOKENS - 1 {
                deadline(due)?;
                let token = TensorRef::from_array_view(([b], &tokens[..b]))?;
                let scalar = [step as i64];
                let position = TensorRef::from_array_view(((), &scalar[..]))?;
                let self_cache: [SessionInputValue<'_>; 2] = match &cache {
                    Some([k, v]) => [k.into(), v.into()],
                    None => [
                        TensorRef::from_array_view(([3, b, 8, 0, 24], &[] as &[f32]))?.into(),
                        TensorRef::from_array_view(([3, b, 8, 0, 24], &[] as &[f32]))?.into(),
                    ],
                };
                let [self_k, self_v] = self_cache;
                let outputs = self.step.run(ort::inputs![
                    token, position, self_k, self_v, &cross_k, &cross_v, &mask
                ])?;
                let mut outputs = outputs.into_iter();
                let logits: Tensor<f32> = outputs
                    .next()
                    .ok_or(EngineError::Shape("missing math logits"))?
                    .1
                    .downcast()?;
                let new_k: Tensor<f32> = outputs
                    .next()
                    .ok_or(EngineError::Shape("missing math keys"))?
                    .1
                    .downcast()?;
                let new_v: Tensor<f32> = outputs
                    .next()
                    .ok_or(EngineError::Shape("missing math values"))?
                    .1
                    .downcast()?;
                if outputs.next().is_some() {
                    return Err(EngineError::Shape("excess math step outputs"));
                }
                cache_shape(&new_k, b, step + 1)?;
                cache_shape(&new_v, b, step + 1)?;
                // Moving the owned output tensors transfers cache ownership; model pixels are never copied here.
                cache = Some([new_k, new_v]);
                let (shape, values) = logits.try_extract_tensor::<f32>()?;
                trace.tensor("math-logits", shape, values)?;
                if shape.as_ref() != [b as i64, 652] {
                    return Err(EngineError::Shape("math logits shape"));
                }
                for row in 0..b {
                    let logits = &values[row * 652..(row + 1) * 652];
                    if !logits.iter().all(|v| v.is_finite()) {
                        return Err(EngineError::Shape("non-finite math logits"));
                    }
                    let (argmax, probability) = top_probability(logits);
                    tokens[row] = argmax as i64;
                    if done[row] {
                        continue;
                    }
                    let n = probability_lengths[row];
                    self.probabilities[row][n] = f64::from(probability);
                    probability_lengths[row] += 1;
                    if argmax == 2 || (n + 1 >= 8 && mean(&self.probabilities[row][..n + 1]) < 0.8)
                    {
                        done[row] = true;
                    } else if argmax >= 4 {
                        self.sequences[row][lengths[row]] = argmax as i32;
                        lengths[row] += 1;
                    }
                }
                trace.tensor("math-next-token", &[b as i64], &tokens[..b])?;
                trace.tensor("math-done", &[b as i64], &done[..b])?;
                if done[..b].iter().all(|v| *v) {
                    break;
                }
            }
            for (row, index) in indices.iter().enumerate() {
                let confidence = mean(&self.probabilities[row][..probability_lengths[row]]);
                if confidence < 0.9 {
                    continue;
                }
                self.raw.clear();
                self.clean.clear();
                detokenize(
                    &self.sequences[row][..lengths[row]],
                    &mut self.raw,
                    &mut self.clean,
                )?;
                if !self.clean.contains('$') {
                    continue;
                }
                let start = lines.math_text.len();
                recognition::append(&mut lines.math_text, &self.clean)?;
                lines.metadata[*index].script = Script::Math;
                lines.metadata[*index].count = 1;
                lines.metadata[*index].confidence = 1.0;
                lines.metadata[*index].math_start = start;
                lines.metadata[*index].math_end = lines.math_text.len();
            }
        }
        deadline(due)
    }
}

fn two(outputs: SessionOutputs<'_>) -> Result<[Tensor<f32>; 2]> {
    let mut values = outputs.into_iter();
    let a = values
        .next()
        .ok_or(EngineError::Shape("missing math tensor"))?
        .1
        .downcast()?;
    let b = values
        .next()
        .ok_or(EngineError::Shape("missing math tensor"))?
        .1
        .downcast()?;
    if values.next().is_some() {
        return Err(EngineError::Shape("excess math tensors"));
    }
    Ok([a, b])
}

fn cache_shape(value: &Tensor<f32>, batch: usize, steps: usize) -> Result<()> {
    let (shape, _) = value.try_extract_tensor::<f32>()?;
    if shape.as_ref() != [3, batch as i64, 8, steps as i64, 24] {
        return Err(EngineError::Shape("math key/value cache dimensions"));
    }
    Ok(())
}

fn top_probability(logits: &[f32]) -> (usize, f32) {
    let mut best = 0;
    for i in 1..logits.len() {
        if logits[i] > logits[best] {
            best = i;
        }
    }
    let mut exponentials = [0.0_f32; 652];
    for (target, value) in exponentials.iter_mut().zip(logits) {
        *target = (*value - logits[best]).exp();
    }
    (
        best,
        1.0 / crate::order::sum32(&exponentials[..logits.len()]),
    )
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn detokenize(ids: &[i32], raw: &mut String, clean: &mut String) -> Result<()> {
    let mut in_math = false;
    let mut opening = 0;
    let mut math_tokens = 0;
    for id in ids {
        let token = usize::try_from(*id - 4)
            .ok()
            .and_then(|i| vocabulary::MATH.get(i))
            .copied()
            .ok_or(EngineError::Shape("math vocabulary token"))?;
        if token == "$" {
            if !in_math {
                opening = raw.len();
                math_tokens = 0;
            }
            recognition::append(raw, "$")?;
            in_math = !in_math;
        } else {
            if in_math {
                if math_tokens > 0 {
                    recognition::append(raw, " ")?;
                }
                math_tokens += 1;
            }
            recognition::append(raw, token)?;
        }
    }
    if in_math {
        if math_tokens == 0 {
            raw.truncate(opening);
        } else {
            recognition::append(raw, "$")?;
        }
    }
    let mut space = false;
    for character in raw.chars() {
        if character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}') {
            space = !clean.is_empty();
        } else {
            if space {
                recognition::append(clean, " ")?;
                space = false;
            }
            let mut bytes = [0; 4];
            recognition::append(clean, character.encode_utf8(&mut bytes))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn softmax_is_stable_and_uses_first_tie() {
        let (i, p) = top_probability(&[1000.0, 1000.0, -1000.0]);
        assert_eq!(i, 0);
        assert_eq!(p, 0.5);
    }
    #[test]
    fn unmatched_math_opener_is_closed_only_if_it_has_content() {
        let dollar = vocabulary::MATH.iter().position(|v| *v == "$").expect("$") as i32 + 4;
        let x = vocabulary::MATH.iter().position(|v| *v == "x").expect("x") as i32 + 4;
        let mut raw = String::new();
        let mut clean = String::new();
        detokenize(&[dollar, x], &mut raw, &mut clean).expect("text");
        assert_eq!(clean, "$x$");
        raw.clear();
        clean.clear();
        detokenize(&[dollar], &mut raw, &mut clean).expect("empty");
        assert!(clean.is_empty());
    }
}
