use crate::{InferenceMetrics, Options, TelemetryError};
use opentelemetry::{
    KeyValue,
    metrics::{Counter, Histogram, MeterProvider, UpDownCounter},
};
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::{
    Resource,
    metrics::{PeriodicReader, SdkMeterProvider},
    trace::{BatchConfigBuilder, BatchSpanProcessor, Sampler, SdkTracerProvider},
};
use std::{collections::HashMap, sync::OnceLock, time::Duration};

static METRICS: OnceLock<Metrics> = OnceLock::new();
static STAGES: [&str; 7] = [
    "decode",
    "orientation",
    "detection",
    "recognition",
    "math",
    "reading_order",
    "assembly",
];

pub(crate) struct Providers {
    pub traces: SdkTracerProvider,
    meters: SdkMeterProvider,
}

fn resource(options: &Options) -> Resource {
    // SDK resources require owned 'static strings. These two explicit startup
    // copies are not request-path clones and never contain credentials.
    Resource::builder_empty()
        .with_attributes([
            KeyValue::new("service.name", String::from(options.service_name.as_str())),
            KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
            KeyValue::new(
                "deployment.environment.name",
                String::from(options.environment.as_str()),
            ),
        ])
        .build()
}

fn headers(options: &Options) -> HashMap<String, String> {
    let mut headers = HashMap::with_capacity(usize::from(options.otlp_authorization.is_some()));
    if let Some(authorization) = &options.otlp_authorization {
        // Each concrete exporter owns its own one-entry header map at startup.
        headers.insert(
            String::from("authorization"),
            String::from(authorization.as_str()),
        );
    }
    headers
}

pub(crate) fn initialize(options: &Options) -> Result<Providers, TelemetryError> {
    let endpoint = options
        .otlp_endpoint
        .as_deref()
        .ok_or(TelemetryError::Initialize("missing OTLP endpoint"))?
        .trim_end_matches('/');
    let timeout = Duration::from_millis(options.export_timeout_ms);
    let spans = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(format!("{endpoint}/v1/traces"))
        .with_timeout(timeout)
        .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
        .with_http_client(http_client(timeout)?)
        .with_headers(headers(options))
        .build()
        .map_err(|_| TelemetryError::Initialize("OTLP trace exporter"))?;
    let processor = BatchSpanProcessor::builder(spans)
        .with_batch_config(
            BatchConfigBuilder::default()
                .with_max_queue_size(options.export_queue_size)
                .with_max_export_batch_size(options.export_batch_size)
                .with_scheduled_delay(Duration::from_millis(options.export_interval_ms))
                .build(),
        )
        .build();
    let traces = SdkTracerProvider::builder()
        .with_span_processor(processor)
        // This SDK enum requires one boxed root sampler at initialization.
        // Existing parent decisions then remain coherent across process hops;
        // the configured ratio applies only to roots, including an exact zero.
        .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
            options.trace_sample_ratio,
        ))))
        .with_max_attributes_per_span(32)
        .with_max_events_per_span(16)
        .with_max_links_per_span(2)
        .with_resource(resource(options))
        .build();
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .with_endpoint(format!("{endpoint}/v1/metrics"))
        .with_timeout(timeout)
        .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
        .with_http_client(http_client(timeout)?)
        .with_headers(headers(options))
        .build()
        .map_err(|_| TelemetryError::Initialize("OTLP metric exporter"))?;
    let reader = PeriodicReader::builder(exporter)
        .with_interval(Duration::from_millis(options.export_interval_ms))
        .build();
    let meters = SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(resource(options))
        .build();
    let meter = meters.meter("indra-runtime");
    let metrics = Metrics {
        gateway_requests: meter.u64_counter("indra.gateway.requests").with_description("Finished public Tika handlers, including rejected and canceled requests").build(),
        gateway_duration: meter.f64_histogram("indra.gateway.request.duration").with_unit("s").build(),
        gateway_active: meter.i64_up_down_counter("indra.gateway.active").with_description("Uploads, retained Tika parses and response transport owners").build(),
        gateway_backend: meter.u64_counter("indra.gateway.backend.requests").with_description("Finished retained Tika backend exchanges, including after public client disconnect").build(),
        gateway_backend_duration: meter.f64_histogram("indra.gateway.backend.duration").with_unit("s").build(),
        requests: meter
            .u64_counter("indra.ocr.requests")
            .with_description("Finished HTTP OCR handlers, including rejections and cancellation")
            .build(),
        active: meter
            .i64_up_down_counter("indra.ocr.active")
            .with_description(
                "Admitted uploads, queue entries, inference calls and retained response buffers",
            )
            .build(),
        duration: meter
            .f64_histogram("indra.ocr.request.duration")
            .with_unit("s")
            .build(),
        queue: meter
            .f64_histogram("indra.ocr.queue.duration")
            .with_unit("s")
            .build(),
        stage: meter
            .f64_histogram("indra.ocr.stage.duration")
            .with_unit("s")
            .build(),
        batch: meter.u64_counter("indra.ocr.recognition.batches").build(),
        columns: meter.u64_counter("indra.ocr.recognition.columns").build(),
        frames: meter.u64_counter("indra.ocr.frames").build(),
        lines: meter.u64_counter("indra.ocr.lines").build(),
        input: meter
            .u64_counter("indra.ocr.input.bytes")
            .with_unit("By")
            .build(),
        output: meter
            .u64_counter("indra.ocr.output.bytes")
            .with_unit("By")
            .build(),
    };
    METRICS
        .set(metrics)
        .map_err(|_| TelemetryError::Initialize("metrics already initialized"))?;
    Ok(Providers { traces, meters })
}

fn http_client(timeout: Duration) -> Result<reqwest::blocking::Client, TelemetryError> {
    reqwest::blocking::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| TelemetryError::Initialize("OTLP HTTP client"))
}

impl Providers {
    pub fn shutdown(self, timeout: Duration) -> bool {
        // Both providers have an independent finite shutdown allowance.
        let traces = self.traces.shutdown_with_timeout(timeout).is_ok();
        let meters = self.meters.shutdown_with_timeout(timeout).is_ok();
        traces && meters
    }
}

struct Metrics {
    gateway_requests: Counter<u64>,
    gateway_duration: Histogram<f64>,
    gateway_active: UpDownCounter<i64>,
    gateway_backend: Counter<u64>,
    gateway_backend_duration: Histogram<f64>,
    requests: Counter<u64>,
    active: UpDownCounter<i64>,
    duration: Histogram<f64>,
    queue: Histogram<f64>,
    stage: Histogram<f64>,
    batch: Counter<u64>,
    columns: Counter<u64>,
    frames: Counter<u64>,
    lines: Counter<u64>,
    input: Counter<u64>,
    output: Counter<u64>,
}

pub(crate) fn gateway_request_finished(outcome: &'static str, elapsed: Duration) {
    if let Some(metrics) = METRICS.get() {
        let labels = [KeyValue::new("outcome", outcome)];
        metrics.gateway_requests.add(1, &labels);
        metrics
            .gateway_duration
            .record(elapsed.as_secs_f64(), &labels);
    }
}

pub(crate) fn gateway_backend_finished(outcome: &'static str, elapsed: Duration) {
    if let Some(metrics) = METRICS.get() {
        let labels = [KeyValue::new("outcome", outcome)];
        metrics.gateway_backend.add(1, &labels);
        metrics
            .gateway_backend_duration
            .record(elapsed.as_secs_f64(), &labels);
    }
}

pub(crate) fn gateway_admission(delta: i64) {
    if let Some(metrics) = METRICS.get() {
        metrics.gateway_active.add(delta, &[]);
    }
}

pub(crate) fn request_finished(outcome: &'static str, elapsed: Duration) {
    if let Some(metrics) = METRICS.get() {
        let labels = [KeyValue::new("outcome", outcome)];
        metrics.requests.add(1, &labels);
        metrics.duration.record(elapsed.as_secs_f64(), &labels);
    }
}

pub(crate) fn admission(delta: i64) {
    if let Some(metrics) = METRICS.get() {
        metrics.active.add(delta, &[]);
    }
}

pub(crate) fn queue_wait(elapsed: Duration) {
    if let Some(metrics) = METRICS.get() {
        metrics.queue.record(elapsed.as_secs_f64(), &[]);
    }
}

pub(crate) fn inference(observed: &InferenceMetrics<'_>) {
    if let Some(metrics) = METRICS.get() {
        for (stage, elapsed) in STAGES.iter().zip(observed.stages) {
            metrics
                .stage
                .record(elapsed.as_secs_f64(), &[KeyValue::new("stage", *stage)]);
        }
        for (index, count) in observed
            .batches
            .iter()
            .enumerate()
            .filter(|(_, count)| **count > 0)
        {
            metrics.batch.add(
                u64::from(*count),
                &[KeyValue::new("rows", (index + 1) as i64)],
            );
        }
        metrics
            .columns
            .add(observed.valid_columns, &[KeyValue::new("kind", "valid")]);
        metrics
            .columns
            .add(observed.padded_columns, &[KeyValue::new("kind", "padded")]);
        metrics.frames.add(observed.frames as u64, &[]);
        metrics.lines.add(observed.lines as u64, &[]);
        metrics.input.add(observed.input_bytes as u64, &[]);
        metrics.output.add(observed.output_bytes as u64, &[]);
    }
}
