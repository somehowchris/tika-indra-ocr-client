//! Opt-in same-age decoder experiment. Never called by the production pipeline.

use std::{
    fs::{self, File},
    io::{BufReader, Read},
    path::{Path, PathBuf},
    time::Instant,
};

use super::gather::{Movement, gather};
use super::{MAX_MATH_TOKENS, Reader, cache_shape, detokenize, mean, top_probability, two};
use crate::{Engine, EngineError, MAX_BATCH, Result, deadline};
use opencv::prelude::*;
use ort::{
    session::{Session, SessionInputValue},
    value::{Tensor, TensorRef},
};

/// Deliberately absent from EngineConfig and the HTTP service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathArm {
    Ordinary,
    Compact,
}

#[derive(Debug)]
pub struct MathReport {
    pub cohort_ns: u128,
    pub decoder_ns: u128,
    pub row_steps: usize,
    /// Cross K/V, self K/V, mask, token copies, respectively.
    pub gathered_bytes: [usize; 4],
    pub verified_gather_elements: usize,
    pub tensor_views: usize,
    pub live_sizes: [usize; MAX_BATCH],
    pub live_size_count: usize,
    pub stop_steps: [usize; MAX_BATCH],
    pub original_rows: usize,
    pub compared_float_elements: usize,
}

impl MathReport {
    const fn new() -> Self {
        Self {
            cohort_ns: 0,
            decoder_ns: 0,
            row_steps: 0,
            gathered_bytes: [0; 4],
            verified_gather_elements: 0,
            tensor_views: 0,
            live_sizes: [0; MAX_BATCH],
            live_size_count: 0,
            stop_steps: [0; MAX_BATCH],
            original_rows: 0,
            compared_float_elements: 0,
        }
    }
}

impl Engine {
    /// Replay the single math cohort in a trace from unchanged Engine::recognize.
    /// `gate=true` runs an independent ordinary stream alongside the selected
    /// candidate and verifies production snapshots. Gate I/O is NEVER timed.
    /// Without the gate this measures only encoder/init/decoder, not a page or Tika.
    pub fn math_compaction_experiment(
        &mut self,
        reference: &Path,
        arm: MathArm,
        gate: bool,
        due: Instant,
    ) -> Result<MathReport> {
        self.shared
            .math
            .as_mut()
            .ok_or(EngineError::Config("math experiment requires math=true"))?
            .experiment(reference, arm, gate, due)
    }
}

struct Reference {
    directory: PathBuf,
    input_index: usize,
}
impl Reference {
    fn open(directory: &Path) -> Result<Self> {
        let mut index = None;
        for item in fs::read_dir(directory)? {
            let item = item?;
            let name = item.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if let Some(prefix) = name.strip_suffix("-math-input.npy") {
                if index.is_some() {
                    return Err(EngineError::Shape(
                        "experiment requires exactly one math cohort",
                    ));
                }
                index = Some(
                    prefix
                        .parse::<usize>()
                        .map_err(|_| EngineError::Shape("math trace index"))?,
                );
            }
        }
        Ok(Self {
            directory: PathBuf::from(directory),
            input_index: index.ok_or(EngineError::Shape("missing math trace input"))?,
        })
    }
    fn path(&self, offset: usize, name: &str) -> PathBuf {
        self.directory
            .join(format!("{:06}-{name}.npy", self.input_index + offset))
    }
    fn reader(&self, offset: usize, name: &str, descriptor: &str, shape: &[usize]) -> Result<Npy> {
        let (reader, actual, rank) = Npy::open(&self.path(offset, name), descriptor)?;
        if &actual[..rank] != shape {
            return Err(EngineError::Shape("production trace shape"));
        }
        Ok(reader)
    }
    fn floats(&self, offset: usize, name: &str, shape: &[usize], actual: &[f32]) -> Result<()> {
        let mut reader = self.reader(offset, name, "<f4", shape)?;
        for (index, value) in actual.iter().enumerate() {
            let expected = f32::from_le_bytes(reader.bytes()?);
            near(*value, expected, name, index)?;
        }
        reader.end()
    }
}

struct Npy(BufReader<File>);
impl Npy {
    fn open(path: &Path, descriptor: &str) -> Result<(Self, [usize; 5], usize)> {
        let mut reader = Self(BufReader::new(File::open(path)?));
        if reader.bytes::<8>()? != *b"\x93NUMPY\x01\x00" {
            return Err(EngineError::Shape("NPY v1 magic"));
        }
        let length = u16::from_le_bytes(reader.bytes()?) as usize;
        let mut header = [0_u8; 512];
        let header = header
            .get_mut(..length)
            .ok_or(EngineError::Shape("NPY header limit"))?;
        reader.0.read_exact(header)?;
        let header =
            std::str::from_utf8(header).map_err(|_| EngineError::Shape("NPY header UTF-8"))?;
        let descriptor_field = header
            .split("'descr': '")
            .nth(1)
            .and_then(|tail| tail.split('\'').next());
        if descriptor_field != Some(descriptor) || !header.contains("'fortran_order': False") {
            return Err(EngineError::Shape("NPY type/order"));
        }
        let dimensions = header
            .split("'shape': (")
            .nth(1)
            .and_then(|tail| tail.split(')').next())
            .ok_or(EngineError::Shape("NPY dimensions"))?;
        let mut shape = [0; 5];
        let mut rank = 0;
        for dimension in dimensions
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            *shape
                .get_mut(rank)
                .ok_or(EngineError::Shape("NPY rank limit"))? = dimension
                .parse()
                .map_err(|_| EngineError::Shape("NPY dimension"))?;
            rank += 1;
        }
        Ok((reader, shape, rank))
    }
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut bytes = [0; N];
        self.0.read_exact(&mut bytes)?;
        Ok(bytes)
    }
    fn end(&mut self) -> Result<()> {
        if self.0.read(&mut [0])? != 0 {
            return Err(EngineError::Shape("NPY trailing elements"));
        }
        Ok(())
    }
}

struct State {
    mask: Tensor<f32>,
    cross: [Tensor<f32>; 2],
    cache: Option<[Tensor<f32>; 2]>,
    live: [usize; MAX_BATCH],
    batch: usize,
    original: usize,
    memory: usize,
    tokens: [i64; MAX_BATCH],
    done: [bool; MAX_BATCH],
    sequences: [[i32; MAX_MATH_TOKENS]; MAX_BATCH],
    probabilities: [[f64; MAX_MATH_TOKENS]; MAX_BATCH],
    lengths: [usize; MAX_BATCH],
    probability_lengths: [usize; MAX_BATCH],
    stop_steps: [usize; MAX_BATCH],
    stop_reasons: [u8; MAX_BATCH],
    report: MathReport,
}
impl State {
    fn new(mask: Tensor<f32>, cross: [Tensor<f32>; 2], batch: usize, memory: usize) -> Self {
        let mut state = Self {
            mask,
            cross,
            cache: None,
            live: std::array::from_fn(|index| index),
            batch,
            original: batch,
            memory,
            tokens: [1; MAX_BATCH],
            done: [false; MAX_BATCH],
            sequences: [[0; MAX_MATH_TOKENS]; MAX_BATCH],
            probabilities: [[0.0; MAX_MATH_TOKENS]; MAX_BATCH],
            lengths: [0; MAX_BATCH],
            probability_lengths: [0; MAX_BATCH],
            stop_steps: [0; MAX_BATCH],
            stop_reasons: [0; MAX_BATCH],
            report: MathReport::new(),
        };
        state.report.original_rows = batch;
        state.report.live_sizes[0] = batch;
        state.report.live_size_count = 1;
        state
    }
    fn run(&mut self, session: &mut Session, step: usize, compact: bool) -> Result<Tensor<f32>> {
        let b = self.batch;
        self.report.row_steps += b;
        let token = TensorRef::from_array_view(([b], &self.tokens[..b]))?;
        let scalar = [step as i64];
        let position = TensorRef::from_array_view(((), &scalar[..]))?;
        self.report.tensor_views += 2;
        let self_cache: [SessionInputValue<'_>; 2] = match &self.cache {
            Some([k, v]) if compact && k.shape()[1] != b as i64 => {
                self.report.tensor_views += 2;
                [view(k, b, step)?.into(), view(v, b, step)?.into()]
            }
            Some([k, v]) => [k.into(), v.into()],
            None => {
                self.report.tensor_views += 2;
                [
                    TensorRef::from_array_view(([3, b, 8, 0, 24], &[] as &[f32]))?.into(),
                    TensorRef::from_array_view(([3, b, 8, 0, 24], &[] as &[f32]))?.into(),
                ]
            }
        };
        let [self_k, self_v] = self_cache;
        let [cross_k, cross_v]: [SessionInputValue<'_>; 2] = if b != self.original {
            self.report.tensor_views += 2;
            [
                view(&self.cross[0], b, self.memory)?.into(),
                view(&self.cross[1], b, self.memory)?.into(),
            ]
        } else {
            [&self.cross[0], &self.cross[1]].map(Into::into)
        };
        let mask: SessionInputValue<'_> = if b != self.original {
            self.report.tensor_views += 1;
            TensorRef::from_array_view((
                [b, self.memory],
                &self.mask.try_extract_tensor::<f32>()?.1[..b * self.memory],
            ))?
            .into()
        } else {
            (&self.mask).into()
        };
        let outputs = session.run(ort::inputs![
            token, position, self_k, self_v, cross_k, cross_v, mask
        ])?;
        let mut outputs = outputs.into_iter();
        let logits: Tensor<f32> = outputs
            .next()
            .ok_or(EngineError::Shape("experiment logits missing"))?
            .1
            .downcast()?;
        let k: Tensor<f32> = outputs
            .next()
            .ok_or(EngineError::Shape("experiment self K missing"))?
            .1
            .downcast()?;
        let v: Tensor<f32> = outputs
            .next()
            .ok_or(EngineError::Shape("experiment self V missing"))?
            .1
            .downcast()?;
        if outputs.next().is_some() || logits.shape().as_ref() != [b as i64, 652] {
            return Err(EngineError::Shape("experiment decoder outputs"));
        }
        cache_shape(&k, b, step + 1)?;
        cache_shape(&v, b, step + 1)?;
        self.cache = Some([k, v]);
        Ok(logits)
    }
    fn advance(&mut self, logits: &Tensor<f32>, step: usize) -> Result<()> {
        let values = logits.try_extract_tensor::<f32>()?.1;
        for row in 0..self.batch {
            let original = self.live[row];
            let values = &values[row * 652..(row + 1) * 652];
            if !values.iter().all(|value| value.is_finite()) {
                return Err(EngineError::Shape("non-finite experimental logits"));
            }
            let (token, probability) = top_probability(values);
            self.tokens[row] = token as i64;
            if self.done[original] {
                continue;
            }
            let n = self.probability_lengths[original];
            self.probabilities[original][n] = f64::from(probability);
            self.probability_lengths[original] += 1;
            let reason = if token == 2 {
                1
            } else if n + 1 >= 8 && mean(&self.probabilities[original][..n + 1]) < 0.8 {
                2
            } else {
                0
            };
            if reason != 0 {
                self.done[original] = true;
                self.stop_steps[original] = step + 1;
                self.stop_reasons[original] = reason;
            } else if token >= 4 {
                self.sequences[original][self.lengths[original]] = token as i32;
                self.lengths[original] += 1;
            }
        }
        Ok(())
    }
    fn compact<const VERIFY: bool>(&mut self, steps: usize) -> Result<()> {
        let mut rows = [0; MAX_BATCH];
        let mut count = 0;
        for row in 0..self.batch {
            if !self.done[self.live[row]] {
                rows[count] = row;
                count += 1;
            }
        }
        if count == self.batch {
            return Ok(());
        }
        if count == 0 {
            self.batch = 0;
            return Ok(());
        }
        let rows = &rows[..count];
        let mut add = |kind: usize, movement: Movement| {
            self.report.gathered_bytes[kind] += movement.bytes;
            self.report.verified_gather_elements += movement.verified_elements;
        };
        for cross in &mut self.cross {
            add(
                0,
                gather::<_, VERIFY>(
                    cross.try_extract_tensor_mut::<f32>()?.1,
                    3,
                    self.batch,
                    rows,
                    8 * self.memory * 24,
                )
                .map_err(EngineError::Shape)?,
            );
        }
        for cache in self
            .cache
            .as_mut()
            .ok_or(EngineError::Shape("missing compaction cache"))?
        {
            add(
                1,
                gather::<_, VERIFY>(
                    cache.try_extract_tensor_mut::<f32>()?.1,
                    3,
                    self.batch,
                    rows,
                    8 * steps * 24,
                )
                .map_err(EngineError::Shape)?,
            );
        }
        add(
            2,
            gather::<_, VERIFY>(
                self.mask.try_extract_tensor_mut::<f32>()?.1,
                1,
                self.batch,
                rows,
                self.memory,
            )
            .map_err(EngineError::Shape)?,
        );
        add(
            3,
            gather::<_, VERIFY>(&mut self.tokens, 1, self.batch, rows, 1)
                .map_err(EngineError::Shape)?,
        );
        for (target, source) in rows.iter().enumerate() {
            self.live[target] = self.live[*source];
        }
        self.batch = count;
        self.report.live_sizes[self.report.live_size_count] = count;
        self.report.live_size_count += 1;
        Ok(())
    }
}

fn view(tensor: &Tensor<f32>, batch: usize, steps: usize) -> Result<TensorRef<'_, f32>> {
    let count = 3 * batch * 8 * steps * 24;
    Ok(TensorRef::from_array_view((
        [3, batch, 8, steps, 24],
        &tensor.try_extract_tensor::<f32>()?.1[..count],
    ))?)
}

impl Reader {
    fn load_input(&mut self, reference: &Reference) -> Result<(usize, usize, [i64; MAX_BATCH])> {
        let (mut input, shape, rank) = Npy::open(&reference.path(0, "math-input"), "<f4")?;
        if rank != 4 || shape[0] != 6 || shape[1] != 1 || shape[2] != 64 || shape[3] == 0 {
            return Err(EngineError::Shape("required six-row math input"));
        }
        let b = shape[0];
        let count = b
            .checked_mul(64)
            .and_then(|n| n.checked_mul(shape[3]))
            .ok_or(EngineError::Shape("math input count"))?;
        let values = self
            .batch
            .data_typed_mut::<f32>()?
            .get_mut(..count)
            .ok_or(EngineError::Limit("math input arena"))?;
        for value in values.iter_mut() {
            *value = f32::from_le_bytes(input.bytes()?);
        }
        input.end()?;
        let mut widths = [0_i64; MAX_BATCH];
        let mut source = reference.reader(1, "math-widths", "<i8", &[b])?;
        for width in &mut widths[..b] {
            *width = i64::from_le_bytes(source.bytes()?);
        }
        source.end()?;
        Ok((b, shape[3], widths))
    }
    fn prepare(
        &mut self,
        reference: &Reference,
        b: usize,
        padded: usize,
        widths: &[i64; MAX_BATCH],
        gate: bool,
        due: Instant,
    ) -> Result<State> {
        deadline(due)?;
        let x = TensorRef::from_array_view((
            [b, 1, 64, padded],
            &self.batch.data_typed::<f32>()?[..b * 64 * padded],
        ))?;
        let widths = TensorRef::from_array_view(([b], &widths[..b]))?;
        let [memory, mask] = two(self.encoder.run(ort::inputs![x, widths])?)?;
        let (shape, data) = memory.try_extract_tensor::<f32>()?;
        if shape.len() != 3
            || shape[0] != b as i64
            || shape[2] != 192
            || !(1..=1024).contains(&shape[1])
            || mask.shape().as_ref() != [b as i64, shape[1]]
        {
            return Err(EngineError::Shape("math experiment encoder outputs"));
        }
        let m = shape[1] as usize;
        if gate {
            reference.floats(2, "math-memory", &[b, m, 192], data)?;
            reference.floats(
                3,
                "math-memory-mask",
                &[b, m],
                mask.try_extract_tensor::<f32>()?.1,
            )?;
        }
        let cross = two(self.init.run(ort::inputs![&memory])?)?;
        for (index, tensor) in cross.iter().enumerate() {
            cache_shape(tensor, b, m)?;
            if gate {
                reference.floats(
                    4 + index,
                    if index == 0 {
                        "math-cross-k"
                    } else {
                        "math-cross-v"
                    },
                    &[3, b, 8, m, 24],
                    tensor.try_extract_tensor::<f32>()?.1,
                )?;
            }
        }
        let mut state = State::new(mask, cross, b, m);
        state.report.tensor_views = 2;
        Ok(state)
    }
    fn experiment(
        &mut self,
        path: &Path,
        arm: MathArm,
        gate: bool,
        due: Instant,
    ) -> Result<MathReport> {
        let reference = Reference::open(path)?;
        // All trace I/O and input preparation precedes the measured cohort.
        let (b, padded, widths) = self.load_input(&reference)?;
        let mut ordinary = if gate {
            Some(self.prepare(&reference, b, padded, &widths, true, due)?)
        } else {
            None
        };
        let start = Instant::now();
        let mut candidate = self.prepare(&reference, b, padded, &widths, gate, due)?;
        let decoder = Instant::now();
        for step in 0..MAX_MATH_TOKENS - 1 {
            deadline(due)?;
            if gate {
                eprintln!(
                    "math_gate step={} live_original_rows={:?}",
                    step + 1,
                    &candidate.live[..candidate.batch]
                );
            }
            if let Some(ordinary) = &ordinary {
                candidate.report.compared_float_elements +=
                    compare_state(&candidate, ordinary, step, true)?;
            }
            let logits = candidate.run(&mut self.step, step, arm == MathArm::Compact)?;
            candidate.advance(&logits, step)?;
            if let Some(ordinary) = &mut ordinary {
                let expected = ordinary.run(&mut self.step, step, false)?;
                ordinary.advance(&expected, step)?;
                reference.floats(
                    6 + step * 3,
                    "math-logits",
                    &[ordinary.batch, 652],
                    expected.try_extract_tensor::<f32>()?.1,
                )?;
                let mut tokens =
                    reference.reader(7 + step * 3, "math-next-token", "<i8", &[ordinary.batch])?;
                let mut done =
                    reference.reader(8 + step * 3, "math-done", "|b1", &[ordinary.batch])?;
                for row in 0..ordinary.batch {
                    if i64::from_le_bytes(tokens.bytes()?) != ordinary.tokens[row]
                        || done.bytes::<1>()?[0] != u8::from(ordinary.done[row])
                    {
                        return Err(EngineError::Shape(
                            "ordinary replay differs from production decisions",
                        ));
                    }
                }
                tokens.end()?;
                done.end()?;
                compare_logits(&mut candidate, ordinary, &logits, &expected)?;
                candidate.report.compared_float_elements +=
                    compare_state(&candidate, ordinary, step + 1, false)?;
                compare_histories(&candidate, ordinary)?;
            }
            if candidate.done[..candidate.original]
                .iter()
                .all(|done| *done)
            {
                break;
            }
            if arm == MathArm::Compact {
                if gate {
                    candidate.compact::<true>(step + 1)?;
                } else {
                    candidate.compact::<false>(step + 1)?;
                }
            }
        }
        candidate.report.decoder_ns = decoder.elapsed().as_nanos();
        candidate.report.cohort_ns = start.elapsed().as_nanos();
        candidate.report.stop_steps = candidate.stop_steps;
        if candidate.stop_steps[..6] != [8, 32, 8, 27, 60, 23] {
            return Err(EngineError::Shape(
                "required six-row stop trajectory changed",
            ));
        }
        if arm == MathArm::Compact
            && candidate.report.live_sizes[..candidate.report.live_size_count] != [6, 4, 3, 2, 1]
        {
            return Err(EngineError::Shape(
                "required actual compaction trajectory absent",
            ));
        }
        if let Some(ordinary) = &ordinary {
            compare_decoded(&candidate, ordinary, &reference)?;
        }
        Ok(candidate.report)
    }
}

fn near(actual: f32, expected: f32, stage: &str, index: usize) -> Result<()> {
    if !actual.is_finite()
        || !expected.is_finite()
        || (actual - expected).abs() > 1e-5 + 1e-5 * expected.abs()
    {
        eprintln!(
            "math parity failure stage={stage} element={index} actual={actual:?} expected={expected:?}"
        );
        return Err(EngineError::Shape("math compaction numerical parity"));
    }
    Ok(())
}
fn mapped(
    actual: &[f32],
    expected: &[f32],
    candidate: &State,
    ordinary: &State,
    layers: usize,
    block: usize,
    stage: &str,
) -> Result<usize> {
    for layer in 0..layers {
        for row in 0..candidate.batch {
            let original = candidate.live[row];
            let a = (layer * candidate.batch + row) * block;
            let b = (layer * ordinary.batch + original) * block;
            for element in 0..block {
                near(
                    actual[a + element],
                    expected[b + element],
                    stage,
                    a + element,
                )?;
            }
        }
    }
    Ok(layers * candidate.batch * block)
}
fn compare_state(candidate: &State, ordinary: &State, steps: usize, before: bool) -> Result<usize> {
    let mut compared = mapped(
        candidate.mask.try_extract_tensor::<f32>()?.1,
        ordinary.mask.try_extract_tensor::<f32>()?.1,
        candidate,
        ordinary,
        1,
        candidate.memory,
        "mask",
    )?;
    for index in 0..2 {
        compared += mapped(
            candidate.cross[index].try_extract_tensor::<f32>()?.1,
            ordinary.cross[index].try_extract_tensor::<f32>()?.1,
            candidate,
            ordinary,
            3,
            8 * candidate.memory * 24,
            "cross cache",
        )?;
    }
    match (&candidate.cache, &ordinary.cache) {
        (Some(actual), Some(expected)) => {
            for index in 0..2 {
                compared += mapped(
                    actual[index].try_extract_tensor::<f32>()?.1,
                    expected[index].try_extract_tensor::<f32>()?.1,
                    candidate,
                    ordinary,
                    3,
                    8 * steps * 24,
                    if before { "self input" } else { "self output" },
                )?;
            }
        }
        (None, None) if steps == 0 => {}
        _ => return Err(EngineError::Shape("math cache availability")),
    }
    Ok(compared)
}
fn compare_logits(
    candidate: &mut State,
    ordinary: &State,
    actual: &Tensor<f32>,
    expected: &Tensor<f32>,
) -> Result<()> {
    candidate.report.compared_float_elements += mapped(
        actual.try_extract_tensor::<f32>()?.1,
        expected.try_extract_tensor::<f32>()?.1,
        candidate,
        ordinary,
        1,
        652,
        "logits",
    )?;
    for row in 0..candidate.batch {
        if candidate.tokens[row] != ordinary.tokens[candidate.live[row]] {
            return Err(EngineError::Shape("mapped selected token"));
        }
    }
    Ok(())
}
fn compare_histories(candidate: &State, ordinary: &State) -> Result<()> {
    for row in 0..candidate.original {
        let n = candidate.lengths[row];
        let p = candidate.probability_lengths[row];
        if n != ordinary.lengths[row]
            || p != ordinary.probability_lengths[row]
            || candidate.sequences[row][..n] != ordinary.sequences[row][..n]
            || candidate.done[row] != ordinary.done[row]
            || candidate.stop_steps[row] != ordinary.stop_steps[row]
            || candidate.stop_reasons[row] != ordinary.stop_reasons[row]
        {
            return Err(EngineError::Shape("mapped history/stop reason/step"));
        }
        for index in 0..p {
            near(
                candidate.probabilities[row][index] as f32,
                ordinary.probabilities[row][index] as f32,
                "probability history",
                index,
            )?;
        }
    }
    Ok(())
}
fn compare_decoded(candidate: &State, ordinary: &State, reference: &Reference) -> Result<()> {
    let mut actual = String::with_capacity(16 * 1024);
    let mut expected = String::with_capacity(16 * 1024);
    let mut raw_actual = String::with_capacity(16 * 1024);
    let mut raw_expected = String::with_capacity(16 * 1024);
    for row in 0..candidate.original {
        actual.clear();
        expected.clear();
        raw_actual.clear();
        raw_expected.clear();
        detokenize(
            &candidate.sequences[row][..candidate.lengths[row]],
            &mut raw_actual,
            &mut actual,
        )?;
        detokenize(
            &ordinary.sequences[row][..ordinary.lengths[row]],
            &mut raw_expected,
            &mut expected,
        )?;
        let accepted = if mean(&candidate.probabilities[row][..candidate.probability_lengths[row]])
            >= 0.9
            && actual.contains('$')
        {
            actual.as_bytes()
        } else {
            &[]
        };
        let offset = 6
            + ordinary.stop_steps[..ordinary.original]
                .iter()
                .copied()
                .max()
                .unwrap_or(0)
                * 3
            + row * 2;
        let mut identity = reference.reader(offset, "math-final-row", "<i8", &[1])?;
        if i64::from_le_bytes(identity.bytes()?) != row as i64 {
            return Err(EngineError::Shape("production final original row"));
        }
        identity.end()?;
        let mut production =
            reference.reader(offset + 1, "math-final-text", "|u1", &[accepted.len()])?;
        for byte in accepted {
            if production.bytes::<1>()?[0] != *byte {
                return Err(EngineError::Shape("production final accepted UTF-8"));
            }
        }
        production.end()?;
        if actual != expected
            || raw_actual != raw_expected
            || (mean(&candidate.probabilities[row][..candidate.probability_lengths[row]]) >= 0.9)
                != (mean(&ordinary.probabilities[row][..ordinary.probability_lengths[row]]) >= 0.9)
        {
            return Err(EngineError::Shape(
                "mapped decoded UTF-8/confidence decision",
            ));
        }
    }
    Ok(())
}

/// Actual final accepted row text from the unchanged production decision path.
/// Feature-only diagnostics are appended after the last ordinary decoder step.
pub(super) fn production_output(
    trace: &mut crate::trace::Recorder,
    row: usize,
    text: &str,
) -> Result<()> {
    trace.tensor("math-final-row", &[1], &[row as i64])?;
    trace.tensor("math-final-text", &[text.len() as i64], text.as_bytes())
}
