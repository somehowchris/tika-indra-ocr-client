//! Separate full-stack SDK study; frozen continuous cases are imported unchanged.
use clap::{Parser, ValueEnum};
use rustix::process::{self, Pid, Signal};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use thiserror::Error;
#[path = "continuous_http/config.rs"]
mod config;
#[path = "continuous_http/http.rs"]
mod http;
#[path = "continuous_http/process.rs"]
mod processes;
// Its original seven-case verifier is deliberately unused; this study audits its own12 parents.
#[path = "telemetry_http/audit.rs"]
mod audit;
#[allow(
    dead_code,
    reason = "Reuse the bounded OTLP collector with opt-in header privacy; preserve its original graph cases"
)]
#[path = "trace_acceptance/capture.rs"]
mod capture;
#[path = "telemetry_http/sentry.rs"]
mod sentry;
use processes::Process;
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;
static INPUTS: [usize; 12] = [0, 1, 2, 0, 2, 1, 1, 0, 2, 2, 1, 0];
static FIXTURES: [&[u8]; 3] = [
    include_bytes!("../../../../backend/tests/data/smoke.png"),
    include_bytes!("../../../../backend/tests/data/two-pages.tiff"),
    include_bytes!("../../indra-engine/tests/data/math.png"),
];
static MIMES: [&str; 3] = ["image/png", "image/tiff", "image/png"];
static NAMES: [&str; 3] = ["smoke", "two-pages", "math-page"];
static REQUEST_PARENTS: [&str; 12] = [
    "00-4bf92f3577b34da6a3ce929d0e0e4701-00f067aa0ba902b7-01",
    "00-4bf92f3577b34da6a3ce929d0e0e4702-00f067aa0ba902b7-01",
    "00-4bf92f3577b34da6a3ce929d0e0e4703-00f067aa0ba902b7-01",
    "00-4bf92f3577b34da6a3ce929d0e0e4704-00f067aa0ba902b7-01",
    "00-4bf92f3577b34da6a3ce929d0e0e4705-00f067aa0ba902b7-01",
    "00-4bf92f3577b34da6a3ce929d0e0e4706-00f067aa0ba902b7-01",
    "00-4bf92f3577b34da6a3ce929d0e0e4707-00f067aa0ba902b7-01",
    "00-4bf92f3577b34da6a3ce929d0e0e4708-00f067aa0ba902b7-01",
    "00-4bf92f3577b34da6a3ce929d0e0e4709-00f067aa0ba902b7-01",
    "00-4bf92f3577b34da6a3ce929d0e0e470a-00f067aa0ba902b7-01",
    "00-4bf92f3577b34da6a3ce929d0e0e470b-00f067aa0ba902b7-01",
    "00-4bf92f3577b34da6a3ce929d0e0e470c-00f067aa0ba902b7-01",
];
static PARENTS: [&str; 4] = [
    REQUEST_PARENTS[0],
    REQUEST_PARENTS[1],
    REQUEST_PARENTS[2],
    REQUEST_PARENTS[3],
];
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Arm {
    Reference,
    Off,
    Enabled,
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
    #[arg(long)]
    goldens: Option<PathBuf>,
    #[arg(long, value_enum)]
    arm: Arm,
    #[arg(long,value_parser=clap::value_parser!(u8).range(1..=2))]
    cpus: u8,
}
#[derive(Debug, Error)]
enum Failure {
    #[error("telemetry study: {0}")]
    Gate(&'static str),
    #[error("I/O operation failed")]
    Io(#[from] io::Error),
    #[error("HTTP transport failed")]
    Http(#[from] hyper::Error),
    #[error("HTTP construction failed")]
    Request(#[from] axum::http::Error),
    #[error("Evidence encoding failed")]
    Json(#[from] serde_json::Error),
    #[error("OTLP protobuf failed")]
    Decode(#[from] prost::DecodeError),
}
fn check(ok: bool, why: &'static str) -> Result<(), Failure> {
    if ok { Ok(()) } else { Err(Failure::Gate(why)) }
}
fn create_new(path: &Path) -> Result<File, Failure> {
    Ok(OpenOptions::new().create_new(true).write(true).open(path)?)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), Failure> {
    create_new(path)?.write_all(bytes)?;
    Ok(())
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
    failure: Option<String>,
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
            failure: None,
        }
    }
}
#[derive(Serialize)]
struct Summary {
    schema_version: u8,
    protocol_sha256: String,
    scope: &'static str,
    arm: &'static str,
    cpus: u8,
    concurrency: u8,
    planned_requests: usize,
    successful_requests: usize,
    complete: bool,
    wall_us: Option<u128>,
    cohort_exports_observed: Option<usize>,
    median_us: Option<u128>,
    successful_responses_per_wall_second: Option<f64>,
    fault_owned_pid: Option<u32>,
    fault_owned_group: Option<u32>,
    fault_wall_us: Option<u128>,
    gateway_exit: Option<i32>,
    native_exit: Option<i32>,
    child_cleanup_complete: bool,
    export_audit: Option<audit::Audit>,
    terminal_error: Option<String>,
    collector_errors: [Option<String>; 2],
    cleanup_error: Option<String>,
    audit_error: Option<String>,
    resource_error: Option<String>,
}
fn main() -> Result<(), Failure> {
    let cli = Cli::parse();
    fs::create_dir(&cli.output)?;
    let mut summary = Summary {
        schema_version: 1,
        protocol_sha256: format!(
            "{:x}",
            Sha256::digest(include_bytes!(
                "../../../validation/telemetry-http-protocol.json"
            ))
        ),
        scope: "full HTTP stack+client+local receivers in one inherited cgroup; synthetic SDK diagnostic; success latency separate from terminal fault/export/shutdown",
        arm: match cli.arm {
            Arm::Reference => "reference",
            Arm::Off => "off",
            Arm::Enabled => "enabled",
        },
        cpus: cli.cpus,
        concurrency: 2,
        planned_requests: if cli.arm == Arm::Reference { 0 } else { 12 },
        successful_requests: 0,
        complete: false,
        wall_us: None,
        cohort_exports_observed: None,
        median_us: None,
        successful_responses_per_wall_second: None,
        fault_owned_pid: None,
        fault_owned_group: None,
        fault_wall_us: None,
        gateway_exit: None,
        native_exit: None,
        child_cleanup_complete: false,
        export_audit: None,
        terminal_error: None,
        collector_errors: std::array::from_fn(|_| None),
        cleanup_error: None,
        audit_error: None,
        resource_error: None,
    };
    let outcomes: [Mutex<Outcome>; 12] = std::array::from_fn(|i| Mutex::new(Outcome::missing(i)));
    process::set_child_subreaper(Pid::from_raw(1)).map_err(io::Error::from)?;
    let otlp = TcpListener::bind("127.0.0.1:0")?;
    let otlp_address = otlp.local_addr()?;
    let errors = TcpListener::bind("127.0.0.1:0")?;
    let sentry_address = errors.local_addr()?;
    let stop = AtomicBool::new(false);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = std::thread::scope(|scope| -> Result<(), Failure> {
        let traces = scope.spawn(|| {
            capture::collect_with_header_validator(
                otlp,
                &stop,
                &cli.output,
                audit::validate_export_headers,
            )
        });
        let envelopes = scope.spawn(|| sentry::collect(errors, &stop, &cli.output));
        let result = runtime.block_on(async {
            let mut terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            let mut interrupt =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
            tokio::select! {
                result=run(&cli,otlp_address,sentry_address,&outcomes,&mut summary)=>result,
                _=terminate.recv()=>Err(Failure::Gate("controller terminated")),
                _=interrupt.recv()=>Err(Failure::Gate("controller interrupted")),
            }
        });
        let termination = processes::kill_owned_children();
        let reaped = runtime.block_on(processes::reap_all(Duration::from_secs(2)));
        summary.cleanup_error = if termination.is_ok() && matches!(reaped, Ok(true)) {
            None
        } else {
            Some(format!("termination={termination:?}; reaping={reaped:?}"))
        };
        let cleanup = termination.and(reaped);
        summary.child_cleanup_complete = matches!(cleanup, Ok(true));
        stop.store(true, Ordering::Release);
        let captured = traces
            .join()
            .unwrap_or_else(|_| Err(Failure::Gate("OTLP collector thread panicked")));
        let errors = envelopes
            .join()
            .unwrap_or_else(|_| Err(Failure::Gate("Sentry collector thread panicked")));
        summary.collector_errors = [
            captured.as_ref().err().map(ToString::to_string),
            errors.as_ref().err().map(ToString::to_string),
        ];
        summary.terminal_error = result.as_ref().err().map(ToString::to_string);
        let audit = (|| {
            let captured = captured?;
            captured.write(&cli.output.join("spans.json"))?;
            audit::verify(
                &cli.output,
                cli.arm == Arm::Enabled,
                errors?,
                summary.cohort_exports_observed.unwrap_or(0),
                cli.goldens.as_deref(),
            )
        })();
        summary.audit_error = audit.as_ref().err().map(ToString::to_string);
        // Always retain post-reap, post-join resources, including failed/partial arms.
        let terminal_resources = snapshot(&cli, "terminal-after");
        summary.resource_error = terminal_resources.as_ref().err().map(ToString::to_string);
        result?;
        check(cleanup?, "owned process cleanup incomplete")?;
        summary.export_audit = Some(audit?);
        terminal_resources?;
        Ok(())
    });
    if summary.terminal_error.is_none() {
        summary.terminal_error = result.as_ref().err().map(ToString::to_string);
    }
    summary.complete = result.is_ok() && summary.child_cleanup_complete;
    let mut journal = create_new(&cli.output.join("requests.jsonl"))?;
    if cli.arm != Arm::Reference {
        for record in &outcomes {
            serde_json::to_writer(
                &mut journal,
                &*record
                    .lock()
                    .map_err(|_| Failure::Gate("outcome mutex poisoned"))?,
            )?;
            journal.write_all(b"\n")?;
        }
    }
    journal.sync_all()?;
    let mut file = create_new(&cli.output.join("summary.json"))?;
    serde_json::to_writer_pretty(&mut file, &summary)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    result
}
async fn run(
    cli: &Cli,
    otlp: SocketAddr,
    sentry: SocketAddr,
    outcomes: &[Mutex<Outcome>; 12],
    summary: &mut Summary,
) -> Result<(), Failure> {
    for path in [&cli.gateway, &cli.native, &cli.java, &cli.jar, &cli.bridge] {
        check(
            path.is_absolute() && path.is_file(),
            "explicit role paths required",
        )?;
    }
    check(
        cli.arm == Arm::Reference || cli.goldens.as_ref().is_some_and(|p| p.is_dir()),
        "paired arm requires admitted ordinary goldens",
    )?;
    let admission = cli
        .output
        .parent()
        .ok_or(Failure::Gate("output parent absent"))?
        .join("host-admission.json");
    let due = Instant::now() + Duration::from_secs(30);
    while !admission.is_file() {
        check(Instant::now() < due, "host cgroup admission deadline")?;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let admitted: serde_json::Value = serde_json::from_reader(File::open(admission)?)?;
    check(
        admitted["admitted"] == true,
        "host cgroup admission rejected",
    )?;
    snapshot(cli, "startup")?;
    assets(cli)?;
    let native_address = spare_address()?;
    let tika_address = spare_address()?;
    let gateway_address = spare_address()?;
    check(
        native_address != tika_address
            && native_address != gateway_address
            && tika_address != gateway_address
            && ![otlp, sentry].contains(&native_address)
            && ![otlp, sentry].contains(&gateway_address)
            && ![otlp, sentry].contains(&tika_address),
        "ephemeral port collision",
    )?;
    config::write_config(cli, tika_address, &cli.output.join("tika-config.xml"))?;
    let mut native_command = Command::new(&cli.native);
    native_command
        .args(["--listen", &native_address.to_string(), "--model-dir"])
        .arg(&cli.model_dir)
        .args([
            "--threads",
            &cli.cpus.to_string(),
            "--cv-threads",
            &cli.cpus.to_string(),
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
            "false",
        ])
        .env_clear()
        .env("LANG", "C.UTF-8");
    configure_sdk(
        &mut native_command,
        cli.arm,
        otlp,
        sentry,
        "telemetry-native",
    );
    native_command.arg("serve");
    let mut native = Process::start(native_command, "native", &cli.output)?;
    ready(native_address, "/readyz", &mut native).await?;
    let mut command = Command::new(&cli.gateway);
    command
        .args([
            "--listen",
            &gateway_address.to_string(),
            "--backend",
            &tika_address.to_string(),
            "--ocr-ready-endpoint",
            &native_address.to_string(),
            "--java",
        ])
        .arg(&cli.java)
        .arg("--jar")
        .arg(&cli.jar)
        .arg("--tika-config")
        .arg(cli.output.join("tika-config.xml"))
        .args(["--max-active", "3", "--child-shutdown-seconds", "8"])
        .env_clear()
        .env("LANG", "C.UTF-8")
        .env("INDRA_ADDR", native_address.to_string())
        .env("INDRA_TIMEOUT_SECONDS", "70")
        .env("INDRA_CONNECT_TIMEOUT_MS", "1000");
    configure_sdk(&mut command, cli.arm, otlp, sentry, "telemetry-gateway");
    if cli.arm == Arm::Enabled {
        command
            .env(
                "INDRA_TIKA_OTLP_ENDPOINT",
                format!("http://{otlp}/v1/traces"),
            )
            .env("INDRA_TIKA_TRACE_SAMPLE_RATIO", "1.0")
            .env("INDRA_TIKA_EXPORT_INTERVAL_MS", "5000")
            .env("INDRA_TIKA_EXPORT_QUEUE", "256")
            .env("INDRA_TIKA_EXPORT_BATCH", "16")
            .env("INDRA_TIKA_EXPORT_TIMEOUT_MS", "2000");
    }
    command.arg("serve");
    let mut gateway = Process::start(command, "gateway", &cli.output)?;
    ready(gateway_address, "/readyz", &mut gateway).await?;
    let version = ready(tika_address, "/version", &mut gateway).await?;
    check(
        std::str::from_utf8(&version.bytes[..version.used])
            .ok()
            .is_some_and(|s| s.trim() == "Apache Tika 3.3.2"),
        "Tika version differs",
    )?;
    let mut goldens: [http::Reply; 3] = std::array::from_fn(|_| http::Reply {
        status: 200,
        bytes: [0; 16384],
        used: 0,
    });
    for index in 0..3 {
        let reply =
            http::request(gateway_address, "/tika", FIXTURES[index], MIMES[index], &[]).await?;
        check(
            reply.status == 200 && reply.used > 0,
            "warmup/reference OCR failed",
        )?;
        if cli.arm == Arm::Reference {
            write_new(
                &cli.output.join(format!("golden-{}.txt", NAMES[index])),
                &reply.bytes[..reply.used],
            )?;
            goldens[index] = reply;
        } else {
            let path = cli
                .goldens
                .as_ref()
                .ok_or(Failure::Gate("golden directory absent"))?
                .join(format!("golden-{}.txt", NAMES[index]));
            let mut f = File::open(path)?;
            let mut used = 0;
            loop {
                if used == 16384 {
                    let mut extra = [0_u8; 1];
                    check(f.read(&mut extra)? == 0, "golden body cap")?;
                    break;
                }
                let n = f.read(&mut goldens[index].bytes[used..])?;
                if n == 0 {
                    break;
                }
                used += n;
            }
            goldens[index].used = used;
            check(
                reply.bytes[..reply.used] == goldens[index].bytes[..used],
                "selected warmup differs from off golden",
            )?;
        }
    }
    if cli.arm == Arm::Reference {
        summary.gateway_exit = gateway.shutdown(Duration::from_secs(20)).await?;
        summary.native_exit = native.shutdown(Duration::from_secs(20)).await?;
        check(
            summary.gateway_exit == Some(0) && summary.native_exit == Some(0),
            "reference cleanup failed",
        )?;
        snapshot(cli, "cleanup-after")?;
        return Ok(());
    }
    snapshot(cli, "cohort-before")?;
    let exports_before = audit::count_exports(&cli.output)?;
    let started = Instant::now();
    let next = AtomicUsize::new(0);
    let (a, b) = tokio::join!(
        worker(gateway_address, &next, &goldens, outcomes),
        worker(gateway_address, &next, &goldens, outcomes)
    );
    let elapsed = started.elapsed();
    summary.cohort_exports_observed = Some(
        audit::count_exports(&cli.output)?
            .checked_sub(exports_before)
            .ok_or(Failure::Gate("collector journal count decreased"))?,
    );
    snapshot(cli, "cohort-after")?;
    a?;
    b?;
    summary.wall_us = Some(elapsed.as_micros());
    let mut latencies = [0_u128; 12];
    for record in outcomes {
        let record = record
            .lock()
            .map_err(|_| Failure::Gate("outcome mutex poisoned"))?;
        if record.status == "success" {
            latencies[summary.successful_requests] = record
                .elapsed_us
                .ok_or(Failure::Gate("success timing absent"))?;
            summary.successful_requests += 1;
        }
    }
    check(
        summary.successful_requests == 12,
        "failed or missing success-cohort request",
    )?;
    latencies.sort_unstable();
    summary.median_us = Some(latencies[5]);
    summary.successful_responses_per_wall_second = Some(12.0 / elapsed.as_secs_f64());
    snapshot(cli, "fault-before")?;
    let fault_started = Instant::now();
    let parent = gateway.child.id();
    let children = fs::read_to_string(format!("/proc/{parent}/task/{parent}/children"))?;
    check(children.len() <= 128, "watchdog child inventory cap")?;
    let mut ids = children.split_ascii_whitespace();
    let child: u32 = ids
        .next()
        .ok_or(Failure::Gate("Java watchdog child absent"))?
        .parse()
        .map_err(|_| Failure::Gate("invalid Java watchdog PID"))?;
    check(
        ids.next().is_none(),
        "gateway has ambiguous direct children",
    )?;
    let pid = Pid::from_raw(i32::try_from(child).map_err(|_| Failure::Gate("Java PID range"))?)
        .ok_or(Failure::Gate("zero Java PID"))?;
    let status = fs::read_to_string(format!("/proc/{child}/status"))?;
    check(
        status
            .lines()
            .find_map(|l| l.strip_prefix("PPid:"))
            .and_then(|s| s.trim().parse::<u32>().ok())
            == Some(parent),
        "fault PID is not gateway owned",
    )?;
    let group = process::getpgid(Some(pid)).map_err(io::Error::from)?;
    check(group == pid, "Java fault process group differs")?;
    summary.fault_owned_pid = Some(child);
    summary.fault_owned_group = Some(child);
    write_new(
        &cli.output.join("fault-owned-java.status"),
        status.as_bytes(),
    )?;
    process::kill_process(pid, Signal::TERM).map_err(io::Error::from)?;
    let due = Instant::now() + Duration::from_secs(35);
    loop {
        if let Some(status) = gateway.child.try_wait()? {
            summary.gateway_exit = status.code();
            break;
        }
        check(Instant::now() < due, "faulted gateway exit deadline")?;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    check(
        summary.gateway_exit.is_some_and(|code| code != 0),
        "controlled Java fault did not terminate gateway",
    )?;
    summary.native_exit = native.shutdown(Duration::from_secs(20)).await?;
    check(
        summary.native_exit == Some(0),
        "native clean shutdown failed",
    )?;
    summary.fault_wall_us = Some(fault_started.elapsed().as_micros());
    snapshot(cli, "fault-after")?;
    snapshot(cli, "cleanup-after")?;
    Ok(())
}
fn configure_sdk(
    command: &mut Command,
    arm: Arm,
    otlp: SocketAddr,
    sentry: SocketAddr,
    service: &str,
) {
    command.args(["--service-name", service, "--log-filter", "info"]);
    if arm == Arm::Enabled {
        command.args([
            "--otlp-endpoint",
            &format!("http://{otlp}"),
            "--trace-sample-ratio",
            "1.0",
            "--export-queue-size",
            "256",
            "--export-batch-size",
            "16",
            "--export-interval-ms",
            "5000",
            "--export-timeout-ms",
            "2000",
            "--sentry-dsn",
            &format!("http://public@{sentry}/1"),
            "--sentry-sample-ratio",
            "1.0",
            "--sentry-queue-size",
            "16",
        ]);
    }
}
async fn worker(
    address: SocketAddr,
    next: &AtomicUsize,
    goldens: &[http::Reply; 3],
    outcomes: &[Mutex<Outcome>; 12],
) -> Result<(), Failure> {
    loop {
        let sequence = next.fetch_add(1, Ordering::Relaxed);
        if sequence >= 12 {
            return Ok(());
        }
        let fixture = INPUTS[sequence];
        let headers = [
            ("traceparent", REQUEST_PARENTS[sequence]),
            ("baggage", "secret=DOCUMENT_SECRET"),
            ("authorization", "Bearer SDK_PRIVATE_CANARY"),
            ("cookie", "session=COOKIE_PRIVATE_CANARY"),
        ];
        let started = Instant::now();
        let reply = http::request(
            address,
            "/tika",
            FIXTURES[fixture],
            MIMES[fixture],
            &headers,
        )
        .await;
        let mut outcome = Outcome::missing(sequence);
        outcome.elapsed_us = Some(started.elapsed().as_micros());
        match reply {
            Ok(reply) => {
                outcome.http_status = Some(reply.status);
                outcome.output_bytes = Some(reply.used);
                outcome.output_sha256 = Some(Sha256::digest(&reply.bytes[..reply.used]).into());
                outcome.status = if reply.status != 200 {
                    "http_failure"
                } else if reply.bytes[..reply.used]
                    != goldens[fixture].bytes[..goldens[fixture].used]
                {
                    "golden_mismatch"
                } else {
                    "success"
                };
            }
            Err(error) => {
                outcome.status = "transport_or_deadline_failure";
                outcome.failure = Some(format!("{error:?}"));
            }
        }
        *outcomes[sequence]
            .lock()
            .map_err(|_| Failure::Gate("outcome mutex poisoned"))? = outcome;
    }
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
        record_asset(&mut file, role, path)?;
    }
    let directory = cli
        .jar
        .parent()
        .ok_or(Failure::Gate("JAR parent absent"))?
        .join("trace");
    let mut count = 0;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        check(
            count < 16 && entry.file_type()?.is_file(),
            "unexpected trace extension/dependency file",
        )?;
        record_asset(&mut file, "trace_dependency", &entry.path())?;
        count += 1;
    }
    check(count == 16, "trace dependency closure incomplete")?;
    Ok(())
}
fn record_asset(output: &mut File, role: &str, path: &Path) -> Result<(), Failure> {
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
        &mut *output,
        &Asset {
            role,
            path,
            bytes,
            sha256: format!("{:x}", hash.finalize()),
        },
    )?;
    output.write_all(b"\n")?;
    Ok(())
}
