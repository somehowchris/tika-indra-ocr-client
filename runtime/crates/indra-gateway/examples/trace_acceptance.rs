//! Actual gateway → forked Tika 3.3.2 → Rust CLI → native OCR → OTLP gate.
//! No synthetic OCR, collector mock SDK, per-request JVM or global trace env.

#[path = "trace_acceptance/capture.rs"]
mod capture;
#[path = "trace_acceptance/http.rs"]
mod http;

use clap::Parser;
use rustix::process::{self, Pid, Signal, WaitOptions};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener},
    os::unix::{fs::symlink, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

static FIXTURE: &[u8] = include_bytes!("../../../../backend/tests/data/two-pages.tiff");
static PARENTS: [&str; 4] = [
    "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
    "00-abcd2f3577b34da6a3ce929d0e0e4736-11f067aa0ba902b7-01",
    "00-cdef2f3577b34da6a3ce929d0e0e4736-22f067aa0ba902b7-01",
    "00-efab2f3577b34da6a3ce929d0e0e4736-33f067aa0ba902b7-01",
];

#[derive(Parser, Serialize)]
#[command(
    about = "Run actual bounded forked Tika/native OCR trace acceptance with a local OTLP sink"
)]
struct Cli {
    #[arg(long)]
    gateway: PathBuf,
    #[arg(long)]
    java: PathBuf,
    #[arg(long)]
    jar: PathBuf,
    #[arg(long)]
    bridge: PathBuf,
    #[arg(long)]
    native: PathBuf,
    #[arg(long)]
    models: PathBuf,
    #[arg(long)]
    evidence: PathBuf,
    /// Inherited CPU quota/affinity is recorded by the caller; this sets both native thread controls.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=2))]
    threads: u8,
}

#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error("trace acceptance: {0}")]
    Gate(&'static str),
    #[error("I/O: {0}")]
    Io(#[from] io::Error),
    #[error("HTTP transport: {0}")]
    Http(#[from] hyper::Error),
    #[error("HTTP construction: {0}")]
    Request(#[from] axum::http::Error),
    #[error("evidence JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("OTLP protobuf: {0}")]
    Decode(#[from] prost::DecodeError),
}

fn check(ok: bool, why: &'static str) -> Result<(), Failure> {
    if ok { Ok(()) } else { Err(Failure::Gate(why)) }
}

fn create_new(path: &Path) -> Result<File, Failure> {
    Ok(OpenOptions::new().write(true).create_new(true).open(path)?)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), Failure> {
    create_new(path)?.write_all(bytes)?;
    Ok(())
}

#[derive(Serialize)]
struct RequestEvidence {
    case: &'static str,
    status: u16,
    elapsed_us: u128,
    response_bytes: usize,
    response_sha256: String,
}

#[derive(Serialize)]
struct Evidence {
    schema_version: u8,
    gate: &'static str,
    stage: &'static str,
    passed: bool,
    failure: Option<String>,
    cpu_scope: &'static str,
    sdk_schedule_delay_ms: u64,
    native_ready: bool,
    tika_base_version_verified: bool,
    concurrent_pair_completed: bool,
    requests: [Option<RequestEvidence>; 7],
    gateway_exit: Option<i32>,
    native_exit: Option<i32>,
    orphan_reaping_complete: bool,
    graph_verified: bool,
}

fn main() -> Result<(), Failure> {
    let cli = Cli::parse();
    for path in [&cli.gateway, &cli.java, &cli.jar, &cli.bridge, &cli.native] {
        check(
            path.is_absolute() && path.is_file(),
            "executables/JAR must be explicit absolute regular-file paths",
        )?;
    }
    check(
        cli.evidence.is_absolute() && cli.models.is_absolute() && cli.models.is_dir(),
        "evidence/models must be explicit absolute paths",
    )?;
    fs::create_dir(&cli.evidence)?;
    let mut invocation = create_new(&cli.evidence.join("invocation.json"))?;
    serde_json::to_writer_pretty(&mut invocation, &cli)?;
    assets(&cli)?;
    process::set_child_subreaper(Pid::from_raw(1)).map_err(io::Error::from)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let collector_address = listener.local_addr()?;
    let stop = AtomicBool::new(false);
    let mut evidence = Evidence {
        schema_version: 1,
        gate: "actual_gateway_forked_tika_native_trace_graph",
        stage: "startup",
        passed: false,
        failure: None,
        cpu_scope: "native thread controls only; inherited whole-stack CPU/cgroup limits require separate caller evidence",
        sdk_schedule_delay_ms: 60000,
        native_ready: false,
        tika_base_version_verified: false,
        concurrent_pair_completed: false,
        requests: std::array::from_fn(|_| None),
        gateway_exit: None,
        native_exit: None,
        orphan_reaping_complete: false,
        graph_verified: false,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let outcome = std::thread::scope(|scope| {
        let collector = scope.spawn(|| capture::collect(listener, &stop, &cli.evidence));
        let outcome = runtime.block_on(run(&cli, collector_address, &mut evidence));
        // The production gateway deliberately owns a separate Java process
        // group. On failure, kill only children adopted by this subreaper.
        let forced_cleanup = if outcome.is_err() {
            kill_owned_children()
        } else {
            Ok(())
        };
        let cleanup = runtime.block_on(reap_all(Duration::from_secs(2)));
        stop.store(true, Ordering::Release);
        let captured = collector
            .join()
            .map_err(|_| Failure::Gate("collector thread panicked"))?;
        forced_cleanup?;
        check(cleanup?, "owned process cleanup did not finish")?;
        match captured {
            Ok(captured) => {
                captured.write(&cli.evidence.join("spans.json"))?;
                outcome?;
                evidence.stage = "trace_graph";
                captured.verify()?;
                evidence.graph_verified = true;
                Ok(())
            }
            Err(error) => Err(error),
        }
    });
    evidence.passed = outcome.is_ok();
    if evidence.passed {
        evidence.stage = "complete";
    }
    evidence.failure = outcome.as_ref().err().map(ToString::to_string);
    let mut report = create_new(&cli.evidence.join("trace-acceptance.json"))?;
    serde_json::to_writer_pretty(&mut report, &evidence)?;
    report.write_all(b"\n")?;
    outcome?;
    println!(
        "Actual gateway/Tika/CLI/native OTLP graph passed: {}",
        cli.evidence.display()
    );
    Ok(())
}

fn assets(cli: &Cli) -> Result<(), Failure> {
    #[derive(Serialize)]
    struct Asset<'a> {
        role: &'a str,
        path: &'a Path,
        bytes: u64,
        sha256: String,
    }
    let mut output = create_new(&cli.evidence.join("assets.jsonl"))?;
    let mut record = |role, path: &Path| -> Result<(), Failure> {
        let mut input = File::open(path)?;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 65_536];
        let mut bytes = 0_u64;
        loop {
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            bytes += n as u64;
            check(bytes <= 536_870_912, "asset exceeds fixed 512 MiB gate cap")?;
            digest.update(&buffer[..n]);
        }
        serde_json::to_writer(
            &mut output,
            &Asset {
                role,
                path,
                bytes,
                sha256: format!("{:x}", digest.finalize()),
            },
        )?;
        output.write_all(b"\n")?;
        Ok(())
    };
    for (role, path) in [
        ("gateway", &cli.gateway),
        ("java", &cli.java),
        ("patched_tika", &cli.jar),
        ("bridge", &cli.bridge),
        ("native", &cli.native),
    ] {
        record(role, path)?;
    }
    let directory = cli
        .jar
        .parent()
        .ok_or(Failure::Gate("jar parent"))?
        .join("trace");
    let mut count = 0;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        check(
            entry.file_type()?.is_file() && count < 16,
            "unexpected trace bundle entry",
        )?;
        record("trace_dependency_or_extension", &entry.path())?;
        count += 1;
    }
    check(count == 16, "incomplete trace dependency bundle")?;
    write_new(&cli.evidence.join("fixture.tiff"), FIXTURE)?;
    Ok(())
}

async fn run(cli: &Cli, collector: SocketAddr, evidence: &mut Evidence) -> Result<(), Failure> {
    let native_address = spare_address()?;
    let tika_address = spare_address()?;
    let gateway_address = spare_address()?;
    check(
        native_address != tika_address
            && native_address != collector
            && tika_address != collector
            && gateway_address != native_address
            && gateway_address != tika_address
            && gateway_address != collector,
        "ephemeral port collision",
    )?;
    let config_path = cli.evidence.join("tika-config.xml");
    write_config(cli, tika_address, &config_path)?;
    let mut native_command = Command::new(&cli.native);
    native_command
        .args(["--listen", &native_address.to_string(), "--model-dir"])
        .arg(&cli.models)
        .args([
            "--threads",
            &cli.threads.to_string(),
            "--cv-threads",
            &cli.threads.to_string(),
            "--otlp-endpoint",
            &format!("http://{collector}"),
            "--trace-sample-ratio",
            "1.0",
            "--export-interval-ms",
            "60000",
            "serve",
        ])
        .env_clear()
        .env("LANG", "C.UTF-8");
    let mut native = Process::start(native_command, "native", &cli.evidence)?;
    wait_ready(native_address, "/readyz", &mut native).await?;
    evidence.native_ready = true;
    let mut gateway_command = Command::new(&cli.gateway);
    gateway_command
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
        .arg(&config_path)
        .args([
            "--otlp-endpoint",
            &format!("http://{collector}"),
            "--trace-sample-ratio",
            "1.0",
            "--export-interval-ms",
            "60000",
            "--child-shutdown-seconds",
            "8",
            "serve",
        ])
        .env_clear()
        .env("LANG", "C.UTF-8")
        .env("INDRA_ADDR", native_address.to_string())
        .env(
            "INDRA_TIKA_OTLP_ENDPOINT",
            format!("http://{collector}/v1/traces"),
        )
        .env("INDRA_TIKA_EXPORT_INTERVAL_MS", "60000")
        .env("INDRA_TIKA_TRACE_SAMPLE_RATIO", "1.0");
    let mut gateway = Process::start(gateway_command, "gateway", &cli.evidence)?;
    wait_ready(gateway_address, "/readyz", &mut gateway).await?;
    let version = wait_ready(tika_address, "/version", &mut gateway).await?;
    check(
        std::str::from_utf8(&version.bytes[..version.used])
            .ok()
            .is_some_and(|body| body.trim() == "Apache Tika 3.3.2"),
        "real Tika base version differs",
    )?;
    evidence.tika_base_version_verified = true;
    evidence.stage = "ocr_requests";
    let first = [
        ("traceparent", PARENTS[0]),
        ("tracestate", "case=first"),
        ("baggage", "secret=DOCUMENT_SECRET"),
    ];
    let second = [
        ("traceparent", PARENTS[1]),
        ("tracestate", "case=second"),
        ("baggage", "secret=DOCUMENT_SECRET"),
    ];
    let timed = |headers| async move {
        let start = Instant::now();
        let reply = http::request(gateway_address, "/tika", FIXTURE, headers).await;
        (reply, start.elapsed())
    };
    let (a, b) = tokio::join!(timed(&first), timed(&second));
    record_reply(cli, evidence, 0, "concurrent_first", a, true)?;
    record_reply(cli, evidence, 1, "concurrent_second", b, true)?;
    evidence.concurrent_pair_completed = true;
    let partial = [
        ("traceparent", PARENTS[2]),
        ("tracestate", "duplicate=one,duplicate=two"),
    ];
    let absent = [("tracestate", "orphan=discard")];
    let invalid = [
        (
            "traceparent",
            "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
        ),
        ("tracestate", "orphan=discard"),
    ];
    let duplicate = [
        ("traceparent", PARENTS[0]),
        ("traceparent", PARENTS[1]),
        ("tracestate", "orphan=discard"),
    ];
    for (index, name, headers) in [
        (2, "invalid_state_keeps_parent", partial.as_slice()),
        (3, "missing_parent", absent.as_slice()),
        (4, "invalid_parent", invalid.as_slice()),
        (5, "duplicate_parent", duplicate.as_slice()),
    ] {
        record_reply(cli, evidence, index, name, timed(headers).await, true)?;
    }
    let error = [
        ("traceparent", PARENTS[3]),
        ("X-Tika-OCROutputType", "hocr"),
    ];
    record_reply(
        cli,
        evidence,
        6,
        "unsupported_cli_output",
        timed(&error).await,
        false,
    )?;
    evidence.stage = "shutdown_flush";
    // Signal the actual public gateway. It must drain accepted work, terminate
    // Java, permit child SDK shutdown hooks, then flush its own exporters.
    evidence.gateway_exit = gateway.shutdown(Duration::from_secs(15)).await?;
    check(
        evidence.gateway_exit == Some(0),
        "gateway did not drain, flush and reap Java normally",
    )?;
    evidence.native_exit = native.shutdown(Duration::from_secs(10)).await?;
    check(
        evidence.native_exit == Some(0),
        "native service did not drain/flush successfully",
    )?;
    evidence.orphan_reaping_complete = reap_all(Duration::from_secs(2)).await?;
    check(
        evidence.orphan_reaping_complete,
        "orphaned process remained after normal shutdown",
    )?;
    Ok(())
}

fn record_reply(
    cli: &Cli,
    evidence: &mut Evidence,
    index: usize,
    name: &'static str,
    result: (Result<http::Reply, Failure>, Duration),
    success: bool,
) -> Result<(), Failure> {
    let reply = result.0?;
    let body = &reply.bytes[..reply.used];
    write_new(
        &cli.evidence.join(format!("response-{index}-{name}.txt")),
        body,
    )?;
    evidence.requests[index] = Some(RequestEvidence {
        case: name,
        status: reply.status,
        elapsed_us: result.1.as_micros(),
        response_bytes: reply.used,
        response_sha256: format!("{:x}", Sha256::digest(body)),
    });
    if success {
        check(reply.status == 200, "real Tika OCR request failed")?;
        let text = std::str::from_utf8(body).map_err(|_| Failure::Gate("non-UTF-8 Tika output"))?;
        check(
            text.split(['\r', '\n', '\u{000c}'])
                .filter(|line| !line.is_empty())
                .eq(["INVOICE 12345", "SECOND PAGE 67890"]),
            "real two-page Tika output differs",
        )?;
        if index > 0 {
            let baseline = evidence.requests[0]
                .as_ref()
                .ok_or(Failure::Gate("missing initial response"))?;
            let current = evidence.requests[index]
                .as_ref()
                .ok_or(Failure::Gate("missing current response"))?;
            check(
                current.response_bytes == baseline.response_bytes
                    && current.response_sha256 == baseline.response_sha256,
                "trace metadata changed exact Tika output bytes",
            )?;
        }
    } else {
        check(
            reply.status == 422,
            "unsupported OCR option must retain Tika's unprocessable-document status",
        )?;
    }
    Ok(())
}

fn spare_address() -> Result<SocketAddr, Failure> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?)
}

async fn wait_ready(
    address: SocketAddr,
    path: &'static str,
    process: &mut Process,
) -> Result<http::Reply, Failure> {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        check(
            Instant::now() < deadline && process.child.try_wait()?.is_none(),
            "child startup failed or exceeded deadline",
        )?;
        if let Ok(Ok(reply)) = tokio::time::timeout(
            Duration::from_secs(1),
            http::request(address, path, b"", &[]),
        )
        .await
            && reply.status == 200
        {
            return Ok(reply);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

struct Process {
    child: Child,
    group: Pid,
    armed: bool,
}

impl Process {
    fn start(mut command: Command, name: &'static str, output: &Path) -> Result<Self, Failure> {
        let child = command
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(create_new(&output.join(format!("{name}.stdout")))?)
            .stderr(create_new(&output.join(format!("{name}.stderr")))?)
            .spawn()?;
        let group = Pid::from_raw(child.id() as i32).ok_or(Failure::Gate("invalid child PID"))?;
        Ok(Self {
            child,
            group,
            armed: true,
        })
    }

    async fn shutdown(&mut self, grace: Duration) -> Result<Option<i32>, Failure> {
        process::kill_process(self.group, Signal::TERM).map_err(io::Error::from)?;
        let deadline = Instant::now() + grace;
        loop {
            if let Some(status) = self.child.try_wait()? {
                check(
                    process::test_kill_process_group(self.group) == Err(rustix::io::Errno::SRCH),
                    "descendant remains after its watchdog exits",
                )?;
                self.armed = false;
                return Ok(status.code());
            }
            check(Instant::now() < deadline, "process shutdown deadline")?;
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if self.armed {
            let _ = process::kill_process_group(self.group, Signal::KILL);
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                match self.child.try_wait() {
                    Ok(Some(_)) | Err(_) => break,
                    Ok(None) => std::thread::sleep(Duration::from_millis(2)),
                }
            }
        }
    }
}

fn kill_owned_children() -> Result<(), Failure> {
    let pid = std::process::id();
    let children = format!("/proc/self/task/{pid}/children");
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let input = fs::read_to_string(&children)?;
        check(input.len() <= 8192, "owned child PID inventory cap")?;
        if input.trim().is_empty() {
            return Ok(());
        }
        for raw in input.split_ascii_whitespace() {
            let raw: i32 = raw
                .parse()
                .map_err(|_| Failure::Gate("invalid adopted child PID"))?;
            let child = Pid::from_raw(raw).ok_or(Failure::Gate("zero adopted child PID"))?;
            match process::kill_process(child, Signal::KILL) {
                Ok(()) | Err(rustix::io::Errno::SRCH) => {}
                Err(error) => return Err(io::Error::from(error).into()),
            }
        }
        while matches!(process::wait(WaitOptions::NOHANG), Ok(Some(_))) {}
        std::thread::sleep(Duration::from_millis(2));
    }
    Err(Failure::Gate("owned child cleanup deadline"))
}

async fn reap_all(grace: Duration) -> Result<bool, Failure> {
    let deadline = Instant::now() + grace;
    loop {
        match process::wait(WaitOptions::NOHANG) {
            Err(rustix::io::Errno::CHILD) => return Ok(true),
            Err(error) => return Err(io::Error::from(error).into()),
            Ok(_) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(2)).await
            }
            Ok(_) => return Ok(false),
        }
    }
}

fn write_config(cli: &Cli, address: SocketAddr, destination: &Path) -> Result<(), Failure> {
    let bridge_dir = cli.evidence.join("bridge");
    fs::create_dir(&bridge_dir)?;
    symlink(&cli.bridge, bridge_dir.join("tesseract"))?;
    let java = xml(&cli.java)?;
    let bridge = xml(&bridge_dir)?;
    let mut output = create_new(destination)?;
    write!(
        output,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<properties>
  <server><host>127.0.0.1</host><port>{port}</port><noFork>false</noFork>
    <javaPath>{java}</javaPath><taskTimeoutMillis>120000</taskTimeoutMillis>
    <minimumTimeoutMillis>1000</minimumTimeoutMillis><taskPulseMillis>1000</taskPulseMillis>
    <maxForkedStartupMillis>120000</maxForkedStartupMillis><maxFiles>10000</maxFiles>
    <forkedJvmArgs><arg>-Xms64m</arg><arg>-Xmx512m</arg><arg>-XX:ActiveProcessorCount={threads}</arg>
      <arg>-XX:+UseSerialGC</arg><arg>-Djava.awt.headless=true</arg></forkedJvmArgs>
    <endpoints><endpoint>tika</endpoint><endpoint>version</endpoint></endpoints>
  </server>
  <parsers>
    <parser class="io.indra.tika.TracedParser"><parser class="org.apache.tika.parser.DefaultParser">
      <parser-exclude class="org.apache.tika.parser.pdf.PDFParser"/>
      <parser-exclude class="org.apache.tika.parser.ocr.TesseractOCRParser"/>
    </parser></parser>
    <parser class="io.indra.tika.TracedParser"><parser class="org.apache.tika.parser.pdf.PDFParser"><params>
      <param name="ocrStrategy" type="string">ocr_only</param><param name="ocrStrategyAuto" type="string">better</param>
      <param name="ocrRenderingStrategy" type="string">all</param><param name="ocrDPI" type="int">300</param>
      <param name="ocrImageType" type="string">gray</param><param name="ocrImageFormatName" type="string">png</param>
      <param name="extractInlineImages" type="bool">false</param><param name="catchIntermediateExceptions" type="bool">false</param>
    </params></parser></parser>
    <parser class="io.indra.tika.TracedParser"><parser class="org.apache.tika.parser.ocr.TesseractOCRParser"><params>
      <param name="tesseractPath" type="string">{bridge}/</param><param name="preloadLangs" type="bool">false</param>
      <param name="language" type="string"></param><param name="pageSegMode" type="string">3</param>
      <param name="outputType" type="string">txt</param><param name="timeout" type="int">60</param>
      <param name="enableImagePreprocessing" type="bool">false</param><param name="applyRotation" type="bool">false</param>
      <param name="preserveInterwordSpacing" type="bool">false</param>
    </params></parser></parser>
  </parsers>
</properties>
"#,
        port = address.port(),
        threads = cli.threads
    )?;
    Ok(())
}

fn xml(path: &Path) -> Result<String, Failure> {
    let path = path
        .to_str()
        .ok_or(Failure::Gate("test paths must be UTF-8"))?;
    check(
        path.len() <= 2048 && !path.chars().any(|c| c.is_control()),
        "test path is too long or contains controls",
    )?;
    let mut escaped = String::with_capacity(path.len());
    for character in path.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            other => escaped.push(other),
        }
    }
    Ok(escaped)
}
