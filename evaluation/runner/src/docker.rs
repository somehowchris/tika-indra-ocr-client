//! One prespecified Docker arm, with an attested systemd/cgroup-v2 parent.
//!
//! This controller runs outside the measured parent. The load generator, gateway,
//! Java descendants and OCR workers all run inside it. No OCR request is retried.
//! External command arguments and bounded raw streams are retained for audit.
mod command;
mod runner;

use crate::corpus::{self, Corpus, Downloads};
use arrayvec::ArrayVec;
use command::{Completed, Control, create, read_text, write_json};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

static MEMORY_BYTES: u64 = 3 * 1024 * 1024 * 1024;
static MODEL_DIRECTORY: &str = "/models/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8";
static OWNER_LABEL: &str = "org.tika-indra.evaluation";
static PARSE_LIMIT: u64 = 2 * 1024 * 1024;
static COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Arm {
    Native,
    Python,
    Tesseract,
}

#[derive(Debug, Serialize)]
pub struct DockerOptions<'a> {
    pub arm: Arm,
    pub tika_image: &'a str,
    pub backend_image: Option<&'a str>,
    pub evaluator: &'a Path,
    pub tika_config: &'a Path,
    pub models: Option<&'a Path>,
    pub tesseract_models_lock: Option<&'a Path>,
    pub corpus: &'a Path,
    pub manifest: &'a Path,
    pub protocol: &'a Path,
    pub output: &'a Path,
    pub label: &'a str,
    pub cpus: u32,
    pub parent_slice: &'a str,
    pub parent_cgroup: &'a Path,
    pub docker_runner: bool,
    pub docker_parent_receipt: Option<&'a Path>,
    pub affinity: &'a str,
    pub repetitions: u16,
    pub threads: u8,
    pub batch_size: u8,
    pub math: bool,
    pub tesseract_psm: Option<u8>,
}

#[derive(Debug, Error)]
pub enum DockerError {
    #[error(transparent)]
    Corpus(#[from] corpus::CorpusError),
    #[error("Docker driver I/O failed")]
    Io(#[from] std::io::Error),
    #[error("Docker driver JSON failed")]
    Json(#[from] serde_json::Error),
    #[error("Docker driver admission failed: {0}")]
    Invalid(&'static str),
    #[error(
        "command {step} failed (exit={code:?}, timeout={timed_out}, output_limit={output_limit}); see retained command artifacts"
    )]
    Command {
        step: &'static str,
        code: Option<i32>,
        timed_out: bool,
        output_limit: bool,
    },
    #[error(
        "arm execution or cleanup failed; see driver-result.json and retained command artifacts"
    )]
    Incomplete,
}

#[derive(Deserialize)]
struct Protocol {
    schema_version: u32,
    status: String,
    corpus_sha256: String,
    tesseract_models_lock_sha256: String,
    tika_indra_config_sha256: String,
    tika_tesseract_config_sha256: String,
    budgets: ProtocolBudget,
    primary_variants: ArrayVec<Variant, 32>,
    execution: Execution,
}

#[derive(Deserialize)]
struct ProtocolBudget {
    cpus: ArrayVec<u32, 2>,
    aggregate_memory_bytes: u64,
    swap_bytes: u64,
}

#[derive(Deserialize)]
struct Execution {
    repetitions: u16,
    requests_per_repetition: usize,
    concurrency: usize,
    warmup_cases: usize,
    request_deadline_seconds: u64,
}

#[derive(Deserialize)]
struct Variant {
    id: String,
    engine: String,
    tessdata: Option<String>,
    psm: Option<u8>,
    openmp_threads: Option<u8>,
    math: Option<bool>,
    batch_size: Option<u8>,
    workers: Option<u8>,
    intra_op_threads: Option<String>,
    opencv_threads: Option<String>,
    intra_op_spinning: Option<bool>,
    inter_op_spinning: Option<bool>,
}

#[derive(Serialize, PartialEq, Eq)]
struct DigestRecord {
    path: PathBuf,
    bytes: u64,
    sha256: String,
}

#[derive(Serialize, PartialEq, Eq)]
struct ParentBudget {
    path: PathBuf,
    device: u64,
    inode: u64,
    cpu_max: String,
    memory_max: u64,
    memory_swap_max: u64,
    cpuset_effective: String,
    ancestors: ArrayVec<AncestorBudget, 32>,
}

#[derive(Serialize)]
struct ParentCounters {
    cpu_usage_usec: u64,
    cpu_stat: String,
    memory_current_bytes: u64,
    memory_peak_bytes: u64,
    memory_events: String,
    descendants: u64,
    dying_descendants: u64,
}

#[derive(Serialize, PartialEq, Eq)]
struct AncestorBudget {
    path: PathBuf,
    cpu_max: String,
    memory_max: String,
}

struct Paths {
    evaluator: PathBuf,
    config: PathBuf,
    corpus: PathBuf,
    manifest: PathBuf,
    models: PathBuf,
    output: PathBuf,
    requests: PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DockerInfo {
    cgroup_driver: String,
    cgroup_version: String,
    #[serde(rename = "OSType")]
    os_type: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Image {
    id: String,
    repo_digests: Option<ArrayVec<String, 32>>,
    os: String,
    architecture: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Container {
    id: String,
    image: String,
    state: ContainerState,
    config: ContainerConfig,
    host_config: HostConfig,
    mounts: ArrayVec<Mount, 8>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ContainerState {
    running: bool,
    pid: u32,
    exit_code: i32,
    #[serde(rename = "OOMKilled")]
    oom_killed: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ContainerConfig {
    user: String,
    labels: Labels,
}

#[derive(Deserialize)]
struct Labels {
    #[serde(rename = "org.tika-indra.evaluation")]
    owner: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct HostConfig {
    cgroup_parent: String,
    memory: u64,
    memory_swap: u64,
    nano_cpus: u64,
    cpuset_cpus: String,
    pids_limit: Option<u64>,
    readonly_rootfs: bool,
    privileged: bool,
    network_mode: String,
    cap_drop: ArrayVec<String, 8>,
    security_opt: ArrayVec<String, 8>,
    restart_policy: RestartPolicy,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RestartPolicy {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Mount {
    #[serde(rename = "Type")]
    kind: String,
    source: PathBuf,
    destination: PathBuf,
    #[serde(rename = "RW")]
    writable: bool,
}

struct OwnedContainer {
    name: String,
    image: String,
    id: Option<String>,
}

struct Session<'a> {
    options: &'a DockerOptions<'a>,
    paths: &'a Paths,
    control: Control,
    token: String,
    unit: String,
    unit_attempted: bool,
    tika: Option<OwnedContainer>,
    backend: Option<OwnedContainer>,
    runner: Option<OwnedContainer>,
    cleanup_attempted: bool,
    tika_stopped: bool,
}

fn identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn image_reference(value: &str) -> bool {
    if let Some(hash) = value.strip_prefix("sha256:") {
        return digest(hash);
    }
    value
        .split_once("@sha256:")
        .is_some_and(|(repository, hash)| {
            !repository.is_empty()
                && repository.len() <= 256
                && !repository.starts_with('-')
                && repository
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/._:-".contains(&b))
                && digest(hash)
        })
}

fn path_text(path: &Path) -> Result<&str, DockerError> {
    let text = path
        .to_str()
        .ok_or(DockerError::Invalid("driver paths must be UTF-8"))?;
    if text.is_empty()
        || text.len() > 4096
        || text.bytes().any(|b| b <= b' ' || b",\\\"':%$".contains(&b))
    {
        return Err(DockerError::Invalid(
            "driver paths contain unsupported Docker/systemd separators",
        ));
    }
    Ok(text)
}

fn canonical(path: &Path, directory: bool) -> Result<PathBuf, DockerError> {
    let path = fs::canonicalize(path)?;
    path_text(&path)?;
    let metadata = fs::metadata(&path)?;
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err(DockerError::Invalid("input path has the wrong file type"));
    }
    Ok(path)
}

fn cpu_set(text: &str) -> Result<[bool; 4096], DockerError> {
    let mut values = [false; 4096];
    for part in text.trim().split(',') {
        let (first, last) = part.split_once('-').unwrap_or((part, part));
        let first = first
            .parse::<usize>()
            .map_err(|_| DockerError::Invalid("invalid CPU affinity"))?;
        let last = last
            .parse::<usize>()
            .map_err(|_| DockerError::Invalid("invalid CPU affinity"))?;
        if first > last || last >= values.len() {
            return Err(DockerError::Invalid(
                "CPU affinity exceeds supported host bound",
            ));
        }
        for value in &mut values[first..=last] {
            if *value {
                return Err(DockerError::Invalid("duplicate CPU affinity member"));
            }
            *value = true;
        }
    }
    Ok(values)
}

fn numeric(path: &Path, name: &str) -> Result<u64, DockerError> {
    read_text(&path.join(name), 4096)?
        .trim()
        .parse()
        .map_err(|_| DockerError::Invalid("unbounded or invalid cgroup limit"))
}

fn counter(text: &str, name: &str) -> Result<u64, DockerError> {
    let mut value = None;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        if fields.next() != Some(name) {
            continue;
        }
        let parsed = fields.next().and_then(|field| field.parse().ok());
        if value.is_some() || parsed.is_none() || fields.next().is_some() {
            return Err(DockerError::Invalid("invalid or duplicate cgroup counter"));
        }
        value = parsed;
    }
    value.ok_or(DockerError::Invalid("required cgroup counter is missing"))
}

fn parent_counters(path: &Path) -> Result<ParentCounters, DockerError> {
    let groups = read_text(&path.join("cgroup.stat"), 4096)?;
    let cpu_stat = read_text(&path.join("cpu.stat"), 4096)?;
    Ok(ParentCounters {
        cpu_usage_usec: counter(&cpu_stat, "usage_usec")?,
        cpu_stat,
        memory_current_bytes: numeric(path, "memory.current")?,
        memory_peak_bytes: numeric(path, "memory.peak")?,
        memory_events: read_text(&path.join("memory.events"), 4096)?,
        descendants: counter(&groups, "nr_descendants")?,
        dying_descendants: counter(&groups, "nr_dying_descendants")?,
    })
}

fn require_unused_parent(path: &Path) -> Result<ParentCounters, DockerError> {
    let freshness = parent_counters(path)?;
    if [
        freshness.cpu_usage_usec,
        freshness.memory_current_bytes,
        freshness.memory_peak_bytes,
        freshness.descendants,
        freshness.dying_descendants,
    ]
    .into_iter()
    .any(|value| value != 0)
    {
        return Err(DockerError::Invalid(
            "parent has prior CPU/memory activity or child cgroups; create a fresh unique slice for this arm",
        ));
    }
    Ok(freshness)
}

fn ancestor_allows(cpu_max: &str, memory_max: &str, cpus: u32) -> Result<(), DockerError> {
    let mut fields = cpu_max.split_whitespace();
    let quota = fields
        .next()
        .ok_or(DockerError::Invalid("missing ancestor CPU quota"))?;
    let period = fields
        .next()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v != 0)
        .ok_or(DockerError::Invalid("invalid ancestor CPU period"))?;
    if fields.next().is_some()
        || (quota != "max"
            && quota
                .parse::<u64>()
                .ok()
                .is_none_or(|quota| u128::from(quota) < u128::from(period) * u128::from(cpus)))
    {
        return Err(DockerError::Invalid(
            "ancestor CPU cap is below the declared parent budget",
        ));
    }
    if memory_max.trim() != "max"
        && memory_max
            .trim()
            .parse::<u64>()
            .ok()
            .is_none_or(|value| value < MEMORY_BYTES)
    {
        return Err(DockerError::Invalid(
            "ancestor memory cap is below the declared parent budget",
        ));
    }
    Ok(())
}

fn parent_budget(
    options: &DockerOptions<'_>,
    require_empty: bool,
) -> Result<ParentBudget, DockerError> {
    let path = canonical(options.parent_cgroup, true)?;
    let identity = fs::metadata(&path)?;
    if path == Path::new("/sys/fs/cgroup") || !path.starts_with("/sys/fs/cgroup") {
        return Err(DockerError::Invalid(
            "parent must be a real descendant of /sys/fs/cgroup",
        ));
    }
    let mounts = read_text(Path::new("/proc/self/mountinfo"), PARSE_LIMIT)?;
    if !mounts.lines().any(|line| {
        line.split_whitespace().nth(4) == Some("/sys/fs/cgroup") && line.contains(" - cgroup2 ")
    }) {
        return Err(DockerError::Invalid(
            "requires the real unified cgroup-v2 mount",
        ));
    }
    let cpu_max = read_text(&path.join("cpu.max"), 4096)?;
    let mut fields = cpu_max.split_whitespace();
    let quota = fields
        .next()
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or(DockerError::Invalid("unbounded parent CPU quota"))?;
    let period = fields
        .next()
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or(DockerError::Invalid("invalid parent CPU period"))?;
    if period == 0
        || fields.next().is_some()
        || period.checked_mul(u64::from(options.cpus)) != Some(quota)
    {
        return Err(DockerError::Invalid(
            "parent CPU quota differs from declared budget",
        ));
    }
    let memory_max = numeric(&path, "memory.max")?;
    let memory_swap_max = numeric(&path, "memory.swap.max")?;
    if memory_max != MEMORY_BYTES || memory_swap_max != 0 {
        return Err(DockerError::Invalid(
            "prespecified parent requires exactly 3 GiB memory and zero swap",
        ));
    }
    let cpuset_effective = read_text(&path.join("cpuset.cpus.effective"), 4096)?;
    let actual = cpu_set(&cpuset_effective)?;
    if actual != cpu_set(options.affinity)?
        || actual.into_iter().filter(|v| *v).count() != options.cpus as usize
    {
        return Err(DockerError::Invalid(
            "effective parent affinity differs from declared one/two CPUs",
        ));
    }
    if require_empty
        && !read_text(&path.join("cgroup.events"), 4096)?
            .lines()
            .any(|line| line == "populated 0")
    {
        return Err(DockerError::Invalid(
            "benchmark parent already contains running processes",
        ));
    }
    let mut ancestors = ArrayVec::new();
    let mut ancestor = path.parent();
    while let Some(directory) =
        ancestor.filter(|directory| *directory != Path::new("/sys/fs/cgroup"))
    {
        let cpu_max = read_text(&directory.join("cpu.max"), 4096)?;
        let memory_max = read_text(&directory.join("memory.max"), 4096)?;
        ancestor_allows(&cpu_max, &memory_max, options.cpus)?;
        ancestors
            .try_push(AncestorBudget {
                path: PathBuf::from(directory),
                cpu_max,
                memory_max,
            })
            .map_err(|_| DockerError::Invalid("cgroup ancestry exceeds 32 levels"))?;
        ancestor = directory.parent();
    }
    Ok(ParentBudget {
        path,
        device: identity.dev(),
        inode: identity.ino(),
        cpu_max,
        memory_max,
        memory_swap_max,
        cpuset_effective,
        ancestors,
    })
}

fn file_digest(path: &Path, relative: &Path, limit: u64) -> Result<DigestRecord, DockerError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.len() > limit {
        return Err(DockerError::Invalid(
            "model/config file is not regular or exceeds its byte bound",
        ));
    }
    let mut input = File::open(path)?.take(limit + 1);
    let mut buffer = [0_u8; 65_536];
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > limit {
            return Err(DockerError::Invalid(
                "model/config file grew beyond its byte bound",
            ));
        }
        hash.update(&buffer[..count]);
    }
    if bytes != meta.len() {
        return Err(DockerError::Invalid(
            "model/config file changed size while hashing",
        ));
    }
    Ok(DigestRecord {
        path: PathBuf::from(relative),
        bytes,
        sha256: format!("{:x}", hash.finalize()),
    })
}

fn snapshot_file(
    source: &Path,
    destination: &Path,
    limit: u64,
) -> Result<DigestRecord, DockerError> {
    let mut input = File::open(source)?.take(limit + 1);
    let mut output = create(destination)?;
    let mut buffer = [0_u8; 65_536];
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > limit {
            return Err(DockerError::Invalid("snapshot exceeds its fixed bound"));
        }
        output.write_all(&buffer[..count])?;
        hash.update(&buffer[..count]);
    }
    output.sync_all()?;
    Ok(DigestRecord {
        path: PathBuf::from(
            destination
                .file_name()
                .ok_or(DockerError::Invalid("snapshot has no filename"))?,
        ),
        bytes,
        sha256: format!("{:x}", hash.finalize()),
    })
}

fn inventory(root: &Path) -> Result<ArrayVec<DigestRecord, 128>, DockerError> {
    fn visit(
        root: &Path,
        directory: &Path,
        depth: usize,
        out: &mut ArrayVec<DigestRecord, 128>,
        total: &mut u64,
        entries: &mut usize,
    ) -> Result<(), DockerError> {
        if depth > 2 {
            return Err(DockerError::Invalid(
                "model directory nesting exceeds two levels",
            ));
        }
        for entry in fs::read_dir(directory)? {
            *entries += 1;
            if *entries > 256 {
                return Err(DockerError::Invalid(
                    "model directory entry count exceeds 256",
                ));
            }
            let path = entry?.path();
            let kind = fs::symlink_metadata(&path)?;
            if kind.is_dir() {
                visit(root, &path, depth + 1, out, total, entries)?;
            } else {
                let record = file_digest(
                    &path,
                    path.strip_prefix(root)
                        .map_err(|_| DockerError::Invalid("model path escaped root"))?,
                    512 * 1024 * 1024,
                )?;
                *total += record.bytes;
                if *total > 2 * 1024 * 1024 * 1024 {
                    return Err(DockerError::Invalid(
                        "mounted model files exceed the 2 GiB inventory bound",
                    ));
                }
                out.try_push(record)
                    .map_err(|_| DockerError::Invalid("mounted model file count exceeds 128"))?;
            }
        }
        Ok(())
    }
    let mut files = ArrayVec::new();
    visit(root, root, 0, &mut files, &mut 0, &mut 0)?;
    files.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    if files.is_empty() {
        return Err(DockerError::Invalid("empty model directory"));
    }
    Ok(files)
}

fn verify_tesseract(
    corpus: &Corpus,
    flavor: &str,
    lock: &Downloads,
    files: &[DigestRecord],
) -> Result<(), DockerError> {
    if lock.schema_version != 1 || !["fast", "best"].contains(&flavor) {
        return Err(DockerError::Invalid(
            "unsupported Tesseract model lock/flavor",
        ));
    }
    let prefix = PathBuf::from(format!("tesseract/{flavor}"));
    let mut matched = 0;
    for download in &lock.assets {
        let Ok(relative) = download.asset.path.strip_prefix(&prefix) else {
            continue;
        };
        let file = files
            .iter()
            .find(|file| file.path == relative)
            .ok_or(DockerError::Invalid(
                "mounted Tesseract leaf omits a locked file",
            ))?;
        if file.bytes != download.asset.bytes || file.sha256 != download.asset.sha256 {
            return Err(DockerError::Invalid(
                "mounted Tesseract file differs from selected lock",
            ));
        }
        matched += 1;
    }
    if matched != files.len() {
        return Err(DockerError::Invalid(
            "Tesseract leaf includes an unpinned file or duplicate lock entry",
        ));
    }
    for case in &corpus.cases {
        for language in case.tesseract_language.split('+') {
            if !identifier(language, 32)
                || !files
                    .iter()
                    .any(|file| file.path == Path::new(&format!("{language}.traineddata")))
            {
                return Err(DockerError::Invalid(
                    "frozen Tesseract language is absent from selected model leaf",
                ));
            }
        }
    }
    if !files
        .iter()
        .any(|file| file.path == Path::new("configs/txt"))
    {
        return Err(DockerError::Invalid(
            "Tesseract text-output configuration is missing",
        ));
    }
    Ok(())
}

fn admit_protocol(
    options: &DockerOptions<'_>,
    protocol: &Protocol,
    corpus: &Corpus,
    manifest_hash: &str,
) -> Result<(), DockerError> {
    if protocol.schema_version != 1
        || protocol.status != "prespecified_before_new_tika_predictions"
        || protocol.corpus_sha256 != manifest_hash
        || !digest(&protocol.tika_indra_config_sha256)
        || !digest(&protocol.tika_tesseract_config_sha256)
        || protocol.budgets.cpus.as_slice() != [1, 2]
        || protocol.budgets.aggregate_memory_bytes != MEMORY_BYTES
        || protocol.budgets.swap_bytes != 0
        || protocol.execution.repetitions != options.repetitions
        || !(1..=20).contains(&options.repetitions)
        || protocol.execution.requests_per_repetition != corpus.cases.len()
        || protocol.execution.concurrency != 1
        || protocol.execution.warmup_cases != 1
        || protocol.execution.request_deadline_seconds != 120
    {
        return Err(DockerError::Invalid(
            "options differ from the frozen protocol/corpus execution contract",
        ));
    }
    if protocol
        .primary_variants
        .iter()
        .filter(|variant| variant.id == options.label)
        .count()
        != 1
    {
        return Err(DockerError::Invalid(
            "arm label must name exactly one prespecified variant",
        ));
    }
    let variant = protocol
        .primary_variants
        .iter()
        .find(|variant| variant.id == options.label)
        .ok_or(DockerError::Invalid("missing variant"))?;
    if options.arm == Arm::Tesseract {
        if variant.engine != "tesseract"
            || variant.psm != options.tesseract_psm
            || variant.openmp_threads != Some(1)
            || options.threads != 1
            || options.backend_image.is_some()
            || options.math
            || options.tesseract_models_lock.is_none()
            || !matches!(variant.tessdata.as_deref(), Some("fast" | "best"))
        {
            return Err(DockerError::Invalid(
                "Tesseract settings differ from the prespecified variant",
            ));
        }
    } else if variant.engine
        != if options.arm == Arm::Native {
            "rust"
        } else {
            "python"
        }
        || variant.math != Some(options.math)
        || variant.batch_size != Some(options.batch_size)
        || variant.workers != Some(1)
        || variant.intra_op_threads.as_deref() != Some("cpu_budget")
        || variant.opencv_threads.as_deref() != Some("cpu_budget")
        || variant.intra_op_spinning != Some(true)
        || variant.inter_op_spinning != Some(true)
        || u32::from(options.threads) != options.cpus
        || options.backend_image.is_none()
        || options.tesseract_psm.is_some()
        || options.tesseract_models_lock.is_some()
    {
        return Err(DockerError::Invalid(
            "Indra settings differ from the prespecified variant",
        ));
    }
    Ok(())
}

fn admit_tika_config(arm: &Arm, protocol: &Protocol, actual_hash: &str) -> Result<(), DockerError> {
    let expected = match arm {
        Arm::Native | Arm::Python => &protocol.tika_indra_config_sha256,
        Arm::Tesseract => &protocol.tika_tesseract_config_sha256,
    };
    if actual_hash != expected {
        return Err(DockerError::Invalid(
            "Tika XML snapshot differs from this arm's frozen protocol identity",
        ));
    }
    Ok(())
}

fn docker() -> Command {
    let mut command = Command::new("docker");
    command.arg("--host=unix:///var/run/docker.sock");
    for key in [
        "DOCKER_HOST",
        "DOCKER_CONTEXT",
        "DOCKER_TLS_VERIFY",
        "DOCKER_CERT_PATH",
    ] {
        command.env_remove(key);
    }
    command.env("LC_ALL", "C");
    command
}

fn systemctl() -> Command {
    let mut command = Command::new("systemctl");
    command
        .env("LC_ALL", "C")
        .env("SYSTEMD_PAGER", "cat")
        .env("SYSTEMD_COLORS", "0");
    command
}

fn bind(command: &mut Command, source: &Path, destination: &str) -> Result<(), DockerError> {
    command.args([
        "--mount",
        &format!(
            "type=bind,src={},dst={destination},readonly,bind-propagation=rprivate,bind-recursive=readonly",
            path_text(source)?
        ),
    ]);
    Ok(())
}

fn base_container(options: &DockerOptions<'_>, name: &str, token: &str) -> Command {
    let mut command = docker();
    command.args([
        "container",
        "create",
        "--name",
        name,
        "--label",
        &format!("{OWNER_LABEL}={token}"),
        "--pull=never",
        "--no-healthcheck",
        "--cgroup-parent",
        options.parent_slice,
        "--cpus",
        &options.cpus.to_string(),
        "--cpuset-cpus",
        options.affinity,
        "--memory",
        &MEMORY_BYTES.to_string(),
        "--memory-swap",
        &MEMORY_BYTES.to_string(),
        "--pids-limit",
        "128",
        "--read-only",
        "--cap-drop=ALL",
        "--security-opt=no-new-privileges",
        "--user",
        "65532:65532",
        "--restart=no",
        "--stop-signal=SIGTERM",
        "--stop-timeout=340",
        "--log-driver=local",
        "--log-opt=max-size=10m",
        "--log-opt=max-file=2",
        "--tmpfs",
        "/tmp:rw,nosuid,nodev,noexec,size=268435456,mode=1777",
    ]);
    command
}

impl Session<'_> {
    fn checked(
        &mut self,
        step: &'static str,
        command: &mut Command,
        timeout: Duration,
    ) -> Result<Completed, DockerError> {
        let completed = self.control.execute(step, command, timeout)?;
        completed.require_success(step)?;
        Ok(completed)
    }

    fn image(&mut self, reference: &str, step: &'static str) -> Result<String, DockerError> {
        let completed = self.checked(
            step,
            docker().args(["image", "inspect", "--format", "{{json .}}", reference]),
            COMMAND_TIMEOUT,
        )?;
        let image: Image = serde_json::from_str(&read_text(&completed.stdout, PARSE_LIMIT)?)?;
        if !image.id.strip_prefix("sha256:").is_some_and(digest)
            || image.os != "linux"
            || image.architecture != "amd64"
        {
            return Err(DockerError::Invalid(
                "image must identify an immutable Linux/amd64 local image",
            ));
        }
        if reference != image.id
            && !image
                .repo_digests
                .as_ref()
                .is_some_and(|values| values.iter().any(|value| value == reference))
        {
            return Err(DockerError::Invalid(
                "requested image digest does not match local image inspection",
            ));
        }
        Ok(image.id)
    }

    fn inspect(&mut self, reference: &str, step: &'static str) -> Result<Container, DockerError> {
        let completed = self.checked(
            step,
            docker().args(["container", "inspect", "--format", "{{json .}}", reference]),
            COMMAND_TIMEOUT,
        )?;
        Ok(serde_json::from_str(&read_text(
            &completed.stdout,
            PARSE_LIMIT,
        )?)?)
    }

    fn validate_container(
        &self,
        container: &Container,
        image: &str,
        backend: bool,
    ) -> Result<(), DockerError> {
        let host = &container.host_config;
        if !digest(&container.id)
            || container.image != image
            || container.config.labels.owner.as_deref() != Some(&self.token)
            || container.config.user != "65532:65532"
            || host.cgroup_parent != self.options.parent_slice
            || host.memory != MEMORY_BYTES
            || host.memory_swap != MEMORY_BYTES
            || host.nano_cpus != u64::from(self.options.cpus) * 1_000_000_000
            || cpu_set(&host.cpuset_cpus)? != cpu_set(self.options.affinity)?
            || host.pids_limit != Some(128)
            || !host.readonly_rootfs
            || host.privileged
            || host.restart_policy.name != "no"
            || host.cap_drop.as_slice() != ["ALL"]
            || !host
                .security_opt
                .iter()
                .any(|value| value == "no-new-privileges" || value == "no-new-privileges=true")
        {
            return Err(DockerError::Invalid(
                "created container differs from requested isolation/resource contract",
            ));
        }
        if backend {
            let tika = self
                .tika
                .as_ref()
                .and_then(|value| value.id.as_deref())
                .ok_or(DockerError::Invalid("backend started before Tika identity"))?;
            if host.network_mode != format!("container:{tika}") {
                return Err(DockerError::Invalid(
                    "OCR does not share the owned Tika network namespace",
                ));
            }
        } else if host.network_mode != "bridge" {
            return Err(DockerError::Invalid("unexpected Tika network mode"));
        }
        for mount in &container.mounts {
            if mount.kind == "bind" && mount.writable {
                return Err(DockerError::Invalid("writable input bind mount"));
            }
            if mount.kind != "bind"
                && !(mount.kind == "tmpfs" && mount.destination == Path::new("/tmp"))
            {
                return Err(DockerError::Invalid(
                    "unrequested container volume or writable mount",
                ));
            }
        }
        let model_destination = if self.options.arm == Arm::Tesseract {
            "/opt/tessdata"
        } else {
            MODEL_DIRECTORY
        };
        let expected = if backend {
            [
                Some((self.paths.models.as_path(), model_destination)),
                Some((self.paths.corpus.as_path(), "/corpus")),
                None,
            ]
        } else {
            [
                Some((self.paths.config.as_path(), "/etc/tika/tika-config.xml")),
                Some((self.paths.corpus.as_path(), "/corpus")),
                (self.options.arm == Arm::Tesseract)
                    .then_some((self.paths.models.as_path(), model_destination)),
            ]
        };
        if container
            .mounts
            .iter()
            .filter(|mount| mount.kind == "bind")
            .count()
            != expected.iter().flatten().count()
        {
            return Err(DockerError::Invalid("unexpected container bind mount"));
        }
        for (source, destination) in expected.into_iter().flatten() {
            if !container.mounts.iter().any(|mount| {
                mount.kind == "bind"
                    && mount.source == source
                    && mount.destination == Path::new(destination)
                    && !mount.writable
            }) {
                return Err(DockerError::Invalid("missing exact readonly input mount"));
            }
        }
        Ok(())
    }

    fn start_container(&mut self, backend: bool) -> Result<(), DockerError> {
        let owned = if backend {
            self.backend.as_ref()
        } else {
            self.tika.as_ref()
        }
        .ok_or(DockerError::Invalid("missing owned container"))?;
        let name = String::from(owned.name.as_str());
        let image = String::from(owned.image.as_str());
        let mut command = base_container(self.options, &name, &self.token);
        bind(&mut command, &self.paths.corpus, "/corpus")?;
        if backend {
            let tika_id = self
                .tika
                .as_ref()
                .and_then(|value| value.id.as_deref())
                .ok_or(DockerError::Invalid("missing Tika identity"))?;
            command.args(["--network", &format!("container:{tika_id}")]);
            bind(&mut command, &self.paths.models, MODEL_DIRECTORY)?;
            for key in [
                "OMP_NUM_THREADS",
                "OMP_THREAD_LIMIT",
                "OPENBLAS_NUM_THREADS",
                "MKL_NUM_THREADS",
                "NUMEXPR_NUM_THREADS",
            ] {
                command.args(["--env", &format!("{key}={}", self.options.threads)]);
            }
            command.args(["--env", "HF_HUB_OFFLINE=1"]);
            if self.options.arm == Arm::Native {
                command.args([
                    "--entrypoint",
                    "/indra-server",
                    &image,
                    "--model-dir",
                    MODEL_DIRECTORY,
                    "--listen",
                    "127.0.0.1:8081",
                    "--workers",
                    "1",
                    "--request-timeout-seconds",
                    "65",
                    "--threads",
                    &self.options.threads.to_string(),
                    "--cv-threads",
                    &self.options.threads.to_string(),
                    "--batch-size",
                    &self.options.batch_size.to_string(),
                    "--math",
                    if self.options.math { "true" } else { "false" },
                    "--intra-op-spinning",
                    "true",
                    "--inter-op-spinning",
                    "true",
                    "serve",
                ]);
            } else {
                for (key, value) in [
                    ("INDRA_HOST", "127.0.0.1"),
                    ("INDRA_PORT", "8081"),
                    ("INDRA_WORKERS", "1"),
                    ("INDRA_REQUEST_TIMEOUT_SECONDS", "65"),
                    ("INDRA_MODEL_DIR", MODEL_DIRECTORY),
                    ("INDRA_MATH", if self.options.math { "1" } else { "0" }),
                    ("INDRA_BATCH_METRICS", "0"),
                ] {
                    command.args(["--env", &format!("{key}={value}")]);
                }
                command.args([
                    "--env",
                    &format!("INDRA_THREADS={}", self.options.threads),
                    "--env",
                    &format!("INDRA_RECOGNITION_BATCH={}", self.options.batch_size),
                    "--entrypoint",
                    "/venv/bin/python",
                    &image,
                    "/app/backend/server.py",
                ]);
            }
        } else {
            command.args([
                "--network",
                "bridge",
                "--publish",
                "127.0.0.1::9998/tcp",
                "--entrypoint",
                "/indra-gateway",
            ]);
            bind(
                &mut command,
                &self.paths.config,
                "/etc/tika/tika-config.xml",
            )?;
            if self.options.arm == Arm::Tesseract {
                bind(&mut command, &self.paths.models, "/opt/tessdata")?;
                command.args([
                    "--env",
                    "TESSDATA_PREFIX=/opt/tessdata",
                    "--env",
                    "OMP_NUM_THREADS=1",
                    "--env",
                    "OMP_THREAD_LIMIT=1",
                ]);
            }
            command.arg(&image);
        }
        let result = self.checked(
            if backend { "ocr-create" } else { "tika-create" },
            &mut command,
            COMMAND_TIMEOUT,
        )?;
        let id = read_text(&result.stdout, 128)?;
        let id = id.trim();
        if !digest(id) {
            return Err(DockerError::Invalid(
                "Docker create returned an invalid container ID",
            ));
        }
        let inspected = self.inspect(
            id,
            if backend {
                "ocr-created-inspect"
            } else {
                "tika-created-inspect"
            },
        )?;
        self.validate_container(&inspected, &image, backend)?;
        if inspected.id != id {
            return Err(DockerError::Invalid(
                "Docker create/inspect identity differs",
            ));
        }
        let owned = if backend {
            self.backend.as_mut()
        } else {
            self.tika.as_mut()
        }
        .ok_or(DockerError::Invalid("missing container state"))?;
        owned.id = Some(String::from(id));
        self.checked(
            if backend { "ocr-start" } else { "tika-start" },
            docker().args(["container", "start", id]),
            COMMAND_TIMEOUT,
        )?;
        Ok(())
    }

    fn ready(&mut self, address: &str, step: &'static str) -> Result<u128, DockerError> {
        let tika = self
            .tika
            .as_ref()
            .and_then(|value| value.id.as_deref())
            .ok_or(DockerError::Invalid("readiness requires owned Tika"))?;
        let mut command = docker();
        command.args([
            "container",
            "exec",
            tika,
            "/indra-gateway",
            "--listen",
            address,
            "probe",
            "ready",
        ]);
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(180) {
            let completed = self
                .control
                .execute(step, &mut command, Duration::from_secs(10))?;
            if completed.output_limit {
                return Err(DockerError::Invalid(
                    "readiness probe exceeded its output bound",
                ));
            }
            if completed.success() {
                return Ok(started.elapsed().as_millis());
            }
            thread::sleep(Duration::from_secs(1));
        }
        Err(DockerError::Invalid(
            "readiness deadline exceeded; no cohort requests issued",
        ))
    }

    fn execute_arm(&mut self, maximum_seconds: u64) -> Result<(), DockerError> {
        let info = self.checked(
            "docker-info",
            docker().args(["info", "--format", "{{json .}}"]),
            COMMAND_TIMEOUT,
        )?;
        let info: DockerInfo = serde_json::from_str(&read_text(&info.stdout, PARSE_LIMIT)?)?;
        if info.cgroup_driver != "systemd" || info.cgroup_version != "2" || info.os_type != "linux"
        {
            return Err(DockerError::Invalid(
                "requires local Linux Docker with systemd cgroup-v2 driver",
            ));
        }
        let slice = self.checked(
            "slice-identity",
            systemctl().args([
                "show",
                self.options.parent_slice,
                "--property=ControlGroup",
                "--value",
            ]),
            COMMAND_TIMEOUT,
        )?;
        let group = read_text(&slice.stdout, 4096)?;
        if !group.trim().starts_with('/')
            || canonical(
                &Path::new("/sys/fs/cgroup").join(group.trim().trim_start_matches('/')),
                true,
            )? != canonical(self.options.parent_cgroup, true)?
        {
            return Err(DockerError::Invalid(
                "systemd slice does not resolve to the declared real parent",
            ));
        }
        let image = self.image(self.options.tika_image, "tika-image")?;
        self.tika = Some(OwnedContainer {
            name: format!("indra-tika-{}", self.token),
            image,
            id: None,
        });
        if let Some(reference) = self.options.backend_image {
            let image = self.image(reference, "ocr-image")?;
            self.backend = Some(OwnedContainer {
                name: format!("indra-ocr-{}", self.token),
                image,
                id: None,
            });
        }
        let start = Instant::now();
        self.start_container(false)?;
        if self.backend.is_some() {
            self.start_container(true)?;
        }
        let backend_ready_ms = if self.backend.is_some() {
            Some(self.ready("127.0.0.1:8081", "ocr-readiness")?)
        } else {
            None
        };
        let gateway_ready_ms = self.ready("127.0.0.1:9998", "tika-readiness")?;
        #[derive(Serialize)]
        struct Readiness {
            aggregate_start_to_ready_ms: u128,
            backend_poll_ms: Option<u128>,
            gateway_poll_ms: u128,
        }
        write_json(
            &self.paths.output.join("readiness.json"),
            &Readiness {
                aggregate_start_to_ready_ms: start.elapsed().as_millis(),
                backend_poll_ms: backend_ready_ms,
                gateway_poll_ms: gateway_ready_ms,
            },
        )?;
        let tika_name = String::from(
            self.tika
                .as_ref()
                .ok_or(DockerError::Invalid("missing Tika"))?
                .name
                .as_str(),
        );
        let tika = self.inspect(&tika_name, "tika-running-inspect")?;
        if !tika.state.running || tika.state.pid == 0 {
            return Err(DockerError::Invalid("Tika exited before measurement"));
        }
        let ocr_pid = if let Some(backend) = &self.backend {
            let name = String::from(backend.name.as_str());
            let value = self.inspect(&name, "ocr-running-inspect")?;
            if !value.state.running || value.state.pid == 0 {
                return Err(DockerError::Invalid("OCR exited before measurement"));
            }
            Some(value.state.pid)
        } else {
            None
        };
        let published = self.checked(
            "tika-port",
            docker().args(["container", "port", &tika.id, "9998/tcp"]),
            COMMAND_TIMEOUT,
        )?;
        let port = port(&read_text(&published.stdout, 128)?)?;
        let endpoint = format!("http://127.0.0.1:{port}/tika");
        fs::create_dir(&self.paths.requests)?;
        if self.options.docker_runner {
            return self.execute_docker_runner(&tika, ocr_pid, maximum_seconds);
        }
        let mut runner = Command::new("systemd-run");
        runner.env("LC_ALL", "C").args([
            "--wait",
            "--pipe",
            "--collect",
            "--quiet",
            "--expand-environment=no",
            "--unit",
            &self.unit,
            "--description",
            &format!("indra-evaluation-{}", self.token),
            "--slice",
            self.options.parent_slice,
            "--property",
            &format!("CPUQuota={}%", self.options.cpus * 100),
            "--property",
            &format!("AllowedCPUs={}", self.options.affinity),
            "--property=MemoryMax=1G",
            "--property=MemorySwapMax=0",
            "--property=TasksMax=64",
            "--property=ProtectSystem=strict",
            "--property",
            &format!(
                "ReadOnlyPaths={} {} {}",
                path_text(&self.paths.corpus)?,
                path_text(&self.paths.manifest)?,
                path_text(&self.paths.evaluator)?
            ),
            "--property",
            &format!("ReadWritePaths={}", path_text(&self.paths.requests)?),
            "--property=PrivateTmp=yes",
            "--property=NoNewPrivileges=yes",
            "--property=ProtectControlGroups=yes",
            "--property=KillMode=control-group",
            "--property=TimeoutStopSec=10s",
            "--property",
            &format!("RuntimeMaxSec={maximum_seconds}s"),
            "--",
        ]);
        runner
            .arg(&self.paths.evaluator)
            .arg("run")
            .arg("--manifest")
            .arg(&self.paths.manifest)
            .arg("--root")
            .arg(&self.paths.corpus)
            .arg("--output")
            .arg(&self.paths.requests)
            .args([
                "--endpoint",
                &endpoint,
                "--label",
                self.options.label,
                "--cgroup",
            ])
            .arg(self.options.parent_cgroup)
            .args([
                "--cpus",
                &self.options.cpus.to_string(),
                "--tika-pid",
                &tika.state.pid.to_string(),
                "--repetitions",
                &self.options.repetitions.to_string(),
            ]);
        if let Some(pid) = ocr_pid {
            runner.args(["--ocr-pid", &pid.to_string()]);
        }
        if let Some(psm) = self.options.tesseract_psm {
            runner.args(["--tesseract-psm", &psm.to_string()]);
        }
        self.unit_attempted = true;
        self.checked(
            "measured-run",
            &mut runner,
            Duration::from_secs(maximum_seconds + 30),
        )?;
        Ok(())
    }

    fn stop_unit(&mut self) -> Result<(), DockerError> {
        if self.options.docker_runner {
            return self.cleanup_docker_runner();
        }
        if !self.unit_attempted {
            return Ok(());
        }
        let mut command = systemctl();
        command.args([
            "show",
            &self.unit,
            "--property=LoadState",
            "--property=Description",
        ]);
        let result =
            self.control
                .execute("runner-cleanup-identity", &mut command, COMMAND_TIMEOUT)?;
        let text = read_text(&result.stdout, 4096)?;
        if text.lines().any(|line| line == "LoadState=not-found")
            && !result.timed_out
            && !result.output_limit
        {
            return Ok(());
        }
        result.require_success("runner-cleanup-identity")?;
        if !text
            .lines()
            .any(|line| line == format!("Description=indra-evaluation-{}", self.token))
        {
            return Err(DockerError::Invalid(
                "refusing to stop a systemd unit without this run's ownership marker",
            ));
        }
        self.checked(
            "runner-stop",
            systemctl().args(["stop", &self.unit]),
            COMMAND_TIMEOUT,
        )?;
        Ok(())
    }

    fn cleanup_container(&mut self, backend: bool) -> Result<(), DockerError> {
        let Some(owned) = (if backend { &self.backend } else { &self.tika }) else {
            return Ok(());
        };
        let name = String::from(owned.name.as_str());
        let list = self.checked(
            if backend {
                "ocr-cleanup-find"
            } else {
                "tika-cleanup-find"
            },
            docker().args([
                "container",
                "ls",
                "--all",
                "--filter",
                &format!("name=^/{name}$"),
                "--format",
                "{{.Names}}",
            ]),
            COMMAND_TIMEOUT,
        )?;
        let names = read_text(&list.stdout, 4096)?;
        if names.trim().is_empty() {
            if !backend {
                self.tika_stopped = true;
            }
            return Ok(());
        }
        if names.trim() != name {
            return Err(DockerError::Invalid(
                "container cleanup lookup is ambiguous",
            ));
        }
        let inspected = self.inspect(
            &name,
            if backend {
                "ocr-cleanup-inspect"
            } else {
                "tika-cleanup-inspect"
            },
        )?;
        if inspected.config.labels.owner.as_deref() != Some(&self.token) {
            return Err(DockerError::Invalid(
                "refusing to stop/remove a container without this run's ownership marker",
            ));
        }
        if inspected.state.running {
            self.checked(
                if backend {
                    "ocr-stop"
                } else {
                    "tika-drain-stop"
                },
                docker().args([
                    "container",
                    "stop",
                    "--timeout",
                    if backend { "100" } else { "340" },
                    &inspected.id,
                ]),
                Duration::from_secs(if backend { 125 } else { 365 }),
            )?;
        }
        let stopped = self.inspect(
            &inspected.id,
            if backend {
                "ocr-stopped-inspect"
            } else {
                "tika-stopped-inspect"
            },
        )?;
        if stopped.state.running {
            return Err(DockerError::Invalid(
                "owned container remains running after bounded stop",
            ));
        }
        if !backend {
            self.tika_stopped = true;
        }
        self.checked(
            if backend { "ocr-logs" } else { "tika-logs" },
            docker().args([
                "container",
                "logs",
                "--timestamps",
                "--details",
                &inspected.id,
            ]),
            COMMAND_TIMEOUT,
        )?;
        self.checked(
            if backend { "ocr-remove" } else { "tika-remove" },
            docker().args(["container", "rm", &inspected.id]),
            COMMAND_TIMEOUT,
        )?;
        if stopped.state.oom_killed || stopped.state.exit_code != 0 {
            return Err(DockerError::Invalid(
                "container terminated with an error/OOM; raw exit and logs retained",
            ));
        }
        Ok(())
    }

    fn cleanup(&mut self) -> ArrayVec<String, 4> {
        self.cleanup_attempted = true;
        self.control.begin_cleanup();
        let mut failures = ArrayVec::new();
        if let Err(error) = self.stop_unit() {
            failures.push(error.to_string());
        }
        // OCR remains available while Tika drains accepted requests and reaps Java.
        if let Err(error) = self.cleanup_container(false) {
            failures.push(error.to_string());
        }
        if self.tika_stopped || self.tika.is_none() {
            if let Err(error) = self.cleanup_container(true) {
                failures.push(error.to_string());
            }
        } else if self.backend.is_some() {
            failures.push(String::from("OCR retained because Tika shutdown was not confirmed; owned names are recorded in command artifacts"));
        }
        failures
    }
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        if !self.cleanup_attempted {
            for failure in self.cleanup() {
                eprintln!("evaluation emergency cleanup: {failure}");
            }
        }
    }
}

fn port(text: &str) -> Result<u16, DockerError> {
    let text = text.trim();
    if text.lines().count() != 1 {
        return Err(DockerError::Invalid(
            "Tika must publish exactly one loopback port",
        ));
    }
    text.strip_prefix("127.0.0.1:")
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|value| *value != 0)
        .ok_or(DockerError::Invalid(
            "Tika publication is not an IPv4-loopback port",
        ))
}

pub fn run_arm(options: &DockerOptions<'_>) -> Result<(), DockerError> {
    if ![1, 2].contains(&options.cpus)
        || !identifier(options.label, 80)
        || !image_reference(options.tika_image)
        || options
            .backend_image
            .is_some_and(|value| !image_reference(value))
        || !(1..=64).contains(&options.batch_size)
        || !options.parent_slice.ends_with(".slice")
        || !identifier(options.parent_slice.trim_end_matches(".slice"), 100)
    {
        return Err(DockerError::Invalid(
            "invalid arm label, immutable image, batch/CPU bound or slice name",
        ));
    }
    let parent = parent_budget(options, !options.docker_runner)?;
    let setup_identity = options
        .docker_parent_receipt
        .map(|path| file_digest(path, Path::new("docker-parent.json"), 65536))
        .transpose()?;
    let freshness = if options.docker_runner {
        runner::admit_docker_parent(options, &parent)?
    } else {
        if options.docker_parent_receipt.is_some() {
            return Err(DockerError::Invalid(
                "Docker setup receipt requires the Docker runner",
            ));
        }
        require_unused_parent(&parent.path)?
    };
    let (corpus, manifest_hash, manifest_bytes): (Corpus, _, _) =
        corpus::read_json_hashed(options.manifest)?;
    corpus.validate(options.corpus)?;
    let (protocol, protocol_hash, protocol_bytes): (Protocol, _, _) =
        corpus::read_json_hashed(options.protocol)?;
    admit_protocol(options, &protocol, &corpus, &manifest_hash)?;
    let models = canonical(
        options.models.ok_or(DockerError::Invalid(
            "every arm requires an explicit readonly model leaf",
        ))?,
        true,
    )?;
    let model_files = inventory(&models)?;
    let tesseract_lock = if let Some(path) = options.tesseract_models_lock {
        let (lock, hash, bytes): (Downloads, _, _) = corpus::read_json_hashed(path)?;
        if hash != protocol.tesseract_models_lock_sha256 {
            return Err(DockerError::Invalid(
                "Tesseract acquisition lock differs from frozen protocol",
            ));
        }
        let flavor = protocol
            .primary_variants
            .iter()
            .find(|variant| variant.id == options.label)
            .and_then(|variant| variant.tessdata.as_deref())
            .ok_or(DockerError::Invalid("missing Tesseract flavor"))?;
        verify_tesseract(&corpus, flavor, &lock, &model_files)?;
        Some((hash, bytes))
    } else {
        None
    };
    let evaluator = canonical(options.evaluator, false)?;
    let config = canonical(options.tika_config, false)?;
    let corpus_path = canonical(options.corpus, true)?;
    path_text(options.output)?;
    fs::create_dir(options.output)?;
    let output = canonical(options.output, true)?;
    let configuration = output.join("configuration");
    fs::create_dir(&configuration)?;
    fs::create_dir(output.join("commands"))?;
    let hardware = output.join("hardware");
    fs::create_dir(&hardware)?;
    snapshot_file(
        Path::new("/proc/cpuinfo"),
        &hardware.join("cpuinfo.txt"),
        8 * 1024 * 1024,
    )?;
    snapshot_file(
        Path::new("/proc/version"),
        &hardware.join("kernel.txt"),
        4096,
    )?;
    snapshot_file(
        Path::new("/sys/devices/system/cpu/online"),
        &hardware.join("online-cpus.txt"),
        4096,
    )?;
    write_json(&output.join("options.json"), options)?;
    write_json(&output.join("parent-before.json"), &parent)?;
    write_json(&output.join("parent-freshness.json"), &freshness)?;
    write_json(&output.join("models-before.json"), &model_files)?;
    let setup_record = options
        .docker_parent_receipt
        .map(|path| {
            let copied = snapshot_file(path, &configuration.join("docker-parent.json"), 65536)?;
            if setup_identity.as_ref().is_none_or(|original| {
                original.bytes != copied.bytes || original.sha256 != copied.sha256
            }) {
                return Err(DockerError::Invalid(
                    "Docker parent receipt changed during admission",
                ));
            }
            Ok(copied)
        })
        .transpose()?;
    let config_copy = configuration.join("tika-config.xml");
    let config_record = snapshot_file(&config, &config_copy, PARSE_LIMIT)?;
    admit_tika_config(&options.arm, &protocol, &config_record.sha256)?;
    let manifest_copy = configuration.join("corpus-frozen.json");
    let manifest_record = snapshot_file(options.manifest, &manifest_copy, PARSE_LIMIT)?;
    let protocol_record = snapshot_file(
        options.protocol,
        &configuration.join("protocol.json"),
        PARSE_LIMIT,
    )?;
    if manifest_record.sha256 != manifest_hash
        || manifest_record.bytes != manifest_bytes
        || protocol_record.sha256 != protocol_hash
        || protocol_record.bytes != protocol_bytes
    {
        return Err(DockerError::Invalid(
            "manifest/protocol changed while retaining exact snapshots",
        ));
    }
    let lock_record = if let Some(path) = options.tesseract_models_lock {
        let record = snapshot_file(
            path,
            &configuration.join("tesseract-models.json"),
            PARSE_LIMIT,
        )?;
        if tesseract_lock
            .as_ref()
            .is_none_or(|(hash, bytes)| record.sha256 != *hash || record.bytes != *bytes)
        {
            return Err(DockerError::Invalid(
                "Tesseract lock changed while snapshotting",
            ));
        }
        Some(record)
    } else {
        None
    };
    #[derive(Serialize)]
    struct Inputs {
        config: DigestRecord,
        manifest: DigestRecord,
        protocol: DigestRecord,
        tesseract_lock: Option<DigestRecord>,
        evaluator: DigestRecord,
        docker_parent: Option<DigestRecord>,
    }
    write_json(
        &output.join("input-identities.json"),
        &Inputs {
            config: config_record,
            manifest: manifest_record,
            protocol: protocol_record,
            tesseract_lock: lock_record,
            docker_parent: setup_record,
            evaluator: file_digest(&evaluator, Path::new("indra-evaluate"), 256 * 1024 * 1024)?,
        },
    )?;
    let paths = Paths {
        evaluator,
        config: config_copy,
        corpus: corpus_path,
        manifest: manifest_copy,
        models,
        requests: output.join("run"),
        output,
    };
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DockerError::Invalid("clock precedes Unix epoch"))?
        .as_nanos();
    let token = format!("{nonce:x}-{}", std::process::id());
    let mut session = Session {
        options,
        paths: &paths,
        control: Control::new(paths.output.join("commands")),
        unit: format!("indra-eval-{token}.service"),
        token,
        unit_attempted: false,
        tika: None,
        backend: None,
        runner: None,
        cleanup_attempted: false,
        tika_stopped: false,
    };
    // One warmup plus every serial attempt. The service and controlling command
    // have independent finite deadlines; neither is an OCR retry mechanism.
    let maximum_seconds =
        (corpus.cases.len() as u64 * u64::from(options.repetitions) + 1) * 130 + 60;
    let result = session.execute_arm(maximum_seconds);
    let cleanup = session.cleanup();
    let integrity = (|| -> Result<(), DockerError> {
        let after = parent_budget(options, !options.docker_runner)?;
        write_json(&paths.output.join("parent-after.json"), &after)?;
        if parent != after {
            return Err(DockerError::Invalid(
                "parent budget changed during arm execution",
            ));
        }
        write_json(
            &paths.output.join("parent-counters-after.json"),
            &parent_counters(&after.path)?,
        )?;
        let after = inventory(&paths.models)?;
        write_json(&paths.output.join("models-after.json"), &after)?;
        if model_files != after {
            return Err(DockerError::Invalid(
                "mounted model contents changed during arm execution",
            ));
        }
        Ok(())
    })();
    #[derive(Serialize)]
    struct Outcome {
        schema_version: u32,
        label: String,
        execution_error: Option<String>,
        cleanup_errors: ArrayVec<String, 4>,
        integrity_error: Option<String>,
        complete: bool,
    }
    let complete = result.is_ok() && cleanup.is_empty() && integrity.is_ok();
    write_json(
        &paths.output.join("driver-result.json"),
        &Outcome {
            schema_version: 1,
            label: String::from(options.label),
            execution_error: result.err().map(|error| error.to_string()),
            cleanup_errors: cleanup,
            integrity_error: integrity.err().map(|error| error.to_string()),
            complete,
        },
    )?;
    if complete {
        Ok(())
    } else {
        Err(DockerError::Incomplete)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> DockerOptions<'static> {
        DockerOptions {
            arm: Arm::Native,
            tika_image: "unused",
            backend_image: Some("unused"),
            evaluator: Path::new("/unused/evaluator"),
            tika_config: Path::new("/unused/tika.xml"),
            models: Some(Path::new("/unused/models")),
            tesseract_models_lock: None,
            corpus: Path::new("/unused/corpus"),
            manifest: Path::new("/unused/manifest"),
            protocol: Path::new("/unused/protocol"),
            output: Path::new("/unused/output"),
            label: "rust-math-b64",
            cpus: 2,
            parent_slice: "indra-eval.slice",
            parent_cgroup: Path::new("/sys/fs/cgroup/indra-eval.slice"),
            docker_runner: false,
            docker_parent_receipt: None,
            affinity: "0,1",
            repetitions: 3,
            threads: 2,
            batch_size: 64,
            math: true,
            tesseract_psm: None,
        }
    }

    #[test]
    fn image_references_require_exact_hashes_and_ports_require_loopback() {
        assert!(image_reference(&format!("sha256:{}", "a".repeat(64))));
        assert!(image_reference(&format!(
            "ghcr.io/owner/image@sha256:{}",
            "1".repeat(64)
        )));
        for value in [
            "image:latest",
            "image@sha256:abc",
            "--help",
            "sha256:ABC",
            "image@sha256:",
        ] {
            assert!(!image_reference(value));
        }
        assert_eq!(port("127.0.0.1:49153\n").expect("port"), 49153);
        for value in [
            "0.0.0.0:1234",
            "[::]:1234",
            "127.0.0.1:0",
            "127.0.0.1:1\n127.0.0.1:2",
        ] {
            assert!(port(value).is_err());
        }
    }

    #[test]
    fn cpu_sets_are_exact_and_mount_property_separators_fail_closed() {
        assert_eq!(
            cpu_set("2,3").expect("CPU list"),
            cpu_set("2-3\n").expect("range")
        );
        for value in ["", "max", "3-2", "1,1", "0-4096", "0 1", "0;1"] {
            assert!(cpu_set(value).is_err());
        }
        for value in ["/a,b", "/a b", "/a\nb", "/a$b", "/a%b", "/a:b"] {
            assert!(path_text(Path::new(value)).is_err());
        }
        assert_eq!(
            path_text(Path::new("/workspace/source-1/a.xml")).expect("path"),
            "/workspace/source-1/a.xml"
        );
    }

    #[test]
    fn ancestor_limits_cannot_hide_a_smaller_effective_budget() {
        assert!(ancestor_allows("max 100000", "max\n", 2).is_ok());
        assert!(ancestor_allows("200000 100000", "3221225472", 2).is_ok());
        assert!(ancestor_allows("100000 50000", "6442450944", 2).is_ok());
        for (cpu, memory) in [
            ("199999 100000", "max"),
            ("max 0", "max"),
            ("max 100000 extra", "max"),
            ("max 100000", "3221225471"),
        ] {
            assert!(ancestor_allows(cpu, memory, 2).is_err());
        }
    }

    #[test]
    fn unused_parent_rejects_prior_cpu_memory_and_lingering_child_groups() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("indra-parent-freshness-{nonce:x}"));
        fs::create_dir(&directory).expect("isolated counter fixture");
        for (file, text) in [
            ("cpu.stat", "usage_usec 0\nuser_usec 0\nsystem_usec 0\n"),
            ("memory.current", "0\n"),
            ("memory.peak", "0\n"),
            ("memory.events", "oom 0\noom_kill 0\n"),
            ("cgroup.stat", "nr_descendants 0\nnr_dying_descendants 0\n"),
        ] {
            fs::write(directory.join(file), text).expect("counter fixture");
        }
        assert!(require_unused_parent(&directory).is_ok());
        for (file, changed, restored) in [
            ("cpu.stat", "usage_usec 1\n", "usage_usec 0\n"),
            ("memory.current", "1\n", "0\n"),
            ("memory.peak", "1\n", "0\n"),
            (
                "cgroup.stat",
                "nr_descendants 1\nnr_dying_descendants 0\n",
                "nr_descendants 0\nnr_dying_descendants 0\n",
            ),
            (
                "cgroup.stat",
                "nr_descendants 0\nnr_dying_descendants 1\n",
                "nr_descendants 0\nnr_dying_descendants 0\n",
            ),
        ] {
            fs::write(directory.join(file), changed).expect("prior activity");
            assert!(require_unused_parent(&directory).is_err(), "{file}");
            fs::write(directory.join(file), restored).expect("restore empty fixture");
        }
        for malformed in ["", "usage_usec nope\n", "usage_usec 0\nusage_usec 0\n"] {
            assert!(counter(malformed, "usage_usec").is_err());
        }
        fs::remove_dir_all(directory).expect("remove isolated counter fixture");
    }

    fn frozen_protocol_fixture() {
        let mut protocol: Protocol =
            serde_json::from_str(include_str!("../../protocol.json")).expect("frozen protocol");
        let corpus: Corpus =
            serde_json::from_str(include_str!("../../corpus-frozen.json")).expect("frozen corpus");
        let lock: Downloads =
            serde_json::from_str(include_str!("../../acquisition/tesseract-models.json"))
                .expect("frozen Tesseract lock");
        let mut option = options();
        assert!(admit_protocol(&option, &protocol, &corpus, &protocol.corpus_sha256).is_ok());
        let previous = std::mem::replace(&mut protocol.tika_indra_config_sha256, "g".repeat(64));
        assert!(admit_protocol(&option, &protocol, &corpus, &protocol.corpus_sha256).is_err());
        protocol.tika_indra_config_sha256 = previous;
        let previous =
            std::mem::replace(&mut protocol.tika_tesseract_config_sha256, "a".repeat(63));
        assert!(admit_protocol(&option, &protocol, &corpus, &protocol.corpus_sha256).is_err());
        protocol.tika_tesseract_config_sha256 = previous;
        option.math = false;
        assert!(admit_protocol(&option, &protocol, &corpus, &protocol.corpus_sha256).is_err());
        option.math = true;
        option.batch_size = 8;
        assert!(admit_protocol(&option, &protocol, &corpus, &protocol.corpus_sha256).is_err());
        option.batch_size = 64;
        option.repetitions = 2;
        assert!(admit_protocol(&option, &protocol, &corpus, &protocol.corpus_sha256).is_err());
        option.repetitions = 3;
        assert!(admit_protocol(&option, &protocol, &corpus, "wrong manifest hash").is_err());
        option.label = "undeclared-rust-variant";
        assert!(admit_protocol(&option, &protocol, &corpus, &protocol.corpus_sha256).is_err());
        let mut files = ArrayVec::<DigestRecord, 128>::new();
        for download in &lock.assets {
            if let Ok(path) = download.asset.path.strip_prefix("tesseract/fast") {
                files.push(DigestRecord {
                    path: PathBuf::from(path),
                    bytes: download.asset.bytes,
                    sha256: String::from(download.asset.sha256.as_str()),
                });
            }
        }
        assert!(verify_tesseract(&corpus, "fast", &lock, &files).is_ok());
        assert!(verify_tesseract(&corpus, "best", &lock, &files).is_err());
        files[0].bytes += 1;
        assert!(verify_tesseract(&corpus, "fast", &lock, &files).is_err());
    }

    #[test]
    fn frozen_protocol_rejects_unregistered_settings_and_wrong_model_flavor() {
        // The real CLI uses the ordinary Linux 8 MiB main stack. Serde's debug
        // temporaries for fixed-capacity corpus arrays exceed the test harness's
        // default 2 MiB stack; keep this full-frozen fixture on the CLI's bound.
        thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(frozen_protocol_fixture)
            .expect("fixture thread")
            .join()
            .expect("protocol fixture");
    }

    #[test]
    fn frozen_tika_xml_snapshots_bind_each_engine_and_reject_byte_changes() {
        let protocol: Protocol =
            serde_json::from_str(include_str!("../../protocol.json")).expect("frozen protocol");
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let output = std::env::temp_dir().join(format!("indra-tika-xml-{nonce:x}"));
        fs::create_dir(&output).expect("isolated snapshots");
        let config = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/evaluation");
        let indra_copy = output.join("indra.xml");
        let indra = snapshot_file(&config.join("tika-indra.xml"), &indra_copy, PARSE_LIMIT)
            .expect("Indra XML snapshot");
        let tesseract = snapshot_file(
            &config.join("tika-tesseract.xml"),
            &output.join("tesseract.xml"),
            PARSE_LIMIT,
        )
        .expect("Tesseract XML snapshot");
        assert!(admit_tika_config(&Arm::Native, &protocol, &indra.sha256).is_ok());
        assert!(admit_tika_config(&Arm::Python, &protocol, &indra.sha256).is_ok());
        assert!(admit_tika_config(&Arm::Tesseract, &protocol, &tesseract.sha256).is_ok());
        if indra.sha256 != tesseract.sha256 {
            assert!(admit_tika_config(&Arm::Tesseract, &protocol, &indra.sha256).is_err());
            assert!(admit_tika_config(&Arm::Native, &protocol, &tesseract.sha256).is_err());
            assert!(admit_tika_config(&Arm::Python, &protocol, &tesseract.sha256).is_err());
        }
        File::options()
            .append(true)
            .open(&indra_copy)
            .expect("isolated XML")
            .write_all(b"\n")
            .expect("one byte change");
        let changed = snapshot_file(&indra_copy, &output.join("changed.xml"), PARSE_LIMIT)
            .expect("changed XML snapshot");
        assert!(admit_tika_config(&Arm::Native, &protocol, &changed.sha256).is_err());
        assert!(admit_tika_config(&Arm::Python, &protocol, &changed.sha256).is_err());
        fs::remove_dir_all(output).expect("remove isolated XML snapshots");
    }

    #[test]
    fn container_arguments_enforce_shared_parent_and_never_pull_or_restart() {
        let option = options();
        let command = base_container(&option, "test-container", "test-owner");
        let mut arguments = ArrayVec::<&str, 128>::new();
        for value in command.get_args() {
            arguments.push(value.to_str().expect("UTF-8 argument"));
        }
        for (flag, value) in [
            ("--cgroup-parent", "indra-eval.slice"),
            ("--cpus", "2"),
            ("--cpuset-cpus", "0,1"),
            ("--memory", "3221225472"),
            ("--memory-swap", "3221225472"),
            ("--user", "65532:65532"),
            ("--pids-limit", "128"),
        ] {
            assert!(arguments.windows(2).any(|pair| pair == [flag, value]));
        }
        for flag in [
            "--pull=never",
            "--no-healthcheck",
            "--restart=no",
            "--read-only",
            "--security-opt=no-new-privileges",
            "--cap-drop=ALL",
        ] {
            assert!(arguments.contains(&flag));
        }
        assert_eq!(arguments[0], "--host=unix:///var/run/docker.sock");
        assert!(
            !arguments
                .iter()
                .any(|arg| arg.contains("JAVA_TOOL_OPTIONS"))
        );
    }
}
