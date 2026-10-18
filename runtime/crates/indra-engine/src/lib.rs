//! CPU-only Indra inference with explicit resource and allocation boundaries.
//!
//! The selected image-to-`Page.text` path is a Rust port of Indra 0.3.2.
//! Native allocations remain inside ONNX Runtime/OpenCV; Rust request state uses
//! bounded arrays and reusable arenas. See `docs/native-engine.md` for limits.

use std::{
    io,
    time::{Duration, Instant},
};
use thiserror::Error;

mod assembly;
mod batching;
mod continuous;
mod geometry;
mod images;
mod math;
mod order;
mod recognition;
mod runtime;
mod trace;
mod vocabulary;

pub use continuous::{Continuous, ContinuousInvocation, ContinuousProgress, ContinuousRequest};
pub use runtime::Engine;
#[cfg(feature = "math-compaction-experiment")]
pub use math::{MathArm, MathReport};

pub use indra_models::{MODEL_REPOSITORY, MODEL_REVISION, ModelArtifact, model_artifacts};

pub const MAX_INPUT_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_PIXELS: usize = 25_000_000;
pub const MAX_SIDE: usize = 16_384;
pub const MAX_FRAMES: usize = 16;
pub const MAX_LINES: usize = 1024;
pub const MAX_BATCH: usize = 64;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("invalid image: {0}")]
    InvalidImage(&'static str),
    #[error("unsupported image: {0}")]
    UnsupportedImage(&'static str),
    #[error("resource limit exceeded: {0}")]
    Limit(&'static str),
    #[error("OCR deadline exceeded")]
    Deadline,
    #[error("invalid engine configuration: {0}")]
    Config(&'static str),
    #[error("unexpected model tensor or intermediate shape: {0}")]
    Shape(&'static str),
    #[error("model cache verification failed")]
    Cache(#[from] indra_models::CacheError),
    #[error("ONNX Runtime inference failed")]
    Ort(#[from] ort::Error),
    #[error("OpenCV native operation failed")]
    OpenCv(#[from] opencv::Error),
    #[error("engine I/O failed")]
    Io(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, EngineError>;

pub(crate) fn deadline(due: Instant) -> Result<()> {
    if Instant::now() >= due {
        Err(EngineError::Deadline)
    } else {
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct EngineConfig {
    pub threads: usize,
    pub cv_threads: usize,
    pub intra_op_spinning: bool,
    pub inter_op_spinning: bool,
    pub math: bool,
    pub batch_size: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            threads: 1,
            cv_threads: 1,
            intra_op_spinning: true,
            inter_op_spinning: true,
            math: false,
            batch_size: MAX_BATCH,
        }
    }
}

impl EngineConfig {
    pub fn validate(&self) -> Result<()> {
        if !(1..=2).contains(&self.threads) || !(1..=2).contains(&self.cv_threads) {
            return Err(EngineError::Config(
                "ORT and OpenCV thread counts must be one or two",
            ));
        }
        if !(1..=MAX_BATCH).contains(&self.batch_size) {
            return Err(EngineError::Config(
                "recognition batch size must be between one and 64",
            ));
        }
        Ok(())
    }

    /// Construct a small settings record explicitly; no heap allocation occurs.
    pub fn settings(&self) -> Self {
        Self {
            threads: self.threads,
            cv_threads: self.cv_threads,
            intra_op_spinning: self.intra_op_spinning,
            inter_op_spinning: self.inter_op_spinning,
            math: self.math,
            batch_size: self.batch_size,
        }
    }
}

#[derive(Debug)]
pub struct OcrStats {
    pub output_bytes: usize,
    pub frames: usize,
    pub lines: usize,
    /// Bin i counts text-recognition ONNX calls with i + 1 rows, across scripts.
    /// Classifier and math-decoder calls are deliberately separate measurements.
    pub batch_counts: [u32; MAX_BATCH],
    pub valid_columns: u64,
    pub padded_columns: u64,
    pub settings: EngineConfig,
    pub opencv_threads: i32,
    /// Decode, orientation, detection, recognition, math, order, assembly.
    pub stage_times: [Duration; 7],
}

impl OcrStats {
    pub fn new(settings: &EngineConfig, opencv_threads: i32) -> Self {
        Self {
            output_bytes: 0,
            frames: 0,
            lines: 0,
            batch_counts: [0; MAX_BATCH],
            valid_columns: 0,
            padded_columns: 0,
            settings: settings.settings(),
            opencv_threads,
            stage_times: [Duration::ZERO; 7],
        }
    }
}
