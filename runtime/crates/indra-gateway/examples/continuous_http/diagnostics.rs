//! Post-shutdown native log admission by event timestamp, never async write offset.
//! This prevents buffered warmup logs from entering the timed model counters.
use super::{Failure, check};
use serde::{Serialize, Serializer};
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

#[derive(Serialize)]
pub(super) struct NativeDiagnostics {
    scope: &'static str,
    measurement_begin_unix_ns: u128,
    measurement_end_unix_ns: u128,
    native_completions: u64,
    valid_columns: u64,
    padded_columns: u64,
    shared_invocations: u64,
    #[serde(serialize_with = "histogram")]
    continuous_row_histogram: [u64; 64],
    continuous_invocations: u64,
    continuous_invocation_padded_columns: u64,
    ignored_non_json_lines: u64,
}
fn histogram<S: Serializer>(values: &[u64; 64], serializer: S) -> Result<S::Ok, S::Error> {
    values.as_slice().serialize(serializer)
}
pub(super) fn read(
    path: &Path,
    start: u128,
    end: u128,
    continuous: bool,
) -> Result<NativeDiagnostics, Failure> {
    check(end >= start, "system clock regressed during measurement")?;
    let file = File::open(path)?;
    check(
        file.metadata()?.len() <= 16 * 1024 * 1024,
        "native diagnostic log exceeds16MiB cap",
    )?;
    let mut result = NativeDiagnostics {
        scope: if continuous {
            "Native INFO events with creation timestamps inside complete HTTP workload; read after telemetry shutdown flush; all warmup events excluded"
        } else {
            "Ordinary native completion columns only; shared invocation count is zero by architecture; no continuous invocation histogram is measured"
        },
        measurement_begin_unix_ns: start,
        measurement_end_unix_ns: end,
        native_completions: 0,
        valid_columns: 0,
        padded_columns: 0,
        shared_invocations: 0,
        continuous_row_histogram: [0; 64],
        continuous_invocations: 0,
        continuous_invocation_padded_columns: 0,
        ignored_non_json_lines: 0,
    };
    for line in BufReader::new(file).lines() {
        let line = line?;
        check(line.len() <= 65536, "native diagnostic record exceeds64KiB")?;
        let Ok(row) = serde_json::from_str::<serde_json::Value>(&line) else {
            result.ignored_non_json_lines += 1;
            continue;
        };
        let fields = row.get("fields").unwrap_or(&row);
        let event = fields["event"].as_str().unwrap_or("");
        check(
            event != "log_queue_overflow",
            "native logging dropped records; diagnostic sample inadmissible",
        )?;
        if event != "inference_finished" && event != "continuous_invocation" {
            continue;
        }
        let timestamp = row["timestamp"]
            .as_str()
            .ok_or(Failure::Gate("native event creation timestamp missing"))?;
        let ns = OffsetDateTime::parse(timestamp, &Rfc3339)
            .map_err(|_| Failure::Gate("invalid native event timestamp"))?
            .unix_timestamp_nanos();
        check(ns >= 0, "native timestamp precedes epoch")?;
        let ns = ns as u128;
        if ns < start || ns > end {
            continue;
        }
        if event == "inference_finished" {
            result.native_completions += 1;
            result.valid_columns += number(fields, "valid_columns")?;
            result.padded_columns += number(fields, "padded_columns")?;
        } else {
            check(
                continuous,
                "ordinary arm unexpectedly emitted a shared scheduler event",
            )?;
            let first = number(fields, "owner_zero_rows")?;
            let second = number(fields, "owner_one_rows")?;
            let rows = first + second;
            check(
                (1..=64).contains(&rows),
                "native invocation row cap differs",
            )?;
            let shared = first > 0 && second > 0;
            check(
                fields["shared"].as_bool() == Some(shared),
                "native shared-row ownership contradicts flag",
            )?;
            result.shared_invocations += u64::from(shared);
            result.continuous_invocations += 1;
            result.continuous_row_histogram[rows as usize - 1] += 1;
            result.continuous_invocation_padded_columns += rows * number(fields, "padded_width")?;
        }
    }
    check(
        result.native_completions > 0,
        "no timed native completions retained; diagnostics inadmissible",
    )?;
    Ok(result)
}
fn number(row: &serde_json::Value, key: &str) -> Result<u64, Failure> {
    row[key]
        .as_u64()
        .ok_or(Failure::Gate("native diagnostic numeric field missing"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        io::Write,
        sync::atomic::{AtomicU64, Ordering},
    };
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new(bytes: &[u8]) -> Self {
            let path = std::env::temp_dir().join(format!(
                "indra-native-log-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
                .expect("new owned log fixture");
            file.write_all(bytes).expect("write fixture");
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    const FINISHED: &str = "{\"timestamp\":\"1970-01-01T00:00:05Z\",\"fields\":{\"event\":\"inference_finished\",\"valid_columns\":20,\"padded_columns\":24}}\n";
    #[test]
    fn creation_time_excludes_delayed_warmup_records() {
        let log=Fixture::new(format!("{{\"timestamp\":\"1970-01-01T00:00:01Z\",\"fields\":{{\"event\":\"inference_finished\",\"valid_columns\":900,\"padded_columns\":900}}}}\n{FINISHED}").as_bytes());
        let result = read(&log.0, 3_000_000_000, 7_000_000_000, false).expect("timed window");
        assert_eq!(result.native_completions, 1);
        assert_eq!(result.valid_columns, 20);
        assert_eq!(result.padded_columns, 24);
    }
    #[test]
    fn lossy_log_sample_is_not_accepted() {
        let log = Fixture::new(
            format!("{FINISHED}{{\"event\":\"log_queue_overflow\",\"dropped_lines\":1}}\n")
                .as_bytes(),
        );
        assert!(read(&log.0, 3_000_000_000, 7_000_000_000, false).is_err());
    }
    #[test]
    fn exact_owner_rows_must_agree_with_shared_flag() {
        let log=Fixture::new(format!("{FINISHED}{{\"timestamp\":\"1970-01-01T00:00:05Z\",\"fields\":{{\"event\":\"continuous_invocation\",\"owner_zero_rows\":1,\"owner_one_rows\":1,\"shared\":false,\"padded_width\":64}}}}\n").as_bytes());
        assert!(read(&log.0, 3_000_000_000, 7_000_000_000, true).is_err());
    }
    #[test]
    fn shared_invocation_histogram_counts_actual_rows() {
        let log=Fixture::new(format!("{FINISHED}{{\"timestamp\":\"1970-01-01T00:00:05Z\",\"fields\":{{\"event\":\"continuous_invocation\",\"owner_zero_rows\":2,\"owner_one_rows\":3,\"shared\":true,\"padded_width\":64}}}}\n").as_bytes());
        let result = read(&log.0, 3_000_000_000, 7_000_000_000, true).expect("shared actual rows");
        assert_eq!(result.shared_invocations, 1);
        assert_eq!(result.continuous_row_histogram[4], 1);
        assert_eq!(result.continuous_invocation_padded_columns, 320);
    }
    #[test]
    fn ordinary_arm_cannot_claim_a_continuous_invocation() {
        let log=Fixture::new(format!("{FINISHED}{{\"timestamp\":\"1970-01-01T00:00:05Z\",\"fields\":{{\"event\":\"continuous_invocation\",\"owner_zero_rows\":1,\"owner_one_rows\":1,\"shared\":true,\"padded_width\":64}}}}\n").as_bytes());
        assert!(read(&log.0, 3_000_000_000, 7_000_000_000, false).is_err());
    }
}
