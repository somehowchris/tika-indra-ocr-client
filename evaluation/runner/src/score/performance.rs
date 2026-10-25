//! Descriptive timing of every verified, non-warmup journal attempt.
//!
//! Repetitions are timing observations, never additional accuracy samples.
//! Failed-attempt termination time is not an inferred OCR completion time.

use super::{ScoreError, ScoreReport, csv_field, csv_option};
use crate::{
    corpus::Corpus,
    journal::{Attempt, RepetitionEnd},
};
use arrayvec::{ArrayString, ArrayVec};
use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::Path,
};

struct AttemptTiming {
    case_index: usize,
    repetition: u16,
    sequence: usize,
    elapsed_us: u128,
    status: Option<u16>,
    error: Option<ArrayString<64>>,
}

impl AttemptTiming {
    fn successful(&self) -> bool {
        self.status
            .is_some_and(|status| (200..300).contains(&status))
            && self.error.is_none()
    }
}

#[derive(Debug, Serialize)]
struct RepetitionTiming {
    repetition: u16,
    attempts: usize,
    successful_attempts: usize,
    failed_attempts: usize,
    sum_attempt_elapsed_us: u128,
    wall_us: u128,
}

#[derive(Debug, Serialize)]
struct Distribution {
    samples: usize,
    sum_elapsed_us: u128,
    min_us: Option<u128>,
    p50_us: Option<f64>,
    p95_us: Option<f64>,
    p99_us: Option<f64>,
    max_us: Option<u128>,
}

#[derive(Serialize)]
struct CohortTiming {
    cohort: &'static str,
    selected_cases: usize,
    expected_attempts: usize,
    observed_attempts: usize,
    successful_attempts: usize,
    failed_attempts: usize,
    missing_attempts: usize,
    successful_complete_body_latency: Distribution,
    failed_attempt_termination_time: Distribution,
}

#[derive(Serialize)]
struct PerformanceReport<'a> {
    schema_version: u32,
    evaluator_version: &'a str,
    corpus_id: &'a str,
    manifest_sha256: &'a str,
    manifest_bytes: u64,
    journal_sha256: &'a str,
    journal_bytes: u64,
    variant: &'a str,
    cpus: u32,
    journal_complete: bool,
    truncated_final_record_bytes: usize,
    warmup_attempts_excluded: usize,
    selected_cases: usize,
    expected_repetitions: u16,
    expected_attempts: usize,
    observed_attempts: usize,
    successful_attempts: usize,
    failed_attempts: usize,
    missing_attempts: usize,
    completed_repetition_wall_us: u128,
    successful_case_responses_per_complete_workload_wall_second: Option<f64>,
    latency_scope: &'static str,
    quantile_method: &'static str,
    throughput_scope: &'static str,
    validation_scope: &'static str,
    cohorts: ArrayVec<CohortTiming, 2>,
    completed_repetitions: &'a [RepetitionTiming],
}

pub(super) struct Timings {
    // Corpus admission bounds cases at 128 and repetitions at 20.
    attempts: ArrayVec<AttemptTiming, 2560>,
    repetitions: ArrayVec<RepetitionTiming, 20>,
}

impl Timings {
    pub(super) fn new() -> Self {
        Self {
            attempts: ArrayVec::new(),
            repetitions: ArrayVec::new(),
        }
    }

    pub(super) fn attempt(
        &mut self,
        case_index: usize,
        attempt: &Attempt<'_>,
    ) -> Result<(), ScoreError> {
        if case_index >= 128 || !(1..=20).contains(&attempt.repetition) {
            return Err(ScoreError::Invalid(
                "timing case/repetition exceeds its fixed bound",
            ));
        }
        let error = if let Some(error) = attempt.error {
            let mut label = ArrayString::new();
            label
                .try_push_str(error)
                .map_err(|_| ScoreError::Invalid("timing error label exceeds 64 bytes"))?;
            Some(label)
        } else {
            None
        };
        self.attempts
            .try_push(AttemptTiming {
                case_index,
                repetition: attempt.repetition,
                sequence: attempt.sequence,
                elapsed_us: attempt.elapsed_us,
                status: attempt.status,
                error,
            })
            .map_err(|_| ScoreError::Invalid("timing attempts exceed the 128×20 bound"))
    }

    pub(super) fn finish_repetition(&mut self, end: &RepetitionEnd) -> Result<(), ScoreError> {
        if !(1..=20).contains(&end.repetition)
            || self
                .repetitions
                .iter()
                .any(|row| row.repetition == end.repetition)
        {
            return Err(ScoreError::Invalid(
                "invalid or duplicate timing repetition end",
            ));
        }
        let mut attempts = 0;
        let mut successes = 0;
        let mut elapsed = 0_u128;
        for row in self
            .attempts
            .iter()
            .filter(|row| row.repetition == end.repetition)
        {
            attempts += 1;
            successes += usize::from(row.successful());
            elapsed = elapsed
                .checked_add(row.elapsed_us)
                .ok_or(ScoreError::Invalid("attempt elapsed-time sum overflows"))?;
        }
        if attempts == 0 || attempts != end.attempted_pages || end.wall_us < elapsed {
            return Err(ScoreError::Invalid(
                "repetition wall time/count disagrees with its serial attempts",
            ));
        }
        self.repetitions
            .try_push(RepetitionTiming {
                repetition: end.repetition,
                attempts,
                successful_attempts: successes,
                failed_attempts: attempts - successes,
                sum_attempt_elapsed_us: elapsed,
                wall_us: end.wall_us,
            })
            .map_err(|_| ScoreError::Invalid("timing repetitions exceed 20"))
    }

    fn distribution(
        &self,
        include: impl Fn(&AttemptTiming) -> bool,
    ) -> Result<Distribution, ScoreError> {
        let mut values = ArrayVec::<u128, 2560>::new();
        for row in &self.attempts {
            if include(row) {
                values.push(row.elapsed_us);
            }
        }
        describe(&mut values)
    }

    fn write_attempts(
        &self,
        output: &Path,
        corpus: &Corpus,
        report: &ScoreReport<&str>,
    ) -> Result<(), ScoreError> {
        let mut csv = BufWriter::with_capacity(16 * 1024, create(&output.join("attempts.csv"))?);
        csv.write_all(b"variant,cpus,case_id,original_page_id,document_cluster,cohort,language,script,domain,appearance,reference_quality,task,repetition,sequence,http_status,error,successful,elapsed_us\n")?;
        for row in &self.attempts {
            let case = &corpus.cases[row.case_index];
            csv_field(&mut csv, report.variant)?;
            write!(csv, ",{},", report.cpus)?;
            for value in [
                case.id.as_str(),
                case.original_page_id.as_str(),
                case.document_cluster.as_str(),
                case.cohort.as_str(),
                case.language.as_str(),
                case.script.as_str(),
                case.domain.as_str(),
                case.appearance.as_str(),
                case.reference_quality.as_str(),
                case.task.as_str(),
            ] {
                csv_field(&mut csv, value)?;
                csv.write_all(b",")?;
            }
            write!(csv, "{},{},", row.repetition, row.sequence)?;
            csv_option(&mut csv, row.status)?;
            csv.write_all(b",")?;
            csv_field(&mut csv, row.error.as_ref().map_or("", ArrayString::as_str))?;
            writeln!(csv, ",{},{}", row.successful(), row.elapsed_us)?;
        }
        csv.flush()?;
        csv.get_ref().sync_all()?;
        Ok(())
    }

    pub(super) fn write(
        &self,
        output: &Path,
        corpus: &Corpus,
        report: &ScoreReport<&str>,
    ) -> Result<(), ScoreError> {
        if self
            .attempts
            .iter()
            .any(|row| row.case_index >= corpus.cases.len())
            || report.cases.iter().map(|case| case.attempts).sum::<usize>() != self.attempts.len()
        {
            return Err(ScoreError::Invalid(
                "timing rows disagree with admitted case observations",
            ));
        }
        let expected_attempts = corpus.cases.len() * usize::from(report.expected_repetitions);
        let observed_attempts = self.attempts.len();
        let successes = self.attempts.iter().filter(|row| row.successful()).count();
        let missing =
            expected_attempts
                .checked_sub(observed_attempts)
                .ok_or(ScoreError::Invalid(
                    "timing attempts exceed the selected workload",
                ))?;
        let wall = self
            .repetitions
            .iter()
            .try_fold(0_u128, |sum, repetition| {
                sum.checked_add(repetition.wall_us)
                    .ok_or(ScoreError::Invalid("repetition wall-time sum overflows"))
            })?;
        let complete = report.journal_complete
            && missing == 0
            && self.repetitions.len() == usize::from(report.expected_repetitions);
        let throughput =
            (complete && wall > 0).then(|| successes as f64 * 1_000_000.0 / wall as f64);
        let mut cohorts = ArrayVec::new();
        for cohort in ["primary", "challenge"] {
            let selected_cases = corpus
                .cases
                .iter()
                .filter(|case| case.cohort == cohort)
                .count();
            if selected_cases == 0 {
                continue;
            }
            let successful = self.distribution(|row| {
                corpus.cases[row.case_index].cohort == cohort && row.successful()
            })?;
            let failed = self.distribution(|row| {
                corpus.cases[row.case_index].cohort == cohort && !row.successful()
            })?;
            let observed = successful.samples + failed.samples;
            let expected = selected_cases * usize::from(report.expected_repetitions);
            cohorts.push(CohortTiming {
                cohort,
                selected_cases,
                expected_attempts: expected,
                observed_attempts: observed,
                successful_attempts: successful.samples,
                failed_attempts: failed.samples,
                missing_attempts: expected.checked_sub(observed).ok_or(ScoreError::Invalid(
                    "cohort timing attempts exceed its selection",
                ))?,
                successful_complete_body_latency: successful,
                failed_attempt_termination_time: failed,
            });
        }
        let performance = PerformanceReport {
            schema_version: 1,
            evaluator_version: report.evaluator_version,
            corpus_id: report.corpus_id,
            manifest_sha256: &report.manifest_sha256,
            manifest_bytes: report.manifest_bytes,
            journal_sha256: &report.journal_sha256,
            journal_bytes: report.journal_bytes,
            variant: report.variant,
            cpus: report.cpus,
            journal_complete: report.journal_complete,
            truncated_final_record_bytes: report.truncated_final_record_bytes,
            warmup_attempts_excluded: report.warmup_attempts,
            selected_cases: corpus.cases.len(),
            expected_repetitions: report.expected_repetitions,
            expected_attempts,
            observed_attempts,
            successful_attempts: successes,
            failed_attempts: observed_attempts - successes,
            missing_attempts: missing,
            completed_repetition_wall_us: wall,
            successful_case_responses_per_complete_workload_wall_second: throughput,
            latency_scope: "All non-warmup repetitions. Successful samples require HTTP 2xx and no recorded error; their elapsed_us spans send through complete response body. Failed samples retain time until observed termination, not inferred OCR completion. Offline metric limits do not turn an HTTP success into a transport failure. Repetitions are correlated timing observations, never extra accuracy samples.",
            quantile_method: "Descriptive type 7 linear interpolation at rank (n-1)*p, in microseconds. Empty distributions have null quantiles. No confidence interval or production tail guarantee is inferred from these finite repeated observations.",
            throughput_scope: "Completed successful frozen case responses divided by the sum of completed repetition wall times, only for a complete journal with every expected attempt/repetition. Cases can be full pages or oracle crops, and each repetition contributes responses rather than independent pages. The denominator includes failed-attempt time and intervening validation, hashing, file and journal work. It covers the entire frozen sequence, including both primary and challenge cases when selected; no cohort-specific wall throughput is inferred.",
            validation_scope: "Attempt identities, raw response hashes and journal boundaries were checked by the untimed scorer. This report does not replace driver-result.json, immutable input/image identities or live resource attestations. Parent CPU and charged-memory scope remain in the source journal and driver artifacts.",
            cohorts,
            completed_repetitions: &self.repetitions,
        };
        self.write_attempts(output, corpus, report)?;
        let mut json =
            BufWriter::with_capacity(16 * 1024, create(&output.join("performance.json"))?);
        serde_json::to_writer_pretty(&mut json, &performance)?;
        json.write_all(b"\n")?;
        json.flush()?;
        json.get_ref().sync_all()?;
        Ok(())
    }
}

fn create(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

fn describe(values: &mut [u128]) -> Result<Distribution, ScoreError> {
    values.sort_unstable();
    let sum = values.iter().try_fold(0_u128, |sum, value| {
        sum.checked_add(*value)
            .ok_or(ScoreError::Invalid("latency sample sum overflows"))
    })?;
    Ok(Distribution {
        samples: values.len(),
        sum_elapsed_us: sum,
        min_us: values.first().copied(),
        p50_us: quantile(values, 50),
        p95_us: quantile(values, 95),
        p99_us: quantile(values, 99),
        max_us: values.last().copied(),
    })
}

fn quantile(sorted: &[u128], percentile: usize) -> Option<f64> {
    let rank = sorted.len().checked_sub(1)? * percentile;
    let low = rank / 100;
    let remainder = rank % 100;
    let high = (low + 1).min(sorted.len() - 1);
    Some(sorted[low] as f64 + (sorted[high] - sorted[low]) as f64 * remainder as f64 / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::Kind;

    fn row(elapsed_us: u128, status: Option<u16>, error: Option<&str>) -> Attempt<'_> {
        Attempt {
            kind: Kind::Attempt,
            variant: "test",
            cpus: 1,
            repetition: 1,
            sequence: 0,
            case_id: "test",
            document_cluster: "test",
            writer_cluster: None,
            language: "deu",
            script: "Latn",
            domain: "test",
            appearance: "print",
            reference_quality: "test",
            cohort: "primary",
            task: "full_page",
            tesseract_language: None,
            tesseract_psm: None,
            elapsed_us,
            status,
            error,
            output_path: Path::new("test-r01.txt"),
            output_sha256: "",
            output_bytes: 0,
        }
    }

    #[test]
    fn analytical_quantiles_are_descriptive_and_empty_values_stay_undefined() {
        let empty = describe(&mut []).expect("empty");
        assert_eq!(empty.samples, 0);
        assert_eq!(empty.sum_elapsed_us, 0);
        assert!(empty.min_us.is_none() && empty.p50_us.is_none() && empty.max_us.is_none());
        let one = describe(&mut [17]).expect("singleton");
        assert_eq!(one.p50_us, Some(17.0));
        assert_eq!(one.p95_us, Some(17.0));
        assert_eq!(one.p99_us, Some(17.0));
        let four = describe(&mut [30, 0, 20, 10]).expect("four known samples");
        assert_eq!(four.sum_elapsed_us, 60);
        assert_eq!(four.min_us, Some(0));
        assert_eq!(four.p50_us, Some(15.0));
        assert_eq!(four.p95_us, Some(28.5));
        assert!((four.p99_us.unwrap() - 29.7).abs() < 1e-12);
        assert_eq!(four.max_us, Some(30));
        assert!(describe(&mut [u128::MAX, 1]).is_err());
    }

    #[test]
    fn failures_never_enter_success_quantiles_and_serial_wall_bounds_are_checked() {
        let mut timings = Timings::new();
        timings
            .attempt(0, &row(10, Some(200), None))
            .expect("first success");
        timings
            .attempt(1, &row(120_000_000, None, Some("timeout")))
            .expect("timeout");
        timings
            .attempt(2, &row(30, Some(200), None))
            .expect("second success");
        timings
            .attempt(3, &row(8, Some(200), Some("response_utf8")))
            .expect("invalid UTF-8");
        let success = timings
            .distribution(AttemptTiming::successful)
            .expect("success distribution");
        assert_eq!(success.samples, 2);
        assert_eq!(success.p50_us, Some(20.0));
        let failed = timings
            .distribution(|row| !row.successful())
            .expect("failure distribution");
        assert_eq!(failed.samples, 2);
        assert_eq!(failed.max_us, Some(120_000_000));
        let mut end = RepetitionEnd {
            kind: Kind::RepetitionEnd,
            repetition: 1,
            attempted_pages: 4,
            wall_us: 120_000_047,
        };
        assert!(timings.finish_repetition(&end).is_err());
        assert!(timings.repetitions.is_empty());
        end.wall_us = 120_000_100;
        timings
            .finish_repetition(&end)
            .expect("serial wall includes all failures and gaps");
        assert_eq!(timings.repetitions[0].sum_attempt_elapsed_us, 120_000_048);
        assert_eq!(timings.repetitions[0].successful_attempts, 2);
        assert_eq!(timings.repetitions[0].failed_attempts, 2);
        assert!(timings.finish_repetition(&end).is_err());
        let mut warmup = row(7, Some(200), None);
        warmup.repetition = 0;
        assert!(timings.attempt(0, &warmup).is_err());
    }
}
