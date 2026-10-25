//! The `requests.jsonl` record schema. `run` writes these records and `score`
//! reads the same types back, so a renamed field or kind changes both sides.
//! Field order is the serialized order and part of the sealed journal format.

use arrayvec::ArrayVec;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Environment,
    Warmup,
    Attempt,
    RepetitionEnd,
    ResourceEnd,
}

/// The kind alone, read first to select the record type.
#[derive(Deserialize)]
pub(crate) struct Tag {
    pub kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PeerKind {
    Tika,
    Ocr,
}

/// `S` is `&str` when written and `String` when the scorer keeps it.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Environment<S> {
    pub kind: Kind,
    pub schema_version: u32,
    pub corpus_id: S,
    pub manifest_sha256: S,
    pub manifest_bytes: u64,
    pub variant: S,
    pub selection_seed: u64,
    pub order_seed: u64,
    pub repetitions: u16,
    pub budget: Budget,
    pub scope: S,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Budget {
    pub cpus: u32,
    pub quota_us: u64,
    pub period_us: u64,
    pub memory_limit_bytes: u64,
    pub parent_path: PathBuf,
    pub runner_cgroup: String,
    pub cpuset_effective: String,
    pub cpu_stat: String,
    pub memory_current_bytes: u64,
    pub memory_peak_bytes: Option<u64>,
    pub memory_events: String,
    pub peers: ArrayVec<Peer, 2>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Peer {
    pub kind: PeerKind,
    pub pid: u32,
    pub process_start_ticks: u64,
    pub cgroup_path: PathBuf,
    pub cpu_max: String,
    pub memory_max: String,
    pub cpu_stat: String,
    pub cpuset_effective: String,
    pub memory_current_bytes: u64,
    pub memory_peak_bytes: Option<u64>,
    pub memory_events: String,
}

/// One warmup or measured request.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Attempt<'a> {
    pub kind: Kind,
    pub variant: &'a str,
    pub cpus: u32,
    pub repetition: u16,
    pub sequence: usize,
    pub case_id: &'a str,
    pub document_cluster: &'a str,
    pub writer_cluster: Option<&'a str>,
    pub language: &'a str,
    pub script: &'a str,
    pub domain: &'a str,
    pub appearance: &'a str,
    pub reference_quality: &'a str,
    pub cohort: &'a str,
    pub task: &'a str,
    pub tesseract_language: Option<&'a str>,
    pub tesseract_psm: Option<u8>,
    pub elapsed_us: u128,
    pub status: Option<u16>,
    pub error: Option<&'a str>,
    pub output_path: &'a Path,
    pub output_bytes: u64,
    pub output_sha256: &'a str,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct RepetitionEnd {
    pub kind: Kind,
    pub repetition: u16,
    pub attempted_pages: usize,
    pub wall_us: u128,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct ResourceEnd {
    pub kind: Kind,
    pub budget: Budget,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::record;
    use std::fs::{self, File};

    /// Verbatim records of two sealed journals; the README beside them names
    /// the source journals and their SHA-256s.
    static SEALED: &str =
        include_str!("../tests/evidence/journal-39810108196/requests-excerpt.jsonl");

    #[test]
    fn sealed_journal_records_are_written_back_byte_for_byte() {
        let path =
            std::env::temp_dir().join(format!("journal-golden-{}.jsonl", std::process::id()));
        let mut out = File::create_new(&path).unwrap();
        let mut kinds: ArrayVec<Kind, 5> = ArrayVec::new();
        for line in SEALED.lines() {
            let tag: Tag = serde_json::from_str(line).unwrap();
            match tag.kind {
                Kind::Environment => record(
                    &mut out,
                    &serde_json::from_str::<Environment<String>>(line).unwrap(),
                ),
                Kind::Warmup | Kind::Attempt => record(
                    &mut out,
                    &serde_json::from_str::<Attempt<'_>>(line).unwrap(),
                ),
                Kind::RepetitionEnd => record(
                    &mut out,
                    &serde_json::from_str::<RepetitionEnd>(line).unwrap(),
                ),
                Kind::ResourceEnd => record(
                    &mut out,
                    &serde_json::from_str::<ResourceEnd>(line).unwrap(),
                ),
            }
            .unwrap();
            if !kinds.contains(&tag.kind) {
                kinds.push(tag.kind);
            }
        }
        drop(out);
        assert!(kinds.is_full(), "every record kind is covered: {kinds:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), SEALED);
        fs::remove_file(path).unwrap();
    }
}
