//! Execute the actual fixed-array selection source without invoking native OCR.
use indra_engine::{EngineError, MAX_BATCH, MAX_LINES, Result};

#[path = "../src/batching.rs"]
#[allow(dead_code)]
mod batching;
#[path = "../src/continuous/plan.rs"]
mod plan;
