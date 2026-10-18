//! Synthetic protocol fixtures exercise the evaluator, never OCR quality.

use super::*;
use serde_json::{Value, json};

static REFERENCES: [&str; 4] = [
    "Ärger.\nÜberraschung",
    "deuxième\ncas",
    "繁體中文",
    "另一裁切",
];
static IDS: [&str; 4] = ["a", "b", "c", "d"];
static LANGUAGES: [&str; 4] = ["deu", "fra", "zho", "zho"];

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn new_output(path: &Path, bytes: &[u8]) {
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    output.write_all(bytes).unwrap();
}

fn record(output: &mut File, value: &Value) {
    serde_json::to_writer(&mut *output, value).unwrap();
    output.write_all(b"\n").unwrap();
}

fn fixture(root: &Path) -> PathBuf {
    let input_hash = hash(b"SYNTHETIC SCORER FIXTURE; NOT AN OCR INPUT");
    new_output(
        &root.join("input.dat"),
        b"SYNTHETIC SCORER FIXTURE; NOT AN OCR INPUT",
    );
    for (id, text) in IDS.iter().zip(REFERENCES) {
        new_output(&root.join(format!("{id}.txt")), text.as_bytes());
    }
    let cases = std::array::from_fn::<_, 4, _>(|index| {
        json!({
            "id": IDS[index], "source_id": "synthetic-evaluator-test",
            "original_page_id": if index >= 2 { "unresolved-page-group" } else { IDS[index] },
            "document_cluster": if index >= 2 { "unresolved-source-cluster" } else { IDS[index] }, "writer_cluster": null,
            "cohort": if index >= 2 { "challenge" } else { "primary" },
            "task": "full_page", "language": LANGUAGES[index],
            "script": if index >= 2 { "Hant" } else { "Latn" },
            "tesseract_language": LANGUAGES[index],
            "word_tokenizer": if index >= 2 { "none" } else { "whitespace" },
            "english": {"status": "absent", "reference_letters": 0},
            "domain": "test-fixture", "appearance": "synthetic", "reference_quality": "fixture",
            "input": {"path": "input.dat", "bytes": b"SYNTHETIC SCORER FIXTURE; NOT AN OCR INPUT".len(), "sha256": input_hash},
            "reference": {"path": format!("{}.txt", IDS[index]), "bytes": REFERENCES[index].len(), "sha256": hash(REFERENCES[index].as_bytes())},
            "annotation": null, "width": 1, "height": 1, "input_mime": "image/png",
            "audit": {"status": "reviewed", "input_reference_alignment": true,
                "reference_covers_task": true, "reading_order_reviewed": true,
                "english_reviewed": true, "transcription_policy_reviewed": true}
        })
    });
    let manifest = root.join("corpus.json");
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&manifest)
        .unwrap();
    record(
        &mut output,
        &json!({"schema_version": 1, "corpus_id": "synthetic-scorer-test",
        "status": "frozen", "selection_seed": 20261017, "expected_primary_pages": 2,
            "sources": {"synthetic-evaluator-test": {
                "title":"Synthetic evaluator fixture", "url":"https://example.invalid/fixture",
                "revision":"test-v1", "license":"CC0-1.0", "license_evidence_url":"https://example.invalid/license",
                "license_scope":"Original test strings; not a real OCR dataset", "attribution":"Evaluator tests",
                "admission":"accepted"}},
        "cases": cases}),
    );
    manifest
}

fn budget() -> Value {
    json!({"cpus": 1, "quota_us": 100_000, "period_us": 100_000,
        "memory_limit_bytes": 3 * 1024 * 1024 * 1024_u64})
}

fn attempt(
    output: &mut File,
    run: &Path,
    variant: &str,
    repetition: u16,
    index: usize,
    fail: bool,
    change: bool,
) {
    let name = format!("{}-r{repetition:02}.txt", IDS[index]);
    let changed = format!("{}\nCHANGED OUTPUT", REFERENCES[index]);
    let bytes = if fail {
        &[0xff, 0xfe][..]
    } else if change {
        changed.as_bytes()
    } else {
        REFERENCES[index].as_bytes()
    };
    new_output(&run.join(&name), bytes);
    record(
        output,
        &json!({
            "kind": if repetition == 0 { "warmup" } else { "attempt" },
            "variant": variant, "cpus": 1, "repetition": repetition, "sequence": index,
            "case_id": IDS[index], "document_cluster": if index >= 2 { "unresolved-source-cluster" } else { IDS[index] }, "language": LANGUAGES[index],
            "script": if index >= 2 { "Hant" } else { "Latn" }, "domain": "test-fixture",
            "appearance": "synthetic", "reference_quality": "fixture",
            "cohort": if index >= 2 { "challenge" } else { "primary" }, "task": "full_page",
            "elapsed_us": 7, "status": if fail { 503 } else { 200 },
            "error": if fail { Some("response_utf8") } else { None },
            "output_path": name, "output_sha256": hash(bytes), "output_bytes": bytes.len()
        }),
    );
}

fn journal(root: &Path, variant: &str, failures: bool, truncate: bool) -> PathBuf {
    let manifest = fs::read_to_string(root.join("corpus.json")).unwrap();
    let run = root.join(variant);
    fs::create_dir(&run).unwrap();
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(run.join("requests.jsonl"))
        .unwrap();
    record(
        &mut output,
        &json!({"kind": "environment", "schema_version": 1,
        "corpus_id": "synthetic-scorer-test", "selection_seed": 20261017,
        "manifest_sha256":hash(manifest.as_bytes()), "manifest_bytes":manifest.len(),
        "variant": variant, "repetitions": 3, "budget": budget()}),
    );
    attempt(&mut output, &run, variant, 0, 0, false, false);
    record(
        &mut output,
        &json!({"kind": "repetition_end", "repetition": 0, "attempted_pages": 1, "wall_us": 10}),
    );
    for repetition in 1..=3 {
        for index in 0..4 {
            if truncate && repetition == 1 && index == 1 {
                output
                    .write_all(b"{\"kind\":\"attempt\",\"repetition\":1,")
                    .unwrap();
                // An interrupted journal may stop in the middle of UTF-8.
                output.write_all(&[0xf0, 0x9f]).unwrap();
                return run;
            }
            attempt(
                &mut output,
                &run,
                variant,
                repetition,
                index,
                failures && repetition == 1 && index != 1,
                failures && repetition == 3 && index == 1,
            );
        }
        record(
            &mut output,
            &json!({"kind": "repetition_end", "repetition": repetition, "attempted_pages": 4, "wall_us": 40}),
        );
    }
    record(
        &mut output,
        &json!({"kind": "resource_end", "budget": budget()}),
    );
    run
}

fn assert_csv_shape(path: &Path, columns: usize, rows: usize) {
    let text = fs::read_to_string(path).unwrap();
    assert_eq!(text.lines().count(), rows + 1);
    // Fixture field labels contain no commas or embedded newlines. CSV quoting
    // itself is checked independently below.
    for line in text.lines() {
        assert_eq!(line.split(',').count(), columns, "{line}");
    }
}

#[test]
fn complete_and_interrupted_journals_preserve_failures_and_pair_only_first_attempts() {
    // The evaluator uses fixed-capacity buffers. Match the ordinary Linux CLI
    // stack instead of Rust's smaller default test-worker stack.
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(complete_and_interrupted_fixture)
        .unwrap()
        .join()
        .unwrap();
}

fn complete_and_interrupted_fixture() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("score-journal-test-{}-{nonce}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let manifest = fixture(&root);
    for invalid in 0..3 {
        let mut corpus: Corpus = corpus::read_json(&manifest).unwrap();
        match invalid {
            0 => corpus.cases[0].source_id = String::from("unknown-source"),
            1 => corpus.cases[0].word_tokenizer = String::from("undeclared"),
            _ => {
                corpus.cases[1].original_page_id = String::from("a");
                corpus.cases[1].document_cluster = String::from("a");
            }
        }
        assert!(corpus.validate(&root).is_err());
    }
    let perfect = journal(&root, "synthetic-perfect", false, false);
    let failed = journal(&root, "synthetic-failures", true, false);
    let interrupted = journal(&root, "synthetic-interrupted", false, true);
    for run in [&perfect, &failed, &interrupted] {
        score(&ScoreOptions {
            manifest: &manifest,
            root: &root,
            run,
            output: &run.join("scores"),
        })
        .unwrap();
    }
    let left = perfect.join("scores/scores.json");
    let right = failed.join("scores/scores.json");
    let report: ScoredRun = corpus::read_json(&right).unwrap();
    assert!(report.journal_complete);
    assert_eq!(report.cases.len(), 4);
    assert_eq!(report.cases[0].outcome, Outcome::ServiceFailure);
    assert_eq!(report.cases[0].successful_attempts, 2);
    assert!(report.cases[0].metrics.is_none());
    assert_eq!(report.cases[1].metrics.as_ref().unwrap().cer.errors, 0);
    let document: Value = corpus::read_json(&right).unwrap();
    assert_eq!(document["nondeterministic_successful_output_cases"], 1);
    assert_eq!(document["mixed_success_failure_cases"], 3);
    let summaries = document["summaries"].as_array().unwrap();
    let primary = summaries
        .iter()
        .find(|row| row["cohort"] == "primary" && row["axis"] == "overall")
        .unwrap();
    assert_eq!(primary["selected_cases"], 2);
    assert_eq!(primary["scored_cases"], 1);
    assert_eq!(primary["service_failures"], 1);
    assert_eq!(primary["cer"]["micro_rate"], 0.0);
    let challenge = summaries
        .iter()
        .find(|row| row["cohort"] == "challenge" && row["axis"] == "overall")
        .unwrap();
    assert_eq!(challenge["scored_cases"], 0);
    assert!(challenge["cer"]["micro_rate"].is_null());
    assert_eq!(challenge["wer"]["cases"], 0);
    assert_csv_shape(&failed.join("scores/cases.csv"), 38, 4);
    assert_csv_shape(&failed.join("scores/attempts.csv"), 18, 12);
    let timing: Value = corpus::read_json(&failed.join("scores/performance.json")).unwrap();
    let source_journal = fs::read_to_string(failed.join("requests.jsonl")).unwrap();
    assert_eq!(timing["journal_sha256"], hash(source_journal.as_bytes()));
    assert_eq!(timing["journal_bytes"], source_journal.len());
    assert_eq!(timing["manifest_sha256"], document["manifest_sha256"]);
    assert_eq!(timing["manifest_bytes"], document["manifest_bytes"]);
    assert_eq!(timing["warmup_attempts_excluded"], 1);
    assert_eq!(timing["expected_attempts"], 12);
    assert_eq!(timing["observed_attempts"], 12);
    assert_eq!(timing["successful_attempts"], 9);
    assert_eq!(timing["failed_attempts"], 3);
    assert_eq!(timing["missing_attempts"], 0);
    assert_eq!(timing["completed_repetition_wall_us"], 120);
    assert_eq!(
        timing["successful_case_responses_per_complete_workload_wall_second"],
        75_000.0
    );
    let timing_cohorts = timing["cohorts"].as_array().unwrap();
    assert_eq!(timing_cohorts.len(), 2);
    let primary_timing = timing_cohorts
        .iter()
        .find(|row| row["cohort"] == "primary")
        .unwrap();
    assert_eq!(
        primary_timing["successful_complete_body_latency"]["samples"],
        5
    );
    assert_eq!(
        primary_timing["successful_complete_body_latency"]["p95_us"],
        7.0
    );
    assert_eq!(
        primary_timing["failed_attempt_termination_time"]["samples"],
        1
    );
    let challenge_timing = timing_cohorts
        .iter()
        .find(|row| row["cohort"] == "challenge")
        .unwrap();
    assert_eq!(
        challenge_timing["successful_complete_body_latency"]["samples"],
        4
    );
    assert_eq!(
        challenge_timing["failed_attempt_termination_time"]["samples"],
        2
    );
    assert_eq!(timing["completed_repetitions"][0]["failed_attempts"], 3);
    assert_eq!(
        timing["completed_repetitions"][0]["sum_attempt_elapsed_us"],
        28
    );
    let perfect_timing: Value =
        corpus::read_json(&perfect.join("scores/performance.json")).unwrap();
    assert_eq!(perfect_timing["successful_attempts"], 12);
    assert_eq!(perfect_timing["failed_attempts"], 0);
    assert!(perfect_timing["cohorts"][0]["failed_attempt_termination_time"]["p95_us"].is_null());
    // Trailing JSON whitespace is semantically irrelevant but is part of the
    // exact input identity consumed by comparison.
    OpenOptions::new()
        .append(true)
        .open(&left)
        .unwrap()
        .write_all(b"\n\t ")
        .unwrap();
    let left_bytes = fs::read_to_string(&left).unwrap();
    let right_bytes = fs::read_to_string(&right).unwrap();
    let comparison = root.join("comparison");
    compare(&CompareOptions {
        left: &left,
        right: &right,
        output: &comparison,
        seed: 20261017,
        resamples: 1000,
    })
    .unwrap();
    let paired: Value = corpus::read_json(&comparison.join("comparison.json")).unwrap();
    assert_eq!(paired["left_scores_sha256"], hash(left_bytes.as_bytes()));
    assert_eq!(paired["right_scores_sha256"], hash(right_bytes.as_bytes()));
    assert_eq!(paired["left_scores_bytes"], left_bytes.len());
    assert_eq!(paired["right_scores_bytes"], right_bytes.len());
    let rows = paired["strata"].as_array().unwrap();
    let primary = rows
        .iter()
        .find(|row| row["cohort"] == "primary" && row["axis"] == "overall")
        .unwrap();
    assert_eq!(primary["selected_cases"], 2);
    assert_eq!(primary["joint_success_cases"], 1);
    assert_eq!(primary["right_service_failures"], 1);
    assert_eq!(primary["cer"]["right_minus_left"], 0.0);
    assert!(primary["cer"]["ci_95_low"].is_null());
    let challenge = rows
        .iter()
        .find(|row| row["cohort"] == "challenge" && row["axis"] == "overall")
        .unwrap();
    assert_eq!(challenge["selected_cases"], 2);
    assert_eq!(challenge["joint_success_cases"], 0);
    assert!(challenge["cer"]["right_minus_left"].is_null());
    assert_eq!(challenge["wer"]["paired_cases"], 0);
    assert_csv_shape(&comparison.join("pairs.csv"), 8, 4);
    assert_csv_shape(&comparison.join("paired.csv"), 27, rows.len() * 3);
    let descriptive = root.join("descriptive");
    compare(&CompareOptions {
        left: &left,
        right: &left,
        output: &descriptive,
        seed: 20261018,
        resamples: 0,
    }).unwrap();
    let descriptive: Value = corpus::read_json(&descriptive.join("comparison.json")).unwrap();
    assert_eq!(descriptive["requested_bootstrap_samples"], 0);
    for stratum in descriptive["strata"].as_array().unwrap() {
        for metric in ["cer", "collapsed_cer", "wer"] {
            assert!(stratum[metric]["ci_95_low"].is_null());
            assert!(stratum[metric]["ci_95_high"].is_null());
            assert_eq!(stratum[metric]["bootstrap_samples"], 0);
        }
    }
    let partial = interrupted.join("scores/scores.json");
    let report: ScoredRun = corpus::read_json(&partial).unwrap();
    assert!(!report.journal_complete);
    assert!(report.truncated_final_record_bytes > 0);
    assert_eq!(report.cases[0].outcome, Outcome::Success);
    assert_eq!(report.cases[1].outcome, Outcome::MissingAttempt);
    assert_eq!(report.cases[1].missing_attempts, 3);
    assert_csv_shape(&interrupted.join("scores/attempts.csv"), 18, 1);
    let partial_timing: Value =
        corpus::read_json(&interrupted.join("scores/performance.json")).unwrap();
    assert_eq!(partial_timing["journal_complete"], false);
    assert_eq!(partial_timing["observed_attempts"], 1);
    assert_eq!(partial_timing["missing_attempts"], 11);
    assert_eq!(partial_timing["completed_repetition_wall_us"], 0);
    assert!(
        partial_timing["completed_repetitions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        partial_timing["successful_case_responses_per_complete_workload_wall_second"].is_null()
    );
    assert!(
        compare(&CompareOptions {
            left: &left,
            right: &partial,
            output: &root.join("must-not-create"),
            seed: 20261017,
            resamples: 1000
        })
        .is_err()
    );
    assert!(!root.join("must-not-create").exists());
    fs::remove_dir_all(root).expect("remove only isolated synthetic test outputs");
}

#[test]
fn csv_quotes_commas_newlines_and_embedded_quotes() {
    let mut buffer = [0_u8; 128];
    let mut writer = io::Cursor::new(&mut buffer[..]);
    csv_field(&mut writer, "a,\"b\"\nc").unwrap();
    let len = writer.position() as usize;
    assert_eq!(&buffer[..len], b"\"a,\"\"b\"\"\nc\"");
}
