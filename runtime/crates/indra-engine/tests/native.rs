//! Acceptance gates against actual linked libraries and explicitly acquired model files.
use std::{
    path::Path,
    time::{Duration, Instant},
};

use indra_engine::{Engine, EngineConfig, EngineError};

static PNG: &[u8] = include_bytes!("../../../../backend/tests/data/smoke.png");
static TIFF: &[u8] = include_bytes!("../../../../backend/tests/data/two-pages.tiff");

#[test]
#[ignore = "requires actual native libraries and INDRA_TEST_MODEL_DIR"]
fn pinned_models_smoke_and_all_frames() {
    let directory = std::env::var_os("INDRA_TEST_MODEL_DIR").expect("explicit pinned cache");
    let mut engine =
        Engine::load(Path::new(&directory), &EngineConfig::default()).expect("verified models");
    engine
        .warmup(Instant::now() + Duration::from_secs(300))
        .expect("all enabled models warm");
    let mut text = String::new();
    let stats = engine
        .recognize(PNG, Instant::now() + Duration::from_secs(300), &mut text)
        .expect("real PNG OCR");
    assert_eq!(stats.frames, 1);
    assert_eq!(
        text,
        include_str!("../../../../backend/tests/data/smoke.txt").trim_end_matches('\n')
    );
    let stats = engine
        .recognize(TIFF, Instant::now() + Duration::from_secs(300), &mut text)
        .expect("all TIFF pages");
    assert_eq!(stats.frames, 2);
    assert!(text.contains("\n\u{c}\n"), "page boundary must be explicit");
    assert!(stats.padded_columns >= stats.valid_columns);
}

#[test]
#[ignore = "requires actual native libraries and INDRA_TEST_MODEL_DIR"]
fn failures_clear_prior_output_and_do_not_poison_engine() {
    let directory = std::env::var_os("INDRA_TEST_MODEL_DIR").expect("explicit pinned cache");
    let mut engine =
        Engine::load(Path::new(&directory), &EngineConfig::default()).expect("verified models");
    let mut text = String::from("prior confidential output");
    assert!(matches!(
        engine.recognize(PNG, Instant::now(), &mut text),
        Err(EngineError::Deadline)
    ));
    assert!(text.is_empty());
    assert!(
        engine
            .recognize(
                &TIFF[..TIFF.len() / 2],
                Instant::now() + Duration::from_secs(300),
                &mut text
            )
            .is_err()
    );
    assert!(text.is_empty());
    assert!(
        engine
            .recognize(PNG, Instant::now() + Duration::from_secs(300), &mut text)
            .is_ok()
    );
}

#[test]
#[ignore = "requires actual native libraries and INDRA_TEST_MODEL_DIR with math artifacts"]
fn optional_math_models_warm_and_complete() {
    let directory = std::env::var_os("INDRA_TEST_MODEL_DIR").expect("explicit pinned cache");
    let settings = EngineConfig {
        math: true,
        ..EngineConfig::default()
    };
    let mut engine =
        Engine::load(Path::new(&directory), &settings).expect("all math models verified");
    engine
        .warmup(Instant::now() + Duration::from_secs(300))
        .expect("classifier, encoder, cross cache and autoregressive decoder");
    let mut text = String::new();
    let stats = engine
        .recognize(PNG, Instant::now() + Duration::from_secs(300), &mut text)
        .expect("math-enabled OCR");
    assert!(stats.settings.math);
    assert_eq!(stats.frames, 1);
}
