//! Single-owner synchronous inference; the server runs this outside the async executor.
//!
//! The page flow follows `OrtOCR.__call__`, `_turn`, `_read` and `assemble` in
//! `indra_ocr/pipeline.py` at upstream commit
//! `0e96b93c6810f17cf8352acbb4487cc9e45b66d5` (Apache-2.0).

use std::{
    path::Path,
    sync::{Mutex, OnceLock},
    time::Instant,
};

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
    images, lm, math, order,
    recognition::{self, Lines, Rescorer, Routing, Script},
};

#[cfg(test)]
mod tests;

/// A turn is applied only above this probability (`ORIENT_MIN`).
const ORIENT_MIN: f32 = 0.95;
const ORIENT_CLASSES: usize = 5;

// OpenCV thread configuration is process-global and explicitly not thread safe.
// Configure it once before any worker starts inference. Later engines may reuse
// the setting, but must not reconfigure an already-running native thread pool.
static OPENCV_THREADS: Mutex<Option<usize>> = Mutex::new(None);

// ort permits one environment construction for the entire process, including
// after all engines are dropped. Keep it alive and initialize it fallibly once.
// Session-specific thread and allocator settings remain on each session builder.
static ORT_ENVIRONMENT: OnceLock<Environment> = OnceLock::new();
static ORT_INITIALIZATION: Mutex<()> = Mutex::new(());

fn environment() -> Result<&'static Environment> {
    if let Some(environment) = ORT_ENVIRONMENT.get() {
        return Ok(environment);
    }
    // Stable OnceLock has no fallible initializer. Serialize construction, then
    // recheck because another startup worker may have initialized while we waited.
    let _guard = ORT_INITIALIZATION
        .lock()
        .map_err(|_| EngineError::Config("ORT environment initialization lock is poisoned"))?;
    if let Some(environment) = ORT_ENVIRONMENT.get() {
        return Ok(environment);
    }
    let environment = ort::init()
        .with_name("indra-cpu")
        .with_telemetry(false)
        .build()?;
    ORT_ENVIRONMENT
        .set(environment)
        .map_err(|_| EngineError::Config("ORT environment initialized outside its startup lock"))?;
    ORT_ENVIRONMENT
        .get()
        .ok_or(EngineError::Config("ORT environment was not retained"))
}

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
    pub(crate) shared: SharedEngine,
    pub(crate) page: PageState,
}

/// Per-page outputs move once into their owner at startup. Shared model sessions
/// and tensor scratch never move when the experimental scheduler changes owners.
pub(crate) struct PageState {
    pub(crate) boxes: [[f32; 4]; MAX_LINES],
    pub(crate) lines: Lines,
    pub(crate) routing: Routing,
    pub(crate) page: assembly::Page,
}

impl PageState {
    pub(crate) fn new() -> Result<Self> {
        Ok(Self {
            boxes: [[0.0; 4]; MAX_LINES],
            lines: Lines::new()?,
            routing: Routing::new(),
            page: assembly::Page::new(),
        })
    }
}

pub(crate) struct SharedEngine {
    pub(crate) detector: Session,
    pub(crate) orient: Session,
    /// `rec`, `rec_cjk`, `rec_rtl`, `rec_world`, in `Script::GROUPS` order.
    pub(crate) recognizers: [Session; 4],
    pub(crate) script: Session,
    pub(crate) order: Session,
    pub(crate) math: Option<math::Reader>,
    /// `lm.npz`, `lm_cjk.npz`, `lm_world.npz`; the RTL recogniser has none.
    pub(crate) models: [Option<lm::Model>; 4],
    pub(crate) beam: Box<lm::Scratch>,
    pub(crate) settings: EngineConfig,
    pub(crate) scratch: Scratch,
    pub(crate) detector_tensor: Mat,
    pub(crate) orient_tensor: Mat,
    /// One-band crops: classifiers and group recognisers.
    pub(crate) text_tensor: Mat,
    /// Tall-window crops of the base recogniser.
    pub(crate) window_tensor: Mat,
    pub(crate) preview: Mat,
    pub(crate) preview_turned: Mat,
    pub(crate) order_workspace: order::Workspace,
    pub(crate) trace: crate::trace::Recorder,
}

/// The upright page and how it was turned and straightened.
pub(crate) struct Upright {
    pub gray: Mat,
    pub class: usize,
    pub skew: f64,
}

impl Upright {
    pub const fn vertical(&self) -> bool {
        self.class == geometry::VERTICAL
    }
}

impl Engine {
    /// Verify every selected artifact before creating native sessions. No download occurs.
    pub fn load(cache: &Path, settings: &EngineConfig) -> Result<Self> {
        settings.validate()?;
        indra_models::verify(cache, settings.math)?;
        configure_opencv(settings.cv_threads)?;
        let environment = environment()?;
        let recognizer = |group: Script| {
            session(
                environment,
                cache,
                match group {
                    Script::Base => "rec.onnx",
                    Script::Cjk => "rec_cjk.onnx",
                    Script::Rtl => "rec_rtl.onnx",
                    Script::World => "rec_world.onnx",
                    Script::Math => {
                        return Err(EngineError::Config("math is not a CTC recogniser"));
                    }
                },
                settings,
                &["x", "widths"],
                &["ids", "conf", "lengths", "blank", "topv", "topi"],
            )
        };
        let models = if settings.lm {
            [
                Some(lm::Model::load(&cache.join("lm.npz"))?),
                Some(lm::Model::load(&cache.join("lm_cjk.npz"))?),
                None,
                Some(lm::Model::load(&cache.join("lm_world.npz"))?),
            ]
        } else {
            [None, None, None, None]
        };
        Ok(Self {
            shared: SharedEngine {
                detector: session(environment, cache, "det.onnx", settings, &["x"], &["out"])?,
                orient: session(environment, cache, "orient.onnx", settings, &["x"], &["p"])?,
                recognizers: [
                    recognizer(Script::Base)?,
                    recognizer(Script::Cjk)?,
                    recognizer(Script::Rtl)?,
                    recognizer(Script::World)?,
                ],
                script: session(
                    environment,
                    cache,
                    "script.onnx",
                    settings,
                    &["x", "widths"],
                    &["p"],
                )?,
                order: session(
                    environment,
                    cache,
                    "order.onnx",
                    settings,
                    &["feats", "mask"],
                    &["a", "cls", "same", "tab"],
                )?,
                math: if settings.math {
                    Some(math::Reader::load(environment, cache, settings)?)
                } else {
                    None
                },
                models,
                beam: lm::Scratch::new(),
                settings: settings.settings(),
                scratch: Scratch::new()?,
                detector_tensor: geometry::floats(geometry::DET_MAX_SIDE * geometry::DET_MAX_SIDE)?,
                orient_tensor: geometry::floats(geometry::ORIENT_SIDE * geometry::ORIENT_SIDE)?,
                text_tensor: geometry::floats(batching::TEXT_COLUMNS * geometry::LINE_HEIGHT)?,
                window_tensor: geometry::floats(
                    batching::WINDOW_COLUMNS * geometry::WINDOW_HEIGHT,
                )?,
                preview: Mat::default(),
                preview_turned: Mat::default(),
                order_workspace: order::Workspace::new()?,
                trace: crate::trace::Recorder::Disabled,
            },
            page: PageState::new()?,
        })
    }

    /// Export real intermediate tensors for an explicit diagnostic run.
    /// The empty destination receives at most 512 MiB and can contain document data.
    /// Production logging never enables or includes this evidence.
    pub fn set_trace_directory(&mut self, path: &Path) -> Result<()> {
        self.shared.trace = crate::trace::Recorder::directory(path)?;
        Ok(())
    }

    /// Execute every enabled model before readiness: orientation, detection,
    /// the script classifier, all four recognisers and the math KV cache steps.
    pub fn warmup(&mut self, due: Instant) -> Result<()> {
        deadline(due)?;
        let blank = Mat::new_rows_cols_with_default(128, 256, core::CV_8UC1, Scalar::all(255.0))?;
        let _ = self.shared.orientation(&blank, due)?;
        let _ = self.shared.detect(&blank, &mut self.page.boxes, due)?;
        self.page.boxes[0] = [4.0, 4.0, 200.0, 32.0];
        self.page.boxes[1] = [4.0, 45.0, 200.0, 73.0];
        self.page.lines.reset();
        let boxes = &self.page.boxes[..2];
        self.page.routing.prepare(&blank, boxes)?;
        let mut probabilities = [0.0_f32; 2 * Script::GROUPS.len()];
        recognition::classify(
            &mut self.shared.script,
            &blank,
            boxes,
            &self.page.routing,
            self.shared.settings.batch_size,
            &mut self.shared.scratch,
            &mut self.shared.text_tensor,
            Script::GROUPS.len(),
            &mut probabilities,
            due,
            &mut self.shared.trace,
            "script",
        )?;
        let mut stats = OcrStats::new(&self.shared.settings, core::get_num_threads()?);
        for group in Script::GROUPS {
            self.page.routing.group[..2].fill(group);
            self.page.routing.sure[..2].fill(true);
            self.shared.read(
                group,
                &blank,
                boxes,
                &mut self.page.routing,
                &mut self.page.lines,
                &mut stats,
                due,
            )?;
        }
        if let Some(math) = &mut self.shared.math {
            math.recognize(
                &blank,
                boxes,
                &self.page.routing,
                self.shared.settings.batch_size,
                &mut self.shared.scratch,
                &mut self.shared.text_tensor,
                &mut self.page.lines,
                due,
                &mut self.shared.trace,
            )?;
            math.read(
                &blank,
                boxes,
                &[true, true],
                &self.page.routing.dark,
                self.shared.settings.batch_size,
                &mut self.shared.scratch,
                &mut self.page.lines,
                due,
                &mut self.shared.trace,
            )?;
        }
        self.page.page.assemble(boxes, &self.page.lines, due)?;
        let mut indices = [0; MAX_LINES];
        self.shared
            .reading_order(&self.page.page, [256.0, 128.0], false, &mut indices, due)?;
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
        let mut stats = OcrStats::new(&self.shared.settings, core::get_num_threads()?);
        stats.stage_times[0] += start.elapsed();
        for frame in 0..header.frames {
            deadline(due)?;
            let start = Instant::now();
            let gray = images::decode(bytes, &header, frame)?;
            self.shared.trace.tensor(
                "decoded",
                &[gray.rows() as i64, gray.cols() as i64],
                gray.data_typed::<u8>()?,
            )?;
            stats.stage_times[0] += start.elapsed();
            let start = Instant::now();
            let upright = self.shared.upright(gray, due)?;
            stats.stage_times[1] += start.elapsed();
            let start = Instant::now();
            let count = self
                .shared
                .detect(&upright.gray, &mut self.page.boxes, due)?;
            stats.stage_times[2] += start.elapsed();
            let boxes = &self.page.boxes[..count];
            self.page.lines.reset();
            let start = Instant::now();
            self.shared.recognize_lines(
                &upright,
                boxes,
                &mut self.page.routing,
                &mut self.page.lines,
                &mut stats,
                due,
            )?;
            stats.stage_times[3] += start.elapsed();
            let start = Instant::now();
            if let Some(math) = &mut self.shared.math {
                math.recognize(
                    &upright.gray,
                    boxes,
                    &self.page.routing,
                    self.shared.settings.batch_size,
                    &mut self.shared.scratch,
                    &mut self.shared.text_tensor,
                    &mut self.page.lines,
                    due,
                    &mut self.shared.trace,
                )?;
            }
            stats.stage_times[4] += start.elapsed();
            let start = Instant::now();
            self.page.page.assemble(boxes, &self.page.lines, due)?;
            stats.stage_times[6] += start.elapsed();
            let start = Instant::now();
            let mut indices = [0_usize; MAX_LINES];
            self.shared.reading_order(
                &self.page.page,
                [upright.gray.cols() as f32, upright.gray.rows() as f32],
                upright.vertical(),
                &mut indices,
                due,
            )?;
            stats.stage_times[5] += start.elapsed();
            let start = Instant::now();
            self.shared.trace_page_boxes(&self.page.page, &upright)?;
            if frame > 0 {
                recognition::append(output, "\n\u{c}\n")?;
            }
            self.page
                .page
                .write(&indices[..self.page.page.count], output)?;
            stats.frames += 1;
            stats.lines += self.page.page.count;
            if upright.vertical() {
                stats.vertical_frames += 1;
            }
            stats.stage_times[6] += start.elapsed();
            deadline(due)?;
        }
        stats.output_bytes = output.len();
        Ok(stats)
    }
}

impl SharedEngine {
    /// `__call__` and `_turn`: the skew of the preview, the orientation of the
    /// straightened preview, the quarter turn, a second skew estimate for a
    /// turned page, then the straightened full page.
    pub(crate) fn upright(&mut self, gray: Mat, due: Instant) -> Result<Upright> {
        let has_preview = geometry::preview(&gray, &mut self.preview)?;
        let mut angle = {
            let small = if has_preview { &self.preview } else { &gray };
            self.scratch.estimate_skew(small, due)?
        };
        let class = if geometry::deskews(angle) {
            let small = if has_preview { &self.preview } else { &gray };
            geometry::rotate_page(small, angle, &mut self.preview_turned)?;
            orientation(
                &mut self.orient,
                &mut self.scratch,
                &mut self.orient_tensor,
                &mut self.trace,
                &self.preview_turned,
                due,
            )?
        } else {
            orientation(
                &mut self.orient,
                &mut self.scratch,
                &mut self.orient_tensor,
                &mut self.trace,
                if has_preview { &self.preview } else { &gray },
                due,
            )?
        };
        let mut turned = Mat::default();
        let gray = if geometry::turn(&gray, class, &mut turned)? {
            turned
        } else {
            gray
        };
        if matches!(class, 1 | 3 | geometry::VERTICAL) {
            // A half turn keeps the rows; after a quarter turn the rows were
            // columns, so the angle is measured again on the preview turned
            // like the page (`np.rot90(small, q)`).
            angle = if has_preview {
                geometry::turn(&self.preview, class, &mut self.preview_turned)?;
                self.scratch.estimate_skew(&self.preview_turned, due)?
            } else {
                self.scratch.estimate_skew(&gray, due)?
            };
        }
        let (gray, skew) = if geometry::deskews(angle) {
            let mut straight = Mat::default();
            geometry::rotate_page(&gray, angle, &mut straight)?;
            (straight, angle)
        } else {
            (gray, 0.0)
        };
        self.trace
            .tensor("turn-skew", &[2], &[class as f64, skew])?;
        self.trace.tensor(
            "upright",
            &[gray.rows() as i64, gray.cols() as i64],
            gray.data_typed::<u8>()?,
        )?;
        Ok(Upright { gray, class, skew })
    }

    pub(crate) fn orientation(&mut self, gray: &Mat, due: Instant) -> Result<usize> {
        orientation(
            &mut self.orient,
            &mut self.scratch,
            &mut self.orient_tensor,
            &mut self.trace,
            gray,
            due,
        )
    }

    fn detect_at(
        &mut self,
        gray: &Mat,
        boxes: &mut [[f32; 4]; MAX_LINES],
        side: usize,
        due: Instant,
    ) -> Result<(usize, f64)> {
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
        let count = self
            .scratch
            .decode_detector(values, height / 2, width / 2, boxes, due)?;
        self.trace.tensor(
            "detection-boxes-unscaled",
            &[count as i64, 4],
            boxes[..count].as_flattened(),
        )?;
        tracing::debug!(detected_lines = count, "detector components decoded");
        Ok((count, scale))
    }

    pub(crate) fn detect(
        &mut self,
        gray: &Mat,
        boxes: &mut [[f32; 4]; MAX_LINES],
        due: Instant,
    ) -> Result<usize> {
        let (mut count, mut scale) = self.detect_at(gray, boxes, geometry::DET_SIDE, due)?;
        if count >= 8 {
            let mut heights = [0.0_f32; MAX_LINES];
            for (height, b) in heights[..count].iter_mut().zip(&boxes[..count]) {
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
                    (count, scale) = self.detect_at(gray, boxes, side, due)?;
                }
            }
        }
        for b in &mut boxes[..count] {
            b[0] = (b[0] / scale as f32).clamp(0.0, gray.cols() as f32);
            b[2] = (b[2] / scale as f32).clamp(0.0, gray.cols() as f32);
            b[1] = (b[1] / scale as f32).clamp(0.0, gray.rows() as f32);
            b[3] = (b[3] / scale as f32).clamp(0.0, gray.rows() as f32);
        }
        self.trace.tensor(
            "detection-boxes",
            &[count as i64, 4],
            boxes[..count].as_flattened(),
        )?;
        Ok(count)
    }

    /// `recognize`: crops and dark decisions, routing by the script classifier
    /// (every line is a CJK column on a vertical page), then the base read and
    /// each group read in the reference's order.
    pub(crate) fn recognize_lines(
        &mut self,
        upright: &Upright,
        boxes: &[[f32; 4]],
        routing: &mut Routing,
        lines: &mut Lines,
        stats: &mut OcrStats,
        due: Instant,
    ) -> Result<()> {
        routing.prepare(&upright.gray, boxes)?;
        let mut dark = [false; MAX_LINES];
        for (flag, value) in dark.iter_mut().zip(&routing.dark[..boxes.len()]) {
            *flag = *value;
        }
        self.trace
            .tensor("line-dark", &[boxes.len() as i64], &dark[..boxes.len()])?;
        if upright.vertical() {
            routing.vertical(boxes.len());
        } else if !boxes.is_empty() {
            let mut probabilities = [0.0_f32; MAX_LINES * 4];
            recognition::classify(
                &mut self.script,
                &upright.gray,
                boxes,
                routing,
                self.settings.batch_size,
                &mut self.scratch,
                &mut self.text_tensor,
                Script::GROUPS.len(),
                &mut probabilities[..boxes.len() * 4],
                due,
                &mut self.trace,
                "script",
            )?;
            stats.dual_read_lines += routing.route(
                &probabilities[..boxes.len() * 4],
                boxes.len(),
                self.settings.route_sure,
            )?;
            let mut groups = [0_i64; MAX_LINES];
            for (slot, group) in groups.iter_mut().zip(&routing.group[..boxes.len()]) {
                *slot = Script::GROUPS
                    .iter()
                    .position(|candidate| candidate == group)
                    .unwrap_or(0) as i64;
            }
            self.trace
                .tensor("line-groups", &[boxes.len() as i64], &groups[..boxes.len()])?;
        }
        // The reference reads base first, then the groups in meta.json order.
        for group in [Script::Base, Script::World, Script::Rtl, Script::Cjk] {
            self.read(group, &upright.gray, boxes, routing, lines, stats, due)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn read(
        &mut self,
        group: Script,
        gray: &Mat,
        boxes: &[[f32; 4]],
        routing: &mut Routing,
        lines: &mut Lines,
        stats: &mut OcrStats,
        due: Instant,
    ) -> Result<()> {
        let index = Script::GROUPS
            .iter()
            .position(|candidate| *candidate == group)
            .ok_or(EngineError::Config("math is not a CTC recogniser"))?;
        let rescorer = self.models[index].as_ref().map(|model| Rescorer {
            model,
            params: if group == Script::Base {
                lm::Params::BASE
            } else {
                lm::Params::GROUP
            },
        });
        let batch = if group == Script::Base {
            &mut self.window_tensor
        } else {
            &mut self.text_tensor
        };
        recognition::text(
            &mut self.recognizers[index],
            group,
            rescorer,
            &mut self.beam,
            gray,
            boxes,
            routing,
            self.settings.batch_size,
            &mut self.scratch,
            batch,
            lines,
            stats,
            due,
            &mut self.trace,
        )
    }

    /// Reading order. A right-to-left page is mirrored left to right for the
    /// order features, so its first column is the right one; a vertical page
    /// takes XY-cut blocks instead of the order model's decision.
    pub(crate) fn reading_order(
        &mut self,
        page: &assembly::Page,
        wh: [f32; 2],
        vertical: bool,
        indices: &mut [usize],
        due: Instant,
    ) -> Result<()> {
        let n = page.count;
        let _span = tracing::info_span!("ocr.reading_order", model = "order", lines = n).entered();
        if n < 2 {
            if n == 1 {
                indices[0] = 0;
            }
            return Ok(());
        }
        let mut mirrored = [[0.0_f32; 4]; MAX_LINES];
        for (target, b) in mirrored[..n].iter_mut().zip(&page.boxes[..n]) {
            *target = if page.rtl {
                [wh[0] - b[2], b[1], wh[0] - b[0], b[3]]
            } else {
                *b
            };
        }
        let boxes = &mirrored[..n];
        let mut features = [0.0_f32; MAX_LINES * order::N_FEATURES];
        let mut texts = [""; MAX_LINES];
        for (text, range) in texts[..n].iter_mut().zip(&page.ranges[..n]) {
            *text = &page.text[range.start..range.end];
        }
        order::features(
            boxes,
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
            boxes,
            &mut self.order_workspace,
            &mut indices[..n],
            due,
        )?;
        let mut trace_order = [0_i64; MAX_LINES];
        for (out, index) in trace_order[..n].iter_mut().zip(&indices[..n]) {
            *out = *index as i64;
        }
        self.trace
            .tensor("order-permutation", &[n as i64], &trace_order[..n])?;
        if vertical {
            // The order model saw no vertical pages: blocks and order come from
            // XY-cut on the boxes as they are, replacing the model's decision.
            for (slot, index) in indices[..n].iter_mut().enumerate() {
                *index = slot;
            }
            order::xycut_blocks(&page.boxes[..n], &mut indices[..n], due)?;
            for (out, index) in trace_order[..n].iter_mut().zip(&indices[..n]) {
                *out = *index as i64;
            }
            self.trace
                .tensor("order-blocks", &[n as i64], &trace_order[..n])?;
        }
        Ok(())
    }

    /// Diagnostic only: the final line boxes mapped back to the page as it was
    /// decoded (`unrotate_box` for the deskew, `(x, y) → (W − y, x)` for a
    /// vertical page), as `Page.lines[*].box` reports them.
    fn trace_page_boxes(&mut self, page: &assembly::Page, upright: &Upright) -> Result<()> {
        if matches!(self.trace, crate::trace::Recorder::Disabled) {
            return Ok(());
        }
        let n = page.count;
        let mut boxes = [[0.0_f32; 4]; MAX_LINES];
        let wh = [upright.gray.cols(), upright.gray.rows()];
        for (target, b) in boxes[..n].iter_mut().zip(&page.boxes[..n]) {
            let mut b = [
                round_tenth(b[0]),
                round_tenth(b[1]),
                round_tenth(b[2]),
                round_tenth(b[3]),
            ];
            if upright.skew != 0.0 {
                b = geometry::unrotate_box(&b, upright.skew, wh)?;
            }
            if upright.vertical() {
                b = geometry::vertical_box_back(&b, wh[1]);
            }
            *target = b;
        }
        self.trace
            .tensor("page-boxes", &[n as i64, 4], boxes[..n].as_flattened())
    }
}

/// `orientation`: the most likely of the five classes when it passes
/// `ORIENT_MIN`, else upright. Borrows only the fields it needs so the page
/// preview can live in the same engine while it runs.
fn orientation(
    orient: &mut Session,
    scratch: &mut Scratch,
    tensor: &mut Mat,
    trace: &mut crate::trace::Recorder,
    gray: &Mat,
    due: Instant,
) -> Result<usize> {
    let _span = tracing::info_span!(
        "ocr.orientation",
        model = "orient",
        width = gray.cols(),
        height = gray.rows()
    )
    .entered();
    let values = tensor.data_typed_mut::<f32>()?;
    scratch.page_input(gray, geometry::ORIENT_SIDE, true, values)?;
    trace.tensor(
        "orientation-input",
        &[
            1,
            1,
            geometry::ORIENT_SIDE as i64,
            geometry::ORIENT_SIDE as i64,
        ],
        values,
    )?;
    let input = TensorRef::from_array_view((
        [1, 1, geometry::ORIENT_SIDE, geometry::ORIENT_SIDE],
        &*values,
    ))?;
    deadline(due)?;
    let outputs = orient.run(ort::inputs![input])?;
    let (shape, probabilities) = outputs[0].try_extract_tensor::<f32>()?;
    trace.tensor("orientation-output", shape, probabilities)?;
    if shape.as_ref() != [1, ORIENT_CLASSES as i64]
        || !probabilities
            .iter()
            .all(|p| p.is_finite() && (0.0..=1.0).contains(p))
    {
        return Err(EngineError::Shape("orientation probabilities"));
    }
    let mut best = 0;
    for k in 1..ORIENT_CLASSES {
        if probabilities[k] > probabilities[best] {
            best = k;
        }
    }
    deadline(due)?;
    Ok(if probabilities[best] > ORIENT_MIN {
        best
    } else {
        0
    })
}

/// `round(float(v), 1)` of a box coordinate as the reference reports it.
fn round_tenth(value: f32) -> f32 {
    ((f64::from(value) * 10.0).round_ties_even() / 10.0) as f32
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
            .with_config_entry(
                "session.force_spinning_stop",
                if settings.threads > 1 && settings.intra_op_spinning {
                    "1"
                } else {
                    "0"
                },
            )?
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
