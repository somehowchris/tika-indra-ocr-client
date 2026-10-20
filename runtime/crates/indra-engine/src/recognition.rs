//! Script routing, CTC decoding and language-model rescoring on the exact FP32 snapshot.
//!
//! Port of the `recognize` and `line_groups` methods of `indra_ocr/pipeline.py`
//! at upstream commit `0e96b93c6810f17cf8352acbb4487cc9e45b66d5` (Apache-2.0).
//! Every line is classified into one of four script groups. A line goes to its
//! group recogniser and, when the classifier is not sure, to the base recogniser
//! as well; the reading with the higher mean confidence wins. The base
//! recogniser reads a tall window around the line, group recognisers the line
//! band. Unsure readings of a recogniser with a language model are re-decoded
//! by the beam search in `lm.rs`.

use std::time::Instant;

use opencv::{
    core::{self, Mat, Scalar},
    prelude::*,
};
use ort::{session::Session, value::TensorRef};

use crate::{
    EngineError, MAX_BATCH, MAX_LINES, OcrStats, Result,
    batching::{Plan, Policy, tensor_widths},
    deadline,
    geometry::{self, Scratch},
    lm, vocabulary,
};

pub(crate) const MAX_TOKENS: usize = geometry::MAX_WIDTH / 8;
/// A reading under this mean peak probability leaves the line empty.
pub(crate) const MIN_CONF: f32 = 0.5;

/// Recogniser vocabularies. The first four are `script.onnx`'s classes in
/// `meta.json` `script_groups` order; math lines come from the math reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Script {
    Base,
    Cjk,
    Rtl,
    World,
    Math,
}

impl Script {
    pub const GROUPS: [Self; 4] = [Self::Base, Self::Cjk, Self::Rtl, Self::World];

    pub const fn vocabulary(self) -> &'static [&'static str] {
        match self {
            Self::Base => &vocabulary::TEXT,
            Self::Cjk => &vocabulary::CJK,
            Self::Rtl => &vocabulary::RTL,
            Self::World => &vocabulary::WORLD,
            Self::Math => &[],
        }
    }

    pub const fn model(self) -> &'static str {
        match self {
            Self::Base => "rec",
            Self::Cjk => "rec_cjk",
            Self::Rtl => "rec_rtl",
            Self::World => "rec_world",
            Self::Math => "math",
        }
    }

    /// Group recognisers emit characters; the base recogniser emits vocabulary
    /// ids whose decoding knows LaTeX commands.
    pub const fn as_chars(self) -> bool {
        matches!(self, Self::Cjk | Self::Rtl | Self::World)
    }

    /// Line bands stacked in the recogniser input.
    pub const fn window(self) -> usize {
        match self {
            Self::Base => geometry::WINDOW,
            _ => 1,
        }
    }

    const fn trace_prefix(self) -> &'static str {
        match self {
            Self::Base => "text",
            Self::Cjk => "chinese",
            Self::Rtl => "rtl",
            Self::World => "world",
            Self::Math => "math",
        }
    }

    pub const fn input_name(self) -> &'static str {
        match self {
            Self::Base => "text-input",
            Self::Cjk => "chinese-input",
            Self::Rtl => "rtl-input",
            Self::World => "world-input",
            Self::Math => "math-input",
        }
    }
}

pub(crate) struct Line {
    pub script: Script,
    pub count: usize,
    pub confidence: f32,
    /// The best mean confidence any recogniser reached for this line; a later
    /// reading replaces the stored one only when it is strictly higher.
    pub best: f32,
    pub rescored: bool,
    pub math_start: usize,
    pub math_end: usize,
}

impl Line {
    pub const fn empty() -> Self {
        Self {
            script: Script::Base,
            count: 0,
            confidence: 0.0,
            best: -1.0,
            rescored: false,
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

/// Per-page line facts decided before any recogniser runs.
pub(crate) struct Routing {
    pub group: [Script; MAX_LINES],
    pub sure: [bool; MAX_LINES],
    pub dark: [bool; MAX_LINES],
    /// Recogniser input widths of the one-band crop; batches group by these.
    pub band: [usize; MAX_LINES],
    /// Input widths of the base recogniser's tall window.
    pub tall: [usize; MAX_LINES],
}

impl Routing {
    pub const fn new() -> Self {
        Self {
            group: [Script::Base; MAX_LINES],
            sure: [true; MAX_LINES],
            dark: [false; MAX_LINES],
            band: [0; MAX_LINES],
            tall: [0; MAX_LINES],
        }
    }

    /// Crop widths and light-on-dark decisions for every detected box.
    pub fn prepare(&mut self, gray: &Mat, boxes: &[[f32; 4]]) -> Result<()> {
        if boxes.len() > MAX_LINES {
            return Err(EngineError::Limit("detected lines"));
        }
        for (index, b) in boxes.iter().enumerate() {
            self.band[index] = geometry::crop_geometry(b, geometry::LINE_HEIGHT, 1)?.out_width;
            self.tall[index] =
                geometry::crop_geometry(b, geometry::LINE_HEIGHT, geometry::WINDOW)?.out_width;
            self.dark[index] = geometry::is_dark_line(gray, b)?;
            self.group[index] = Script::Base;
            self.sure[index] = true;
        }
        Ok(())
    }

    /// A vertical page: every line is a CJK column, read once.
    pub fn vertical(&mut self, count: usize) {
        self.group[..count].fill(Script::Cjk);
        self.sure[..count].fill(true);
    }

    /// `line_groups`: the group with the highest class probability, and
    /// whether that probability passes `route_sure`. Returns how many lines
    /// the base recogniser reads in addition to their own group.
    pub fn route(&mut self, probabilities: &[f32], count: usize, route_sure: f64) -> Result<u32> {
        if probabilities.len() != count * Script::GROUPS.len() {
            return Err(EngineError::Shape("script classifier output"));
        }
        let mut dual = 0;
        for (index, row) in probabilities.as_chunks::<4>().0.iter().enumerate() {
            dual += u32::from(self.route_line(index, row, route_sure)?);
        }
        Ok(dual)
    }

    /// Route one line from its four class probabilities; true when the base
    /// recogniser reads it in addition to its own group.
    pub fn route_line(&mut self, index: usize, row: &[f32; 4], route_sure: f64) -> Result<bool> {
        if !row.iter().all(|p| p.is_finite()) {
            return Err(EngineError::Shape("script classifier probabilities"));
        }
        let mut best = 0;
        for k in 1..row.len() {
            if row[k] > row[best] {
                best = k;
            }
        }
        self.group[index] = Script::GROUPS[best];
        self.sure[index] = f64::from(row[best]) > route_sure;
        Ok(self.group[index] != Script::Base && !self.sure[index])
    }

    pub fn reads(&self, group: Script, index: usize) -> bool {
        match group {
            Script::Base => self.group[index] == Script::Base || !self.sure[index],
            other => self.group[index] == other,
        }
    }
}

pub(crate) fn widths(boxes: &[[f32; 4]], height: usize, out: &mut [usize]) -> Result<()> {
    for (b, w) in boxes.iter().zip(out) {
        *w = geometry::crop_geometry(b, height, 1)?.out_width;
    }
    Ok(())
}

/// The base recogniser's arena grows only when a batch of tall windows needs
/// more than the startup allocation; the bound is 64 rows of 4,096 columns.
pub(crate) fn reserve_window(batch: &mut Mat, indices: &[usize], routing: &Routing) -> Result<()> {
    let widest = indices
        .iter()
        .map(|index| routing.tall[*index])
        .max()
        .unwrap_or(0);
    let needed = indices.len() * geometry::WINDOW_HEIGHT * widest.div_ceil(8) * 8;
    if batch.total() < needed {
        tracing::warn!(
            floats = needed,
            "tall-window tensor arena grows to a new high-water mark"
        );
        *batch = geometry::floats(needed)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn fill_batch(
    gray: &Mat,
    boxes: &[[f32; 4]],
    indices: &[usize],
    height: usize,
    window: usize,
    dark: &[bool],
    scratch: &mut Scratch,
    batch: &mut Mat,
    widths: &mut [i64; MAX_BATCH],
    due: Instant,
) -> Result<usize> {
    let rows = height * window;
    let data = batch.data_typed_mut::<f32>()?;
    let plan = tensor_widths(
        indices.iter().map(|index| {
            geometry::crop_geometry(&boxes[*index], height, window).map(|crop| crop.out_width)
        }),
        rows,
        data.len(),
        widths,
    )?;
    let padded = plan.width() as usize;
    let area = rows
        .checked_mul(plan.work() as usize)
        .ok_or(EngineError::Limit("recognition tensor scratch"))?;
    let data = data
        .get_mut(..area)
        .ok_or(EngineError::Limit("recognition tensor scratch"))?;
    data.fill(0.0);
    for (slot, index) in indices.iter().enumerate() {
        deadline(due)?;
        let crop = geometry::crop_geometry(&boxes[*index], height, window)?;
        scratch.line_input(
            gray,
            &crop,
            dark[*index],
            &mut data[slot * rows * padded..(slot + 1) * rows * padded],
            padded,
        )?;
    }
    Ok(padded)
}

fn trace_batch(
    trace: &mut crate::trace::Recorder,
    name: &'static str,
    batch: &Mat,
    indices: &[usize],
    rows: usize,
    padded: usize,
    widths: &[i64],
) -> Result<()> {
    let active = indices.len() * rows * padded;
    trace.tensor(
        name,
        &[indices.len() as i64, 1, rows as i64, padded as i64],
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
    )
}

/// Run a line classifier (`script.onnx`, `mathcls.onnx`) on one-band crops and
/// scatter its `classes` probabilities per line into `probabilities`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn classify(
    session: &mut Session,
    gray: &Mat,
    boxes: &[[f32; 4]],
    routing: &Routing,
    max_batch: usize,
    scratch: &mut Scratch,
    batch: &mut Mat,
    classes: usize,
    probabilities: &mut [f32],
    due: Instant,
    trace: &mut crate::trace::Recorder,
    model: &'static str,
) -> Result<()> {
    let _span = tracing::info_span!("ocr.classifier", model, lines = boxes.len()).entered();
    let line_widths = &routing.band[..boxes.len()];
    if probabilities.len() < boxes.len() * classes {
        return Err(EngineError::Shape("classifier output capacity"));
    }
    let mut plan = Plan::new(line_widths, max_batch, Policy::Text, |_| true)?;
    let mut widths = [0_i64; MAX_BATCH];
    while let Some(range) = plan.next(line_widths) {
        let indices = &plan.indices[range];
        let padded = fill_batch(
            gray,
            boxes,
            indices,
            geometry::LINE_HEIGHT,
            1,
            &routing.dark,
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
        trace_batch(
            trace,
            if model == "script" {
                "script-input"
            } else {
                "math-classifier-input"
            },
            batch,
            indices,
            geometry::LINE_HEIGHT,
            padded,
            &widths,
        )?;
        let active = indices.len() * geometry::LINE_HEIGHT * padded;
        let x = TensorRef::from_array_view((
            [indices.len(), 1, geometry::LINE_HEIGHT, padded],
            &batch.data_typed::<f32>()?[..active],
        ))?;
        let w = TensorRef::from_array_view(([indices.len()], &widths[..indices.len()]))?;
        deadline(due)?;
        let outputs = session.run(ort::inputs![x, w])?;
        let (shape, values) = outputs[0].try_extract_tensor::<f32>()?;
        trace.tensor(
            if model == "script" {
                "script-probabilities"
            } else {
                "math-classifier-probabilities"
            },
            shape,
            values,
        )?;
        if values.len() != indices.len() * classes || !values.iter().all(|v| v.is_finite()) {
            return Err(EngineError::Shape("line classifier output"));
        }
        for (slot, index) in indices.iter().enumerate() {
            probabilities[index * classes..(index + 1) * classes]
                .copy_from_slice(&values[slot * classes..(slot + 1) * classes]);
        }
        deadline(due)?;
    }
    Ok(())
}

/// The language model of one recogniser and its tuned weights.
pub(crate) struct Rescorer<'a> {
    pub model: &'a lm::Model,
    pub params: lm::Params,
}

/// Read every selected line with one recogniser and keep the better reading.
#[allow(clippy::too_many_arguments)]
pub(crate) fn text(
    session: &mut Session,
    group: Script,
    rescorer: Option<Rescorer<'_>>,
    beam: &mut lm::Scratch,
    gray: &Mat,
    boxes: &[[f32; 4]],
    routing: &Routing,
    max_batch: usize,
    scratch: &mut Scratch,
    batch: &mut Mat,
    lines: &mut Lines,
    stats: &mut OcrStats,
    due: Instant,
    trace: &mut crate::trace::Recorder,
) -> Result<()> {
    let model = group.model();
    let window = group.window();
    let rows = geometry::LINE_HEIGHT * window;
    let _span = tracing::info_span!("ocr.recognition", model, lines = boxes.len()).entered();
    let line_widths = &routing.band[..boxes.len()];
    let mut plan = Plan::new(line_widths, max_batch, Policy::Text, |index| {
        routing.reads(group, index)
    })?;
    let mut widths = [0_i64; MAX_BATCH];
    while let Some(range) = plan.next(line_widths) {
        let indices = &plan.indices[range];
        if window > 1 {
            reserve_window(batch, indices, routing)?;
        }
        let padded = fill_batch(
            gray,
            boxes,
            indices,
            geometry::LINE_HEIGHT,
            window,
            &routing.dark,
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
        trace_batch(
            trace,
            group.input_name(),
            batch,
            indices,
            rows,
            padded,
            &widths,
        )?;
        let active = indices.len() * rows * padded;
        let x = TensorRef::from_array_view((
            [indices.len(), 1, rows, padded],
            &batch.data_typed::<f32>()?[..active],
        ))?;
        let w = TensorRef::from_array_view(([indices.len()], &widths[..indices.len()]))?;
        deadline(due)?;
        let outputs = session.run(ort::inputs![x, w])?;
        let output = Output::extract(&outputs, indices.len())?;
        output.trace(trace, group, rescorer.is_some())?;
        for (slot, index) in indices.iter().enumerate() {
            let n = output.length(slot)?;
            let timesteps = output.timesteps;
            let lm = rescorer.as_ref().map(|rescorer| LmRow {
                blank: &output.blank[slot * timesteps..slot * timesteps + n],
                topv: &output.topv
                    [slot * timesteps * lm::TOP_K..(slot * timesteps + n) * lm::TOP_K],
                topi: &output.topi
                    [slot * timesteps * lm::TOP_K..(slot * timesteps + n) * lm::TOP_K],
                model: rescorer.model,
                params: rescorer.params,
            });
            decode_ctc(
                CtcRow {
                    ids: &output.ids[slot * timesteps..slot * timesteps + n],
                    probabilities: &output.confidences[slot * timesteps..slot * timesteps + n],
                    timesteps,
                    padded,
                    script: group,
                    lm,
                },
                &boxes[*index],
                line_widths[*index],
                lines,
                *index,
                beam,
                stats,
            )?;
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

/// Borrowed recogniser outputs: greedy ids, peak probabilities, lengths and the
/// per-step blank and top-k class log-probabilities for the beam search.
pub(crate) struct Output<'a> {
    pub ids: &'a [i64],
    pub confidences: &'a [f32],
    pub lengths: &'a [i64],
    pub blank: &'a [f32],
    pub topv: &'a [f32],
    pub topi: &'a [i64],
    pub timesteps: usize,
}

impl<'a> Output<'a> {
    pub fn extract(outputs: &'a ort::session::SessionOutputs<'_>, rows: usize) -> Result<Self> {
        if outputs.len() != 6 {
            return Err(EngineError::Shape("recognizer output count"));
        }
        let (shape, ids) = outputs[0].try_extract_tensor::<i64>()?;
        let (prob_shape, confidences) = outputs[1].try_extract_tensor::<f32>()?;
        let (length_shape, lengths) = outputs[2].try_extract_tensor::<i64>()?;
        let (blank_shape, blank) = outputs[3].try_extract_tensor::<f32>()?;
        let (topv_shape, topv) = outputs[4].try_extract_tensor::<f32>()?;
        let (topi_shape, topi) = outputs[5].try_extract_tensor::<i64>()?;
        if shape.len() != 2
            || shape[0] != rows as i64
            || shape[1] <= 0
            || shape[1] as usize > MAX_TOKENS
            || shape != prob_shape
            || shape != blank_shape
            || length_shape.as_ref() != [rows as i64]
            || topv_shape.as_ref() != [rows as i64, shape[1], lm::TOP_K as i64]
            || topi_shape != topv_shape
        {
            return Err(EngineError::Shape("recognizer output dimensions"));
        }
        Ok(Self {
            ids,
            confidences,
            lengths,
            blank,
            topv,
            topi,
            timesteps: shape[1] as usize,
        })
    }

    pub fn length(&self, slot: usize) -> Result<usize> {
        let n = usize::try_from(self.lengths[slot])
            .map_err(|_| EngineError::Shape("negative CTC length"))?;
        if n > self.timesteps {
            return Err(EngineError::Shape("CTC length exceeds output"));
        }
        Ok(n)
    }

    pub fn trace(&self, trace: &mut crate::trace::Recorder, group: Script, lm: bool) -> Result<()> {
        let rows = self.lengths.len() as i64;
        let t = self.timesteps as i64;
        let prefix = group.trace_prefix();
        trace.tensor(
            match group {
                Script::Base => "text-ids",
                Script::Cjk => "chinese-ids",
                Script::Rtl => "rtl-ids",
                _ => "world-ids",
            },
            &[rows, t],
            self.ids,
        )?;
        trace.tensor(
            match group {
                Script::Base => "text-confidences",
                Script::Cjk => "chinese-confidences",
                Script::Rtl => "rtl-confidences",
                _ => "world-confidences",
            },
            &[rows, t],
            self.confidences,
        )?;
        trace.tensor("ctc-lengths", &[rows], self.lengths)?;
        if lm {
            let k = lm::TOP_K as i64;
            trace.tensor(
                match prefix {
                    "text" => "text-blank",
                    "chinese" => "chinese-blank",
                    _ => "world-blank",
                },
                &[rows, t],
                self.blank,
            )?;
            trace.tensor(
                match prefix {
                    "text" => "text-topv",
                    "chinese" => "chinese-topv",
                    _ => "world-topv",
                },
                &[rows, t, k],
                self.topv,
            )?;
            trace.tensor(
                match prefix {
                    "text" => "text-topi",
                    "chinese" => "chinese-topi",
                    _ => "world-topi",
                },
                &[rows, t, k],
                self.topi,
            )?;
        }
        Ok(())
    }
}

/// The beam-search inputs of one line and the model that scores them.
pub(crate) struct LmRow<'a> {
    pub blank: &'a [f32],
    pub topv: &'a [f32],
    pub topi: &'a [i64],
    pub model: &'a lm::Model,
    pub params: lm::Params,
}

/// A borrowed model-output row. Both schedulers use this one CTC implementation.
pub(crate) struct CtcRow<'a> {
    pub ids: &'a [i64],
    pub probabilities: &'a [f32],
    pub timesteps: usize,
    pub padded: usize,
    pub script: Script,
    pub lm: Option<LmRow<'a>>,
}

/// Greedy CTC collapse, optional beam-search rescoring, token x positions and
/// the better-of-two-readings rule. Returns whether this reading was kept.
pub(crate) fn decode_ctc(
    row: CtcRow<'_>,
    box_bounds: &[f32; 4],
    line_width: usize,
    lines: &mut Lines,
    index: usize,
    beam: &mut lm::Scratch,
    stats: &mut OcrStats,
) -> Result<bool> {
    let CtcRow {
        ids,
        probabilities: probs,
        timesteps,
        padded,
        script,
        lm,
    } = row;
    let n = ids.len();
    if n != probs.len()
        || n > timesteps
        || timesteps == 0
        || timesteps > MAX_TOKENS
        || index >= MAX_LINES
        || line_width == 0
        || script == Script::Math
    {
        return Err(EngineError::Shape("CTC row dimensions"));
    }
    let vocab = script.vocabulary();
    let stride = padded as f64 / timesteps as f64;
    let [x0, _, x1, _] = box_bounds.map(f64::from);
    let h = (f64::from(box_bounds[3]) - f64::from(box_bounds[1])).max(1.0);
    let ix0 = (x0 - 0.3 * h).floor();
    let cw = (x1 + 0.3 * h).ceil() - ix0;
    let factor = cw / line_width as f64;
    let mut greedy_ids = [0_i32; MAX_TOKENS];
    let mut greedy_steps = [0_u16; MAX_TOKENS];
    let mut peaks = [0.0_f32; MAX_TOKENS];
    let mut count = 0;
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
        greedy_ids[count] = ids[t] as i32;
        greedy_steps[count] = t as u16;
        peaks[count] = probs[t];
        count += 1;
    }
    let conf = if count == 0 {
        0.0
    } else {
        crate::order::sum32(&peaks[..count]) / count as f32
    };
    let mut rescored = false;
    if let Some(lm) = lm
        && conf >= MIN_CONF
        && f64::from(conf) < lm.params.gate
    {
        count = lm::beam_search(
            lm.blank,
            lm.topv,
            lm.topi,
            lm.model,
            lm.params,
            beam,
            &mut greedy_ids,
            &mut greedy_steps,
        )?;
        for id in &greedy_ids[..count] {
            if *id <= 0 || *id as usize > vocab.len() {
                return Err(EngineError::Shape("beam search token id"));
            }
        }
        rescored = true;
        stats.lm_lines += 1;
    }
    let line = &mut lines.metadata[index];
    if conf <= line.best {
        return Ok(false);
    }
    let target_ids =
        &mut lines.ids.data_typed_mut::<i32>()?[index * MAX_TOKENS..(index + 1) * MAX_TOKENS];
    let target_positions =
        &mut lines.positions.data_typed_mut::<f64>()?[index * MAX_TOKENS..(index + 1) * MAX_TOKENS];
    for token in 0..count {
        target_ids[token] = greedy_ids[token];
        target_positions[token] = ix0 + (f64::from(greedy_steps[token]) + 0.5) * stride * factor;
    }
    line.script = script;
    line.confidence = conf;
    line.best = conf;
    line.rescored = rescored;
    line.count = if conf >= MIN_CONF { count } else { 0 };
    Ok(true)
}

pub(crate) fn token(script: Script, id: i32) -> Result<&'static str> {
    if script == Script::Math {
        return Err(EngineError::Shape("math token used as CTC token"));
    }
    usize::try_from(id - 1)
        .ok()
        .and_then(|id| script.vocabulary().get(id))
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

/// `Vocab.decode` for the base recogniser (a LaTeX command followed by a
/// letter keeps one space) and the character join of a group recogniser with
/// whitespace runs collapsed, as the reference's `_decode` does.
pub(crate) fn decode_tokens(script: Script, ids: &[i32], output: &mut String) -> Result<()> {
    let mut pending_space = false;
    let start = output.len();
    for (index, id) in ids.iter().enumerate() {
        let text = token(script, *id)?;
        if script.as_chars() {
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
    fn proved_accounting_preserves_prepared_tensor_bits_and_group_membership() {
        use crate::batching::{MATH_COLUMNS, TEXT_COLUMNS};
        let mut gray = Mat::new_rows_cols_with_default(80, 640, core::CV_8UC1, Scalar::all(0.0))
            .expect("synthetic gray page");
        for (index, pixel) in gray
            .data_typed_mut::<u8>()
            .expect("pixels")
            .iter_mut()
            .enumerate()
        {
            *pixel = ((index / 640 * 17 + index % 640 * 31) % 256) as u8;
        }
        for (count, raw_width, height, math) in
            [(62, 657, 32, false), (63, 649, 32, false), (8, 0, 64, true)]
        {
            let mut boxes = [[0.0; 4]; MAX_BATCH];
            for (index, b) in boxes[..count].iter_mut().enumerate() {
                *b = if math {
                    let x = -15.0 + index as f32 * 90.0;
                    let y = -7.0 + index as f32 * 8.0;
                    [x, y, x + 200.0 + (index % 2) as f32 * 7.0, y + 20.0]
                } else {
                    [
                        10.0,
                        10.0,
                        if raw_width == 657 { 573.0 } else { 566.0 },
                        30.0,
                    ]
                };
            }
            let mut line_widths = [0; MAX_BATCH];
            widths(&boxes[..count], height, &mut line_widths[..count]).expect("crop widths");
            if !math {
                assert!(line_widths[..count].iter().all(|width| *width == raw_width));
            }
            let mut selected = Plan::new(
                &line_widths[..count],
                MAX_BATCH,
                if math { Policy::Math } else { Policy::Text },
                |_| true,
            )
            .expect("unchanged reference grouping");
            let range = selected.next(&line_widths[..count]).expect("one group");
            assert_eq!(range.len(), count);
            assert!(selected.next(&line_widths[..count]).is_none());
            let indices = &selected.indices[range];
            let mut expected_indices: [usize; MAX_BATCH] = std::array::from_fn(|index| index);
            expected_indices[..count].sort_unstable_by_key(|index| (line_widths[*index], *index));
            assert_eq!(indices, &expected_indices[..count]);

            let capacity = height * if math { MATH_COLUMNS } else { TEXT_COLUMNS };
            let mut actual = geometry::floats(capacity + 7).expect("actual tensor");
            let mut previous = geometry::floats(capacity + 7).expect("reference tensor");
            actual
                .data_typed_mut::<f32>()
                .expect("actual data")
                .fill(-2.0);
            previous
                .data_typed_mut::<f32>()
                .expect("reference data")
                .fill(-2.0);
            let mut actual_widths = [-1; MAX_BATCH];
            let mut previous_widths = [-1; MAX_BATCH];
            let due = Instant::now() + std::time::Duration::from_secs(60);
            let dark = [false; MAX_LINES];
            let padded = fill_batch(
                &gray,
                &boxes[..count],
                indices,
                height,
                1,
                &dark,
                &mut Scratch::new().expect("scratch"),
                &mut actual,
                &mut actual_widths,
                due,
            )
            .expect("proved tensor preparation");

            // This is the pre-integration max-width scan and packing procedure.
            // The crop sampler is shared; tensor accounting is independently computed.
            let mut widest = 0;
            for (slot, index) in indices.iter().enumerate() {
                let crop = geometry::crop_geometry(&boxes[*index], height, 1).expect("crop");
                previous_widths[slot] = crop.out_width as i64;
                widest = widest.max(crop.out_width);
            }
            let old_padded = widest.div_ceil(8) * 8;
            let old_area = count * height * old_padded;
            let old_data = previous.data_typed_mut::<f32>().expect("reference data");
            old_data[..old_area].fill(0.0);
            let mut old_scratch = Scratch::new().expect("reference scratch");
            for (slot, index) in indices.iter().enumerate() {
                let crop = geometry::crop_geometry(&boxes[*index], height, 1).expect("crop");
                old_scratch
                    .line_input(
                        &gray,
                        &crop,
                        false,
                        &mut old_data[slot * height * old_padded..(slot + 1) * height * old_padded],
                        old_padded,
                    )
                    .expect("reference crop preparation");
            }
            assert_eq!(padded, old_padded);
            assert_eq!(actual_widths, previous_widths);
            for (actual, previous) in actual
                .data_typed::<f32>()
                .expect("actual data")
                .iter()
                .zip(previous.data_typed::<f32>().expect("reference data"))
            {
                assert_eq!(
                    actual.to_bits(),
                    previous.to_bits(),
                    "active padding and untouched tail"
                );
            }
        }
    }

    #[test]
    fn token_decode_separates_latex_commands_from_letters() {
        let alpha = vocabulary::TEXT
            .iter()
            .position(|s| *s == "\\alpha")
            .expect("alpha") as i32
            + 1;
        let x = vocabulary::TEXT.iter().position(|s| *s == "x").expect("x") as i32 + 1;
        let mut output = String::new();
        decode_tokens(Script::Base, &[alpha, x], &mut output).expect("decode");
        assert_eq!(output, "\\alpha x");
        // Group recognisers emit characters; whitespace runs collapse and trim.
        let space = vocabulary::RTL
            .iter()
            .position(|s| *s == " ")
            .expect("space") as i32
            + 1;
        let alef = vocabulary::RTL
            .iter()
            .position(|s| *s == "ا")
            .expect("alef") as i32
            + 1;
        let mut output = String::new();
        decode_tokens(
            Script::Rtl,
            &[space, alef, space, space, alef, space],
            &mut output,
        )
        .expect("decode");
        assert_eq!(output, "ا ا");
    }

    #[test]
    fn invalid_tokens_are_errors() {
        assert!(token(Script::Base, 0).is_err());
        assert!(token(Script::Cjk, 100_000).is_err());
        assert!(token(Script::Math, 1).is_err());
        assert_eq!(Script::GROUPS.map(Script::window), [3, 1, 1, 1]);
    }

    #[test]
    fn routing_takes_the_first_best_class_and_reads_unsure_lines_twice() {
        let mut routing = Box::new(Routing::new());
        let probabilities = [
            0.1, 0.96, 0.02, 0.02, // sure CJK
            0.3, 0.3, 0.3, 0.1, // tie: base, read once
            0.05, 0.05, 0.05, 0.85, // unsure world: world and base
        ];
        let dual = routing.route(&probabilities, 3, 0.95).expect("routing");
        assert_eq!(dual, 1);
        assert_eq!(
            &routing.group[..3],
            &[Script::Cjk, Script::Base, Script::World]
        );
        assert_eq!(&routing.sure[..3], &[true, false, false]);
        assert!(!routing.reads(Script::Base, 0) && routing.reads(Script::Cjk, 0));
        assert!(routing.reads(Script::Base, 1) && !routing.reads(Script::Cjk, 1));
        assert!(routing.reads(Script::Base, 2) && routing.reads(Script::World, 2));
        assert!(routing.route(&probabilities[..8], 3, 0.95).is_err());
        routing.vertical(3);
        assert!(routing.group[..3].iter().all(|group| *group == Script::Cjk));
        assert!(!routing.reads(Script::Base, 2));
    }

    #[test]
    fn a_later_reading_replaces_the_stored_one_only_when_more_confident() {
        let mut lines = Lines::new().expect("lines");
        let mut beam = lm::Scratch::new();
        let mut stats = OcrStats::new(&crate::EngineConfig::default(), 1);
        let a = vocabulary::TEXT.iter().position(|s| *s == "a").expect("a") as i64 + 1;
        let ids = [a, a, 0, a];
        let row = |probabilities: &'static [f32]| CtcRow {
            ids: &ids[..probabilities.len()],
            probabilities,
            timesteps: 4,
            padded: 32,
            script: Script::Base,
            lm: None,
        };
        let b = [0.0, 0.0, 10.0, 10.0];
        assert!(
            decode_ctc(
                row(&[0.9, 0.9, 0.9, 0.7]),
                &b,
                16,
                &mut lines,
                0,
                &mut beam,
                &mut stats
            )
            .expect("first")
        );
        assert_eq!(
            (lines.metadata[0].count, lines.metadata[0].confidence),
            (2, 0.8)
        );
        assert!(
            !decode_ctc(
                row(&[0.8, 0.8, 0.8, 0.8]),
                &b,
                16,
                &mut lines,
                0,
                &mut beam,
                &mut stats
            )
            .expect("tie")
        );
        assert!(
            decode_ctc(
                row(&[0.95, 0.95, 0.95, 0.95]),
                &b,
                16,
                &mut lines,
                0,
                &mut beam,
                &mut stats
            )
            .expect("better")
        );
        assert_eq!(lines.metadata[0].confidence, 0.95);
        // A reading under the confidence floor keeps its box but no text.
        assert!(
            decode_ctc(
                row(&[0.2, 0.2]),
                &b,
                16,
                &mut lines,
                1,
                &mut beam,
                &mut stats
            )
            .expect("low")
        );
        assert_eq!(lines.metadata[1].count, 0);
        assert_eq!(stats.lm_lines, 0);
    }
}
