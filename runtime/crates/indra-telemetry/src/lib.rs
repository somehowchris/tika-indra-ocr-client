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
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

pub use options::Options;
pub use propagation::{Parent, http_span, request_span};

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
    let logs = tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(true)
        .with_span_list(true)
        .with_target(true)
        .with_writer(writer);
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
    });
    let subscriber = tracing_subscriber::registry().with(filter);
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
            eprintln!(
                "{{\"level\":\"WARN\",\"event\":\"log_queue_overflow\",\"dropped_lines\":{dropped}}}"
            );
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
}

impl FailurePhase {
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Inference => "inference",
            Self::Watchdog => "watchdog",
            Self::Gateway => "gateway",
        }
    }
}

pub fn report_failure(context: &FailureContext<'_>) {
    tracing::error!(
        event = "service_failure",
        failure_class = context.class,
        phase = context.phase.name(),
        request_id = context.request_id,
        worker = context.worker,
        elapsed_ms = context.elapsed.as_secs_f64() * 1000.0,
        "Service operation failed"
    );
    #[cfg(feature = "sentry")]
    errors::report(context);
}
