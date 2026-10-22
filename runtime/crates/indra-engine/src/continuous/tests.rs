//! Actual native-model gates. These implementation fixtures are not a quality
//! corpus. Padding comparisons use prespecified atol=rtol=1e-5; discrete values
//! and valid normalized input elements must agree exactly.

use std::{path::Path, time::Duration};

use super::*;

static TIFF: &[u8] = include_bytes!("../../../../../backend/tests/data/two-pages.tiff");
static SMOKE: &[u8] = include_bytes!("../../../../../backend/tests/data/smoke.png");
static MATH: &[u8] = include_bytes!("../../tests/data/math.png");
static TIFF_TEXT: &str = "INVOICE 12345\n\u{c}\nSECOND PAGE 67890";

fn engine() -> Engine {
    let directory = std::env::var_os("INDRA_TEST_MODEL_DIR").expect("explicit verified cache");
    let threads =
        std::env::var("INDRA_TEST_THREADS").map_or(1, |v| v.parse().expect("thread count"));
    Engine::load(
        Path::new(&directory),
        &crate::EngineConfig {
            threads,
            cv_threads: threads,
            ..crate::EngineConfig::default()
        },
    )
    .expect("real pinned sessions")
}

fn close(actual: f64, expected: f64, context: &'static str) {
    assert!(
        actual.is_finite() && expected.is_finite(),
        "{context}: finite values"
    );
    assert!(
        (actual - expected).abs() <= 1.0e-5 + 1.0e-5 * expected.abs(),
        "{context}: {actual} differs from {expected} beyond prespecified tolerance"
    );
}

struct ReferenceRow {
    input: Mat,
    width: usize,
    padded: usize,
    script: f32,
    length: usize,
    ids: [i64; recognition::MAX_TOKENS],
    probabilities: [f32; recognition::MAX_TOKENS],
    decoded_ids: [i32; recognition::MAX_TOKENS],
    positions: [f64; recognition::MAX_TOKENS],
    count: usize,
    confidence: f32,
}

impl ReferenceRow {
    fn new() -> Self {
        Self {
            input: geometry::floats(geometry::LINE_HEIGHT * geometry::MAX_WIDTH)
                .expect("bounded diagnostic row"),
            width: 0,
            padded: 0,
            script: 0.0,
            length: 0,
            ids: [0; recognition::MAX_TOKENS],
            probabilities: [0.0; recognition::MAX_TOKENS],
            decoded_ids: [0; recognition::MAX_TOKENS],
            positions: [0.0; recognition::MAX_TOKENS],
            count: 0,
            confidence: 0.0,
        }
    }
}

#[test]
#[ignore = "requires actual native libraries and INDRA_TEST_MODEL_DIR"]
fn actual_two_owner_script_text_chinese_calls_preserve_canonical_rows() {
    let mut pilot = engine().into_continuous().expect("two owner startup");
    let due = Instant::now() + Duration::from_secs(180);
    for (frame, owner) in pilot.owners.iter_mut().enumerate() {
        let header = images::inspect(TIFF).expect("two independent TIFF pages");
        owner.gray = images::decode(TIFF, &header, frame).expect("distinct page pixels");
        // Different TIFF pixels with equal canonical tensor shape. The original
        // 90%-width regrouping failed real script probability parity and is
        // retained in the operational amendment; it is no longer admitted.
        let right = owner.gray.cols() as f32;
        owner.page.boxes[0] = [0.0, 0.0, right, owner.gray.rows() as f32];
        owner.count = 1;
        owner.indices[0] = 0;
        owner.widths[0] = geometry::crop_geometry(&owner.page.boxes[0], geometry::LINE_HEIGHT)
            .expect("full-page diagnostic line")
            .out_width;
        owner.sequence = frame as u64 + 1;
        owner.frame = frame;
        owner.due = Some(due);
        owner.stats = Some(OcrStats::new(
            &pilot.shared.settings,
            core::get_num_threads().expect("CV threads"),
        ));
    }
    assert_ne!(
        pilot.owners[0].gray.data_typed::<u8>().expect("page one"),
        pilot.owners[1].gray.data_typed::<u8>().expect("page two"),
        "rows must contain different document pixels"
    );
    let mut references = std::array::from_fn::<_, 2, _>(|_| ReferenceRow::new());
    let mut comparisons = 0_usize;
    for stage in ["script", "rec", "rec_cjk"] {
        for owner in &mut pilot.owners {
            owner.phase = match stage {
                "script" => Phase::Script,
                "rec" => Phase::Text,
                _ => Phase::Chinese,
            };
            owner.flags[0] = stage == "rec_cjk";
            owner.position = 0;
            owner.page.lines.reset();
        }
        for (owner_index, reference) in references.iter_mut().enumerate() {
            let mut batch = plan::Batch::new();
            let views = std::array::from_fn(|index| {
                (index == owner_index).then(|| pilot.owners[index].view())
            });
            plan::select(views, owner_index, 64, &mut batch)
                .expect("single-owner comparison batch");
            invoke(
                &mut pilot.shared,
                &mut pilot.owners,
                &batch,
                owner_index,
                |batch, input, padded, outputs| {
                    assert_eq!(batch.count, 1);
                    reference.width = batch.rows[0].width;
                    reference.padded = padded;
                    let target = reference.input.data_typed_mut::<f32>()?;
                    for y in 0..geometry::LINE_HEIGHT {
                        // Explicit test-only copies preserve the unpadded reference.
                        target[y * reference.width..(y + 1) * reference.width]
                            .copy_from_slice(&input[y * padded..y * padded + reference.width]);
                    }
                    if stage == "script" {
                        reference.script = outputs[0].try_extract_tensor::<f32>()?.1[0];
                    } else {
                        let (shape, ids) = outputs[0].try_extract_tensor::<i64>()?;
                        let (_, probabilities) = outputs[1].try_extract_tensor::<f32>()?;
                        reference.length =
                            usize::try_from(outputs[2].try_extract_tensor::<i64>()?.1[0])
                                .map_err(|_| EngineError::Shape("reference CTC length"))?;
                        assert!(
                            reference.length <= shape[1] as usize
                                && reference.length <= recognition::MAX_TOKENS
                        );
                        reference.ids[..reference.length].copy_from_slice(&ids[..reference.length]);
                        reference.probabilities[..reference.length]
                            .copy_from_slice(&probabilities[..reference.length]);
                    }
                    Ok(())
                },
            )
            .expect("actual independent native model invocation");
            let lines = &pilot.owners[owner_index].page.lines;
            reference.count = lines.metadata[0].count;
            reference.confidence = lines.metadata[0].confidence;
            reference.decoded_ids[..reference.count].copy_from_slice(
                &lines.ids.data_typed::<i32>().expect("tokens")[..reference.count],
            );
            reference.positions[..reference.count].copy_from_slice(
                &lines.positions.data_typed::<f64>().expect("positions")[..reference.count],
            );
            pilot.owners[owner_index].position = 0;
        }
        let mut batch = plan::Batch::new();
        plan::select(
            std::array::from_fn(|index| Some(pilot.owners[index].view())),
            0,
            64,
            &mut batch,
        )
        .expect("compatible distinct-owner rows");
        assert_eq!(
            batch.owners,
            [1, 1],
            "a real invocation must include both owners"
        );
        let invocation = invoke(
            &mut pilot.shared,
            &mut pilot.owners,
            &batch,
            0,
            |batch, input, padded, outputs| {
                for (slot, row) in batch.rows[..batch.count].iter().enumerate() {
                    let reference = &references[row.owner];
                    let expected = reference.input.data_typed::<f32>()?;
                    for y in 0..geometry::LINE_HEIGHT {
                        for x in 0..row.width {
                            assert_eq!(
                                input[slot * geometry::LINE_HEIGHT * padded + y * padded + x]
                                    .to_bits(),
                                expected[y * reference.width + x].to_bits(),
                                "exact valid normalized input at owner/line/stage"
                            );
                            comparisons += 1;
                        }
                        assert!(
                            input[slot * geometry::LINE_HEIGHT * padded + y * padded + row.width
                                ..slot * geometry::LINE_HEIGHT * padded + (y + 1) * padded]
                                .iter()
                                .all(|v| *v == 0.0),
                            "padding is exactly zero and excluded from valid-value comparison"
                        );
                    }
                    if stage == "script" {
                        close(
                            f64::from(outputs[0].try_extract_tensor::<f32>()?.1[slot]),
                            f64::from(reference.script),
                            "script probability",
                        );
                        comparisons += 1;
                    } else {
                        let (shape, ids) = outputs[0].try_extract_tensor::<i64>()?;
                        let (_, probabilities) = outputs[1].try_extract_tensor::<f32>()?;
                        let length = outputs[2].try_extract_tensor::<i64>()?.1[slot];
                        assert_eq!(
                            length, reference.length as i64,
                            "canonical valid CTC length"
                        );
                        let start = slot * shape[1] as usize;
                        assert_eq!(
                            &ids[start..start + reference.length],
                            &reference.ids[..reference.length],
                            "canonical discrete CTC ids"
                        );
                        for (actual, expected) in probabilities[start..start + reference.length]
                            .iter()
                            .zip(&reference.probabilities[..reference.length])
                        {
                            close(
                                f64::from(*actual),
                                f64::from(*expected),
                                "valid CTC confidence",
                            );
                            comparisons += 1;
                        }
                    }
                }
                Ok(())
            },
        )
        .expect("actual shared-owner model invocation and canonical comparisons");
        assert!(invocation.shared());
        assert_eq!(invocation.model, stage);
        assert!(
            references
                .iter()
                .all(|row| row.padded == invocation.padded_width),
            "every contributing owner retains its original padded width"
        );
        for (owner, reference) in pilot.owners.iter().zip(&references) {
            if stage == "script" {
                assert_eq!(
                    owner.flags[0],
                    reference.script > 0.5,
                    "canonical script routing threshold decision"
                );
            }
            assert_eq!(owner.page.lines.metadata[0].count, reference.count);
            close(
                f64::from(owner.page.lines.metadata[0].confidence),
                f64::from(reference.confidence),
                "decoded confidence",
            );
            assert_eq!(
                &owner.page.lines.ids.data_typed::<i32>().expect("tokens")[..reference.count],
                &reference.decoded_ids[..reference.count]
            );
            for (actual, expected) in owner
                .page
                .lines
                .positions
                .data_typed::<f64>()
                .expect("positions")[..reference.count]
                .iter()
                .zip(&reference.positions[..reference.count])
            {
                close(*actual, *expected, "decoded position");
            }
        }
        println!(
            "canonical native stage={stage} rows=2 distinct_owners=2 padded_width={} tolerance_atol=0.00001 tolerance_rtol=0.00001",
            invocation.padded_width
        );
    }
    assert!(comparisons > 0);
    println!(
        "canonical native scalar comparisons={comparisons}; input padding excluded; all valid input bits and CTC ids/lengths exact"
    );
}

#[test]
#[ignore = "requires actual native libraries and INDRA_TEST_MODEL_DIR"]
fn actual_requests_refill_a_finished_slot_before_its_peer_and_route_all_frames() {
    let mut engine = engine();
    let due = Instant::now() + Duration::from_secs(180);
    let inputs = [TIFF, SMOKE, MATH];
    let mut expected =
        std::array::from_fn::<_, 3, _>(|_| String::with_capacity(crate::MAX_OUTPUT_BYTES));
    for (input, output) in inputs.iter().zip(&mut expected) {
        engine
            .recognize(input, due, output)
            .expect("independent complete reference request");
    }
    assert_eq!(expected[0], TIFF_TEXT);
    assert_eq!(expected[1], "INVOICE 12345");
    let mut pilot = engine.into_continuous().expect("bounded pilot");
    let mut outputs =
        std::array::from_fn::<_, 2, _>(|_| String::with_capacity(crate::MAX_OUTPUT_BYTES));
    let mut assignments = [Some(0), Some(1)];
    let mut completed = [false; 3];
    let mut shared_calls = [0_usize; 3];
    let mut refilled_with_live_peer = false;
    let mut next = 2;
    for step in 0..10_000 {
        let [first, second] = &mut outputs;
        let requests = [
            assignments[0].map(|job| ContinuousRequest {
                sequence: job as u64 + 1,
                input: inputs[job],
                output: first,
                deadline: due,
                cancelled: false,
            }),
            assignments[1].map(|job| ContinuousRequest {
                sequence: job as u64 + 1,
                input: inputs[job],
                output: second,
                deadline: due,
                cancelled: false,
            }),
        ];
        let progress = pilot
            .advance(requests, |_| Ok(()))
            .expect("continuous native operation");
        if let Some(invocation) = progress.invocation
            && invocation.shared()
        {
            shared_calls[match invocation.model {
                "script" => 0,
                "rec" => 1,
                _ => 2,
            }] += 1;
        }
        for (slot, completion) in progress.completed.into_iter().enumerate() {
            if let Some(result) = completion {
                let job = assignments[slot].take().expect("completed admitted owner");
                let stats = result.expect("complete native request");
                assert_eq!(
                    outputs[slot], expected[job],
                    "exact owner-routed final UTF-8"
                );
                assert_eq!(stats.output_bytes, outputs[slot].len());
                assert_eq!(stats.frames, if job == 0 { 2 } else { 1 });
                assert!(!completed[job], "each admitted owner completes once");
                completed[job] = true;
                if next < inputs.len() {
                    refilled_with_live_peer |= assignments[1 - slot].is_some();
                    assignments[slot] = Some(next);
                    next += 1;
                }
            }
        }
        if completed.iter().all(|done| *done) {
            assert!(
                refilled_with_live_peer,
                "completion permits real continuous refill"
            );
            assert!(
                shared_calls[0] > 0 && shared_calls[1] > 0,
                "actual script and CTC calls must contain both admitted owners"
            );
            println!(
                "continuous requests=3 completed=3 refill_with_live_peer=true shared_script_calls={} shared_text_calls={} shared_chinese_calls={} operations={}",
                shared_calls[0],
                shared_calls[1],
                shared_calls[2],
                step + 1
            );
            return;
        }
    }
    panic!("bounded test operation budget exhausted without completion");
}

#[test]
#[ignore = "requires actual native libraries and INDRA_TEST_MODEL_DIR"]
fn cancellation_discards_only_its_owner_and_input_failure_does_not_poison_peer() {
    let mut pilot = engine().into_continuous().expect("bounded pilot");
    let due = Instant::now() + Duration::from_secs(120);
    let mut outputs =
        std::array::from_fn::<_, 2, _>(|_| String::with_capacity(crate::MAX_OUTPUT_BYTES));
    let mut completed = [false; 2];
    for step in 0..100 {
        let [first, second] = &mut outputs;
        let progress = pilot
            .advance(
                [
                    (!completed[0]).then_some(ContinuousRequest {
                        sequence: 1,
                        input: TIFF,
                        output: first,
                        deadline: due,
                        cancelled: step > 0,
                    }),
                    (!completed[1]).then_some(ContinuousRequest {
                        sequence: 2,
                        input: SMOKE,
                        output: second,
                        deadline: due,
                        cancelled: false,
                    }),
                ],
                |_| Ok(()),
            )
            .expect("cancelled peer is isolated");
        for (slot, result) in progress.completed.into_iter().enumerate() {
            if let Some(result) = result {
                if slot == 0 {
                    assert!(matches!(result, Err(EngineError::Deadline)));
                    assert!(outputs[0].is_empty());
                } else {
                    result.expect("still-live owner completes");
                    assert_eq!(outputs[1], "INVOICE 12345");
                }
                completed[slot] = true;
            }
        }
        if completed.iter().all(|done| *done) {
            break;
        }
    }
    assert_eq!(completed, [true; 2]);
    let [first, second] = &mut outputs;
    let progress = pilot
        .advance(
            [
                Some(ContinuousRequest {
                    sequence: 3,
                    input: b"invalid",
                    output: first,
                    deadline: due,
                    cancelled: false,
                }),
                Some(ContinuousRequest {
                    sequence: 4,
                    input: SMOKE,
                    output: second,
                    deadline: due,
                    cancelled: false,
                }),
            ],
            |_| Ok(()),
        )
        .expect("invalid new owner does not terminate pilot");
    assert!(matches!(
        progress.completed[0],
        Some(Err(EngineError::UnsupportedImage(_)))
    ));
    assert!(progress.completed[1].is_none());
    assert!(outputs[0].is_empty());
    let mut finished = false;
    for _ in 0..100 {
        let progress = pilot
            .advance(
                [
                    None,
                    Some(ContinuousRequest {
                        sequence: 4,
                        input: SMOKE,
                        output: &mut outputs[1],
                        deadline: due,
                        cancelled: false,
                    }),
                ],
                |_| Ok(()),
            )
            .expect("valid owner continues after invalid peer");
        assert!(progress.completed[0].is_none());
        if let Some(result) = &progress.completed[1] {
            assert!(result.is_ok());
            assert_eq!(outputs[1], "INVOICE 12345");
            finished = true;
            break;
        }
    }
    assert!(finished, "valid owner finishes within the operation bound");
}
