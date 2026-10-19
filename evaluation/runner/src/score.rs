//! Untimed, auditable scoring of the frozen corpus and recorded Tika responses.
//! Accuracy always selects repetition 1. Repeated requests measure stability and
//! performance, never additional independent accuracy samples.

mod bootstrap;
#[cfg(test)]
mod journal_tests;
mod performance;

use crate::{
    corpus::{self, Case, Corpus, CorpusError},
    metrics::{Distance, MetricError, Workspace},
};
use arrayvec::{ArrayString, ArrayVec};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Seek, Write},
    path::{Path, PathBuf},
};
use thiserror::Error;

static MAX_JOURNAL_BYTES: u64 = 32 * 1024 * 1024;
static MAX_RECORD_BYTES: u64 = 64 * 1024;
static MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024 + 1;
static METRIC_POLICY: &str = "nfc-unicode-scalar-unit-cost-levenshtein-v1";

fn read_record(
    reader: &mut impl BufRead,
    record: &mut ArrayVec<u8, 65_537>,
) -> Result<(), ScoreError> {
    record.clear();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(());
        }
        let bytes = available
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(available.len(), |index| index + 1)
            .min(record.remaining_capacity());
        record
            .try_extend_from_slice(&available[..bytes])
            .map_err(|_| ScoreError::Invalid("journal record exceeds capacity"))?;
        reader.consume(bytes);
        if record.last() == Some(&b'\n') || record.is_full() {
            return Ok(());
        }
    }
}

pub struct ScoreOptions<'a> {
    pub manifest: &'a Path,
    pub root: &'a Path,
    /// Directory containing requests.jsonl and its raw response files.
    pub run: &'a Path,
    /// New score artifacts are written here; existing artifacts are not replaced.
    pub output: &'a Path,
}

pub struct CompareOptions<'a> {
    /// scores.json artifacts produced by this scorer.
    pub left: &'a Path,
    pub right: &'a Path,
    pub output: &'a Path,
    pub seed: u64,
    pub resamples: usize,
}

#[derive(Debug, Error)]
pub enum ScoreError {
    #[error(transparent)]
    Corpus(#[from] CorpusError),
    #[error("score I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("invalid score journal: {0}")]
    Json(#[from] serde_json::Error),
    #[error("score admission failed: {0}")]
    Invalid(&'static str),
    #[error("raw response length or SHA-256 differs: {0}")]
    ResponseHash(PathBuf),
    #[error("reference bytes consumed by scoring differ from the frozen asset: {0}")]
    ReferenceHash(PathBuf),
}

#[derive(Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    ServiceFailure,
    MetricLimit,
    #[default]
    MissingAttempt,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Metrics {
    pub cer: Distance,
    pub cer_whitespace_collapsed: Distance,
    pub wer: Option<Distance>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CaseScore<S> {
    pub case_id: S,
    pub original_page_id: S,
    pub document_cluster: S,
    pub writer_cluster: Option<S>,
    pub cohort: S,
    pub language: S,
    pub script: S,
    pub domain: S,
    pub appearance: S,
    pub reference_quality: S,
    pub task: S,
    pub word_tokenizer: S,
    pub input_sha256: S,
    pub reference_sha256: S,
    pub outcome: Outcome,
    pub first_http_status: Option<u16>,
    pub first_error: ArrayString<64>,
    pub first_output_sha256: ArrayString<64>,
    pub first_output_bytes: Option<u64>,
    pub first_elapsed_us: Option<u128>,
    pub attempts: usize,
    pub successful_attempts: usize,
    pub failed_attempts: usize,
    pub missing_attempts: usize,
    pub successful_outputs_differing_from_first_success: usize,
    pub mixed_success_and_failure: bool,
    pub metrics: Option<Metrics>,
}

#[derive(Debug, Default, Serialize)]
pub struct Aggregate {
    pub cases: usize,
    pub reference_units: u64,
    pub hypothesis_units: u64,
    pub errors: u64,
    pub micro_rate: Option<f64>,
    pub macro_case_rate: Option<f64>,
    pub empty_reference_cases: usize,
    #[serde(skip)]
    rate_sum: f64,
}

impl Aggregate {
    fn add(&mut self, distance: &Distance) {
        self.cases += 1;
        self.reference_units += distance.reference_units as u64;
        self.hypothesis_units += distance.hypothesis_units as u64;
        self.errors += distance.errors as u64;
        if let Some(rate) = distance.rate() {
            self.rate_sum += rate;
        } else {
            self.empty_reference_cases += 1;
        }
    }
    fn finish(&mut self) {
        self.micro_rate =
            (self.reference_units != 0).then(|| self.errors as f64 / self.reference_units as f64);
        let defined = self.cases - self.empty_reference_cases;
        self.macro_case_rate = (defined != 0).then(|| self.rate_sum / defined as f64);
    }
}

#[derive(Debug, Serialize)]
pub struct Summary<S> {
    pub cohort: S,
    pub axis: S,
    pub level: S,
    pub selected_cases: usize,
    pub scored_cases: usize,
    pub service_failures: usize,
    pub metric_limits: usize,
    pub missing_first_attempts: usize,
    pub document_clusters: usize,
    pub scored_document_clusters: usize,
    pub cer: Aggregate,
    pub cer_whitespace_collapsed: Aggregate,
    pub wer: Aggregate,
}

#[derive(Debug, Serialize)]
pub struct ScoreReport<S> {
    pub schema_version: u32,
    pub evaluator_version: S,
    pub metric_policy: S,
    pub nfc_unicode_version: (u8, u8, u8),
    pub rust_unicode_version: (u8, u8, u8),
    pub corpus_id: S,
    pub selection_seed: u64,
    pub manifest_sha256: String,
    pub manifest_bytes: u64,
    pub variant: S,
    pub cpus: u32,
    pub journal_sha256: String,
    pub journal_bytes: u64,
    pub selected_accuracy_repetition: u16,
    pub expected_repetitions: u16,
    pub journal_complete: bool,
    /// An interrupted final record is retained in the journal hash, never scored.
    pub truncated_final_record_bytes: usize,
    pub warmup_attempts: usize,
    pub nondeterministic_successful_output_cases: usize,
    pub mixed_success_failure_cases: usize,
    pub accuracy_scope: S,
    pub cases: ArrayVec<CaseScore<S>, 128>,
    pub summaries: ArrayVec<Summary<S>, 256>,
}

/// Comparison reads primitive case counts only. Cached summaries are ignored
/// and recomputed, avoiding both trust in derived values and large deserializer
/// stack copies of the entire report's fixed-capacity summary table.
#[derive(Debug, Deserialize)]
struct ScoredRun {
    schema_version: u32,
    metric_policy: String,
    nfc_unicode_version: (u8, u8, u8),
    rust_unicode_version: (u8, u8, u8),
    corpus_id: String,
    selection_seed: u64,
    manifest_sha256: String,
    manifest_bytes: u64,
    variant: String,
    cpus: u32,
    selected_accuracy_repetition: u16,
    expected_repetitions: u16,
    journal_complete: bool,
    truncated_final_record_bytes: usize,
    cases: ArrayVec<CaseScore<String>, 128>,
}

#[derive(Deserialize)]
struct Kind<'a> {
    kind: &'a str,
}
#[derive(Debug, Deserialize)]
struct Budget {
    cpus: u32,
    quota_us: u64,
    period_us: u64,
    memory_limit_bytes: u64,
}
#[derive(Debug, Deserialize)]
struct Environment {
    schema_version: u32,
    corpus_id: String,
    manifest_sha256: String,
    manifest_bytes: u64,
    variant: String,
    selection_seed: u64,
    repetitions: u16,
    budget: Budget,
}
#[derive(Debug, Deserialize)]
struct Attempt<'a> {
    variant: &'a str,
    cpus: u32,
    repetition: u16,
    sequence: usize,
    case_id: &'a str,
    document_cluster: &'a str,
    language: &'a str,
    script: &'a str,
    domain: &'a str,
    appearance: &'a str,
    reference_quality: &'a str,
    cohort: &'a str,
    task: &'a str,
    elapsed_us: u128,
    status: Option<u16>,
    error: Option<&'a str>,
    output_path: &'a Path,
    output_sha256: &'a str,
    output_bytes: u64,
}
#[derive(Deserialize)]
struct RepetitionEnd {
    repetition: u16,
    attempted_pages: usize,
    wall_us: u128,
}
#[derive(Deserialize)]
struct ResourceEnd {
    budget: Budget,
}

#[derive(Default)]
struct Observations {
    seen_repetitions: u32,
    attempts: usize,
    successful_attempts: usize,
    first_success_sha: Option<[u8; 32]>,
    changed_successful_outputs: usize,
    outcome: Outcome,
    first_http_status: Option<u16>,
    first_error: ArrayString<64>,
    first_output_sha256: ArrayString<64>,
    first_output_bytes: Option<u64>,
    first_elapsed_us: Option<u128>,
    metrics: Option<Metrics>,
}

fn validate_environment(
    environment: &Environment,
    corpus: &Corpus,
    manifest_sha256: &str,
    manifest_bytes: u64,
) -> Result<(), ScoreError> {
    if environment.schema_version != 1
        || environment.corpus_id != corpus.corpus_id
        || environment.selection_seed != corpus.selection_seed
        || environment.manifest_sha256 != manifest_sha256
        || environment.manifest_bytes != manifest_bytes
        || !(1..=20).contains(&environment.repetitions)
        || environment.variant.is_empty()
        || !environment
            .variant
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(ScoreError::Invalid(
            "environment does not identify the frozen corpus and bounded run",
        ));
    }
    validate_budget(&environment.budget)
}

fn validate_budget(budget: &Budget) -> Result<(), ScoreError> {
    if ![1, 2].contains(&budget.cpus)
        || budget.period_us == 0
        || budget.period_us.checked_mul(u64::from(budget.cpus)) != Some(budget.quota_us)
        || budget.memory_limit_bytes == 0
        || budget.memory_limit_bytes > 3 * 1024 * 1024 * 1024
    {
        return Err(ScoreError::Invalid(
            "journal lacks a bounded one/two-CPU parent budget",
        ));
    }
    Ok(())
}

fn validate_attempt(
    attempt: &Attempt<'_>,
    environment: &Environment,
    case: &Case,
    warmup: bool,
    cases: usize,
) -> Result<(), ScoreError> {
    if attempt.variant != environment.variant
        || attempt.cpus != environment.budget.cpus
        || attempt.document_cluster != case.document_cluster
        || attempt.language != case.language
        || attempt.script != case.script
        || attempt.domain != case.domain
        || attempt.appearance != case.appearance
        || attempt.reference_quality != case.reference_quality
        || attempt.cohort != case.cohort
        || attempt.task != case.task
        || if warmup {
            attempt.repetition != 0 || attempt.sequence != 0
        } else {
            attempt.repetition == 0
                || attempt.repetition > environment.repetitions
                || attempt.sequence >= cases
        }
    {
        return Err(ScoreError::Invalid(
            "attempt metadata disagrees with corpus or run environment",
        ));
    }
    Ok(())
}

/// Score a run without adding any work to its recorded OCR intervals.
pub fn score(options: &ScoreOptions<'_>) -> Result<(), ScoreError> {
    let (corpus, manifest_sha256, manifest_bytes): (Corpus, _, _) =
        corpus::read_json_hashed(options.manifest)?;
    corpus.validate(options.root)?;
    let mut unique_pages: ArrayVec<&str, 128> = ArrayVec::new();
    for case in &corpus.cases {
        if case.cohort == "primary" && unique_pages.contains(&case.original_page_id.as_str()) {
            return Err(ScoreError::Invalid(
                "primary accuracy requires one preselected case per original page",
            ));
        }
        if case.cohort == "primary" {
            unique_pages.push(&case.original_page_id);
        }
    }
    let journal_path = corpus::safe_path(options.run, Path::new("requests.jsonl"))?;
    let journal = File::open(&journal_path)?;
    if journal.metadata()?.len() > MAX_JOURNAL_BYTES {
        return Err(ScoreError::Invalid("journal exceeds byte bound"));
    }
    let mut reader = BufReader::new(journal);
    let mut record_bytes: ArrayVec<u8, 65_537> = ArrayVec::new();
    let mut reference = String::with_capacity(131_072);
    let mut response = String::with_capacity(131_072);
    let mut workspace = Workspace::default();
    let mut environment: Option<Environment> = None;
    let mut observations: [Observations; 128] = std::array::from_fn(|_| Observations::default());
    let mut timings = performance::Timings::new();
    let mut repetition_ends = 0_u32;
    let mut warmups = 0_usize;
    let mut resource_end = false;
    let mut latest_repetition = 0;
    let mut truncated_final_record_bytes = 0;
    let mut journal_bytes = 0_u64;
    let mut journal_hash = Sha256::new();
    loop {
        read_record(&mut reader, &mut record_bytes)?;
        let bytes = record_bytes.len();
        if bytes == 0 {
            break;
        }
        journal_bytes += bytes as u64;
        journal_hash.update(&record_bytes);
        if bytes as u64 > MAX_RECORD_BYTES || journal_bytes > MAX_JOURNAL_BYTES {
            return Err(ScoreError::Invalid(
                "journal record or total exceeds byte bound",
            ));
        }
        if resource_end {
            return Err(ScoreError::Invalid("records follow resource_end"));
        }
        if record_bytes.last() != Some(&b'\n') {
            // The newline is the journal record boundary. Preserve evidence of
            // an interrupted final write without treating a partial attempt as
            // a successful observation or silently dropping missing cases.
            truncated_final_record_bytes = bytes;
            break;
        }
        let line = std::str::from_utf8(&record_bytes)
            .map_err(|_| ScoreError::Invalid("complete journal record is not valid UTF-8"))?;
        let kind: Kind<'_> = serde_json::from_str(line)?;
        match kind.kind {
            "environment" => {
                if environment.is_some() {
                    return Err(ScoreError::Invalid("duplicate environment record"));
                }
                let parsed: Environment = serde_json::from_str(line)?;
                validate_environment(&parsed, &corpus, &manifest_sha256, manifest_bytes)?;
                environment = Some(parsed);
            }
            "attempt" | "warmup" => {
                let environment = environment
                    .as_ref()
                    .ok_or(ScoreError::Invalid("attempt precedes environment"))?;
                let attempt: Attempt<'_> = serde_json::from_str(line)?;
                let index = corpus
                    .cases
                    .iter()
                    .position(|case| case.id == attempt.case_id)
                    .ok_or(ScoreError::Invalid("attempt references an unknown case"))?;
                let case = &corpus.cases[index];
                let warmup = kind.kind == "warmup";
                validate_attempt(&attempt, environment, case, warmup, corpus.cases.len())?;
                if attempt.repetition < latest_repetition {
                    return Err(ScoreError::Invalid("repetitions are out of order"));
                }
                if repetition_ends & (1 << attempt.repetition) != 0 {
                    return Err(ScoreError::Invalid("attempt follows its repetition end"));
                }
                latest_repetition = attempt.repetition;
                let actual_sha = read_response(options.run, case, &attempt, &mut response)?;
                if warmup {
                    warmups += 1;
                    if warmups > 1 {
                        return Err(ScoreError::Invalid("more than one predeclared warmup"));
                    }
                    continue;
                }
                let observation = &mut observations[index];
                let bit = 1_u32 << attempt.repetition;
                if observation.seen_repetitions & bit != 0 {
                    return Err(ScoreError::Invalid("duplicate case/repetition attempt"));
                }
                observation.seen_repetitions |= bit;
                observe(observation, &attempt, actual_sha)?;
                timings.attempt(index, &attempt)?;
                if attempt.repetition == 1 && observation.outcome == Outcome::Success {
                    read_reference(options.root, &case.reference, &mut reference)?;
                    match measure(
                        &mut workspace,
                        &reference,
                        &response,
                        case.word_tokenizer == "whitespace",
                    ) {
                        Ok(metrics) => observation.metrics = Some(metrics),
                        Err(MetricError::TooLong) => {
                            observation.outcome = Outcome::MetricLimit;
                            observation
                                .first_error
                                .try_push_str("metric_codepoint_limit")
                                .map_err(|_| ScoreError::Invalid("error label exceeds capacity"))?;
                        }
                    }
                }
            }
            "repetition_end" => {
                let environment = environment
                    .as_ref()
                    .ok_or(ScoreError::Invalid("repetition end precedes environment"))?;
                let end: RepetitionEnd = serde_json::from_str(line)?;
                if end.repetition > environment.repetitions
                    || end.repetition != latest_repetition
                    || repetition_ends & (1 << end.repetition) != 0
                    || end.attempted_pages
                        != if end.repetition == 0 {
                            1
                        } else {
                            corpus.cases.len()
                        }
                    || (end.repetition == 0 && warmups != 1)
                    || (end.repetition != 0
                        && observations[..corpus.cases.len()]
                            .iter()
                            .any(|case| case.seen_repetitions & (1 << end.repetition) == 0))
                {
                    return Err(ScoreError::Invalid(
                        "repetition end disagrees with observed attempts",
                    ));
                }
                repetition_ends |= 1 << end.repetition;
                if end.repetition != 0 {
                    timings.finish_repetition(&end)?;
                }
            }
            "resource_end" => {
                let environment = environment
                    .as_ref()
                    .ok_or(ScoreError::Invalid("resource end precedes environment"))?;
                let end: ResourceEnd = serde_json::from_str(line)?;
                validate_budget(&end.budget)?;
                if end.budget.cpus != environment.budget.cpus
                    || end.budget.quota_us != environment.budget.quota_us
                    || end.budget.period_us != environment.budget.period_us
                    || end.budget.memory_limit_bytes != environment.budget.memory_limit_bytes
                {
                    return Err(ScoreError::Invalid(
                        "parent budget changed during measurement",
                    ));
                }
                resource_end = true;
            }
            _ => return Err(ScoreError::Invalid("unknown journal record kind")),
        }
    }
    let environment = environment.ok_or(ScoreError::Invalid("journal has no environment"))?;
    let journal_sha256 = format!("{:x}", journal_hash.finalize());
    if sha256_file(&journal_path)? != journal_sha256 {
        return Err(ScoreError::Invalid("journal changed while scoring"));
    }
    let mut cases = ArrayVec::new();
    for (index, case) in corpus.cases.iter().enumerate() {
        let observed = std::mem::take(&mut observations[index]);
        cases.push(CaseScore {
            case_id: case.id.as_str(),
            original_page_id: case.original_page_id.as_str(),
            document_cluster: case.document_cluster.as_str(),
            writer_cluster: case.writer_cluster.as_deref(),
            cohort: case.cohort.as_str(),
            language: case.language.as_str(),
            script: case.script.as_str(),
            domain: case.domain.as_str(),
            appearance: case.appearance.as_str(),
            reference_quality: case.reference_quality.as_str(),
            task: case.task.as_str(),
            word_tokenizer: case.word_tokenizer.as_str(),
            input_sha256: case.input.sha256.as_str(),
            reference_sha256: case.reference.sha256.as_str(),
            outcome: observed.outcome,
            first_http_status: observed.first_http_status,
            first_error: observed.first_error,
            first_output_sha256: observed.first_output_sha256,
            first_output_bytes: observed.first_output_bytes,
            first_elapsed_us: observed.first_elapsed_us,
            attempts: observed.attempts,
            successful_attempts: observed.successful_attempts,
            failed_attempts: observed.attempts - observed.successful_attempts,
            missing_attempts: usize::from(environment.repetitions) - observed.attempts,
            successful_outputs_differing_from_first_success: observed.changed_successful_outputs,
            mixed_success_and_failure: observed.successful_attempts != 0
                && observed.successful_attempts != observed.attempts,
            metrics: observed.metrics,
        });
    }
    let summaries = summaries(&cases)?;
    let report = ScoreReport {
        schema_version: 1,
        evaluator_version: env!("CARGO_PKG_VERSION"),
        metric_policy: METRIC_POLICY,
        nfc_unicode_version: unicode_normalization::UNICODE_VERSION,
        rust_unicode_version: char::UNICODE_VERSION,
        corpus_id: corpus.corpus_id.as_str(),
        selection_seed: corpus.selection_seed,
        manifest_sha256,
        manifest_bytes,
        variant: environment.variant.as_str(),
        cpus: environment.budget.cpus,
        journal_sha256,
        journal_bytes,
        selected_accuracy_repetition: 1,
        expected_repetitions: environment.repetitions,
        journal_complete: resource_end
            && repetition_ends == (1 << (environment.repetitions + 1)) - 1,
        truncated_final_record_bytes,
        warmup_attempts: warmups,
        nondeterministic_successful_output_cases: cases
            .iter()
            .filter(|case| case.successful_outputs_differing_from_first_success != 0)
            .count(),
        mixed_success_failure_cases: cases
            .iter()
            .filter(|case| case.mixed_success_and_failure)
            .count(),
        accuracy_scope: "Repetition 1 only. Primary cases select distinct original pages, but a supplied input can be a full page or an oracle crop; accuracy covers only the supplied input. Challenge units are prespecified cases, including oracle crops whose original-page grouping may be unresolved. Shared challenge source groups remain one document cluster. CER: NFC Unicode scalars, unit-cost Levenshtein, case/punctuation/whitespace retained. Secondary CER collapses Unicode whitespace and trims its boundaries only. WER: NFC, whitespace tokens only where declared. Rates can exceed 1. Empty-reference rates are undefined; counts remain. Accuracy summaries condition on successful, scoreable first attempts; failures and missing attempts remain explicit. Primary and challenge cohorts are never pooled. This selected corpus does not represent an unknown population of documents.",
        cases,
        summaries,
    };
    write_report(options.output, &report)?;
    timings.write(options.output, &corpus, &report)
}

fn read_response(
    run: &Path,
    case: &Case,
    attempt: &Attempt<'_>,
    response: &mut String,
) -> Result<[u8; 32], ScoreError> {
    let expected_name = format!("{}-r{:02}.txt", case.id, attempt.repetition);
    if attempt
        .output_path
        .file_name()
        .and_then(|name| name.to_str())
        != Some(expected_name.as_str())
    {
        return Err(ScoreError::Invalid(
            "response name disagrees with case/repetition",
        ));
    }
    // The response directory may be relocated as an artifact. Ignore historical
    // parent paths and open only the validated filename beneath the selected run.
    let path = corpus::safe_path(run, Path::new(&expected_name))?;
    let mut file = File::open(&path)?;
    if attempt.output_bytes > MAX_RESPONSE_BYTES || file.metadata()?.len() != attempt.output_bytes {
        return Err(ScoreError::ResponseHash(path));
    }
    response.clear();
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65_536];
    let mut bytes = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        if bytes > attempt.output_bytes {
            return Err(ScoreError::ResponseHash(path));
        }
        hash.update(&buffer[..read]);
    }
    let digest: [u8; 32] = hash.finalize().into();
    if bytes != attempt.output_bytes || !matches_sha256(&digest, attempt.output_sha256) {
        return Err(ScoreError::ResponseHash(path));
    }
    if attempt.error.is_none()
        && attempt
            .status
            .is_some_and(|status| (200..300).contains(&status))
    {
        // Failed bodies may be binary or invalid UTF-8. They remain byte-hashed
        // evidence without becoming text or entering any accuracy denominator.
        file.rewind()?;
        Read::by_ref(&mut file)
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_string(response)?;
        let decoded_digest: [u8; 32] = Sha256::digest(response.as_bytes()).into();
        if response.len() as u64 != bytes || decoded_digest != digest {
            return Err(ScoreError::ResponseHash(path));
        }
    }
    Ok(digest)
}

fn matches_sha256(digest: &[u8; 32], expected: &str) -> bool {
    if expected.len() != 64 {
        return false;
    }
    digest
        .iter()
        .zip(expected.as_bytes().as_chunks::<2>().0)
        .all(|(&byte, pair)| {
            let digit = |value: u8| match value {
                b'0'..=b'9' => Some(value - b'0'),
                b'a'..=b'f' => Some(value - b'a' + 10),
                _ => None,
            };
            digit(pair[0])
                .zip(digit(pair[1]))
                .is_some_and(|(a, b)| a * 16 + b == byte)
        })
}

fn observe(
    observed: &mut Observations,
    attempt: &Attempt<'_>,
    sha: [u8; 32],
) -> Result<(), ScoreError> {
    observed.attempts += 1;
    let success = attempt.error.is_none()
        && attempt
            .status
            .is_some_and(|status| (200..300).contains(&status));
    if success {
        observed.successful_attempts += 1;
        if let Some(first) = observed.first_success_sha {
            if first != sha {
                observed.changed_successful_outputs += 1;
            }
        } else {
            observed.first_success_sha = Some(sha);
        }
    }
    if attempt.repetition == 1 {
        observed.outcome = if success {
            Outcome::Success
        } else {
            Outcome::ServiceFailure
        };
        observed.first_http_status = attempt.status;
        observed
            .first_error
            .try_push_str(
                attempt
                    .error
                    .unwrap_or(if success { "" } else { "non_success_http" }),
            )
            .map_err(|_| ScoreError::Invalid("error label exceeds capacity"))?;
        observed
            .first_output_sha256
            .try_push_str(attempt.output_sha256)
            .map_err(|_| ScoreError::Invalid("invalid SHA-256 length"))?;
        observed.first_output_bytes = Some(attempt.output_bytes);
        observed.first_elapsed_us = Some(attempt.elapsed_us);
    }
    Ok(())
}

fn measure(
    workspace: &mut Workspace,
    reference: &str,
    response: &str,
    words: bool,
) -> Result<Metrics, MetricError> {
    Ok(Metrics {
        cer: workspace.characters(reference, response, false)?,
        cer_whitespace_collapsed: workspace.characters(reference, response, true)?,
        wer: if words {
            Some(workspace.words(reference, response)?)
        } else {
            None
        },
    })
}

fn summaries<'a>(
    cases: &[CaseScore<&'a str>],
) -> Result<ArrayVec<Summary<&'a str>, 256>, ScoreError> {
    let mut out: ArrayVec<Summary<&str>, 256> = ArrayVec::new();
    for case in cases {
        for (axis, level) in [
            ("overall", "all"),
            ("language", case.language),
            ("script", case.script),
            ("domain", case.domain),
            ("appearance", case.appearance),
            ("reference_quality", case.reference_quality),
        ] {
            if !out
                .iter()
                .any(|row| row.cohort == case.cohort && row.axis == axis && row.level == level)
            {
                out.try_push(Summary {
                    cohort: case.cohort,
                    axis,
                    level,
                    selected_cases: 0,
                    scored_cases: 0,
                    service_failures: 0,
                    metric_limits: 0,
                    missing_first_attempts: 0,
                    document_clusters: 0,
                    scored_document_clusters: 0,
                    cer: Aggregate::default(),
                    cer_whitespace_collapsed: Aggregate::default(),
                    wer: Aggregate::default(),
                })
                .map_err(|_| ScoreError::Invalid("more than 256 reporting strata"))?;
            }
        }
    }
    for summary in &mut out {
        let mut clusters: ArrayVec<&str, 128> = ArrayVec::new();
        let mut scored_clusters: ArrayVec<&str, 128> = ArrayVec::new();
        for case in cases.iter().filter(|case| {
            case.cohort == summary.cohort && stratum(case, summary.axis) == summary.level
        }) {
            summary.selected_cases += 1;
            if !clusters.contains(&case.document_cluster) {
                clusters.push(case.document_cluster);
            }
            match case.outcome {
                Outcome::Success => {
                    if !scored_clusters.contains(&case.document_cluster) {
                        scored_clusters.push(case.document_cluster);
                    }
                    let metrics = case.metrics.as_ref().ok_or(ScoreError::Invalid(
                        "successful first attempt has no metrics",
                    ))?;
                    summary.scored_cases += 1;
                    summary.cer.add(&metrics.cer);
                    summary
                        .cer_whitespace_collapsed
                        .add(&metrics.cer_whitespace_collapsed);
                    if let Some(wer) = &metrics.wer {
                        summary.wer.add(wer);
                    }
                }
                Outcome::ServiceFailure => summary.service_failures += 1,
                Outcome::MetricLimit => summary.metric_limits += 1,
                Outcome::MissingAttempt => summary.missing_first_attempts += 1,
            }
        }
        summary.document_clusters = clusters.len();
        summary.scored_document_clusters = scored_clusters.len();
        summary.cer.finish();
        summary.cer_whitespace_collapsed.finish();
        summary.wer.finish();
    }
    Ok(out)
}

fn stratum<'a, S: AsRef<str>>(case: &'a CaseScore<S>, axis: &str) -> &'a str {
    match axis {
        "language" => case.language.as_ref(),
        "script" => case.script.as_ref(),
        "domain" => case.domain.as_ref(),
        "appearance" => case.appearance.as_ref(),
        "reference_quality" => case.reference_quality.as_ref(),
        _ => "all",
    }
}

fn read_reference(root: &Path, asset: &corpus::Asset, text: &mut String) -> Result<(), ScoreError> {
    if asset.bytes > 2 * 1024 * 1024 {
        return Err(ScoreError::Invalid(
            "reference exceeds the frozen 2 MiB limit",
        ));
    }
    let path = corpus::safe_path(root, &asset.path)?;
    text.clear();
    File::open(&path)?
        .take(asset.bytes + 1)
        .read_to_string(text)?;
    let actual: [u8; 32] = Sha256::digest(text.as_bytes()).into();
    if text.len() as u64 != asset.bytes || !matches_sha256(&actual, &asset.sha256) {
        return Err(ScoreError::ReferenceHash(path));
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, ScoreError> {
    let file = File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_JOURNAL_BYTES {
        return Err(ScoreError::Invalid("journal exceeds the fixed byte bound"));
    }
    let mut file = file.take(MAX_JOURNAL_BYTES + 1);
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65_536];
    let mut bytes = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        if bytes > MAX_JOURNAL_BYTES {
            return Err(ScoreError::Invalid(
                "journal grew beyond the fixed byte bound",
            ));
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn write_report(output: &Path, report: &ScoreReport<&str>) -> Result<(), ScoreError> {
    fs::create_dir_all(output)?;
    let mut csv = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("cases.csv"))?;
    csv.write_all(b"variant,cpus,case_id,original_page_id,document_cluster,writer_cluster,cohort,language,script,domain,appearance,reference_quality,task,word_tokenizer,outcome,first_http_status,first_error,first_output_sha256,first_output_bytes,first_elapsed_us,attempts,successful_attempts,failed_attempts,missing_attempts,successful_outputs_differing_from_first_success,mixed_success_and_failure,cer_errors,cer_reference_units,cer_hypothesis_units,cer,collapsed_cer_errors,collapsed_cer_reference_units,collapsed_cer_hypothesis_units,collapsed_cer,wer_errors,wer_reference_units,wer_hypothesis_units,wer\n")?;
    for case in &report.cases {
        csv_field(&mut csv, report.variant)?;
        write!(csv, ",{},", report.cpus)?;
        for value in [
            case.case_id,
            case.original_page_id,
            case.document_cluster,
            case.writer_cluster.unwrap_or(""),
            case.cohort,
            case.language,
            case.script,
            case.domain,
            case.appearance,
            case.reference_quality,
            case.task,
            case.word_tokenizer,
        ] {
            csv_field(&mut csv, value)?;
            csv.write_all(b",")?;
        }
        csv_field(
            &mut csv,
            match case.outcome {
                Outcome::Success => "success",
                Outcome::ServiceFailure => "service_failure",
                Outcome::MetricLimit => "metric_limit",
                Outcome::MissingAttempt => "missing_attempt",
            },
        )?;
        csv.write_all(b",")?;
        csv_option(&mut csv, case.first_http_status)?;
        csv.write_all(b",")?;
        csv_field(&mut csv, &case.first_error)?;
        csv.write_all(b",")?;
        csv_field(&mut csv, &case.first_output_sha256)?;
        csv.write_all(b",")?;
        csv_option(&mut csv, case.first_output_bytes)?;
        csv.write_all(b",")?;
        csv_option(&mut csv, case.first_elapsed_us)?;
        write!(
            csv,
            ",{},{},{},{},{},{}",
            case.attempts,
            case.successful_attempts,
            case.failed_attempts,
            case.missing_attempts,
            case.successful_outputs_differing_from_first_success,
            case.mixed_success_and_failure
        )?;
        if let Some(metrics) = &case.metrics {
            csv_distance(&mut csv, Some(&metrics.cer))?;
            csv_distance(&mut csv, Some(&metrics.cer_whitespace_collapsed))?;
            csv_distance(&mut csv, metrics.wer.as_ref())?;
        } else {
            for _ in 0..3 {
                csv_distance(&mut csv, None)?;
            }
        }
        csv.write_all(b"\n")?;
    }
    csv.sync_all()?;
    let mut json = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("scores.json"))?;
    serde_json::to_writer_pretty(&mut json, report)?;
    json.write_all(b"\n")?;
    json.sync_all()?;
    Ok(())
}

fn csv_field(output: &mut impl Write, value: &str) -> io::Result<()> {
    output.write_all(b"\"")?;
    for part in value.split_inclusive('"') {
        output.write_all(part.as_bytes())?;
        if part.ends_with('"') {
            output.write_all(b"\"")?;
        }
    }
    output.write_all(b"\"")
}
fn csv_option(output: &mut impl Write, value: Option<impl std::fmt::Display>) -> io::Result<()> {
    if let Some(value) = value {
        write!(output, "{value}")?;
    }
    Ok(())
}
fn csv_distance(output: &mut impl Write, distance: Option<&Distance>) -> io::Result<()> {
    if let Some(distance) = distance {
        write!(
            output,
            ",{},{},{},",
            distance.errors, distance.reference_units, distance.hypothesis_units
        )?;
        csv_option(output, distance.rate())
    } else {
        output.write_all(b",,,,")
    }
}

#[derive(Serialize)]
struct PairedSummary<'a> {
    cohort: &'a str,
    axis: &'a str,
    level: &'a str,
    seed: u64,
    selected_cases: usize,
    joint_success_cases: usize,
    left_service_failures: usize,
    right_service_failures: usize,
    left_unscoreable: usize,
    right_unscoreable: usize,
    cer: bootstrap::Estimate,
    cer_whitespace_collapsed: bootstrap::Estimate,
    wer: bootstrap::Estimate,
}

#[derive(Serialize)]
struct Comparison<'a> {
    schema_version: u32,
    metric_policy: &'a str,
    corpus_id: &'a str,
    selection_seed: u64,
    manifest_sha256: &'a str,
    manifest_bytes: u64,
    left_variant: &'a str,
    right_variant: &'a str,
    left_cpus: u32,
    right_cpus: u32,
    left_scores_sha256: String,
    right_scores_sha256: String,
    left_scores_bytes: u64,
    right_scores_bytes: u64,
    bootstrap_seed: u64,
    requested_bootstrap_samples: usize,
    inference_scope: &'static str,
    strata: ArrayVec<PairedSummary<'a>, 256>,
}

struct ClusterEntry<'a> {
    name: &'a str,
    values: bootstrap::Cluster,
}

/// Compare the same frozen cases, paired by original document cluster.
/// Confidence intervals describe this selected corpus under cluster resampling;
/// they are not corpus-independent accuracy guarantees or multiplicity-adjusted tests.
pub fn compare(options: &CompareOptions<'_>) -> Result<(), ScoreError> {
    let (left, left_scores_sha256, left_scores_bytes): (ScoredRun, _, _) =
        corpus::read_json_hashed(options.left)?;
    let (right, right_scores_sha256, right_scores_bytes): (ScoredRun, _, _) =
        corpus::read_json_hashed(options.right)?;
    validate_scores(&left)?;
    validate_scores(&right)?;
    if left.corpus_id != right.corpus_id
        || left.selection_seed != right.selection_seed
        || left.manifest_sha256 != right.manifest_sha256
        || left.manifest_bytes != right.manifest_bytes
        || left.cases.len() != right.cases.len()
        || left.metric_policy != right.metric_policy
        || left.nfc_unicode_version != right.nfc_unicode_version
        || left.rust_unicode_version != right.rust_unicode_version
    {
        return Err(ScoreError::Invalid(
            "score artifacts use different corpora or metric policies",
        ));
    }
    let mut right_indices = [0_usize; 128];
    for (index, case) in left.cases.iter().enumerate() {
        let right_index = right
            .cases
            .iter()
            .position(|other| other.case_id == case.case_id)
            .ok_or(ScoreError::Invalid(
                "paired variant is missing a frozen case",
            ))?;
        same_case(case, &right.cases[right_index])?;
        right_indices[index] = right_index;
    }
    let mut keys: ArrayVec<(&str, &str, &str), 256> = ArrayVec::new();
    for case in &left.cases {
        for axis in [
            "overall",
            "language",
            "script",
            "domain",
            "appearance",
            "reference_quality",
        ] {
            let key = (case.cohort.as_str(), axis, stratum(case, axis));
            if !keys.contains(&key) {
                keys.try_push(key)
                    .map_err(|_| ScoreError::Invalid("too many comparison strata"))?;
            }
        }
    }
    let mut strata = ArrayVec::new();
    let mut workspace = bootstrap::Workspace::default();
    for (cohort, axis, level) in keys {
        let mut selected = 0;
        let mut joint_success = 0;
        let mut left_failures = 0;
        let mut right_failures = 0;
        let mut left_unscoreable = 0;
        let mut right_unscoreable = 0;
        let mut clusters: ArrayVec<ClusterEntry<'_>, 128> = ArrayVec::new();
        for (index, case) in left
            .cases
            .iter()
            .enumerate()
            .filter(|(_, case)| case.cohort == cohort && stratum(case, axis) == level)
        {
            selected += 1;
            let other = &right.cases[right_indices[index]];
            left_failures += usize::from(case.outcome == Outcome::ServiceFailure);
            right_failures += usize::from(other.outcome == Outcome::ServiceFailure);
            left_unscoreable += usize::from(matches!(
                case.outcome,
                Outcome::MetricLimit | Outcome::MissingAttempt
            ));
            right_unscoreable += usize::from(matches!(
                other.outcome,
                Outcome::MetricLimit | Outcome::MissingAttempt
            ));
            let (Some(a), Some(b)) = (&case.metrics, &other.metrics) else {
                continue;
            };
            joint_success += 1;
            let cluster_index = if let Some(index) = clusters
                .iter()
                .position(|cluster| cluster.name == case.document_cluster)
            {
                index
            } else {
                clusters
                    .try_push(ClusterEntry {
                        name: &case.document_cluster,
                        values: bootstrap::Cluster::default(),
                    })
                    .map_err(|_| ScoreError::Invalid("too many document clusters"))?;
                clusters.len() - 1
            };
            let cluster = &mut clusters[cluster_index].values;
            add_pair(cluster, 0, &a.cer, &b.cer)?;
            add_pair(
                cluster,
                1,
                &a.cer_whitespace_collapsed,
                &b.cer_whitespace_collapsed,
            )?;
            match (&a.wer, &b.wer) {
                (Some(a), Some(b)) => add_pair(cluster, 2, a, b)?,
                (None, None) => (),
                _ => {
                    return Err(ScoreError::Invalid(
                        "paired variants disagree on WER eligibility",
                    ));
                }
            }
        }
        clusters.sort_unstable_by(|a, b| a.name.cmp(b.name));
        let mut values: ArrayVec<bootstrap::Cluster, 128> = ArrayVec::new();
        for cluster in clusters {
            values.push(cluster.values);
        }
        let seed = stratum_seed(options.seed, cohort, axis, level);
        let [cer, cer_whitespace_collapsed, wer] =
            workspace.estimate(&values, seed, options.resamples)?;
        strata
            .try_push(PairedSummary {
                cohort,
                axis,
                level,
                seed,
                selected_cases: selected,
                joint_success_cases: joint_success,
                left_service_failures: left_failures,
                right_service_failures: right_failures,
                left_unscoreable,
                right_unscoreable,
                cer,
                cer_whitespace_collapsed,
                wer,
            })
            .map_err(|_| ScoreError::Invalid("too many paired summaries"))?;
    }
    let comparison = Comparison {
        schema_version: 1,
        metric_policy: &left.metric_policy,
        corpus_id: &left.corpus_id,
        selection_seed: left.selection_seed,
        manifest_sha256: &left.manifest_sha256,
        manifest_bytes: left.manifest_bytes,
        left_variant: &left.variant,
        right_variant: &right.variant,
        left_cpus: left.cpus,
        right_cpus: right.cpus,
        left_scores_sha256,
        right_scores_sha256,
        left_scores_bytes,
        right_scores_bytes,
        bootstrap_seed: options.seed,
        requested_bootstrap_samples: options.resamples,
        inference_scope: "Repetition 1, jointly successful and scoreable cases only. Failures remain separate; later successes never repair first-attempt failure. Point estimates are paired micro error-rate differences (right minus left; negative favors right). Requested bootstrap samples 0 means descriptive estimates only: no draws or confidence intervals, regardless of cluster count. Otherwise whole document clusters are resampled with replacement using SplitMix64 and unbiased integer sampling, preserving every selected page within each sampled cluster. 95% percentile intervals use linear interpolation. Stratum seeds derive from SHA-256(base seed and length-prefixed cohort/axis/level), with clusters sorted by identifier. Fewer than two eligible document clusters or any undefined bootstrap draw yields no interval. WER excludes cases without declared whitespace tokenization. Primary/challenge are separate; strata are descriptive, not simultaneous or multiplicity-adjusted significance tests. These conditional, selected-corpus estimates do not establish general population accuracy or performance superiority.",
        strata,
    };
    write_comparison(options.output, &comparison, &left, &right, &right_indices)
}

fn validate_scores(report: &ScoredRun) -> Result<(), ScoreError> {
    if report.schema_version != 1
        || report.metric_policy != METRIC_POLICY
        || report.selected_accuracy_repetition != 1
        || !report.journal_complete
        || report.truncated_final_record_bytes != 0
        || report.nfc_unicode_version != unicode_normalization::UNICODE_VERSION
        || report.rust_unicode_version != char::UNICODE_VERSION
        || !(1..=20).contains(&report.expected_repetitions)
        || report.manifest_bytes == 0
        || report.manifest_bytes > corpus::MAX_MANIFEST_BYTES
        || report.manifest_sha256.len() != 64
        || !report
            .manifest_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || report.cases.is_empty()
        || ![1, 2].contains(&report.cpus)
    {
        return Err(ScoreError::Invalid(
            "comparison requires complete schema-1 runs using the prespecified first-repetition metric policy",
        ));
    }
    let mut ids: ArrayVec<&str, 128> = ArrayVec::new();
    let mut pages: ArrayVec<&str, 128> = ArrayVec::new();
    for case in &report.cases {
        if ids.contains(&case.case_id.as_str())
            || (case.cohort == "primary" && pages.contains(&case.original_page_id.as_str()))
            || !matches!(case.cohort.as_str(), "primary" | "challenge")
            || case.document_cluster.is_empty()
            || (case.outcome == Outcome::Success) != case.metrics.is_some()
            || case.attempts != usize::from(report.expected_repetitions)
            || case.missing_attempts != 0
            || case.successful_attempts > case.attempts
            || case.failed_attempts != case.attempts - case.successful_attempts
        {
            return Err(ScoreError::Invalid(
                "score case identity, cohort or success/metric state is inconsistent",
            ));
        }
        ids.push(&case.case_id);
        if case.cohort == "primary" {
            pages.push(&case.original_page_id);
        }
        if report.cases.iter().any(|other| {
            other.original_page_id == case.original_page_id
                && other.cohort == case.cohort
                && other.document_cluster != case.document_cluster
        }) {
            return Err(ScoreError::Invalid(
                "shared original-page groups must remain one document cluster",
            ));
        }
        if let Some(metrics) = &case.metrics {
            validate_distance(&metrics.cer, 32_768)?;
            validate_distance(&metrics.cer_whitespace_collapsed, 32_768)?;
            if (case.word_tokenizer == "whitespace") != metrics.wer.is_some() {
                return Err(ScoreError::Invalid(
                    "WER eligibility contradicts the declared tokenizer",
                ));
            }
            if let Some(wer) = &metrics.wer {
                validate_distance(wer, 16_384)?;
            }
        }
    }
    Ok(())
}

fn validate_distance(distance: &Distance, maximum: usize) -> Result<(), ScoreError> {
    if distance.reference_units > maximum
        || distance.hypothesis_units > maximum
        || distance.errors > distance.reference_units.max(distance.hypothesis_units)
        || distance.errors < distance.reference_units.abs_diff(distance.hypothesis_units)
    {
        return Err(ScoreError::Invalid(
            "invalid bounded unit-cost edit-distance counts",
        ));
    }
    Ok(())
}

fn same_case(left: &CaseScore<String>, right: &CaseScore<String>) -> Result<(), ScoreError> {
    if left.original_page_id != right.original_page_id
        || left.document_cluster != right.document_cluster
        || left.writer_cluster != right.writer_cluster
        || left.cohort != right.cohort
        || left.language != right.language
        || left.script != right.script
        || left.domain != right.domain
        || left.appearance != right.appearance
        || left.reference_quality != right.reference_quality
        || left.task != right.task
        || left.word_tokenizer != right.word_tokenizer
        || left.input_sha256 != right.input_sha256
        || left.reference_sha256 != right.reference_sha256
    {
        return Err(ScoreError::Invalid(
            "paired case input/reference hashes or analysis strata differ",
        ));
    }
    Ok(())
}

fn add_pair(
    cluster: &mut bootstrap::Cluster,
    metric: usize,
    left: &Distance,
    right: &Distance,
) -> Result<(), ScoreError> {
    if left.reference_units != right.reference_units {
        return Err(ScoreError::Invalid("paired reference-unit counts differ"));
    }
    cluster.reference_units[metric] += left.reference_units as u64;
    cluster.left_errors[metric] += left.errors as u64;
    cluster.right_errors[metric] += right.errors as u64;
    cluster.cases[metric] += 1;
    Ok(())
}

fn stratum_seed(seed: u64, cohort: &str, axis: &str, level: &str) -> u64 {
    let mut hash = Sha256::new();
    hash.update(seed.to_le_bytes());
    for value in [cohort, axis, level] {
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    }
    let digest: [u8; 32] = hash.finalize().into();
    u64::from_le_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ])
}

fn write_comparison(
    output: &Path,
    comparison: &Comparison<'_>,
    left: &ScoredRun,
    right: &ScoredRun,
    right_indices: &[usize; 128],
) -> Result<(), ScoreError> {
    fs::create_dir_all(output)?;
    let mut paired = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("paired.csv"))?;
    paired.write_all(b"left_variant,right_variant,left_cpus,right_cpus,cohort,axis,level,metric,selected_cases,joint_success_cases,left_service_failures,right_service_failures,left_unscoreable,right_unscoreable,paired_cases,document_clusters,reference_units,left_errors,right_errors,left_rate,right_rate,right_minus_left,ci_95_low,ci_95_high,bootstrap_samples,undefined_bootstrap_samples,stratum_seed\n")?;
    for row in &comparison.strata {
        for (name, metric) in [
            ("cer", &row.cer),
            ("cer_whitespace_collapsed", &row.cer_whitespace_collapsed),
            ("wer", &row.wer),
        ] {
            csv_field(&mut paired, &left.variant)?;
            paired.write_all(b",")?;
            csv_field(&mut paired, &right.variant)?;
            write!(paired, ",{},{},", left.cpus, right.cpus)?;
            for value in [row.cohort, row.axis, row.level, name] {
                csv_field(&mut paired, value)?;
                paired.write_all(b",")?;
            }
            write!(
                paired,
                "{},{},{},{},{},{},{},{},{},{},{},",
                row.selected_cases,
                row.joint_success_cases,
                row.left_service_failures,
                row.right_service_failures,
                row.left_unscoreable,
                row.right_unscoreable,
                metric.paired_cases,
                metric.document_clusters,
                metric.reference_units,
                metric.left_errors,
                metric.right_errors
            )?;
            for value in [
                metric.left_rate,
                metric.right_rate,
                metric.right_minus_left,
                metric.ci_95_low,
                metric.ci_95_high,
            ] {
                csv_option(&mut paired, value)?;
                paired.write_all(b",")?;
            }
            writeln!(
                paired,
                "{},{},{}",
                metric.bootstrap_samples, metric.undefined_bootstrap_samples, row.seed
            )?;
        }
    }
    paired.sync_all()?;
    let mut pairs = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("pairs.csv"))?;
    pairs.write_all(b"case_id,cohort,document_cluster,left_outcome,right_outcome,cer_right_minus_left,collapsed_cer_right_minus_left,wer_right_minus_left\n")?;
    for (index, case) in left.cases.iter().enumerate() {
        let other = &right.cases[right_indices[index]];
        for value in [
            case.case_id.as_str(),
            case.cohort.as_str(),
            case.document_cluster.as_str(),
            outcome_name(&case.outcome),
            outcome_name(&other.outcome),
        ] {
            csv_field(&mut pairs, value)?;
            pairs.write_all(b",")?;
        }
        let differences = match (&case.metrics, &other.metrics) {
            (Some(a), Some(b)) => [
                difference(&a.cer, &b.cer),
                difference(&a.cer_whitespace_collapsed, &b.cer_whitespace_collapsed),
                a.wer
                    .as_ref()
                    .zip(b.wer.as_ref())
                    .and_then(|(a, b)| difference(a, b)),
            ],
            _ => [None; 3],
        };
        for (position, value) in differences.into_iter().enumerate() {
            if position != 0 {
                pairs.write_all(b",")?;
            }
            csv_option(&mut pairs, value)?;
        }
        pairs.write_all(b"\n")?;
    }
    pairs.sync_all()?;
    let mut json = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("comparison.json"))?;
    serde_json::to_writer_pretty(&mut json, comparison)?;
    json.write_all(b"\n")?;
    json.sync_all()?;
    Ok(())
}

fn outcome_name(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Success => "success",
        Outcome::ServiceFailure => "service_failure",
        Outcome::MetricLimit => "metric_limit",
        Outcome::MissingAttempt => "missing_attempt",
    }
}

fn difference(left: &Distance, right: &Distance) -> Option<f64> {
    left.rate().zip(right.rate()).map(|(a, b)| b - a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_scored_reference_is_bounded_and_rehashed_after_admission() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("indra-evaluate-reference-integrity-{nonce}"));
        fs::create_dir(&root).expect("fixture directory");
        let path = root.join("reference.txt");
        fs::write(&path, "é\n").expect("fixture reference");
        let mut asset = corpus::Asset {
            path: PathBuf::from("reference.txt"),
            bytes: 3,
            sha256: format!("{:x}", Sha256::digest("é\n".as_bytes())),
        };
        let mut text = String::new();
        read_reference(&root, &asset, &mut text).expect("exact frozen reference");
        assert_eq!(text, "é\n");
        fs::write(&path, "xe\n").expect("same-size mutation");
        assert!(matches!(
            read_reference(&root, &asset, &mut text),
            Err(ScoreError::ReferenceHash(_))
        ));
        fs::write(&path, [b'x'; 4096]).expect("expansion after admission");
        assert!(read_reference(&root, &asset, &mut text).is_err());
        assert_eq!(text.len(), 4);
        asset.bytes = 2 * 1024 * 1024 + 1;
        assert!(read_reference(&root, &asset, &mut text).is_err());
        fs::remove_dir_all(root).expect("remove owned fixture");
    }

    #[test]
    fn cjk_has_cer_but_no_fabricated_word_score() {
        let mut workspace = Workspace::default();
        let scores = measure(&mut workspace, "繁體中文", "繁体中文", false).unwrap();
        assert_eq!(scores.cer.errors, 1);
        assert!(scores.wer.is_none());
    }

    #[test]
    fn aggregate_distinguishes_micro_and_macro_and_keeps_insertions() {
        let mut aggregate = Aggregate::default();
        aggregate.add(&Distance {
            reference_units: 1,
            hypothesis_units: 4,
            errors: 3,
        });
        aggregate.add(&Distance {
            reference_units: 9,
            hypothesis_units: 9,
            errors: 0,
        });
        aggregate.finish();
        assert_eq!(aggregate.micro_rate, Some(0.3));
        assert_eq!(aggregate.macro_case_rate, Some(1.5));
        assert_eq!(aggregate.errors, 3);
    }

    #[test]
    fn hexadecimal_hash_check_rejects_truncation_uppercase_and_changed_bytes() {
        let digest: [u8; 32] = Sha256::digest(b"abc").into();
        assert!(matches_sha256(
            &digest,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        ));
        assert!(!matches_sha256(&digest, "ba7816"));
        assert!(!matches_sha256(
            &digest,
            "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
        ));
        assert!(!matches_sha256(
            &digest,
            "ca7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        ));
    }

    #[test]
    fn later_success_never_replaces_failed_first_accuracy_attempt() {
        let mut observed = Observations::default();
        let json = r#"{"variant":"v","cpus":1,"repetition":1,"sequence":0,"case_id":"c","document_cluster":"d","language":"deu","script":"Latn","domain":"legal","appearance":"print","reference_quality":"audited","cohort":"primary","task":"full_page","elapsed_us":9,"status":503,"error":"http_status","output_path":"c-r01.txt","output_sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","output_bytes":0}"#;
        let mut attempt: Attempt<'_> = serde_json::from_str(json).unwrap();
        observe(&mut observed, &attempt, [0; 32]).unwrap();
        attempt.repetition = 2;
        attempt.status = Some(200);
        attempt.error = None;
        observe(&mut observed, &attempt, [1; 32]).unwrap();
        attempt.repetition = 3;
        observe(&mut observed, &attempt, [2; 32]).unwrap();
        assert_eq!(observed.outcome, Outcome::ServiceFailure);
        assert_eq!(observed.first_http_status, Some(503));
        assert_eq!(observed.successful_attempts, 2);
        assert_eq!(observed.changed_successful_outputs, 1);
        assert!(observed.metrics.is_none());
    }
}
