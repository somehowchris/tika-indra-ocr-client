//! Isolated decoder acceptance: deliberate repetition is not a quality corpus.
use std::time::{Duration, Instant};

use super::*;
use crate::{MAX_BATCH, recognition::Script, trace::Recorder};

static EQUATIONS: &[u8] = include_bytes!("../../tests/data/math.png");
static EQUATION_BOX: [f32; 4] = [386.87564, 188.82909, 776.0931, 237.10597];
static EXPECTED: &str = "$x ^ { 2 } + 2 x + 1 = ( x + 1 ) ^ { 2 }$";

#[test]
fn concurrent_environment_initialization_retains_one_process_handle() {
    let start = std::sync::Barrier::new(2);
    let environments = std::thread::scope(|scope| {
        let handles = std::array::from_fn::<_, 2, _>(|_| {
            scope.spawn(|| {
                start.wait();
                environment().expect("one fallible process-wide environment")
            })
        });
        handles.map(|handle| handle.join().expect("native initialization worker"))
    });
    assert!(std::ptr::eq(environments[0], environments[1]));
    assert!(std::ptr::eq(
        environments[0],
        environment().expect("subsequent engines reuse the environment")
    ));
}

#[test]
#[ignore = "requires actual native libraries and INDRA_TEST_MODEL_DIR with math artifacts"]
fn real_math_decoder_batches_one_eight_sixty_four_preserve_every_line() {
    let directory = std::env::var_os("INDRA_TEST_MODEL_DIR").expect("explicit pinned model cache");
    let threads = std::env::var("INDRA_TEST_THREADS")
        .map_or(1, |value| value.parse().expect("integer test thread count"));
    let settings = EngineConfig {
        threads,
        cv_threads: threads,
        math: true,
        ..EngineConfig::default()
    };
    let mut engine = Engine::load(Path::new(&directory), &settings).expect("all verified models");
    let header = images::inspect(EQUATIONS).expect("synthetic equation PNG header");
    let gray = images::decode(EQUATIONS, &header, 0).expect("synthetic equation pixels");
    let boxes = [EQUATION_BOX; MAX_BATCH];
    let flags = [true; MAX_BATCH];
    let dark = [false; MAX_BATCH];
    for cap in [1, 8, 64] {
        engine.page.lines.reset();
        let mut trace = Recorder::MathBatches {
            histogram: [0; MAX_BATCH],
        };
        engine
            .shared
            .math
            .as_mut()
            .expect("explicitly enabled math reader")
            .read(
                &gray,
                &boxes,
                &flags,
                &dark,
                cap,
                &mut engine.shared.scratch,
                &mut engine.page.lines,
                Instant::now() + Duration::from_secs(300),
                &mut trace,
            )
            .expect("actual encoder, cross cache and autoregressive decoder");
        let Recorder::MathBatches { histogram } = trace else {
            panic!("native shape observation must remain enabled");
        };
        for (index, observed) in histogram.iter().enumerate() {
            let expected = if index + 1 == cap { MAX_BATCH / cap } else { 0 };
            assert_eq!(
                *observed as usize, expected,
                "actual encoder batch histogram"
            );
        }
        for line in &engine.page.lines.metadata[..MAX_BATCH] {
            assert_eq!(line.script, Script::Math, "every equation must be accepted");
            assert_eq!(line.count, 1);
            assert_eq!(
                &engine.page.lines.math_text[line.math_start..line.math_end],
                EXPECTED,
                "same declared equation at every position and batch size"
            );
        }
        assert_eq!(
            engine.page.lines.math_text.len(),
            MAX_BATCH * EXPECTED.len()
        );
        assert!(
            engine.page.lines.metadata[MAX_BATCH..]
                .iter()
                .all(|line| line.count == 0),
            "unused line metadata must remain untouched"
        );
        println!(
            "synthetic decoder gate: native batch={cap}, calls={}, rows={MAX_BATCH}, threads={threads}",
            MAX_BATCH / cap
        );
    }
}
