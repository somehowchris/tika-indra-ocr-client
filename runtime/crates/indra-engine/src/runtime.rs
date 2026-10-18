//! Single-owner synchronous inference; the server runs this outside the async executor.

use std::{path::Path, sync::Mutex, time::Instant};

use opencv::{
    core::{self, Mat, Scalar},
    prelude::*,
};
use ort::{
    environment::Environment,
    session::{Session, builder::GraphOptimizationLevel},
    value::TensorRef,
};

use crate::{
    EngineConfig, EngineError, MAX_LINES, OcrStats, Result, assembly, batching, deadline,
    geometry::{self, Scratch},
    images, math, order,
    recognition::{self, Lines},
};

// OpenCV thread configuration is process-global and explicitly not thread safe.
// Configure it once before any worker starts inference. Later engines may reuse
// the setting, but must not reconfigure an already-running native thread pool.
static OPENCV_THREADS: Mutex<Option<usize>> = Mutex::new(None);

fn configure_opencv(threads: usize) -> Result<()> {
    let mut configured = OPENCV_THREADS
        .lock()
        .map_err(|_| EngineError::Config("OpenCV thread configuration lock is poisoned"))?;
    if let Some(existing) = *configured {
        return if existing == threads {
            Ok(())
        } else {
            Err(EngineError::Config(
                "all engines in a process must use the same OpenCV thread count",
            ))
        };
    }
    core::set_num_threads(threads as i32)?;
    *configured = Some(threads);
    Ok(())
}

pub struct Engine {
    detector: Session,
    orient: Session,
    recognizer: Session,
    chinese: Session,
    script: Session,
    order: Session,
    math: Option<math::Reader>,
    _environment: Environment,
    settings: EngineConfig,
    scratch: Scratch,
    detector_tensor: Mat,
    orient_tensor: Mat,
    text_tensor: Mat,
    boxes: [[f32; 4]; MAX_LINES],
    lines: Lines,
    page: assembly::Page,
    order_workspace: order::Workspace,
    trace: crate::trace::Recorder,
}

impl Engine {
    /// Verify every selected artifact before creating native sessions. No download occurs.
    pub fn load(cache: &Path, settings: &EngineConfig) -> Result<Self> {
        settings.validate()?;
        indra_models::verify(cache, settings.math)?;
        configure_opencv(settings.cv_threads)?;
        let environment = ort::init()
            .with_name("indra-cpu")
            .with_telemetry(false)
            .build()?;
        Ok(Self {
            detector: session(&environment, cache, "det.onnx", settings, &["x"], &["out"])?,
            orient: session(&environment, cache, "orient.onnx", settings, &["x"], &["p"])?,
            recognizer: session(
                &environment,
                cache,
                "rec.onnx",
                settings,
                &["x", "widths"],
                &["ids", "conf", "lengths"],
            )?,
            chinese: session(
                &environment,
                cache,
                "rec_cjk.onnx",
                settings,
                &["x", "widths"],
                &["ids", "conf", "lengths"],
            )?,
            script: session(
                &environment,
                cache,
                "script.onnx",
                settings,
                &["x", "widths"],
                &["p"],
            )?,
            order: session(
                &environment,
                cache,
                "order.onnx",
                settings,
                &["feats", "mask"],
                &["a", "cls", "same", "tab"],
            )?,
            math: if settings.math {
                Some(math::Reader::load(&environment, cache, settings)?)
            } else {
                None
            },
            _environment: environment,
            settings: settings.settings(),
            scratch: Scratch::new()?,
            detector_tensor: geometry::floats(geometry::DET_MAX_SIDE * geometry::DET_MAX_SIDE)?,
            orient_tensor: geometry::floats(geometry::ORIENT_SIDE * geometry::ORIENT_SIDE)?,
            text_tensor: geometry::floats(batching::TEXT_COLUMNS * geometry::LINE_HEIGHT)?,
            boxes: [[0.0; 4]; MAX_LINES],
            lines: Lines::new()?,
            page: assembly::Page::new(),
            order_workspace: order::Workspace::new()?,
            trace: crate::trace::Recorder::Disabled,
        })
    }

    /// Export real intermediate tensors for an explicit diagnostic run.
    /// The empty destination receives at most 512 MiB and can contain document data.
    /// Production logging never enables or includes this evidence.
    pub fn set_trace_directory(&mut self, path: &Path) -> Result<()> {
        self.trace = crate::trace::Recorder::directory(path)?;
        Ok(())
    }

    /// Execute every enabled model before readiness, including both scripts and math KV cache steps.
    pub fn warmup(&mut self, due: Instant) -> Result<()> {
        deadline(due)?;
        let blank = Mat::new_rows_cols_with_default(128, 256, core::CV_8UC1, Scalar::all(255.0))?;
        let _ = self.orientation(&blank, due)?;
        let _ = self.detect(&blank, due)?;
        self.boxes[0] = [4.0, 4.0, 200.0, 32.0];
        self.boxes[1] = [4.0, 45.0, 200.0, 73.0];
        self.lines.reset();
        let boxes = &self.boxes[..2];
        let mut widths = [0_usize; 2];
        recognition::widths(boxes, geometry::LINE_HEIGHT, &mut widths)?;
        let mut flags = [false; 2];
        recognition::classify(
            &mut self.script,
            &blank,
            boxes,
            &widths,
            self.settings.batch_size,
            &mut self.scratch,
            &mut self.text_tensor,
            &mut flags,
            due,
            &mut self.trace,
            "script",
        )?;
        let mut stats = OcrStats::new(&self.settings, core::get_num_threads()?);
        recognition::text(
            &mut self.recognizer,
            &blank,
            boxes,
            &widths,
            &[false, false],
            false,
            self.settings.batch_size,
            &mut self.scratch,
            &mut self.text_tensor,
            &mut self.lines,
            &mut stats,
            due,
            &mut self.trace,
        )?;
        recognition::text(
            &mut self.chinese,
            &blank,
            boxes,
            &widths,
            &[true, true],
            true,
            self.settings.batch_size,
            &mut self.scratch,
            &mut self.text_tensor,
            &mut self.lines,
            &mut stats,
            due,
            &mut self.trace,
        )?;
        if let Some(math) = &mut self.math {
            math.recognize(
                &blank,
                boxes,
                &widths,
                self.settings.batch_size,
                &mut self.scratch,
                &mut self.text_tensor,
                &mut self.lines,
                due,
                &mut self.trace,
            )?;
            math.read(
                &blank,
                boxes,
                &[true, true],
                self.settings.batch_size,
                &mut self.scratch,
                &mut self.lines,
                due,
                &mut self.trace,
            )?;
        }
        self.page.assemble(boxes, &self.lines, due)?;
        let mut indices = [0; MAX_LINES];
        self.reading_order([256.0, 128.0], &mut indices, due)?;
        deadline(due)
    }

    /// Transactional text output: any malformed frame, deadline or native error clears the result.
    pub fn recognize(
        &mut self,
        bytes: &[u8],
        due: Instant,
        output: &mut String,
    ) -> Result<OcrStats> {
        output.clear();
        let result = self.recognize_inner(bytes, due, output);
        if result.is_err() {
            output.clear();
        }
        result
    }

    fn recognize_inner(
        &mut self,
        bytes: &[u8],
        due: Instant,
        output: &mut String,
    ) -> Result<OcrStats> {
        deadline(due)?;
        let start = Instant::now();
        let header = images::inspect(bytes)?;
        let mut stats = OcrStats::new(&self.settings, core::get_num_threads()?);
        stats.stage_times[0] += start.elapsed();
        for frame in 0..header.frames {
            deadline(due)?;
            let start = Instant::now();
            let gray = images::decode(bytes, &header, frame)?;
            self.trace.tensor(
                "decoded",
                &[gray.rows() as i64, gray.cols() as i64],
                gray.data_typed::<u8>()?,
            )?;
            stats.stage_times[0] += start.elapsed();
            let start = Instant::now();
            let turn = self.orientation(&gray, due)?;
            let gray = geometry::turn(gray, turn)?;
            let skew = self.scratch.estimate_skew(&gray, due)?;
            self.trace.tensor("turn-skew", &[2], &[turn as f64, skew])?;
            let gray = geometry::deskew(gray, skew)?;
            self.trace.tensor(
                "upright",
                &[gray.rows() as i64, gray.cols() as i64],
                gray.data_typed::<u8>()?,
            )?;
            stats.stage_times[1] += start.elapsed();
            let start = Instant::now();
            let count = self.detect(&gray, due)?;
            stats.stage_times[2] += start.elapsed();
            let boxes = &self.boxes[..count];
            self.lines.reset();
            let start = Instant::now();
            let mut widths = [0_usize; MAX_LINES];
            let mut flags = [false; MAX_LINES];
            recognition::widths(boxes, geometry::LINE_HEIGHT, &mut widths[..count])?;
            recognition::classify(
                &mut self.script,
                &gray,
                boxes,
                &widths[..count],
                self.settings.batch_size,
                &mut self.scratch,
                &mut self.text_tensor,
                &mut flags[..count],
                due,
                &mut self.trace,
                "script",
            )?;
            recognition::text(
                &mut self.recognizer,
                &gray,
                boxes,
                &widths[..count],
                &flags[..count],
                false,
                self.settings.batch_size,
                &mut self.scratch,
                &mut self.text_tensor,
                &mut self.lines,
                &mut stats,
                due,
                &mut self.trace,
            )?;
            recognition::text(
                &mut self.chinese,
                &gray,
                boxes,
                &widths[..count],
                &flags[..count],
                true,
                self.settings.batch_size,
                &mut self.scratch,
                &mut self.text_tensor,
                &mut self.lines,
                &mut stats,
                due,
                &mut self.trace,
            )?;
            stats.stage_times[3] += start.elapsed();
            let start = Instant::now();
            if let Some(math) = &mut self.math {
                math.recognize(
                    &gray,
                    boxes,
                    &widths[..count],
                    self.settings.batch_size,
                    &mut self.scratch,
                    &mut self.text_tensor,
                    &mut self.lines,
                    due,
                    &mut self.trace,
                )?;
            }
            stats.stage_times[4] += start.elapsed();
            let start = Instant::now();
            self.page.assemble(boxes, &self.lines, due)?;
            stats.stage_times[6] += start.elapsed();
            let start = Instant::now();
            let mut indices = [0_usize; MAX_LINES];
            self.reading_order([gray.cols() as f32, gray.rows() as f32], &mut indices, due)?;
            stats.stage_times[5] += start.elapsed();
            let start = Instant::now();
            if frame > 0 {
                recognition::append(output, "\n\u{c}\n")?;
            }
            self.page.write(&indices[..self.page.count], output)?;
            stats.frames += 1;
            stats.lines += self.page.count;
            stats.stage_times[6] += start.elapsed();
            deadline(due)?;
        }
        stats.output_bytes = output.len();
        Ok(stats)
    }

    fn orientation(&mut self, gray: &Mat, due: Instant) -> Result<usize> {
        let _span = tracing::info_span!(
            "ocr.orientation",
            model = "orient",
            width = gray.cols(),
            height = gray.rows()
        )
        .entered();
        let values = self.orient_tensor.data_typed_mut::<f32>()?;
        self.scratch
            .page_input(gray, geometry::ORIENT_SIDE, true, values)?;
        self.trace.tensor(
            "orientation-input",
            &[
                1,
                1,
                geometry::ORIENT_SIDE as i64,
                geometry::ORIENT_SIDE as i64,
            ],
            values,
        )?;
        let tensor = TensorRef::from_array_view((
            [1, 1, geometry::ORIENT_SIDE, geometry::ORIENT_SIDE],
            &*values,
        ))?;
        deadline(due)?;
        let outputs = self.orient.run(ort::inputs![tensor])?;
        let (shape, probabilities) = outputs[0].try_extract_tensor::<f32>()?;
        self.trace
            .tensor("orientation-output", shape, probabilities)?;
        if shape.as_ref() != [1, 4]
            || !probabilities
                .iter()
                .all(|p| p.is_finite() && (0.0..=1.0).contains(p))
        {
            return Err(EngineError::Shape("orientation probabilities"));
        }
        let mut best = 0;
        for k in 1..4 {
            if probabilities[k] > probabilities[best] {
                best = k;
            }
        }
        deadline(due)?;
        Ok(if probabilities[best] > 0.95 { best } else { 0 })
    }

    fn detect_at(&mut self, gray: &Mat, side: usize, due: Instant) -> Result<(usize, f64)> {
        let _span = tracing::info_span!("ocr.detection", model = "det", long_side = side).entered();
        let values = self.detector_tensor.data_typed_mut::<f32>()?;
        let (height, width, scale) = self.scratch.page_input(gray, side, false, values)?;
        self.trace.tensor(
            "detection-input",
            &[1, 1, height as i64, width as i64],
            &values[..height * width],
        )?;
        let input = TensorRef::from_array_view(([1, 1, height, width], &values[..height * width]))?;
        deadline(due)?;
        let outputs = self.detector.run(ort::inputs![input])?;
        let (shape, values) = outputs[0].try_extract_tensor::<f32>()?;
        self.trace.tensor("detection-output", shape, values)?;
        if shape.as_ref() != [1, 3, (height / 2) as i64, (width / 2) as i64] {
            return Err(EngineError::Shape("detector output shape"));
        }
        let count =
            self.scratch
                .decode_detector(values, height / 2, width / 2, &mut self.boxes, due)?;
        self.trace.tensor(
            "detection-boxes-unscaled",
            &[count as i64, 4],
            self.boxes[..count].as_flattened(),
        )?;
        tracing::debug!(detected_lines = count, "detector components decoded");
        Ok((count, scale))
    }

    fn detect(&mut self, gray: &Mat, due: Instant) -> Result<usize> {
        let (mut count, mut scale) = self.detect_at(gray, geometry::DET_SIDE, due)?;
        if count >= 8 {
            let mut heights = [0.0_f32; MAX_LINES];
            for (height, b) in heights[..count].iter_mut().zip(&self.boxes[..count]) {
                *height = b[3] - b[1];
            }
            heights[..count].sort_unstable_by(f32::total_cmp);
            let median = if count % 2 == 1 {
                heights[count / 2]
            } else {
                (heights[count / 2 - 1] + heights[count / 2]) * 0.5
            };
            if median < 9.0 {
                let side = (geometry::DET_SIDE as f64 * 12.0 / f64::from(median.max(1.0)))
                    .min(geometry::DET_MAX_SIDE as f64) as usize
                    / 32
                    * 32;
                if side > geometry::DET_SIDE {
                    (count, scale) = self.detect_at(gray, side, due)?;
                }
            }
        }
        for b in &mut self.boxes[..count] {
            b[0] = (b[0] / scale as f32).clamp(0.0, gray.cols() as f32);
            b[2] = (b[2] / scale as f32).clamp(0.0, gray.cols() as f32);
            b[1] = (b[1] / scale as f32).clamp(0.0, gray.rows() as f32);
            b[3] = (b[3] / scale as f32).clamp(0.0, gray.rows() as f32);
        }
        self.trace.tensor(
            "detection-boxes",
            &[count as i64, 4],
            self.boxes[..count].as_flattened(),
        )?;
        Ok(count)
    }

    fn reading_order(&mut self, wh: [f32; 2], indices: &mut [usize], due: Instant) -> Result<()> {
        let n = self.page.count;
        let _span = tracing::info_span!("ocr.reading_order", model = "order", lines = n).entered();
        if n < 2 {
            if n == 1 {
                indices[0] = 0;
            }
            return Ok(());
        }
        let mut features = [0.0_f32; MAX_LINES * order::N_FEATURES];
        let mut texts = [""; MAX_LINES];
        for (text, range) in texts[..n].iter_mut().zip(&self.page.ranges[..n]) {
            *text = &self.page.text[range.start..range.end];
        }
        order::features(
            &self.page.boxes[..n],
            &texts[..n],
            wh,
            &mut features[..n * order::N_FEATURES],
        )?;
        self.trace.tensor(
            "order-features",
            &[1, n as i64, order::N_FEATURES as i64],
            &features[..n * order::N_FEATURES],
        )?;
        let mask = [true; MAX_LINES];
        let f = TensorRef::from_array_view((
            [1, n, order::N_FEATURES],
            &features[..n * order::N_FEATURES],
        ))?;
        let m = TensorRef::from_array_view(([1, n], &mask[..n]))?;
        deadline(due)?;
        let outputs = self.order.run(ort::inputs![f, m])?;
        let (pair_shape, pairwise) = outputs[0].try_extract_tensor::<f32>()?;
        let (class_shape, classes) = outputs[1].try_extract_tensor::<f32>()?;
        let (same_shape, same) = outputs[2].try_extract_tensor::<f32>()?;
        self.trace.tensor("order-pairwise", pair_shape, pairwise)?;
        self.trace.tensor("order-classes", class_shape, classes)?;
        self.trace.tensor("order-same-block", same_shape, same)?;
        if pair_shape.as_ref() != [1, n as i64, n as i64]
            || same_shape != pair_shape
            || class_shape.as_ref() != [1, n as i64, 10]
        {
            return Err(EngineError::Shape("reading-order output shapes"));
        }
        order::decode(
            pairwise,
            same,
            classes,
            &self.page.boxes[..n],
            &mut self.order_workspace,
            &mut indices[..n],
            due,
        )?;
        let mut trace_order = [0_i64; MAX_LINES];
        for (out, index) in trace_order[..n].iter_mut().zip(&indices[..n]) {
            *out = *index as i64;
        }
        self.trace
            .tensor("order-permutation", &[n as i64], &trace_order[..n])
    }
}

pub(crate) fn session(
    environment: &Environment,
    directory: &Path,
    name: &str,
    settings: &EngineConfig,
    inputs: &[&str],
    outputs: &[&str],
) -> Result<Session> {
    fn build(
        environment: &Environment,
        path: &Path,
        settings: &EngineConfig,
    ) -> ort::Result<Session> {
        Session::builder(environment)?
            .with_intra_threads(settings.threads)?
            .with_inter_threads(1)?
            .with_parallel_execution(false)?
            .with_intra_op_spinning(settings.intra_op_spinning)?
            .with_inter_op_spinning(settings.inter_op_spinning)?
            .with_optimization_level(GraphOptimizationLevel::All)?
            .with_memory_pattern(false)?
            .with_execution_providers([ort::ep::CPU::default()
                .with_arena_allocator(false)
                .build()])?
            .commit_from_file(path)
    }
    let session = build(environment, &directory.join(name), settings)?;
    if session.inputs().len() != inputs.len()
        || session.outputs().len() != outputs.len()
        || !session
            .inputs()
            .iter()
            .zip(inputs)
            .all(|(actual, expected)| actual.name() == *expected)
        || !session
            .outputs()
            .iter()
            .zip(outputs)
            .all(|(actual, expected)| actual.name() == *expected)
    {
        return Err(EngineError::Shape(
            "model graph input/output contract changed",
        ));
    }
    Ok(session)
}
