//! Experimental cross-request stateless batching with two admitted owners.
//!
//! Each call advances one owner operation or one shared line-model invocation.
//! The caller can refill a completed slot between calls. It owns the request
//! bytes and response buffer until completion, including after cancellation.

use std::time::{Duration, Instant};

use opencv::{
    core::{self, Mat},
    prelude::*,
};
use ort::value::TensorRef;

use crate::{
    Engine, EngineError, MAX_BATCH, MAX_LINES, OcrStats, Result, batching, deadline, geometry,
    images, recognition,
    runtime::{PageState, SharedEngine},
};

mod plan;
#[cfg(test)]
mod tests;

/// Borrow the same admitted input and output until this owner completes. A new
/// admission must use a new sequence, even when reusing the same buffer slot.
pub struct ContinuousRequest<'a> {
    pub sequence: u64,
    pub input: &'a [u8],
    pub output: &'a mut String,
    pub deadline: Instant,
    /// Sampled between native operations. An in-flight native call is retained.
    pub cancelled: bool,
}

/// Recorded only after an actual stateless model invocation has returned.
/// Request identifiers and document contents do not enter production telemetry.
pub struct ContinuousInvocation {
    pub model: &'static str,
    pub owner_rows: [usize; 2],
    pub padded_width: usize,
    pub elapsed: Duration,
}

impl ContinuousInvocation {
    pub const fn shared(&self) -> bool {
        self.owner_rows[0] != 0 && self.owner_rows[1] != 0
    }
}

pub struct ContinuousProgress {
    pub completed: [Option<Result<OcrStats>>; 2],
    pub invocation: Option<ContinuousInvocation>,
}

impl ContinuousProgress {
    const fn new() -> Self {
        Self {
            completed: [const { None }; 2],
            invocation: None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Prepare,
    Script,
    Text,
    Chinese,
    Math,
    Finish,
}

impl Phase {
    const fn route(&self) -> Option<bool> {
        match self {
            Self::Text => Some(false),
            Self::Chinese => Some(true),
            _ => None,
        }
    }
    const fn model(&self) -> &'static str {
        match self {
            Self::Script => "script",
            Self::Text => "rec",
            Self::Chinese => "rec_cjk",
            _ => "none",
        }
    }
}

struct Owner {
    page: PageState,
    gray: Mat,
    header: Option<images::Header>,
    stats: Option<OcrStats>,
    phase: Phase,
    sequence: u64,
    input_address: usize,
    input_length: usize,
    due: Option<Instant>,
    frame: usize,
    count: usize,
    position: usize,
    indices: [usize; MAX_LINES],
    widths: [usize; MAX_LINES],
    flags: [bool; MAX_LINES],
}

impl Owner {
    fn new(page: PageState) -> Self {
        Self {
            page,
            gray: Mat::default(),
            header: None,
            stats: None,
            phase: Phase::Idle,
            sequence: 0,
            input_address: 0,
            input_length: 0,
            due: None,
            frame: 0,
            count: 0,
            position: 0,
            indices: [0; MAX_LINES],
            widths: [0; MAX_LINES],
            flags: [false; MAX_LINES],
        }
    }
    fn reset(&mut self) {
        self.gray = Mat::default();
        self.header = None;
        self.stats = None;
        self.phase = Phase::Idle;
        self.due = None;
        self.count = 0;
        self.position = 0;
        self.page.lines.reset();
        self.page.page.text.clear();
        self.page.page.count = 0;
    }
    fn due(&self) -> Result<Instant> {
        self.due
            .ok_or(EngineError::Config("continuous owner has no deadline"))
    }
    fn stats(&mut self) -> Result<&mut OcrStats> {
        self.stats
            .as_mut()
            .ok_or(EngineError::Config("continuous owner has no statistics"))
    }
    fn normalize(&mut self, math: bool) {
        loop {
            match self.phase {
                Phase::Script | Phase::Text | Phase::Chinese => {
                    let route = self.phase.route();
                    while self.position < self.count
                        && route
                            .is_some_and(|route| self.flags[self.indices[self.position]] != route)
                    {
                        self.position += 1;
                    }
                    if self.position < self.count {
                        return;
                    }
                    self.position = 0;
                    self.phase = match self.phase {
                        Phase::Script => Phase::Text,
                        Phase::Text => Phase::Chinese,
                        _ if math => Phase::Math,
                        _ => Phase::Finish,
                    };
                }
                _ => return,
            }
        }
    }
    fn view(&self) -> plan::View<'_> {
        plan::View {
            widths: &self.widths[..self.count],
            indices: &self.indices[..self.count],
            flags: &self.flags[..self.count],
            route: self.phase.route(),
            position: self.position,
        }
    }
}

/// One set of native sessions/scratch, two reusable page states, no additional
/// admission queue and no wait-for-batch timer. The normal Engine is unchanged
/// unless its caller explicitly consumes it into this experimental scheduler.
pub struct Continuous {
    shared: SharedEngine,
    owners: [Owner; 2],
    preferred: usize,
    batch: plan::Batch,
}

impl Engine {
    pub fn into_continuous(self) -> Result<Continuous> {
        let Self { shared, page } = self;
        Ok(Continuous {
            shared,
            owners: [Owner::new(page), Owner::new(PageState::new()?)],
            preferred: 0,
            batch: plan::Batch::new(),
        })
    }
}

impl Continuous {
    /// Advance at most one shared stateless model call or one owner preparation,
    /// math, or finalization stage. Preparation can use the detector's bounded
    /// second pass. Math retains the reference's within-page decoder loop.
    ///
    /// Each active owner must be supplied on every call. Output is transactional
    /// per request: cancellation/deadline/invalid input clears that owner's text;
    /// a native/internal failure clears both supplied outputs and stops the run.
    /// `on_operation` receives the exact deadline of the selected owner or the
    /// earliest deadline of the selected shared rows, before native work starts.
    /// A host uses this to arm its terminal watchdog without charging an unrelated
    /// waiting owner's deadline to this operation. The callback is monomorphized.
    pub fn advance(
        &mut self,
        mut requests: [Option<ContinuousRequest<'_>>; 2],
        mut on_operation: impl FnMut(Instant) -> Result<()>,
    ) -> Result<ContinuousProgress> {
        let result = self.advance_inner(&mut requests, &mut on_operation);
        if result.is_err() {
            for (owner, request) in self.owners.iter_mut().zip(&mut requests) {
                if let Some(request) = request {
                    request.output.clear();
                }
                owner.reset();
            }
        }
        result
    }

    fn advance_inner(
        &mut self,
        requests: &mut [Option<ContinuousRequest<'_>>; 2],
        on_operation: &mut impl FnMut(Instant) -> Result<()>,
    ) -> Result<ContinuousProgress> {
        let mut progress = ContinuousProgress::new();
        for (index, (owner, request)) in self.owners.iter_mut().zip(&mut *requests).enumerate() {
            let Some(request) = request else {
                if owner.phase != Phase::Idle {
                    return Err(EngineError::Config(
                        "an active continuous owner was omitted",
                    ));
                }
                continue;
            };
            if owner.phase != Phase::Idle
                && (owner.sequence != request.sequence
                    || owner.input_address != request.input.as_ptr() as usize
                    || owner.input_length != request.input.len()
                    || owner.due != Some(request.deadline))
            {
                return Err(EngineError::Config(
                    "continuous request identity changed before completion",
                ));
            }
            if request.cancelled || Instant::now() >= request.deadline {
                request.output.clear();
                owner.reset();
                progress.completed[index] = Some(Err(EngineError::Deadline));
                continue;
            }
            if owner.phase == Phase::Idle {
                request.output.clear();
                let begin = Instant::now();
                let header = match images::inspect(request.input) {
                    Ok(header) => header,
                    Err(error) => {
                        progress.completed[index] = Some(Err(error));
                        continue;
                    }
                };
                let mut stats = OcrStats::new(&self.shared.settings, core::get_num_threads()?);
                stats.stage_times[0] = begin.elapsed();
                owner.header = Some(header);
                owner.stats = Some(stats);
                owner.sequence = request.sequence;
                owner.input_address = request.input.as_ptr() as usize;
                owner.input_length = request.input.len();
                owner.due = Some(request.deadline);
                owner.frame = 0;
                owner.phase = Phase::Prepare;
            }
            owner.normalize(self.shared.settings.math);
        }
        let anchor = if self.owners[self.preferred].phase != Phase::Idle {
            self.preferred
        } else if self.owners[1 - self.preferred].phase != Phase::Idle {
            1 - self.preferred
        } else {
            return Ok(progress);
        };
        self.preferred = 1 - anchor;
        let result = match self.owners[anchor].phase {
            Phase::Prepare => {
                on_operation(self.owners[anchor].due()?)?;
                let request = requests[anchor]
                    .as_mut()
                    .ok_or(EngineError::Config("missing preparing owner"))?;
                prepare(&mut self.shared, &mut self.owners[anchor], request.input)
            }
            Phase::Math => {
                on_operation(self.owners[anchor].due()?)?;
                math(&mut self.shared, &mut self.owners[anchor])
            }
            Phase::Finish => {
                on_operation(self.owners[anchor].due()?)?;
                let request = requests[anchor]
                    .as_mut()
                    .ok_or(EngineError::Config("missing completing owner"))?;
                match finish(&mut self.shared, &mut self.owners[anchor], request.output) {
                    Ok(Some(stats)) => {
                        progress.completed[anchor] = Some(Ok(stats));
                        Ok(())
                    }
                    Ok(None) => Ok(()),
                    Err(error) => Err(error),
                }
            }
            Phase::Script | Phase::Text | Phase::Chinese => {
                let views = std::array::from_fn(|index| {
                    (self.owners[index].phase == self.owners[anchor].phase)
                        .then(|| self.owners[index].view())
                });
                plan::select(
                    views,
                    anchor,
                    self.shared.settings.batch_size,
                    &mut self.batch,
                )?;
                on_operation(batch_deadline(&self.owners, &self.batch)?)?;
                match invoke(
                    &mut self.shared,
                    &mut self.owners,
                    &self.batch,
                    anchor,
                    #[cfg(test)]
                    |_, _, _, _| Ok(()),
                ) {
                    Ok(invocation) => {
                        progress.invocation = Some(invocation);
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            }
            Phase::Idle => return Err(EngineError::Config("continuous selected an idle owner")),
        };
        if let Err(error) = result {
            match error {
                EngineError::Deadline => {
                    // A shared crop/model call uses the earliest participating
                    // deadline. Expire only owners whose own deadline passed;
                    // the still-live peer retries uncommitted selected rows.
                    let mut expired = false;
                    for (index, owner) in self.owners.iter_mut().enumerate() {
                        if owner.due.is_some_and(|due| Instant::now() >= due) {
                            if let Some(request) = &mut requests[index] {
                                request.output.clear();
                            }
                            owner.reset();
                            progress.completed[index] = Some(Err(EngineError::Deadline));
                            expired = true;
                        }
                    }
                    if !expired {
                        return Err(EngineError::Deadline);
                    }
                }
                EngineError::InvalidImage(_)
                | EngineError::UnsupportedImage(_)
                | EngineError::Limit(_) => {
                    if let Some(request) = &mut requests[anchor] {
                        request.output.clear();
                    }
                    self.owners[anchor].reset();
                    progress.completed[anchor] = Some(Err(error));
                }
                _ => return Err(error),
            }
        }
        // A native invocation can return after only one owner's deadline. The
        // shared output has been scattered, but that expired result is discarded.
        for (index, owner) in self.owners.iter_mut().enumerate() {
            if owner.due.is_some_and(|due| Instant::now() >= due) {
                if let Some(request) = &mut requests[index] {
                    request.output.clear();
                }
                owner.reset();
                progress.completed[index] = Some(Err(EngineError::Deadline));
            }
            owner.normalize(self.shared.settings.math);
        }
        Ok(progress)
    }
}

fn context(shared: &mut SharedEngine, owner: &Owner) -> Result<()> {
    shared.trace.tensor(
        "continuous-owner-frame",
        &[2],
        &[owner.sequence, owner.frame as u64],
    )
}

fn prepare(shared: &mut SharedEngine, owner: &mut Owner, input: &[u8]) -> Result<()> {
    let due = owner.due()?;
    deadline(due)?;
    context(shared, owner)?;
    let begin = Instant::now();
    let header = owner
        .header
        .as_ref()
        .ok_or(EngineError::Config("continuous header missing"))?;
    let gray = images::decode(input, header, owner.frame)?;
    shared.trace.tensor(
        "decoded",
        &[gray.rows() as i64, gray.cols() as i64],
        gray.data_typed::<u8>()?,
    )?;
    owner.stats()?.stage_times[0] += begin.elapsed();
    let begin = Instant::now();
    let turn = shared.orientation(&gray, due)?;
    let gray = geometry::turn(gray, turn)?;
    let skew = shared.scratch.estimate_skew(&gray, due)?;
    shared
        .trace
        .tensor("turn-skew", &[2], &[turn as f64, skew])?;
    let gray = geometry::deskew(gray, skew)?;
    shared.trace.tensor(
        "upright",
        &[gray.rows() as i64, gray.cols() as i64],
        gray.data_typed::<u8>()?,
    )?;
    owner.stats()?.stage_times[1] += begin.elapsed();
    let begin = Instant::now();
    owner.count = shared.detect(&gray, &mut owner.page.boxes, due)?;
    owner.stats()?.stage_times[2] += begin.elapsed();
    recognition::widths(
        &owner.page.boxes[..owner.count],
        geometry::LINE_HEIGHT,
        &mut owner.widths[..owner.count],
    )?;
    for (index, value) in owner.indices[..owner.count].iter_mut().enumerate() {
        *value = index;
    }
    owner.indices[..owner.count].sort_unstable_by_key(|index| (owner.widths[*index], *index));
    owner.flags[..owner.count].fill(false);
    owner.page.lines.reset();
    owner.position = 0;
    owner.gray = gray;
    owner.phase = Phase::Script;
    deadline(due)
}

fn math(shared: &mut SharedEngine, owner: &mut Owner) -> Result<()> {
    let due = owner.due()?;
    context(shared, owner)?;
    let begin = Instant::now();
    if let Some(math) = &mut shared.math {
        math.recognize(
            &owner.gray,
            &owner.page.boxes[..owner.count],
            &owner.widths[..owner.count],
            shared.settings.batch_size,
            &mut shared.scratch,
            &mut shared.text_tensor,
            &mut owner.page.lines,
            due,
            &mut shared.trace,
        )?;
    }
    owner.stats()?.stage_times[4] += begin.elapsed();
    owner.phase = Phase::Finish;
    deadline(due)
}

fn finish(
    shared: &mut SharedEngine,
    owner: &mut Owner,
    output: &mut String,
) -> Result<Option<OcrStats>> {
    let due = owner.due()?;
    deadline(due)?;
    context(shared, owner)?;
    let begin = Instant::now();
    owner
        .page
        .page
        .assemble(&owner.page.boxes[..owner.count], &owner.page.lines, due)?;
    owner.stats()?.stage_times[6] += begin.elapsed();
    let begin = Instant::now();
    let mut indices = [0; MAX_LINES];
    shared.reading_order(
        &owner.page.page,
        [owner.gray.cols() as f32, owner.gray.rows() as f32],
        &mut indices,
        due,
    )?;
    owner.stats()?.stage_times[5] += begin.elapsed();
    let begin = Instant::now();
    if owner.frame > 0 {
        recognition::append(output, "\n\u{c}\n")?;
    }
    owner
        .page
        .page
        .write(&indices[..owner.page.page.count], output)?;
    let lines = owner.page.page.count;
    let stats = owner.stats()?;
    stats.frames += 1;
    stats.lines += lines;
    stats.output_bytes = output.len();
    stats.stage_times[6] += begin.elapsed();
    deadline(due)?;
    owner.frame += 1;
    if owner.frame
        == owner
            .header
            .as_ref()
            .ok_or(EngineError::Config("continuous header missing"))?
            .frames
    {
        let stats = owner
            .stats
            .take()
            .ok_or(EngineError::Config("continuous statistics missing"))?;
        owner.reset();
        Ok(Some(stats))
    } else {
        owner.gray = Mat::default();
        owner.phase = Phase::Prepare;
        Ok(None)
    }
}

fn invoke(
    shared: &mut SharedEngine,
    owners: &mut [Owner; 2],
    batch: &plan::Batch,
    anchor: usize,
    #[cfg(test)] observe: impl FnOnce(
        &plan::Batch,
        &[f32],
        usize,
        &ort::session::SessionOutputs<'_>,
    ) -> Result<()>,
) -> Result<ContinuousInvocation> {
    let begin = Instant::now();
    let due = batch_deadline(owners, batch)?;
    deadline(due)?;
    let rows = &batch.rows[..batch.count];
    let mut widths = [0_i64; MAX_BATCH];
    let values = shared.text_tensor.data_typed_mut::<f32>()?;
    let account = batching::tensor_widths(
        rows.iter().map(|row| Ok(row.width)),
        geometry::LINE_HEIGHT,
        values.len(),
        &mut widths,
    )?;
    let padded = account.width() as usize;
    let active = account.work() as usize * geometry::LINE_HEIGHT;
    let values = values
        .get_mut(..active)
        .ok_or(EngineError::Limit("continuous tensor scratch"))?;
    values.fill(0.0);
    let mut mapping = [0_u64; MAX_BATCH * 4];
    for (slot, row) in rows.iter().enumerate() {
        deadline(due)?;
        let owner = &owners[row.owner];
        let crop = geometry::crop_geometry(&owner.page.boxes[row.line], geometry::LINE_HEIGHT)?;
        if crop.out_width != row.width {
            return Err(EngineError::Shape("continuous crop width changed"));
        }
        shared.scratch.line_input(
            &owner.gray,
            &crop,
            geometry::LINE_HEIGHT,
            &mut values[slot * geometry::LINE_HEIGHT * padded
                ..(slot + 1) * geometry::LINE_HEIGHT * padded],
            padded,
        )?;
        mapping[4 * slot] = owner.sequence;
        mapping[4 * slot + 1] = owner.frame as u64;
        mapping[4 * slot + 2] = row.line as u64;
        mapping[4 * slot + 3] = row.owner as u64;
    }
    let model = owners[anchor].phase.model();
    let _span = tracing::info_span!(
        "ocr.continuous_batch",
        model,
        rows = batch.count,
        owner_count = u8::from(batch.owners[0] != 0) + u8::from(batch.owners[1] != 0),
        owner_zero_rows = batch.owners[0],
        owner_one_rows = batch.owners[1],
        padded_width = padded
    )
    .entered();
    shared.trace.tensor(
        "continuous-row-map",
        &[batch.count as i64, 4],
        &mapping[..batch.count * 4],
    )?;
    shared.trace.tensor(
        match model {
            "script" => "script-input",
            "rec_cjk" => "chinese-input",
            _ => "text-input",
        },
        &[
            batch.count as i64,
            1,
            geometry::LINE_HEIGHT as i64,
            padded as i64,
        ],
        values,
    )?;
    shared
        .trace
        .tensor("line-widths", &[batch.count as i64], &widths[..batch.count])?;
    let x =
        TensorRef::from_array_view(([batch.count, 1, geometry::LINE_HEIGHT, padded], &*values))?;
    let w = TensorRef::from_array_view(([batch.count], &widths[..batch.count]))?;
    let session = match owners[anchor].phase {
        Phase::Script => &mut shared.script,
        Phase::Text => &mut shared.recognizer,
        Phase::Chinese => &mut shared.chinese,
        _ => return Err(EngineError::Shape("continuous stage is not stateless")),
    };
    deadline(due)?;
    let outputs = session.run(ort::inputs![x, w])?;
    #[cfg(test)]
    observe(batch, values, padded, &outputs)?;
    tracing::debug!(
        event = "continuous_native_returned",
        model,
        rows = batch.count,
        owner_zero_rows = batch.owners[0],
        owner_one_rows = batch.owners[1],
        "Shared model invocation returned"
    );
    if model == "script" {
        let (shape, probabilities) = outputs[0].try_extract_tensor::<f32>()?;
        shared
            .trace
            .tensor("script-probabilities", shape, probabilities)?;
        if probabilities.len() != batch.count
            || !probabilities.iter().all(|value| value.is_finite())
        {
            return Err(EngineError::Shape("continuous script output"));
        }
        for (row, probability) in rows.iter().zip(probabilities) {
            owners[row.owner].flags[row.line] = *probability > 0.5;
        }
    } else {
        if outputs.len() != 3 {
            return Err(EngineError::Shape("continuous recognizer output count"));
        }
        let (shape, ids) = outputs[0].try_extract_tensor::<i64>()?;
        let (prob_shape, probabilities) = outputs[1].try_extract_tensor::<f32>()?;
        let (length_shape, lengths) = outputs[2].try_extract_tensor::<i64>()?;
        let chinese = model == "rec_cjk";
        shared
            .trace
            .tensor(if chinese { "chinese-ids" } else { "text-ids" }, shape, ids)?;
        shared.trace.tensor(
            if chinese {
                "chinese-confidences"
            } else {
                "text-confidences"
            },
            prob_shape,
            probabilities,
        )?;
        shared.trace.tensor("ctc-lengths", length_shape, lengths)?;
        if shape.len() != 2
            || shape[0] != batch.count as i64
            || shape[1] <= 0
            || shape[1] as usize > recognition::MAX_TOKENS
            || shape != prob_shape
            || lengths.len() != batch.count
        {
            return Err(EngineError::Shape("continuous recognizer dimensions"));
        }
        let timesteps = shape[1] as usize;
        for (slot, row) in rows.iter().enumerate() {
            let length = usize::try_from(lengths[slot])
                .map_err(|_| EngineError::Shape("negative continuous CTC length"))?;
            if length > timesteps {
                return Err(EngineError::Shape("continuous CTC length exceeds output"));
            }
            let owner = &mut owners[row.owner];
            recognition::decode_ctc(
                recognition::CtcRow {
                    ids: &ids[slot * timesteps..slot * timesteps + length],
                    probabilities: &probabilities[slot * timesteps..slot * timesteps + length],
                    timesteps,
                    padded,
                    chinese,
                },
                &owner.page.boxes[row.line],
                row.width,
                &mut owner.page.lines,
                row.line,
            )?;
            let stats = owner.stats()?;
            stats.valid_columns += row.width as u64;
            stats.padded_columns += padded as u64;
        }
        // Charge a shared invocation once, to its rotating anchor. Aggregating
        // completed request histograms never double-counts the native call.
        owners[anchor].stats()?.batch_counts[batch.count - 1] += 1;
    }
    let elapsed = begin.elapsed();
    for (index, owner) in owners.iter_mut().enumerate() {
        if batch.owners[index] != 0 {
            owner.position = batch.end[index];
            owner.stats()?.stage_times[3] += elapsed;
        }
    }
    Ok(ContinuousInvocation {
        model,
        owner_rows: batch.owners,
        padded_width: padded,
        elapsed,
    })
}

fn batch_deadline(owners: &[Owner; 2], batch: &plan::Batch) -> Result<Instant> {
    owners
        .iter()
        .zip(batch.owners)
        .filter(|(_, rows)| *rows != 0)
        .map(|(owner, _)| owner.due())
        .try_fold(None, |earliest: Option<Instant>, due| {
            let due = due?;
            Ok::<_, EngineError>(Some(earliest.map_or(due, |prior| prior.min(due))))
        })?
        .ok_or(EngineError::Shape("continuous empty invocation"))
}
