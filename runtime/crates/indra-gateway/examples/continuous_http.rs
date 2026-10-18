//! Whole-stack continuous scheduler pilot. Every child and HTTP client stays
//! inside the caller's one Docker cgroup. No mock OCR or per-request JVM.
use clap::{Parser, ValueEnum};
use rustix::process::{self, Pid};
use serde::Serialize;
use sha2::{Digest, Sha256};
use stats_alloc::{Region, StatsAlloc};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use thiserror::Error;

#[path = "continuous_http/config.rs"]
mod config;
#[path = "continuous_http/diagnostics.rs"]
mod diagnostics;
#[path = "continuous_http/http.rs"]
mod http;
#[path = "continuous_http/process.rs"]
mod processes;
use processes::Process;

#[global_allocator]
static ALLOCATOR: StatsAlloc<mimalloc::MiMalloc> = StatsAlloc::new(mimalloc::MiMalloc);
static INPUTS: [usize; 12] = [0, 1, 2, 0, 2, 1, 1, 0, 2, 2, 1, 0];
static FIXTURES: [&[u8]; 3] = [
    include_bytes!("../../../../backend/tests/data/smoke.png"),
    include_bytes!("../../../../backend/tests/data/two-pages.tiff"),
    include_bytes!("../../indra-engine/tests/data/math.png"),
];
static MIMES: [&str; 3] = ["image/png", "image/tiff", "image/png"];
static NAMES: [&str; 3] = ["smoke", "two-pages", "math-page"];
#[derive(Clone, Copy, ValueEnum)]
enum Arm {
    Ordinary,
    Continuous,
}
#[derive(Parser)]
struct Cli {
    #[arg(long, default_value = "/indra-gateway")]
    gateway: PathBuf,
    #[arg(long, default_value = "/usr/bin/java")]
    java: PathBuf,
    #[arg(long, default_value = "/opt/tika/tika-server.jar")]
    jar: PathBuf,
    #[arg(long, default_value = "/opt/indra/tesseract")]
    bridge: PathBuf,
    #[arg(long, default_value = "/indra-server")]
    native: PathBuf,
    #[arg(long)]
    model_dir: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, value_enum)]
    arm: Arm,
    #[arg(long,value_parser=clap::value_parser!(u8).range(1..=3))]
    concurrency: u8,
    #[arg(long,value_parser=clap::value_parser!(u8).range(1..=2))]
    cpus: u8,
}
#[derive(Debug, Error)]
enum Failure {
    #[error("Tika pilot gate: {0}")]
    Gate(&'static str),
    #[error("I/O operation failed")]
    Io(#[from] io::Error),
    #[error("HTTP transport failed")]
    Http(#[from] hyper::Error),
    #[error("HTTP construction failed")]
    Request(#[from] axum::http::Error),
    #[error("Evidence encoding failed")]
    Json(#[from] serde_json::Error),
}
fn check(ok: bool, why: &'static str) -> Result<(), Failure> {
    if ok { Ok(()) } else { Err(Failure::Gate(why)) }
}
fn create_new(path: &Path) -> Result<File, Failure> {
    Ok(OpenOptions::new().create_new(true).write(true).open(path)?)
}
#[derive(Serialize)]
struct Outcome {
    sequence: usize,
    fixture: &'static str,
    status: &'static str,
    http_status: Option<u16>,
    elapsed_us: Option<u128>,
    output_bytes: Option<usize>,
    output_sha256: Option<[u8; 32]>,
}
impl Outcome {
    const fn missing(sequence: usize) -> Self {
        Self {
            sequence,
            fixture: NAMES[INPUTS[sequence]],
            status: "missing",
            http_status: None,
            elapsed_us: None,
            output_bytes: None,
            output_sha256: None,
        }
    }
}
#[derive(Serialize)]
struct Counters {
    allocations: usize,
    deallocations: usize,
    reallocations: usize,
    bytes_allocated: usize,
    bytes_deallocated: usize,
    bytes_reallocated: isize,
}
#[derive(Serialize)]
struct Summary {
    schema_version: u8,
    scope: &'static str,
    arm: &'static str,
    cpus: u8,
    concurrency: u8,
    planned_requests: usize,
    successful_requests: usize,
    complete: bool,
    cancellation_probe: bool,
    wall_us: Option<u128>,
    successful_responses_per_wall_second: Option<f64>,
    p50_us: Option<u128>,
    p95_us: Option<u128>,
    p99_us: Option<u128>,
    successful_latency_samples: usize,
    failure_or_missing_samples: usize,
    quantiles: &'static str,
    driver_rust_allocations: Option<Counters>,
    allocation_scope: &'static str,
    http_response_copy_bytes: usize,
    native_internal_copy_bytes: Option<usize>,
    native_diagnostics: Option<diagnostics::NativeDiagnostics>,
    child_cleanup_complete: bool,
    terminal_error: Option<String>,
}
fn main() -> Result<(), Failure> {
    let cli = Cli::parse();
    fs::create_dir(&cli.output)?;
    let outcomes: [Mutex<Outcome>; 12] = std::array::from_fn(|i| Mutex::new(Outcome::missing(i)));
    let mut summary = Summary {
        schema_version: 1,
        scope: "gateway + forked Tika3.3.2 + bridge + native + HTTP load generator in one inherited cgroup; synthetic implementation pilot",
        arm: match cli.arm {
            Arm::Ordinary => "ordinary",
            Arm::Continuous => "continuous",
        },
        cpus: cli.cpus,
        concurrency: cli.concurrency,
        planned_requests: 12,
        successful_requests: 0,
        complete: false,
        cancellation_probe: false,
        wall_us: None,
        successful_responses_per_wall_second: None,
        p50_us: None,
        p95_us: None,
        p99_us: None,
        successful_latency_samples: 0,
        failure_or_missing_samples: 12,
        quantiles: "successful complete-body nearest-rank microseconds; 12 nested samples cannot resolve stable p99",
        driver_rust_allocations: None,
        allocation_scope: "instrumented mimalloc in HTTP driver only; child/native process allocators excluded; JSON output excluded",
        http_response_copy_bytes: 0,
        native_internal_copy_bytes: None,
        native_diagnostics: None,
        child_cleanup_complete: false,
        terminal_error: None,
    };
    process::set_child_subreaper(Pid::from_raw(1)).map_err(io::Error::from)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(run(&cli, &outcomes, &mut summary));
    let cleanup = processes::kill_owned_children()
        .and_then(|()| runtime.block_on(processes::reap_all(Duration::from_secs(2))));
    summary.child_cleanup_complete = matches!(cleanup, Ok(true));
    summary.terminal_error = result.as_ref().err().map(ToString::to_string);
    summary.complete =
        result.is_ok() && summary.child_cleanup_complete && summary.successful_requests == 12;
    let mut journal = create_new(&cli.output.join("requests.jsonl"))?;
    for outcome in &outcomes {
        let outcome = outcome
            .lock()
            .map_err(|_| Failure::Gate("outcome mutex poisoned"))?;
        serde_json::to_writer(&mut journal, &*outcome)?;
        journal.write_all(b"\n")?;
    }
    journal.sync_all()?;
    let mut output = create_new(&cli.output.join("summary.json"))?;
    serde_json::to_writer_pretty(&mut output, &summary)?;
    output.write_all(b"\n")?;
    output.sync_all()?;
    result?;
    cleanup?;
    check(
        summary.complete,
        "failed or incomplete whole-stack results retained",
    )
}
async fn run(
    cli: &Cli,
    outcomes: &[Mutex<Outcome>; 12],
    summary: &mut Summary,
) -> Result<(), Failure> {
    for path in [&cli.gateway, &cli.native, &cli.java, &cli.jar, &cli.bridge] {
        check(
            path.is_absolute() && path.is_file(),
            "explicit executable paths required",
        )?;
    }
    snapshot(cli, "startup")?;
    assets(cli)?;
    let native_address = spare_address()?;
    let tika_address = spare_address()?;
    let gateway_address = spare_address()?;
    check(
        native_address != tika_address
            && native_address != gateway_address
            && tika_address != gateway_address,
        "ephemeral port collision",
    )?;
    config::write_config(cli, tika_address, &cli.output.join("tika-config.xml"))?;
    let mut native = start_native(cli, native_address, false, "native-reference")?;
    ready(native_address, "/readyz", &mut native).await?;
    let mut command = Command::new(&cli.gateway);
    command
        .arg("--listen")
        .arg(gateway_address.to_string())
        .arg("--backend")
        .arg(tika_address.to_string())
        .arg("--java")
        .arg(&cli.java)
        .arg("--jar")
        .arg(&cli.jar)
        .arg("--tika-config")
        .arg(cli.output.join("tika-config.xml"))
        .args([
            "--max-active",
            "3",
            "--child-shutdown-seconds",
            "8",
            "serve",
        ])
        .env_clear()
        .env("LANG", "C.UTF-8")
        .env("INDRA_ADDR", native_address.to_string())
        .env("INDRA_TIMEOUT_SECONDS", "70")
        .env("INDRA_CONNECT_TIMEOUT_MS", "1000");
    let mut gateway = Process::start(command, "gateway", &cli.output)?;
    ready(gateway_address, "/readyz", &mut gateway).await?;
    let version = ready(tika_address, "/version", &mut gateway).await?;
    check(
        std::str::from_utf8(&version.bytes[..version.used])
            .ok()
            .is_some_and(|s| s.trim() == "Apache Tika 3.3.2"),
        "Tika version differs",
    )?;
    let mut references: [Option<http::Reply>; 3] = std::array::from_fn(|_| None);
    for index in 0..3 {
        let reply =
            http::request(gateway_address, "/tika", FIXTURES[index], MIMES[index], &[]).await?;
        check(reply.status == 200, "ordinary reference request failed")?;
        fs::write(
            cli.output.join(format!("reference-{}.txt", NAMES[index])),
            &reply.bytes[..reply.used],
        )?;
        references[index] = Some(reply);
    }
    if matches!(cli.arm, Arm::Continuous) {
        check(
            native.shutdown(Duration::from_secs(15)).await? == Some(0),
            "reference native shutdown failed",
        )?;
        native = start_native(cli, native_address, true, "native-continuous")?;
        ready(native_address, "/readyz", &mut native).await?;
    }
    for index in 0..3 {
        let reply =
            http::request(gateway_address, "/tika", FIXTURES[index], MIMES[index], &[]).await?;
        let reference = references[index]
            .as_ref()
            .ok_or(Failure::Gate("reference missing"))?;
        check(
            reply.status == 200 && reply.bytes[..reply.used] == reference.bytes[..reference.used],
            "selected arm warmup output differs from ordinary reference",
        )?;
    }
    snapshot(cli, "measurement-before")?;
    let begin_unix_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Failure::Gate("system clock precedes epoch"))?
        .as_nanos();
    let region = Region::new(&ALLOCATOR);
    let started = Instant::now();
    let next = AtomicUsize::new(0);
    let (one, two, three) = tokio::join!(
        worker(0, cli, gateway_address, &next, &references, outcomes),
        worker(1, cli, gateway_address, &next, &references, outcomes),
        worker(2, cli, gateway_address, &next, &references, outcomes)
    );
    let elapsed = started.elapsed();
    let allocations = region.change();
    let end_unix_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Failure::Gate("system clock precedes epoch"))?
        .as_nanos();
    snapshot(cli, "measurement-after")?;
    summary.wall_us = Some(elapsed.as_micros());
    summary.driver_rust_allocations = Some(Counters {
        allocations: allocations.allocations,
        deallocations: allocations.deallocations,
        reallocations: allocations.reallocations,
        bytes_allocated: allocations.bytes_allocated,
        bytes_deallocated: allocations.bytes_deallocated,
        bytes_reallocated: allocations.bytes_reallocated,
    });
    let mut latencies = [0_u128; 12];
    for result in outcomes {
        let result = result
            .lock()
            .map_err(|_| Failure::Gate("outcome mutex poisoned"))?;
        summary.http_response_copy_bytes += result.output_bytes.unwrap_or(0);
        if result.status == "success" {
            latencies[summary.successful_requests] = result
                .elapsed_us
                .ok_or(Failure::Gate("success timing missing"))?;
            summary.successful_requests += 1;
        }
    }
    let count = summary.successful_requests;
    latencies[..count].sort_unstable();
    summary.successful_latency_samples = count;
    summary.failure_or_missing_samples = 12 - count;
    if count > 0 {
        summary.p50_us = Some(latencies[(count * 50).div_ceil(100) - 1]);
        summary.p95_us = Some(latencies[(count * 95).div_ceil(100) - 1]);
        summary.p99_us = Some(latencies[(count * 99).div_ceil(100) - 1]);
    }
    summary.successful_responses_per_wall_second = Some(count as f64 / elapsed.as_secs_f64());
    one?;
    two?;
    three?;
    check(
        gateway.shutdown(Duration::from_secs(15)).await? == Some(0),
        "gateway/Java shutdown failed",
    )?;
    check(
        native.shutdown(Duration::from_secs(15)).await? == Some(0),
        "native shutdown failed",
    )?;
    let native_log = if matches!(cli.arm, Arm::Continuous) {
        "native-continuous.stderr"
    } else {
        "native-reference.stderr"
    };
    summary.native_diagnostics = Some(diagnostics::read(
        &cli.output.join(native_log),
        begin_unix_ns,
        end_unix_ns,
        matches!(cli.arm, Arm::Continuous),
    )?);
    snapshot(cli, "cleanup-after")?;
    Ok(())
}
async fn worker(
    slot: u8,
    cli: &Cli,
    address: SocketAddr,
    next: &AtomicUsize,
    references: &[Option<http::Reply>; 3],
    outcomes: &[Mutex<Outcome>; 12],
) -> Result<(), Failure> {
    if slot >= cli.concurrency {
        return Ok(());
    }
    loop {
        let sequence = next.fetch_add(1, Ordering::Relaxed);
        if sequence >= 12 {
            return Ok(());
        }
        let fixture = INPUTS[sequence];
        let started = Instant::now();
        let reply = http::request(address, "/tika", FIXTURES[fixture], MIMES[fixture], &[]).await;
        let elapsed = started.elapsed();
        let mut outcome = Outcome::missing(sequence);
        outcome.elapsed_us = Some(elapsed.as_micros());
        match reply {
            Ok(reply) => {
                outcome.http_status = Some(reply.status);
                outcome.output_bytes = Some(reply.used);
                outcome.output_sha256 = Some(Sha256::digest(&reply.bytes[..reply.used]).into());
                let reference = references[fixture]
                    .as_ref()
                    .ok_or(Failure::Gate("ordinary reference absent"))?;
                outcome.status = if reply.status != 200 {
                    "http_failure"
                } else if reply.bytes[..reply.used] != reference.bytes[..reference.used] {
                    "output_mismatch"
                } else {
                    "success"
                };
            }
            Err(error) => {
                outcome.status = match error {
                    Failure::Gate("HTTP exchange deadline") => "client_deadline",
                    Failure::Gate("response exceeds fixed test cap") => "response_limit",
                    Failure::Http(_) => "transport_failure",
                    _ => "client_failure",
                };
            }
        }
        *outcomes[sequence]
            .lock()
            .map_err(|_| Failure::Gate("outcome mutex poisoned"))? = outcome;
    }
}
fn start_native(
    cli: &Cli,
    address: SocketAddr,
    continuous: bool,
    name: &'static str,
) -> Result<Process, Failure> {
    let mut command = Command::new(&cli.native);
    command
        .arg("--listen")
        .arg(address.to_string())
        .arg("--model-dir")
        .arg(&cli.model_dir)
        .arg("--threads")
        .arg(cli.cpus.to_string())
        .arg("--cv-threads")
        .arg(cli.cpus.to_string())
        .args([
            "--workers",
            "1",
            "--queue-capacity",
            "2",
            "--batch-size",
            "64",
            "--math",
            "false",
            "--intra-op-spinning",
            "true",
            "--inter-op-spinning",
            "true",
            "--experimental-continuous-batching",
            if continuous { "true" } else { "false" },
            "serve",
        ])
        .env_clear()
        .env("LANG", "C.UTF-8");
    Process::start(command, name, &cli.output)
}
fn spare_address() -> Result<SocketAddr, Failure> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?)
}
async fn ready(
    address: SocketAddr,
    path: &'static str,
    child: &mut Process,
) -> Result<http::Reply, Failure> {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        check(
            Instant::now() < deadline && child.child.try_wait()?.is_none(),
            "startup deadline or child exit",
        )?;
        if let Ok(Ok(reply)) = tokio::time::timeout(
            Duration::from_secs(1),
            http::request(address, path, b"", "application/octet-stream", &[]),
        )
        .await
            && reply.status == 200
        {
            return Ok(reply);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
fn snapshot(cli: &Cli, label: &str) -> Result<(), Failure> {
    let group = Path::new("/sys/fs/cgroup");
    let directory = cli.output.join(label);
    fs::create_dir(&directory)?;
    let quota = fs::read_to_string(group.join("cpu.max"))?;
    let mut fields = quota.split_whitespace();
    let quota = fields.next().and_then(|s| s.parse::<u64>().ok());
    let period = fields.next().and_then(|s| s.parse::<u64>().ok());
    check(
        quota == Some(u64::from(cli.cpus) * 100000)
            && period == Some(100000)
            && fs::read_to_string(group.join("memory.max"))?.trim() == "3221225472"
            && fs::read_to_string(group.join("memory.swap.max"))?.trim() == "0",
        "actual whole-stack cgroup budget differs",
    )?;
    for name in [
        "cpu.max",
        "cpu.stat",
        "cpuset.cpus.effective",
        "memory.max",
        "memory.swap.max",
        "memory.current",
        "memory.peak",
        "memory.events",
        "pids.max",
        "pids.current",
    ] {
        fs::write(directory.join(name), fs::read(group.join(name))?)?;
    }
    let members = fs::read_to_string(group.join("cgroup.procs"))?;
    check(members.len() <= 8192, "process membership inventory cap")?;
    for (count, pid) in members.split_whitespace().enumerate() {
        check(
            count < 128 && pid.bytes().all(|b| b.is_ascii_digit()),
            "invalid or unbounded cgroup process membership",
        )?;
        if let Ok(status) = fs::read(format!("/proc/{pid}/status")) {
            fs::write(directory.join(format!("process-{pid}.status")), status)?;
        }
    }
    fs::write(directory.join("cgroup.procs"), members)?;
    fs::write(
        directory.join("driver-status"),
        fs::read("/proc/self/status")?,
    )?;
    Ok(())
}
fn assets(cli: &Cli) -> Result<(), Failure> {
    let mut file = create_new(&cli.output.join("assets.jsonl"))?;
    for (role, path) in [
        ("gateway", &cli.gateway),
        ("native", &cli.native),
        ("java", &cli.java),
        ("tika", &cli.jar),
        ("bridge", &cli.bridge),
    ] {
        let mut input = File::open(path)?;
        let mut hash = Sha256::new();
        let mut bytes = 0_u64;
        let mut buffer = [0_u8; 65536];
        loop {
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            bytes += n as u64;
            check(bytes <= 536870912, "asset cap exceeded")?;
            hash.update(&buffer[..n]);
        }
        #[derive(Serialize)]
        struct Asset<'a> {
            role: &'a str,
            path: &'a Path,
            bytes: u64,
            sha256: String,
        }
        serde_json::to_writer(
            &mut file,
            &Asset {
                role,
                path,
                bytes,
                sha256: format!("{:x}", hash.finalize()),
            },
        )?;
        file.write_all(b"\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn exchange(response: &'static [u8], body: &'static [u8]) -> Result<http::Reply, Failure> {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");
        let address = listener.local_addr().expect("fixture address");
        std::thread::scope(|scope| {
            let server = scope.spawn(|| {
                let (mut socket, _) = listener.accept().expect("accept");
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .expect("deadline");
                let mut bytes = [0_u8; 8192];
                let mut used = 0;
                loop {
                    let count = socket.read(&mut bytes[used..]).expect("request headers");
                    assert!(count != 0);
                    used += count;
                    if bytes[..used].windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let header = std::str::from_utf8(&bytes[..used]).expect("ASCII HTTP headers");
                assert!(header.starts_with("PUT /tika HTTP/1.1\r\n"));
                assert!(header.contains("content-type: image/png\r\n"));
                socket.write_all(response).expect("response headers");
                let _ = socket.write_all(body);
            });
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");
            let reply = runtime.block_on(http::request(
                address,
                "/tika",
                b"fixture",
                "image/png",
                &[],
            ));
            server.join().expect("server fixture");
            reply
        })
    }

    #[test]
    fn actual_http_preserves_fixture_mime_and_complete_body() {
        let reply = exchange(
            b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\n",
            b"result",
        )
        .expect("complete response");
        assert_eq!(reply.status, 200);
        assert_eq!(&reply.bytes[..reply.used], b"result");
    }
    #[test]
    fn truncated_http_body_never_becomes_success() {
        assert!(
            exchange(
                b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\n",
                b"xx"
            )
            .is_err()
        );
    }
    #[test]
    fn fixed_response_bound_rejects_overflow() {
        static BODY: [u8; 16385] = [b'x'; 16385];
        assert!(matches!(
            exchange(
                b"HTTP/1.1 200 OK\r\nContent-Length: 16385\r\nConnection: close\r\n\r\n",
                &BODY
            ),
            Err(Failure::Gate("response exceeds fixed test cap"))
        ));
    }
    #[test]
    fn compiled_schedule_and_fixture_bytes_match_frozen_protocol() {
        let protocol: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../validation/continuous-protocol.json"
        ))
        .expect("frozen protocol");
        for (sequence, index) in INPUTS.iter().enumerate() {
            assert_eq!(
                protocol["paired_measurement"]["input_schedule"][sequence],
                NAMES[*index]
            );
        }
        for (index, bytes) in FIXTURES.iter().enumerate() {
            assert_eq!(
                protocol["fixtures"][index]["sha256"],
                format!("{:x}", Sha256::digest(bytes))
            );
        }
    }
}
