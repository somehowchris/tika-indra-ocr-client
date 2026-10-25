use crate::{
    corpus::{self, Corpus},
    journal::{Attempt, Budget, Environment, Kind, Peer, PeerKind, RepetitionEnd, ResourceEnd},
};
use arrayvec::ArrayVec;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use thiserror::Error;

pub struct RunOptions<'a> {
    pub manifest: &'a Path,
    pub root: &'a Path,
    pub output: &'a Path,
    pub endpoint: &'a str,
    pub label: &'a str,
    pub cgroup: &'a Path,
    pub cpus: u32,
    pub tika_pid: u32,
    pub ocr_pid: Option<u32>,
    pub peer_proc_root: &'a Path,
    pub repetitions: u16,
    pub tesseract_psm: Option<u8>,
}
#[derive(Debug, Error)]
pub enum RunError {
    #[error(transparent)]
    Corpus(#[from] corpus::CorpusError),
    #[error("benchmark I/O failed")]
    Io(#[from] std::io::Error),
    #[error("benchmark HTTP failed")]
    Http(#[from] reqwest::Error),
    #[error("benchmark JSON failed")]
    Json(#[from] serde_json::Error),
    #[error("benchmark admission failed: {0}")]
    Invalid(&'static str),
}

fn cpu_set_size(text: &str) -> Result<usize, RunError> {
    let mut present = [false; 4096];
    for part in text.trim().split(',') {
        let (first, last) = part.split_once('-').unwrap_or((part, part));
        let first = first
            .parse::<usize>()
            .map_err(|_| RunError::Invalid("invalid effective CPU list"))?;
        let last = last
            .parse::<usize>()
            .map_err(|_| RunError::Invalid("invalid effective CPU list"))?;
        if first > last || last >= present.len() {
            return Err(RunError::Invalid(
                "effective CPU range exceeds the supported host bound",
            ));
        }
        for member in &mut present[first..=last] {
            if *member {
                return Err(RunError::Invalid("duplicate CPU in effective list"));
            }
            *member = true;
        }
    }
    Ok(present.into_iter().filter(|value| *value).count())
}

fn numeric_file(path: &Path, name: &str) -> Result<u64, RunError> {
    fs::read_to_string(path.join(name))?
        .trim()
        .parse()
        .map_err(|_| RunError::Invalid("invalid numeric cgroup counter"))
}

fn peak(path: &Path) -> Result<Option<u64>, RunError> {
    match numeric_file(path, "memory.peak") {
        Ok(value) => Ok(Some(value)),
        Err(RunError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn process_cgroup(text: &str) -> Result<PathBuf, RunError> {
    let relative = text
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .ok_or(RunError::Invalid("requires cgroup v2 unified hierarchy"))?;
    Ok(fs::canonicalize(
        Path::new("/sys/fs/cgroup").join(relative.trim_start_matches('/')),
    )?)
}

fn peer(
    kind: PeerKind,
    pid: u32,
    parent: &Path,
    affinity: &str,
    proc_root: &Path,
) -> Result<Peer, RunError> {
    if pid == 0 || pid == std::process::id() {
        return Err(RunError::Invalid(
            "Tika/OCR must identify a live separate process",
        ));
    }
    let proc_path = proc_root.join(pid.to_string());
    let cgroup_path = process_cgroup(&fs::read_to_string(proc_path.join("cgroup"))?)?;
    if cgroup_path == parent || !cgroup_path.starts_with(parent) {
        return Err(RunError::Invalid(
            "Tika/OCR process is outside the measured parent",
        ));
    }
    let cpuset_effective = fs::read_to_string(cgroup_path.join("cpuset.cpus.effective"))?;
    if cpuset_effective.trim() != affinity.trim() {
        return Err(RunError::Invalid(
            "Tika/OCR effective affinity differs from the shared parent",
        ));
    }
    let stat = fs::read_to_string(proc_path.join("stat"))?;
    let (_, fields) = stat
        .rsplit_once(')')
        .ok_or(RunError::Invalid("invalid process identity record"))?;
    let process_start_ticks = fields
        .split_whitespace()
        .nth(19)
        .and_then(|value| value.parse().ok())
        .ok_or(RunError::Invalid("missing process start identity"))?;
    Ok(Peer {
        kind,
        pid,
        process_start_ticks,
        cpu_max: fs::read_to_string(cgroup_path.join("cpu.max"))?,
        memory_max: fs::read_to_string(cgroup_path.join("memory.max"))?,
        cpu_stat: fs::read_to_string(cgroup_path.join("cpu.stat"))?,
        memory_current_bytes: numeric_file(&cgroup_path, "memory.current")?,
        memory_peak_bytes: peak(&cgroup_path)?,
        memory_events: fs::read_to_string(cgroup_path.join("memory.events"))?,
        cpuset_effective,
        cgroup_path,
    })
}

fn budget(
    path: &Path,
    cpus: u32,
    tika_pid: u32,
    ocr_pid: Option<u32>,
    proc_root: &Path,
) -> Result<Budget, RunError> {
    if ![1, 2].contains(&cpus) {
        return Err(RunError::Invalid("CPU budget must be one or two"));
    }
    let path = fs::canonicalize(path)?;
    if !path.starts_with("/sys/fs/cgroup") {
        return Err(RunError::Invalid("budget must be a real cgroup v2 path"));
    }
    let current = fs::read_to_string("/proc/self/cgroup")?;
    let runner_path = process_cgroup(&current)?;
    if !runner_path.starts_with(&path) {
        return Err(RunError::Invalid(
            "benchmark runner is outside the measured parent cgroup",
        ));
    }
    let cpu_max = fs::read_to_string(path.join("cpu.max"))?;
    let mut fields = cpu_max.split_whitespace();
    let quota = fields
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or(RunError::Invalid("unbounded CPU quota"))?;
    let period = fields
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or(RunError::Invalid("invalid CPU period"))?;
    if fields.next().is_some()
        || quota == 0
        || period == 0
        || quota
            != period
                .checked_mul(u64::from(cpus))
                .ok_or(RunError::Invalid("CPU period overflow"))?
    {
        return Err(RunError::Invalid(
            "parent quota must equal the declared one/two-CPU budget",
        ));
    }
    let memory = fs::read_to_string(path.join("memory.max"))?
        .trim()
        .parse::<u64>()
        .map_err(|_| RunError::Invalid("unbounded parent memory"))?;
    if memory == 0 || memory > 3 * 1024 * 1024 * 1024 {
        return Err(RunError::Invalid(
            "parent memory must be bounded at at most 3 GiB",
        ));
    }
    let affinity = fs::read_to_string(path.join("cpuset.cpus.effective"))?;
    if cpu_set_size(&affinity)? != cpus as usize {
        return Err(RunError::Invalid(
            "shared parent affinity width must equal the declared CPU budget",
        ));
    }
    let mut peers = ArrayVec::new();
    peers.push(peer(PeerKind::Tika, tika_pid, &path, &affinity, proc_root)?);
    if let Some(pid) = ocr_pid {
        if pid == tika_pid {
            return Err(RunError::Invalid(
                "Tika and OCR must have distinct process identities",
            ));
        }
        peers.push(peer(PeerKind::Ocr, pid, &path, &affinity, proc_root)?);
    }
    Ok(Budget {
        cpus,
        quota_us: quota,
        period_us: period,
        memory_limit_bytes: memory,
        runner_cgroup: current,
        cpuset_effective: affinity,
        cpu_stat: fs::read_to_string(path.join("cpu.stat"))?,
        memory_current_bytes: numeric_file(&path, "memory.current")?,
        memory_peak_bytes: peak(&path)?,
        memory_events: fs::read_to_string(path.join("memory.events"))?,
        peers,
        parent_path: path,
    })
}
/// Append one journal record and the newline that completes it.
pub(crate) fn record<T: Serialize>(out: &mut File, value: &T) -> Result<(), RunError> {
    serde_json::to_writer(&mut *out, value)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}
/// reqwest applies the request deadline to every body read and reports its
/// expiry as an `io::Error` of kind `Other` whose source is a reqwest timeout.
fn read_failure(failure: &std::io::Error) -> &'static str {
    if failure
        .get_ref()
        .and_then(|source| source.downcast_ref::<reqwest::Error>())
        .is_some_and(reqwest::Error::is_timeout)
    {
        "timeout"
    } else {
        "response_read"
    }
}
fn random(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

pub fn run(options: &RunOptions<'_>) -> Result<(), RunError> {
    if options.repetitions == 0 || options.repetitions > 20 {
        return Err(RunError::Invalid("repetitions must be 1..20"));
    }
    if options.label.is_empty()
        || !options
            .label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(RunError::Invalid("variant label must be a safe identifier"));
    }
    if options
        .tesseract_psm
        .is_some_and(|psm| ![3, 6, 11].contains(&psm))
    {
        return Err(RunError::Invalid(
            "predeclared Tesseract PSM variants are 3, 6 and 11",
        ));
    }
    let endpoint = reqwest::Url::parse(options.endpoint)
        .map_err(|_| RunError::Invalid("invalid Tika endpoint"))?;
    if endpoint.scheme() != "http"
        || !matches!(endpoint.host_str(), Some("127.0.0.1" | "[::1]"))
        || endpoint.path() != "/tika"
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(RunError::Invalid(
            "measurement endpoint must be a plain loopback /tika URL without credentials, query, or fragment",
        ));
    }
    let (corpus, manifest_sha256, manifest_bytes) =
        corpus::read_json_hashed::<Corpus>(options.manifest)?;
    corpus.validate(options.root)?;
    let budget_before = budget(
        options.cgroup,
        options.cpus,
        options.tika_pid,
        options.ocr_pid,
        options.peer_proc_root,
    )?;
    fs::create_dir_all(options.output)?;
    let mut raw = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(options.output.join("requests.jsonl"))?;
    let seed = corpus.selection_seed ^ 0x74696b612d6f6372;
    let environment = Environment {
        kind: Kind::Environment,
        schema_version: 1,
        corpus_id: corpus.corpus_id.as_str(),
        manifest_sha256: manifest_sha256.as_str(),
        manifest_bytes,
        variant: options.label,
        selection_seed: corpus.selection_seed,
        order_seed: seed,
        repetitions: options.repetitions,
        budget: budget_before,
        scope: "Closed-loop serial Tika HTTP attempts. Successful elapsed_us spans send through complete response body; failed elapsed_us ends at the observed failure. Output validation, hashing and persistence follow the attempt timer and remain inside repetition wall time. Warmup is separate from accuracy. Parent CPU spans client setup, warmup and all repetitions after corpus admission; memory counters are cgroup charged usage and lifetime peaks. Live Tika, optional OCR and load-generator identities are verified under the same bounded parent at start/end.",
    };
    record(&mut raw, &environment)?;
    let budget_before = &environment.budget;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    // One fixed response buffer is initialized before timing and reused. Failed
    // UTF-8 responses remain exact bytes. Oversized responses retain the bounded
    // 4 MiB + 1 prefix and are explicitly failures, never scored as predictions.
    let mut output = [0_u8; 4 * 1024 * 1024 + 1];
    let mut order = std::array::from_fn::<_, 128, _>(|index| index);
    let mut rng = seed;
    for repetition in 0..=options.repetitions {
        for i in (1..corpus.cases.len()).rev() {
            let j = (random(&mut rng) % (i as u64 + 1)) as usize;
            order.swap(i, j);
        }
        let attempts = if repetition == 0 {
            1
        } else {
            corpus.cases.len()
        };
        let repetition_started = Instant::now();
        for (sequence, &index) in order[..attempts].iter().enumerate() {
            let case = &corpus.cases[index];
            if !case
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                return Err(RunError::Invalid("case id is not a safe output filename"));
            }
            // Hash admission happens before warmup. The corpus mount must stay
            // read-only for the run; scoring is a separate, untimed command.
            let input = corpus::safe_path(options.root, &case.input.path)?;
            let input_file = File::open(input)?;
            let mut request = client
                .put(options.endpoint)
                .header("Accept", "text/plain")
                .header("Content-Type", &case.input_mime)
                .body(input_file);
            let psm_text;
            if let Some(psm) = options.tesseract_psm {
                psm_text = psm.to_string();
                request = request
                    .header("X-Tika-OCRLanguage", &case.tesseract_language)
                    .header("X-Tika-OCRPageSegMode", &psm_text);
            }
            let mut output_bytes = 0;
            let started = Instant::now();
            let response = request.send();
            let mut status = None;
            let mut error = None;
            match response {
                Ok(mut response) => {
                    status = Some(response.status().as_u16());
                    while output_bytes < output.len() {
                        match response.read(&mut output[output_bytes..]) {
                            Ok(0) => break,
                            Ok(count) => output_bytes += count,
                            Err(failure) => {
                                error = Some(read_failure(&failure));
                                break;
                            }
                        }
                    }
                    if output_bytes == output.len() {
                        error = Some("response_limit");
                    }
                    if !status.is_some_and(|status| (200..300).contains(&status)) {
                        error = Some("http_status");
                    }
                }
                Err(ref failure) if failure.is_timeout() => error = Some("timeout"),
                Err(_) => error = Some("transport"),
            }
            let elapsed = started.elapsed();
            if error.is_none() && std::str::from_utf8(&output[..output_bytes]).is_err() {
                error = Some("response_utf8");
            }
            let output_name = PathBuf::from(format!("{}-r{repetition:02}.txt", case.id));
            let output_path = options.output.join(&output_name);
            let output_sha256 = format!("{:x}", Sha256::digest(&output[..output_bytes]));
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output_path)?
                .write_all(&output[..output_bytes])?;
            record(
                &mut raw,
                &Attempt {
                    kind: if repetition == 0 {
                        Kind::Warmup
                    } else {
                        Kind::Attempt
                    },
                    variant: options.label,
                    cpus: options.cpus,
                    repetition,
                    sequence,
                    case_id: &case.id,
                    document_cluster: &case.document_cluster,
                    writer_cluster: case.writer_cluster.as_deref(),
                    language: &case.language,
                    script: &case.script,
                    domain: &case.domain,
                    appearance: &case.appearance,
                    reference_quality: &case.reference_quality,
                    cohort: &case.cohort,
                    task: &case.task,
                    tesseract_language: options
                        .tesseract_psm
                        .map(|_| case.tesseract_language.as_str()),
                    tesseract_psm: options.tesseract_psm,
                    elapsed_us: elapsed.as_micros(),
                    status,
                    error,
                    output_path: &output_name,
                    output_bytes: output_bytes as u64,
                    output_sha256: &output_sha256,
                },
            )?;
        }
        record(
            &mut raw,
            &RepetitionEnd {
                kind: Kind::RepetitionEnd,
                repetition,
                attempted_pages: attempts,
                wall_us: repetition_started.elapsed().as_micros(),
            },
        )?;
    }
    let final_budget = budget(
        options.cgroup,
        options.cpus,
        options.tika_pid,
        options.ocr_pid,
        options.peer_proc_root,
    )?;
    if final_budget.parent_path != budget_before.parent_path
        || final_budget.quota_us != budget_before.quota_us
        || final_budget.period_us != budget_before.period_us
        || final_budget.memory_limit_bytes != budget_before.memory_limit_bytes
        || final_budget.cpuset_effective != budget_before.cpuset_effective
        || final_budget.runner_cgroup != budget_before.runner_cgroup
        || final_budget
            .peers
            .iter()
            .zip(&budget_before.peers)
            .any(|(after, before)| {
                after.pid != before.pid
                    || after.process_start_ticks != before.process_start_ticks
                    || after.cgroup_path != before.cgroup_path
                    || after.cpu_max != before.cpu_max
                    || after.memory_max != before.memory_max
            })
    {
        return Err(RunError::Invalid(
            "process identity or aggregate resource contract changed during measurement",
        ));
    }
    record(
        &mut raw,
        &ResourceEnd {
            kind: Kind::ResourceEnd,
            budget: final_budget,
        },
    )?;
    raw.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seeded_order_is_reproducible() {
        let mut a = 20261017;
        let mut b = 20261017;
        for _ in 0..1000 {
            assert_eq!(random(&mut a), random(&mut b));
        }
    }
    #[test]
    fn unconstrained_budget_is_rejected_before_measurement() {
        assert!(matches!(
            budget(Path::new("/tmp"), 8, 1, None, Path::new("/proc")),
            Err(RunError::Invalid(_))
        ));
    }
    #[test]
    fn body_deadline_is_a_timeout_and_a_truncated_body_is_a_read_failure() {
        use std::{net::TcpListener, sync::mpsc};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (release, released) = mpsc::channel::<()>();
        // Both responses promise ten body bytes and send two. The first
        // connection then stalls until the client has seen its read fail; the
        // second closes at once.
        let server = std::thread::spawn(move || {
            for stall in [true, false] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut head = [0_u8; 4096];
                let mut length = 0;
                while !head[..length]
                    .windows(4)
                    .any(|window| window == b"\r\n\r\n")
                {
                    let read = stream.read(&mut head[length..]).unwrap();
                    assert_ne!(read, 0, "client closed before its request head");
                    length += read;
                }
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nab")
                    .unwrap();
                if stall {
                    released.recv().unwrap();
                }
            }
        });
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(500))
            .build()
            .unwrap();
        let mut output = [0_u8; 16];
        for expected in ["timeout", "response_read"] {
            let mut response = client.get(format!("http://{address}/tika")).send().unwrap();
            let failure = loop {
                match response.read(&mut output) {
                    Ok(0) => panic!("a ten-byte body ended after two bytes without an error"),
                    Ok(_) => (),
                    Err(failure) => break failure,
                }
            };
            assert_eq!(read_failure(&failure), expected, "{failure:?}");
            if expected == "timeout" {
                release.send(()).unwrap();
            }
        }
        server.join().unwrap();
    }
    #[test]
    fn effective_cpu_ranges_are_bounded_and_unambiguous() {
        assert_eq!(cpu_set_size("0\n").expect("one CPU"), 1);
        assert_eq!(cpu_set_size("4-5\n").expect("two CPUs"), 2);
        for invalid in ["", "0,0", "1-0", "4096", "-1", "1-2,2"] {
            assert!(cpu_set_size(invalid).is_err(), "{invalid}");
        }
    }
}
