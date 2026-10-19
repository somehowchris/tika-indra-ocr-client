//! Destructive acceptance gate for a caller-created, otherwise idle real stack.
//! Only the explicitly named Compose project or Kubernetes pod is stopped.
//! No shell, model substitute, OCR retry or synthesized successful response.

use axum::http::{Method, Request, StatusCode, header};
use bytes::Bytes;
use clap::{Parser, Subcommand};
use http_body::{Body, Frame, SizeHint};
use http_body_util::BodyExt;
use hyper_util::rt::TokioIo;
use miette::Diagnostic;
use rustix::process::{Pid, Signal};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    convert::Infallible,
    fs::{File, OpenOptions},
    future::Future,
    io::{self, Write},
    net::SocketAddr,
    os::unix::process::CommandExt,
    path::PathBuf,
    pin::Pin,
    process::{Child, Command, ExitStatus, Stdio},
    task::{Context, Poll},
    time::Duration,
};
use thiserror::Error;
use tokio::{net::TcpStream, sync::oneshot, time::Instant};

static FIXTURE: &[u8] = include_bytes!("../../../../backend/tests/data/two-pages.tiff");
static FIXTURE_SHA256: &str = "303ebb45259b96fecc0b200edf8e0a68ed50e2b43f441bceef8694e43699c329";
static PAGE_TEXT: [&str; 2] = ["INVOICE 12345", "SECOND PAGE 67890"];

#[derive(Parser)]
#[command(about = "Verify real Tika/native OCR completion across whole-stack shutdown")]
struct Cli {
    /// Direct address of the specific gateway; do not use a load-balanced Service.
    #[arg(long)]
    endpoint: SocketAddr,
    /// New JSON evidence file; an existing file is never overwritten.
    #[arg(long)]
    output: PathBuf,
    /// Caller-enforced CPU limit, recorded as supplied; this is not a quota audit.
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=2))]
    cpus: u8,
    /// Absolute deadline for each complete OCR HTTP exchange.
    #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..=1800))]
    request_timeout_seconds: u64,
    /// Must leave time within the gateway's configured upload deadline.
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u64).range(1..=60))]
    drain_observation_seconds: u64,
    /// Deadline for the explicit orchestration command to finish normally.
    #[arg(long, default_value_t = 600, value_parser = clap::value_parser!(u64).range(10..=3600))]
    completion_timeout_seconds: u64,
    #[command(subcommand)]
    stop: Stop,
}

#[derive(Subcommand, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
enum Stop {
    /// Run docker compose stop, retaining containers and volumes for inspection.
    Compose {
        #[arg(long)]
        project: String,
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        override_file: Option<PathBuf>,
        #[arg(long)]
        second_override_file: Option<PathBuf>,
        #[arg(long)]
        docker_context: Option<String>,
    },
    /// Delete exactly this pod with its existing termination grace period.
    Kubernetes {
        #[arg(long)]
        context: String,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        pod: String,
    },
}

#[derive(Debug, Error, Diagnostic)]
enum Failure {
    #[error("{0}")]
    Gate(&'static str),
    #[error("{stage}: {source}")]
    Io {
        stage: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("HTTP protocol failed: {0}")]
    Http(#[from] hyper::Error),
    #[error("invalid gateway status JSON: {0}")]
    Json(#[from] serde_json::Error),
}

fn io_failure(stage: &'static str, source: io::Error) -> Failure {
    Failure::Io { stage, source }
}

#[derive(Default)]
struct Evidence {
    stage: &'static str,
    version_verified: bool,
    native_dependency_ready: bool,
    baseline: Option<Reply>,
    during_shutdown: Option<Reply>,
    admitted: bool,
    readiness_removed_ms: Option<u128>,
    new_request_rejected: bool,
    command_completed: bool,
    command_exit_code: Option<i32>,
    shutdown_total_ms: Option<u128>,
}

#[derive(Serialize)]
struct Report<'a> {
    schema_version: u8,
    gate: &'static str,
    passed: bool,
    stage: &'static str,
    failure: Option<&'a str>,
    endpoint: SocketAddr,
    stop: &'a Stop,
    caller_supplied_cpu_limit: u8,
    quota_verification: &'static str,
    runtime_identity: &'static str,
    fixture_sha256: &'a str,
    fixture_bytes: usize,
    expected_page_text: &'static [&'static str; 2],
    initial_text_policy: &'static str,
    tika_3_3_2_verified: bool,
    native_dependency_ready: bool,
    baseline_body_sha256: Option<&'a str>,
    baseline_body: Option<&'a str>,
    admitted_before_shutdown: bool,
    readiness_removed_ms: Option<u128>,
    new_request_rejected: bool,
    shutdown_body_sha256: Option<&'a str>,
    shutdown_body: Option<&'a str>,
    body_bytes_equal: bool,
    command_completed: bool,
    command_exit_code: Option<i32>,
    shutdown_total_ms: Option<u128>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> miette::Result<()> {
    let cli = Cli::parse();
    validate(&cli)?;
    // Reserve the evidence destination before any remote shutdown mutation.
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&cli.output)
        .map_err(|error| io_failure("creating new evidence file", error))?;
    let mut evidence = Evidence {
        stage: "preflight",
        ..Evidence::default()
    };
    let outcome = run(&cli, &mut evidence).await;
    write_report(&mut output, &cli, &evidence, outcome.as_ref().err())?;
    outcome?;
    eprintln!(
        "Real stack shutdown gate passed; evidence: {}",
        cli.output.display()
    );
    Ok(())
}

fn validate(cli: &Cli) -> Result<(), Failure> {
    if !cli.endpoint.is_ipv4() || cli.endpoint.ip().is_unspecified() || cli.endpoint.port() == 0 {
        return Err(Failure::Gate(
            "endpoint must identify one direct, nonzero IPv4 gateway address",
        ));
    }
    if cli.completion_timeout_seconds <= cli.request_timeout_seconds
        || cli.request_timeout_seconds <= cli.drain_observation_seconds + 3
    {
        return Err(Failure::Gate(
            "completion must exceed request timeout, and request timeout must exceed drain observation + 3 seconds",
        ));
    }
    match &cli.stop {
        Stop::Compose {
            project,
            file,
            override_file,
            second_override_file,
            docker_context,
        } => {
            if !identifier(project, 63, false) {
                return Err(Failure::Gate(
                    "Compose project must be 1..63 lowercase letters, digits, dashes or underscores",
                ));
            }
            for file in [
                Some(file),
                override_file.as_ref(),
                second_override_file.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                if !file.is_file() {
                    return Err(Failure::Gate(
                        "each explicit Compose file must exist as a regular file",
                    ));
                }
            }
            if docker_context
                .as_deref()
                .is_some_and(|value| !context_name(value))
            {
                return Err(Failure::Gate(
                    "Docker context name is empty, oversized or contains control characters",
                ));
            }
        }
        Stop::Kubernetes {
            context,
            namespace,
            pod,
        } => {
            if !context_name(context)
                || !identifier(namespace, 63, true)
                || !identifier(pod, 253, true)
            {
                return Err(Failure::Gate(
                    "supply an explicit context and bounded Kubernetes namespace/pod names",
                ));
            }
        }
    }
    Ok(())
}

fn identifier(value: &str, max: usize, kubernetes: bool) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || byte == b'-'
                || (!kubernetes && byte == b'_')
                || (kubernetes && byte == b'.')
        })
        && (!kubernetes || value.as_bytes()[value.len() - 1].is_ascii_alphanumeric())
}

fn context_name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 253 && !value.chars().any(char::is_control)
}

fn stop_command(stop: &Stop, timeout_seconds: u64) -> Command {
    let mut command = match stop {
        Stop::Compose {
            project,
            file,
            override_file,
            second_override_file,
            docker_context,
        } => {
            let mut command = Command::new("docker");
            if let Some(context) = docker_context {
                command.arg("--context").arg(context);
            }
            command.args(["compose", "--project-name", project]);
            for file in [
                Some(file),
                override_file.as_ref(),
                second_override_file.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                command.arg("--file").arg(file);
            }
            // Preserve each service's configured stop_grace_period and Compose's
            // reverse dependency ordering. Do not remove volumes or containers.
            command.arg("stop");
            command
        }
        Stop::Kubernetes {
            context,
            namespace,
            pod,
        } => {
            let mut command = Command::new("kubectl");
            command.args([
                "--context",
                context,
                "--namespace",
                namespace,
                "--request-timeout=10s",
                "delete",
                "pod",
                pod,
                "--wait=true",
            ]);
            command.arg(format!("--timeout={timeout_seconds}s"));
            // No --force or grace override: exercise the actual deployed budget.
            command
        }
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .process_group(0);
    command
}

struct StopProcess {
    child: Child,
    finished: bool,
}

impl StopProcess {
    fn start(cli: &Cli) -> Result<Self, Failure> {
        Ok(Self {
            child: stop_command(&cli.stop, cli.completion_timeout_seconds)
                .spawn()
                .map_err(|error| io_failure("starting explicit shutdown command", error))?,
            finished: false,
        })
    }

    fn status(&mut self) -> Result<Option<ExitStatus>, Failure> {
        let status = self
            .child
            .try_wait()
            .map_err(|error| io_failure("reaping shutdown command", error))?;
        self.finished = status.is_some();
        Ok(status)
    }
}

impl Drop for StopProcess {
    fn drop(&mut self) {
        if !self.finished {
            if let Some(pid) = Pid::from_raw(self.child.id() as i32) {
                let _ = rustix::process::kill_process_group(pid, Signal::KILL);
            }
            // This reaps only our command group. Docker/API-side actions already
            // requested may continue; the caller owns remaining stack cleanup.
            let due = std::time::Instant::now() + Duration::from_secs(2);
            while std::time::Instant::now() < due {
                if self.child.try_wait().ok().flatten().is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

async fn run(cli: &Cli, evidence: &mut Evidence) -> Result<(), Failure> {
    if hash(FIXTURE).as_slice() != FIXTURE_SHA256.as_bytes() {
        return Err(Failure::Gate(
            "compiled two-page TIFF differs from its frozen SHA-256",
        ));
    }
    let initial = probe(cli.endpoint).await?;
    let initial_status = initial.status;
    let initial = status(&initial)?;
    if initial_status != StatusCode::OK
        || initial.phase != "ready"
        || initial.active != 0
        || initial.ocr_ready != Some(true)
    {
        return Err(Failure::Gate(
            "use an idle, ready gateway with the native OCR readiness dependency explicitly enabled",
        ));
    }
    evidence.native_dependency_ready = true;
    let version = request(
        cli.endpoint,
        Method::GET,
        "/version",
        RequestBody::Empty,
        seconds(2),
    )
    .await?;
    if version.status != StatusCode::OK || version.text()?.trim() != "Apache Tika 3.3.2" {
        return Err(Failure::Gate(
            "public gateway did not serve exact Apache Tika 3.3.2 version",
        ));
    }
    evidence.version_verified = true;
    evidence.stage = "initial_real_tika_ocr";
    let baseline = request(
        cli.endpoint,
        Method::PUT,
        "/tika",
        RequestBody::Complete(false),
        seconds(cli.request_timeout_seconds),
    )
    .await?;
    evidence.baseline = Some(baseline);
    verify_text(evidence.baseline.as_ref().expect("stored baseline"))?;
    let empty_due = seconds(3);
    loop {
        let response = probe(cli.endpoint).await?;
        let ready = status(&response)?;
        if response.status == StatusCode::OK
            && ready.phase == "ready"
            && ready.active == 0
            && ready.ocr_ready == Some(true)
        {
            break;
        }
        if Instant::now() >= empty_due {
            return Err(Failure::Gate(
                "initial response did not release gateway capacity",
            ));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    evidence.stage = "held_upload_admission";
    let (release, hold) = oneshot::channel();
    let endpoint = cli.endpoint;
    let due = seconds(cli.request_timeout_seconds);
    // Exactly one document task; Hyper drives both response and held request.
    let mut accepted = tokio::spawn(request(
        endpoint,
        Method::PUT,
        "/tika",
        RequestBody::Held { stage: 0, hold },
        due,
    ));
    let admitted_due = seconds(3);
    loop {
        let response = probe(cli.endpoint).await?;
        let ready = status(&response)?;
        if response.status == StatusCode::OK
            && ready.phase == "ready"
            && ready.active == 1
            && ready.ocr_ready == Some(true)
        {
            evidence.admitted = true;
            break;
        }
        if accepted.is_finished() || Instant::now() >= admitted_due {
            accepted.abort();
            return Err(Failure::Gate(
                "held TIFF was not observably admitted before shutdown",
            ));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    evidence.stage = "orchestrator_shutdown";
    let started = Instant::now();
    let mut shutdown = StopProcess::start(cli)?;
    let withdrawn_due = seconds(cli.drain_observation_seconds);
    loop {
        if shutdown.status()?.is_some() {
            return Err(Failure::Gate(
                "shutdown command ended before the admitted document could complete",
            ));
        }
        // A connection error is not proof of graceful drain. Require a complete
        // explicit 503/draining reply from this same gateway while it holds work.
        if let Ok(response) = probe(cli.endpoint).await
            && response.status == StatusCode::SERVICE_UNAVAILABLE
        {
            let draining = status(&response)?;
            if draining.phase == "draining" && draining.active == 1 {
                evidence.readiness_removed_ms = Some(started.elapsed().as_millis());
                break;
            }
        }
        if accepted.is_finished() || Instant::now() >= withdrawn_due {
            return Err(Failure::Gate(
                "gateway did not report draining with the accepted upload retained",
            ));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let rejected = request(
        cli.endpoint,
        Method::PUT,
        "/tika",
        RequestBody::Empty,
        seconds(1),
    )
    .await?;
    #[derive(Deserialize)]
    struct Rejection<'a> {
        code: &'a str,
    }
    if rejected.status != StatusCode::SERVICE_UNAVAILABLE
        || !serde_json::from_slice::<Rejection<'_>>(rejected.body())
            .is_ok_and(|error| error.code == "unavailable")
    {
        return Err(Failure::Gate(
            "new document admission was not rejected during drain",
        ));
    }
    evidence.new_request_rejected = true;
    evidence.stage = "accepted_real_tika_ocr_after_shutdown";
    release
        .send(())
        .map_err(|_| Failure::Gate("accepted upload disappeared before it was released"))?;
    let response = (&mut accepted)
        .await
        .map_err(|_| Failure::Gate("accepted HTTP task failed"))??;
    let equal = evidence
        .baseline
        .as_ref()
        .is_some_and(|baseline| baseline.body() == response.body());
    evidence.during_shutdown = Some(response);
    verify_text(
        evidence
            .during_shutdown
            .as_ref()
            .expect("stored shutdown response"),
    )?;
    if !equal {
        return Err(Failure::Gate(
            "shutdown response differs byte-for-byte from the initial real Tika response",
        ));
    }
    evidence.stage = "shutdown_completion";
    let completed_due = started + Duration::from_secs(cli.completion_timeout_seconds);
    loop {
        if let Some(status) = shutdown.status()? {
            evidence.command_completed = true;
            evidence.command_exit_code = status.code();
            if !status.success() {
                return Err(Failure::Gate(
                    "orchestration command returned a failing exit status",
                ));
            }
            break;
        }
        if Instant::now() >= completed_due {
            return Err(Failure::Gate(
                "orchestration command exceeded its completion deadline",
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    evidence.shutdown_total_ms = Some(started.elapsed().as_millis());
    evidence.stage = "complete";
    Ok(())
}

fn seconds(seconds: u64) -> Instant {
    Instant::now() + Duration::from_secs(seconds)
}

#[derive(Deserialize)]
struct GatewayStatus<'a> {
    phase: &'a str,
    active: u16,
    ocr_ready: Option<bool>,
}

fn status(reply: &Reply) -> Result<GatewayStatus<'_>, Failure> {
    Ok(serde_json::from_slice(reply.body())?)
}

async fn probe(endpoint: SocketAddr) -> Result<Reply, Failure> {
    request(
        endpoint,
        Method::GET,
        "/readyz",
        RequestBody::Empty,
        Instant::now() + Duration::from_millis(750),
    )
    .await
}

enum RequestBody {
    Empty,
    Complete(bool),
    Held {
        stage: u8,
        hold: oneshot::Receiver<()>,
    },
}

impl Body for RequestBody {
    type Data = Bytes;
    type Error = Infallible;
    fn poll_frame(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        match self.get_mut() {
            Self::Empty => Poll::Ready(None),
            Self::Complete(sent) => {
                if *sent {
                    return Poll::Ready(None);
                }
                *sent = true;
                Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(FIXTURE)))))
            }
            Self::Held { stage, hold } => match *stage {
                0 => {
                    *stage = 1;
                    Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(&FIXTURE[..1])))))
                }
                1 => match Pin::new(hold).poll(context) {
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(Ok(())) => {
                        *stage = 2;
                        Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(&FIXTURE[1..])))))
                    }
                    Poll::Ready(Err(_)) => {
                        *stage = 2;
                        Poll::Ready(None)
                    }
                },
                _ => Poll::Ready(None),
            },
        }
    }
    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(match self {
            Self::Empty | Self::Complete(true) => 0,
            Self::Complete(false) | Self::Held { stage: 0, .. } => FIXTURE.len() as u64,
            Self::Held { stage: 1, .. } => (FIXTURE.len() - 1) as u64,
            Self::Held { .. } => 0,
        })
    }
}

struct Reply {
    status: StatusCode,
    text_plain: bool,
    bytes: [u8; 8192],
    used: usize,
}

impl Reply {
    fn body(&self) -> &[u8] {
        &self.bytes[..self.used]
    }
    fn text(&self) -> Result<&str, Failure> {
        std::str::from_utf8(self.body()).map_err(|_| Failure::Gate("response is not valid UTF-8"))
    }
}

async fn request(
    endpoint: SocketAddr,
    method: Method,
    path: &'static str,
    body: RequestBody,
    due: Instant,
) -> Result<Reply, Failure> {
    tokio::time::timeout_at(due, async {
        let socket = TcpStream::connect(endpoint)
            .await
            .map_err(|error| io_failure("connecting to gateway", error))?;
        socket
            .set_nodelay(true)
            .map_err(|error| io_failure("configuring gateway socket", error))?;
        let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
            .max_buf_size(16 * 1024)
            .handshake(TokioIo::new(socket))
            .await?;
        let exchange = async move {
            let request = Request::builder()
                .method(method)
                .uri(path)
                .header(header::HOST, endpoint.to_string())
                .header(header::CONNECTION, "close")
                .header(
                    header::CONTENT_LENGTH,
                    body.size_hint().exact().expect("fixed fixture length"),
                )
                .header(header::CONTENT_TYPE, "image/tiff")
                .header(header::ACCEPT, "text/plain")
                .body(body)
                .expect("fixed acceptance request");
            let mut response = sender.send_request(request).await?;
            drop(sender);
            let mut output = Reply {
                status: response.status(),
                text_plain: response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|header| header.to_str().ok())
                    .is_some_and(|value| {
                        value
                            .split(';')
                            .next()
                            .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/plain"))
                    }),
                bytes: [0; 8192],
                used: 0,
            };
            while let Some(frame) = response.body_mut().frame().await {
                let bytes = frame?
                    .into_data()
                    .map_err(|_| Failure::Gate("unexpected response trailers"))?;
                if bytes.len() > output.bytes.len() - output.used {
                    return Err(Failure::Gate("acceptance response exceeds 8192-byte bound"));
                }
                output.bytes[output.used..output.used + bytes.len()].copy_from_slice(&bytes);
                output.used += bytes.len();
            }
            Ok(output)
        };
        let (_, output) =
            tokio::try_join!(async { connection.await.map_err(Failure::Http) }, exchange)?;
        Ok(output)
    })
    .await
    .map_err(|_| Failure::Gate("HTTP exchange exceeded its absolute deadline"))?
}

fn verify_text(reply: &Reply) -> Result<(), Failure> {
    if reply.status != StatusCode::OK || !reply.text_plain {
        return Err(Failure::Gate(
            "real Tika OCR did not return HTTP 200 text/plain",
        ));
    }
    // Tika's text handler may add empty paragraph lines around OCR text. Require
    // both exact, ordered nonempty lines and nothing else; preserve every byte
    // for the subsequent comparison, including form feed and outer newlines.
    let mut lines = reply
        .text()?
        .split(['\r', '\n', '\u{c}'])
        .filter(|line| !line.is_empty());
    if lines.next() != Some(PAGE_TEXT[0])
        || lines.next() != Some(PAGE_TEXT[1])
        || lines.next().is_some()
    {
        return Err(Failure::Gate(
            "Tika output must contain both exact, distinct page texts in order without additional text",
        ));
    }
    Ok(())
}

fn hash(bytes: &[u8]) -> [u8; 64] {
    static HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut output = [0; 64];
    for (pair, byte) in output.as_chunks_mut::<2>().0.iter_mut().zip(digest) {
        pair[0] = HEX[usize::from(byte >> 4)];
        pair[1] = HEX[usize::from(byte & 15)];
    }
    output
}

fn ascii(value: &[u8]) -> &str {
    std::str::from_utf8(value).expect("ASCII hex")
}

fn write_report(
    output: &mut File,
    cli: &Cli,
    evidence: &Evidence,
    failure: Option<&Failure>,
) -> Result<(), Failure> {
    let fixture_hash = hash(FIXTURE);
    let initial_hash = evidence.baseline.as_ref().map(|reply| hash(reply.body()));
    let final_hash = evidence
        .during_shutdown
        .as_ref()
        .map(|reply| hash(reply.body()));
    let problem = failure.map(ToString::to_string);
    let report = Report {
        schema_version: 1,
        gate: "real_tika_native_ocr_whole_stack_shutdown",
        passed: failure.is_none(),
        stage: evidence.stage,
        failure: problem.as_deref(),
        endpoint: cli.endpoint,
        stop: &cli.stop,
        caller_supplied_cpu_limit: cli.cpus,
        quota_verification: "caller must retain cgroup/Pod budget evidence; this harness does not measure CPU quota",
        runtime_identity: "caller pins native image digest; gateway must report configured OCR dependency ready",
        fixture_sha256: ascii(&fixture_hash),
        fixture_bytes: FIXTURE.len(),
        expected_page_text: &PAGE_TEXT,
        initial_text_policy: "exact ordered nonempty lines; CR/LF/form-feed delimiters ignored only for initial page assertion; full bodies compared byte-for-byte",
        tika_3_3_2_verified: evidence.version_verified,
        native_dependency_ready: evidence.native_dependency_ready,
        baseline_body_sha256: initial_hash.as_ref().map(|value| ascii(value)),
        baseline_body: evidence
            .baseline
            .as_ref()
            .and_then(|reply| reply.text().ok()),
        admitted_before_shutdown: evidence.admitted,
        readiness_removed_ms: evidence.readiness_removed_ms,
        new_request_rejected: evidence.new_request_rejected,
        shutdown_body_sha256: final_hash.as_ref().map(|value| ascii(value)),
        shutdown_body: evidence
            .during_shutdown
            .as_ref()
            .and_then(|reply| reply.text().ok()),
        body_bytes_equal: matches!((&evidence.baseline, &evidence.during_shutdown), (Some(left), Some(right)) if left.body() == right.body()),
        command_completed: evidence.command_completed,
        command_exit_code: evidence.command_exit_code,
        shutdown_total_ms: evidence.shutdown_total_ms,
    };
    serde_json::to_writer_pretty(&mut *output, &report)?;
    output
        .write_all(b"\n")
        .map_err(|error| io_failure("writing report newline", error))?;
    output
        .sync_all()
        .map_err(|error| io_failure("syncing evidence report", error))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(text: &str) -> Reply {
        let mut reply = Reply {
            status: StatusCode::OK,
            text_plain: true,
            bytes: [0; 8192],
            used: text.len(),
        };
        reply.bytes[..text.len()].copy_from_slice(text.as_bytes());
        reply
    }

    #[test]
    fn frozen_fixture_and_exact_two_page_assertion() {
        assert_eq!(ascii(&hash(FIXTURE)), FIXTURE_SHA256);
        assert!(verify_text(&reply("\n\nINVOICE 12345\n\u{c}\nSECOND PAGE 67890\n\n")).is_ok());
        for text in [
            "INVOICE 12345",
            "SECOND PAGE 67890\nINVOICE 12345",
            "INVOICE 12345\nINVOICE 12345",
            "INVOICE 12345\nSECOND  PAGE 67890",
            "INVOICE 12345\nSECOND PAGE 67890\nextra",
        ] {
            assert!(verify_text(&reply(text)).is_err());
        }
    }

    #[tokio::test]
    async fn held_body_never_releases_the_second_byte_before_explicit_signal() {
        let (release, hold) = oneshot::channel();
        let mut body = RequestBody::Held { stage: 0, hold };
        assert_eq!(body.size_hint().exact(), Some(FIXTURE.len() as u64));
        assert_eq!(
            body.frame()
                .await
                .unwrap()
                .unwrap()
                .into_data()
                .unwrap()
                .as_ref(),
            &FIXTURE[..1]
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(10), body.frame())
                .await
                .is_err()
        );
        release.send(()).unwrap();
        assert_eq!(
            body.frame()
                .await
                .unwrap()
                .unwrap()
                .into_data()
                .unwrap()
                .as_ref(),
            &FIXTURE[1..]
        );
        assert!(body.frame().await.is_none());
    }

    #[test]
    fn commands_address_only_explicit_resources_without_shell_or_force() {
        let stop = Stop::Compose {
            project: String::from("acceptance-1"),
            file: PathBuf::from("compose.yaml"),
            override_file: Some(PathBuf::from("compose.2cpu.yaml")),
            second_override_file: None,
            docker_context: None,
        };
        let command = stop_command(&stop, 600);
        assert_eq!(command.get_program(), "docker");
        assert!(
            command.get_args().eq([
                "compose",
                "--project-name",
                "acceptance-1",
                "--file",
                "compose.yaml",
                "--file",
                "compose.2cpu.yaml",
                "stop"
            ]
            .map(std::ffi::OsStr::new))
        );
        let stop = Stop::Kubernetes {
            context: String::from("test-cluster"),
            namespace: String::from("ocr-test"),
            pod: String::from("tika-1"),
        };
        let command = stop_command(&stop, 600);
        assert_eq!(command.get_program(), "kubectl");
        assert!(
            command.get_args().eq([
                "--context",
                "test-cluster",
                "--namespace",
                "ocr-test",
                "--request-timeout=10s",
                "delete",
                "pod",
                "tika-1",
                "--wait=true",
                "--timeout=600s"
            ]
            .map(std::ffi::OsStr::new))
        );
        assert!(!identifier("--all", 63, false));
        assert!(!identifier("project; rm", 63, false));
        assert!(!context_name("bad\ncontext"));
    }
}
