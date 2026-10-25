use std::path::PathBuf;

use indra_engine::EngineConfig;

pub fn directory() -> PathBuf {
    std::env::var_os("INDRA_TEST_MODEL_DIR")
        .expect("explicit pinned model cache")
        .into()
}

pub fn settings(math: bool) -> EngineConfig {
    let threads = std::env::var("INDRA_TEST_THREADS")
        .map_or(1, |value| value.parse().expect("integer test thread count"));
    assert!([1, 2].contains(&threads), "test threads must be one or two");
    EngineConfig {
        threads,
        cv_threads: threads,
        math,
        ..EngineConfig::default()
    }
}
