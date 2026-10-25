//! Operational telemetry with finite labels and explicit privacy boundaries.
//!
//! Images, OCR text, file names and native exception strings are never telemetry
//! fields. Only validated W3C traceparent/tracestate metadata is read from headers.
//! The optional SDKs own their bounded export queues;
//! application code does not introduce trait objects or clone payload buffers.

#[cfg(feature = "sentry")]
mod errors;
mod options;
#[cfg(feature = "otlp")]
mod otlp;
mod propagation;

use std::time::Duration;
use thiserror::Error;
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

pub use options::Options;
pub use propagation::{
    Carrier, Parent, http_span, inject_current, record_http_status, request_span,
};

/// Source revision baked in by the image builds, whose `SOURCE_REVISION`
/// build argument reaches Cargo as `INDRA_SOURCE_REVISION`. Local builds
/// have none.
#[cfg(any(feature = "otlp", feature = "sentry"))]
static SOURCE_REVISION: Option<&str> = option_env!("INDRA_SOURCE_REVISION");

/// `service.version` and the Sentry release version: the crate version, with
/// a baked-in source revision as SemVer build metadata (`0.2.0+<revision>`).
#[cfg(any(feature = "otlp", feature = "sentry"))]
struct ServiceVersion;

#[cfg(any(feature = "otlp", feature = "sentry"))]
impl std::fmt::Display for ServiceVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(env!("CARGO_PKG_VERSION"))?;
        match SOURCE_REVISION {
            Some(revision) if !revision.is_empty() => write!(f, "+{revision}"),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Error)]
pub enum TelemetryError {
    #[error("invalid telemetry setting: {0}")]
    Configuration(&'static str),
    #[error("configured telemetry support was excluded at build time: {0}")]
    Feature(&'static str),
    #[error("telemetry initialization failed: {0}; credentials and endpoints are redacted")]
    Initialize(&'static str),
    #[error("telemetry shutdown did not complete within its budget")]
    Shutdown,
}

/// Providers must outlive workers and the HTTP runtime. Initialize before Tokio;
/// flush after the runtime stops, because exporters use dedicated blocking I/O.
pub struct Guard {
    log_worker: tracing_appender::non_blocking::WorkerGuard,
    log_errors: tracing_appender::non_blocking::ErrorCounter,
    #[cfg(feature = "otlp")]
    providers: Option<otlp::Providers>,
    #[cfg(feature = "sentry")]
    sentry: Option<sentry::ClientInitGuard>,
    #[cfg(any(feature = "otlp", feature = "sentry"))]
    timeout: Duration,
}

pub fn initialize(options: &Options) -> Result<Guard, TelemetryError> {
    options.validate()?;
    let filter = EnvFilter::try_new(&options.log_filter)
        .map_err(|_| TelemetryError::Configuration("log_filter"))?;
    let (writer, log_worker) = tracing_appender::non_blocking::NonBlockingBuilder::default()
        .buffered_lines_limit(options.log_queue_size)
        .lossy(true)
        .thread_name("indra-json-logs")
        .finish(std::io::stderr());
    let log_errors = writer.error_counter();
    // The JSON log layer follows `log_filter`. Trace export follows the SDK
    // sampler and its own INFO floor, so a quiet log filter cannot silently
    // disable tracing. With `internal-logs`, exporter failures and dropped
    // spans surface as `opentelemetry*` targets in the same JSON stream.
    let logs = tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(true)
        .with_span_list(true)
        .with_target(true)
        .with_writer(writer)
        .with_filter(filter);
    #[cfg(feature = "otlp")]
    let providers = options
        .otlp_endpoint
        .as_ref()
        .map(|_| otlp::initialize(options))
        .transpose()?;
    #[cfg(feature = "otlp")]
    let otel = providers.as_ref().map(|providers| {
        use opentelemetry::trace::TracerProvider;
        tracing_opentelemetry::layer()
            .with_tracer(providers.traces.tracer("indra-runtime"))
            .with_location(false)
            .with_threads(false)
            .with_filter(tracing_subscriber::filter::LevelFilter::INFO)
    });
    let subscriber = tracing_subscriber::registry();
    #[cfg(feature = "otlp")]
    let subscriber = subscriber.with(otel);
    subscriber
        .with(logs)
        .try_init()
        .map_err(|_| TelemetryError::Initialize("subscriber"))?;
    #[cfg(feature = "sentry")]
    let sentry = options
        .sentry_dsn
        .as_ref()
        .map(|_| errors::initialize(options))
        .transpose()?;
    install_panic_hook(Duration::from_millis(options.export_timeout_ms));
    Ok(Guard {
        log_worker,
        log_errors,
        #[cfg(feature = "otlp")]
        providers,
        #[cfg(feature = "sentry")]
        sentry,
        #[cfg(any(feature = "otlp", feature = "sentry"))]
        timeout: Duration::from_millis(options.export_timeout_ms),
    })
}

static PANIC_HOOK: std::sync::Once = std::sync::Once::new();

/// A JSON string literal written without allocation, with only the escapes
/// JSON requires (quote, backslash, control characters).
struct JsonString<'a>(&'a str);

impl std::fmt::Display for JsonString<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("\"")?;
        for character in self.0.chars() {
            match character {
                '"' => f.write_str("\\\"")?,
                '\\' => f.write_str("\\\\")?,
                '\n' => f.write_str("\\n")?,
                '\r' => f.write_str("\\r")?,
                '\t' => f.write_str("\\t")?,
                control if control.is_control() => {
                    write!(f, "\\u{:04x}", u32::from(control))?;
                }
                other => write!(f, "{other}")?,
            }
        }
        f.write_str("\"")
    }
}

/// The JSON log layer's own timestamp (RFC 3339 UTC, microseconds), so the
/// synchronous lines below sort and parse like every tracing line.
struct Timestamp;

impl std::fmt::Display for Timestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use tracing_subscriber::fmt::{format::Writer, time::FormatTime};
        tracing_subscriber::fmt::time::SystemTime.format_time(&mut Writer::new(f))
    }
}

/// The synchronous panic line, in the envelope of the tracing JSON lines
/// (`timestamp`, `level`, `fields.event`, `target`).
struct PanicLine<'a> {
    thread: &'a str,
    file: &'a str,
    line: u32,
}

impl std::fmt::Display for PanicLine<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{{\"timestamp\":\"{Timestamp}\",\"level\":\"ERROR\",\"fields\":{{\"message\":\"Process panicked\",\"event\":\"panic\",\"thread\":{},\"file\":{},\"line\":{}}},\"target\":\"indra_telemetry\"}}",
            JsonString(self.thread),
            JsonString(self.file),
            self.line
        )
    }
}

/// The synchronous shutdown diagnostic for dropped log lines, in the same
/// envelope.
struct OverflowLine(usize);

impl std::fmt::Display for OverflowLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{{\"timestamp\":\"{Timestamp}\",\"level\":\"WARN\",\"fields\":{{\"message\":\"JSON log lines were dropped\",\"event\":\"log_queue_overflow\",\"dropped_lines\":{}}},\"target\":\"indra_telemetry\"}}",
            self.0
        )
    }
}

/// Under `panic = "abort"` the hook is the last chance to record a crash. It
/// writes one synchronous JSON line without the panic payload (payloads may
/// quote document data), reports the finite `panic` failure class and flushes
/// the bounded Sentry queue within the export budget. It replaces std's default
/// hook rather than chaining it: that report prints the payload and the raw
/// thread name to stderr as plain text. `std::panic::set_hook` mandates a boxed
/// callback; this is the only heap-allocated callback in the runtime and it is
/// installed once.
fn install_panic_hook(timeout: Duration) {
    PANIC_HOOK.call_once(|| {
        std::panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current();
            let (file, line) = info.location().map_or(("unknown", 0), |location| {
                (location.file(), location.line())
            });
            eprintln!(
                "{}",
                PanicLine {
                    thread: thread.name().unwrap_or("unnamed"),
                    file,
                    line,
                }
            );
            #[cfg(feature = "sentry")]
            {
                errors::report(&FailureContext {
                    class: "panic",
                    reason: None,
                    phase: FailurePhase::Process,
                    request_id: None,
                    worker: None,
                    elapsed: Duration::ZERO,
                });
                errors::flush(timeout);
            }
            #[cfg(not(feature = "sentry"))]
            let _ = timeout;
        }));
    });
}

impl Guard {
    pub fn shutdown(self) -> Result<(), TelemetryError> {
        #[cfg(feature = "otlp")]
        let traces_complete = self
            .providers
            .is_none_or(|providers| providers.shutdown(self.timeout));
        #[cfg(not(feature = "otlp"))]
        let traces_complete = true;
        #[cfg(feature = "sentry")]
        let errors_complete = self
            .sentry
            .is_none_or(|client| client.close(Some(self.timeout)));
        #[cfg(not(feature = "sentry"))]
        let errors_complete = true;
        let dropped = self.log_errors.dropped_lines();
        if dropped > 0 {
            // A synchronous shutdown diagnostic remains visible even if the
            // bounded asynchronous queue was full. This contains no user data.
            eprintln!("{}", OverflowLine(dropped));
        }
        drop(self.log_worker);
        if traces_complete && errors_complete {
            Ok(())
        } else {
            Err(TelemetryError::Shutdown)
        }
    }
}

/// Only call with one of the application's finite outcome/error names.
pub fn request_finished(outcome: &'static str, elapsed: Duration) {
    #[cfg(feature = "otlp")]
    otlp::request_finished(outcome, elapsed);
    #[cfg(not(feature = "otlp"))]
    let _ = (outcome, elapsed);
}

pub fn admission(delta: i64) {
    #[cfg(feature = "otlp")]
    otlp::admission(delta);
    #[cfg(not(feature = "otlp"))]
    let _ = delta;
}

pub fn gateway_request_finished(outcome: &'static str, elapsed: Duration) {
    #[cfg(feature = "otlp")]
    otlp::gateway_request_finished(outcome, elapsed);
    #[cfg(not(feature = "otlp"))]
    let _ = (outcome, elapsed);
}

pub fn gateway_backend_finished(outcome: &'static str, elapsed: Duration) {
    #[cfg(feature = "otlp")]
    otlp::gateway_backend_finished(outcome, elapsed);
    #[cfg(not(feature = "otlp"))]
    let _ = (outcome, elapsed);
}

pub fn gateway_admission(delta: i64) {
    #[cfg(feature = "otlp")]
    otlp::gateway_admission(delta);
    #[cfg(not(feature = "otlp"))]
    let _ = delta;
}

pub fn queue_wait(elapsed: Duration) {
    #[cfg(feature = "otlp")]
    otlp::queue_wait(elapsed);
    #[cfg(not(feature = "otlp"))]
    let _ = elapsed;
}

pub struct InferenceMetrics<'a> {
    pub stages: &'a [Duration; 7],
    pub batches: &'a [u32; 64],
    pub valid_columns: u64,
    pub padded_columns: u64,
    pub frames: usize,
    pub lines: usize,
    pub input_bytes: usize,
    pub output_bytes: usize,
}

pub fn inference(metrics: &InferenceMetrics<'_>) {
    #[cfg(feature = "otlp")]
    otlp::inference(metrics);
    #[cfg(not(feature = "otlp"))]
    let _ = metrics;
}

pub struct FailureContext<'a> {
    pub class: &'static str,
    /// The compile-time detail of the failed check (`EngineError::reason`),
    /// when there is one; never a runtime or native message.
    pub reason: Option<&'static str>,
    pub phase: FailurePhase,
    pub request_id: Option<&'a str>,
    pub worker: Option<u8>,
    pub elapsed: Duration,
}

pub enum FailurePhase {
    Startup,
    Inference,
    Watchdog,
    Gateway,
    /// A terminal service transition outside a specific request or worker.
    Service,
    /// The process is aborting after a panic.
    Process,
}

impl FailurePhase {
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Inference => "inference",
            Self::Watchdog => "watchdog",
            Self::Gateway => "gateway",
            Self::Service => "service",
            Self::Process => "process",
        }
    }
}

pub fn report_failure(context: &FailureContext<'_>) {
    tracing::error!(
        event = "service_failure",
        failure_class = context.class,
        reason = context.reason,
        phase = context.phase.name(),
        request_id = context.request_id,
        worker = context.worker,
        elapsed_ms = context.elapsed.as_secs_f64() * 1000.0,
        "Service operation failed"
    );
    #[cfg(feature = "sentry")]
    errors::report(context);
}

#[cfg(test)]
mod tests {
    use super::{OverflowLine, PanicLine};

    /// `YYYY-MM-DDTHH:MM:SS.ffffffZ`, as the tracing JSON layer writes it.
    fn tracing_timestamp(value: &serde_json::Value) -> bool {
        value.as_str().is_some_and(|text| {
            text.len() == 27
                && text.bytes().enumerate().all(|(index, byte)| match index {
                    4 | 7 => byte == b'-',
                    10 => byte == b'T',
                    13 | 16 => byte == b':',
                    19 => byte == b'.',
                    26 => byte == b'Z',
                    _ => byte.is_ascii_digit(),
                })
        })
    }

    #[test]
    fn synchronous_lines_use_the_tracing_envelope_and_escape_strings() {
        // Quote, backslash, NUL, newline, other C0/DEL/C1 controls and
        // non-ASCII text, as a thread name may carry any of them.
        let thread = "ocr\"worker\\\0\n\r\t\u{1}\u{1f}\u{7f}\u{85} Zürich 工 🦀";
        let line = PanicLine {
            thread,
            file: "crates/indra-engine/src/lib.rs",
            line: 7,
        }
        .to_string();
        assert!(!line.contains(['\n', '\r', '\0']), "one physical line");
        let record: serde_json::Value = serde_json::from_str(&line).expect("panic line is JSON");
        assert!(tracing_timestamp(&record["timestamp"]), "{line}");
        assert_eq!(record["level"], "ERROR");
        assert_eq!(record["target"], "indra_telemetry");
        assert_eq!(record["fields"]["event"], "panic");
        assert_eq!(record["fields"]["thread"], thread);
        assert_eq!(record["fields"]["file"], "crates/indra-engine/src/lib.rs");
        assert_eq!(record["fields"]["line"], 7);
        let line = OverflowLine(3).to_string();
        let record: serde_json::Value = serde_json::from_str(&line).expect("overflow line is JSON");
        assert!(tracing_timestamp(&record["timestamp"]), "{line}");
        assert_eq!(record["level"], "WARN");
        assert_eq!(record["target"], "indra_telemetry");
        assert_eq!(record["fields"]["event"], "log_queue_overflow");
        assert_eq!(record["fields"]["dropped_lines"], 3);
    }
}
