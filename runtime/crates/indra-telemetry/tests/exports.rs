#![cfg(all(feature = "otlp", feature = "sentry"))]

use indra_telemetry::{FailureContext, FailurePhase, InferenceMetrics, Options};
use opentelemetry_proto::tonic::{
    common::v1::any_value::Value,
    metrics::v1::{HistogramDataPoint, metric::Data},
    resource::v1::Resource,
    trace::v1::status::StatusCode,
};
use prost::Message;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

/// The seconds boundaries every duration histogram must export
/// (`SECONDS` in src/otlp.rs).
static SECONDS: [f64; 22] = [
    0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 2.5, 5.0,
    7.5, 10.0, 30.0, 60.0, 120.0, 300.0,
];

#[derive(Default)]
struct Captured {
    spans: usize,
    remote_parent: bool,
    root_id: [u8; 8],
    forwarded_parent: [u8; 8],
    forwarded_context: bool,
    peer_id: [u8; 8],
    shared_links: [[u8; 8]; 2],
    shared_link_count: usize,
    incorrectly_sampled: bool,
    metrics: usize,
    gateway_metrics: u8,
    histogram_points: usize,
    misplaced_durations: bool,
    gateway_span: bool,
    root_status: i32,
    gateway_status: i32,
    resources: usize,
    instance: String,
    unexpected_resource: bool,
    error_event: bool,
    private: bool,
}

static NAME: &str = "real_exporters_carry_parent_metrics_and_context_without_document_content";
static REQUEST_ID: &str = "0123456789abcdef0123456789abcdef";
static PARENT_TRACE: &str = "4bf92f3577b34da6a3ce929d0e0e4736";

/// Exercises actual HTTP/protobuf exporters and Sentry's envelope transport
/// against a local collector. It does not rely on a loaded configuration flag.
/// `initialize` writes JSON logs straight to the process's stderr, which
/// libtest cannot capture, so the exporters run in a re-executed child and
/// this process reads the child's log lines.
#[test]
fn real_exporters_carry_parent_metrics_and_context_without_document_content() {
    if std::env::var_os("INDRA_EXPORTS_CHILD").is_some() {
        exporters_in_this_process();
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
        .env("INDRA_EXPORTS_CHILD", "1")
        .output()
        .expect("child test process");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "exporter child failed\n{stdout}\n{stderr}"
    );
    let root = stdout
        .split_once("root_span_id=")
        .and_then(|(_, rest)| rest.get(..16))
        .expect("child reports the exported POST /ocr span ID");
    let correlated = stderr
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .any(|record| {
            let request = &record["spans"][0];
            record["fields"]["event"] == "service_failure"
                && request["name"] == "http.request"
                && request["request_id"] == REQUEST_ID
                && request["trace_id"] == PARENT_TRACE
                && request["span_id"] == root
        });
    assert!(
        correlated,
        "the request JSON log line carries the exported trace and span IDs\n{stderr}"
    );
}

fn exporters_in_this_process() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("local collector");
    listener.set_nonblocking(true).expect("nonblocking accept");
    let address = listener.local_addr().expect("collector address");
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let collector = scope.spawn(|| {
            let mut captured = Captured::default();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
                match listener.accept() {
                    Ok((socket, _)) => receive(socket, &mut captured),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) => panic!("collector accept: {error}"),
                }
            }
            captured
        });
        let options = Options {
            otlp_endpoint: Some(format!("http://{address}")),
            sentry_dsn: Some(format!("http://public@{address}/1")),
            trace_sample_ratio: 0.0,
            export_interval_ms: 60_000,
            export_timeout_ms: 2000,
            ..Options::default()
        };
        let guard = indra_telemetry::initialize(&options).expect("real SDK setup");
        let request_id = REQUEST_ID;
        let span = indra_telemetry::request_span(
            request_id,
            Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"),
            Some("vendor=opaque"),
        );
        span.in_scope(|| {
            let peer = indra_telemetry::http_span(
                "POST",
                "/peer",
                "POST /peer",
                "peer",
                Some("00-abcd2f3577b34da6a3ce929d0e0e4736-11f067aa0ba902b7-01"),
                None,
            );
            let shared = tracing::info_span!(parent: &span, "ocr.shared_batch");
            shared.follows_from(span.id());
            shared.follows_from(peer.id());
            drop(shared);
            drop(peer);
            let mut outgoing = indra_telemetry::Carrier::empty();
            indra_telemetry::inject_current(&mut outgoing);
            let ids = indra_telemetry::Parent::parse(outgoing.parent().expect("active SDK span"))
                .expect("valid injected span");
            assert_ne!(ids.span, [0x00, 0xf0, 0x67, 0xaa, 0x0b, 0xa9, 0x02, 0xb7]);
            assert_eq!(outgoing.state(), Some("vendor=opaque"));
            drop(indra_telemetry::http_span(
                "POST",
                "/forwarded",
                "POST /forwarded",
                "forwarded",
                outgoing.parent(),
                outgoing.state(),
            ));
            let native = tracing::info_span!("ocr.recognition", batch_rows = 2, padded_width = 64);
            native.in_scope(|| {
                let mut batches = [0; 64];
                batches[1] = 1;
                indra_telemetry::admission(1);
                indra_telemetry::queue_wait(Duration::from_millis(2));
                indra_telemetry::inference(&InferenceMetrics {
                    stages: &[Duration::from_millis(1); 7],
                    batches: &batches,
                    valid_columns: 100,
                    padded_columns: 128,
                    frames: 1,
                    lines: 2,
                    input_bytes: 1000,
                    output_bytes: 40,
                });
                indra_telemetry::report_failure(&FailureContext {
                    class: "onnx_runtime",
                    phase: FailurePhase::Inference,
                    request_id: Some(request_id),
                    worker: Some(0),
                    elapsed: Duration::from_millis(5),
                });
                indra_telemetry::request_finished("native_failure", Duration::from_millis(8));
                indra_telemetry::admission(-1);
            });
            // The native ERROR event marks only its child span; the 500 must
            // mark the server root span itself.
            indra_telemetry::record_http_status(500);
        });
        drop(span);
        let gateway = indra_telemetry::http_span(
            "PUT",
            "/tika/{path}",
            "Tika HTTP parse",
            "0000000000000001",
            Some("00-cdef2f3577b34da6a3ce929d0e0e4736-22f067aa0ba902b7-01"),
            None,
        );
        gateway.in_scope(|| {
            indra_telemetry::gateway_admission(1);
            indra_telemetry::gateway_backend_finished("complete", Duration::from_millis(9));
            indra_telemetry::gateway_request_finished("client_cancelled", Duration::from_millis(3));
            indra_telemetry::gateway_admission(-1);
            indra_telemetry::record_http_status(413);
        });
        drop(gateway);
        drop(indra_telemetry::http_span(
            "GET",
            "/filtered",
            "root_filtered",
            "root",
            None,
            None,
        ));
        drop(indra_telemetry::http_span(
            "GET",
            "/filtered",
            "remote_filtered",
            "remote",
            Some("00-efab2f3577b34da6a3ce929d0e0e4736-33f067aa0ba902b7-00"),
            None,
        ));
        guard
            .shutdown()
            .expect("bounded trace, metric and Sentry flush");
        stop.store(true, Ordering::Release);
        let captured = collector.join().expect("collector result");
        // The parent process matches this ID against the log line's span_id.
        print!("root_span_id=");
        for byte in captured.root_id {
            print!("{byte:02x}");
        }
        println!();
        assert!(
            captured.spans >= 2,
            "request and native child spans exported"
        );
        assert!(
            captured.remote_parent,
            "W3C parent survives HTTP and worker span creation"
        );
        assert!(
            captured.forwarded_context,
            "forwarded HTTP context reached the real OTLP collector"
        );
        assert_eq!(
            captured.forwarded_parent, captured.root_id,
            "the emitted current span, not the original incoming parent, owns the next hop"
        );
        assert!(
            !captured.incorrectly_sampled,
            "zero-ratio roots and explicitly unsampled remote parents stay unexported"
        );
        assert_eq!(
            captured.shared_link_count, 2,
            "both actual shared-batch links survive the SDK limit"
        );
        assert!(
            captured.shared_links.contains(&captured.root_id)
                && captured.shared_links.contains(&captured.peer_id),
            "shared work retains both distinct owner span IDs"
        );
        assert!(
            captured.metrics >= 10,
            "actual stage, queue, batch, outcome and ownership metrics exported"
        );
        assert_eq!(
            captured.gateway_metrics, 31,
            "all five gateway instruments remain distinct from OCR metrics"
        );
        assert_eq!(
            captured.histogram_points, 11,
            "seven stages, queue, OCR request, gateway request and backend durations"
        );
        assert!(
            !captured.misplaced_durations,
            "millisecond durations land in their own sub-10 ms seconds buckets"
        );
        assert!(
            captured.gateway_span,
            "generic HTTP propagation supports the real gateway route"
        );
        assert!(
            captured.resources >= 2 && !captured.unexpected_resource,
            "traces and metrics carry the same four resource keys and instance ID"
        );
        assert_eq!(
            captured.root_status,
            StatusCode::Error as i32,
            "a 500 response marks the server root span Error"
        );
        assert_eq!(
            captured.gateway_status,
            StatusCode::Unset as i32,
            "a 4xx response leaves the server span status Unset"
        );
        assert!(
            captured.error_event,
            "Sentry envelope carries correlated failure and breadcrumbs"
        );
        assert!(
            !captured.private,
            "request contents, PII and exception strings are absent"
        );
    });
}

/// The crate version, plus the source revision when the build baked one in.
fn service_version() -> String {
    match option_env!("INDRA_SOURCE_REVISION") {
        Some(revision) if !revision.is_empty() => {
            format!("{}+{revision}", env!("CARGO_PKG_VERSION"))
        }
        _ => String::from(env!("CARGO_PKG_VERSION")),
    }
}

/// An exported resource has exactly the four configured keys, and every
/// resource of this process carries the same random 32-hex instance ID.
fn check_resource(resource: Option<&Resource>, captured: &mut Captured) {
    let text = |key: &str| {
        resource?
            .attributes
            .iter()
            .find(|attribute| attribute.key == key)
            .and_then(
                |attribute| match attribute.value.as_ref()?.value.as_ref()? {
                    Value::StringValue(text) => Some(text.as_str()),
                    _ => None,
                },
            )
    };
    let instance = text("service.instance.id").unwrap_or("");
    let expected = resource.is_some_and(|resource| resource.attributes.len() == 4)
        && text("service.name") == Some("indra-ocr")
        && text("service.version") == Some(service_version().as_str())
        && text("deployment.environment.name") == Some("local")
        && instance.len() == 32
        && instance
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        && (captured.instance.is_empty() || captured.instance == instance);
    captured.resources += 1;
    captured.unexpected_resource |= !expected;
    if captured.instance.is_empty() {
        captured.instance = String::from(instance);
    }
}

/// Every data point here holds one millisecond-scale recording. It must sit in
/// the bucket that the explicit seconds boundaries select, below 10 ms, not in
/// the (0, 5] bucket of the SDK's millisecond-scale default.
fn sub_ten_milliseconds(point: &HistogramDataPoint) -> bool {
    let value = point.sum.unwrap_or(f64::NAN) / point.count as f64;
    let bucket = SECONDS.partition_point(|bound| *bound < value);
    point.explicit_bounds == SECONDS
        && point.count == 1
        && SECONDS.get(bucket).is_some_and(|bound| *bound <= 0.01)
        && point.bucket_counts.get(bucket) == Some(&point.count)
}

fn receive(mut socket: TcpStream, captured: &mut Captured) {
    // BSD sockets inherit the listener's non-blocking flag; Linux does not.
    socket.set_nonblocking(false).expect("blocking exchange");
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    socket
        .set_write_timeout(Some(Duration::from_secs(2)))
        .expect("write timeout");
    let mut data = [0_u8; 262_144];
    let mut used = 0;
    let boundary = loop {
        let read = socket.read(&mut data[used..]).expect("request bytes");
        assert!(
            read > 0 && used + read < data.len(),
            "bounded collector request"
        );
        used += read;
        if let Some(position) = data[..used]
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
        {
            break position + 4;
        }
    };
    let (trace, metric, envelope, length) = {
        let headers = std::str::from_utf8(&data[..boundary]).expect("ASCII headers");
        let length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().expect("length"))
            })
            .expect("fixed content length");
        (
            headers.starts_with("POST /v1/traces "),
            headers.starts_with("POST /v1/metrics "),
            headers.starts_with("POST /api/1/envelope/ "),
            length,
        )
    };
    assert!(boundary + length <= data.len());
    while used < boundary + length {
        let read = socket
            .read(&mut data[used..boundary + length])
            .expect("protobuf/envelope bytes");
        assert!(read > 0);
        used += read;
    }
    let body = &data[boundary..boundary + length];
    if trace {
        let request =
            opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest::decode(
                body,
            )
            .expect("trace protobuf");
        for spans in &request.resource_spans {
            check_resource(spans.resource.as_ref(), captured);
        }
        for span in request
            .resource_spans
            .iter()
            .flat_map(|resource| &resource.scope_spans)
            .flat_map(|scope| &scope.spans)
        {
            captured.spans += 1;
            captured.incorrectly_sampled |=
                matches!(span.name.as_str(), "root_filtered" | "remote_filtered");
            if span.name == "POST /peer" {
                captured.peer_id.copy_from_slice(&span.span_id);
            }
            if span.name == "ocr.shared_batch" {
                captured.shared_link_count = span.links.len();
                for (slot, link) in captured.shared_links.iter_mut().zip(&span.links) {
                    slot.copy_from_slice(&link.span_id);
                }
            }
            let status = span.status.as_ref().map_or(0, |status| status.code);
            if span.name == "Tika HTTP parse" {
                captured.gateway_span = true;
                captured.gateway_status = status;
            }
            if span.name == "POST /ocr" {
                captured.root_status = status;
                captured.root_id.copy_from_slice(&span.span_id);
                captured.remote_parent |= span.trace_id.as_slice()
                    == [
                        0x4b, 0xf9, 0x2f, 0x35, 0x77, 0xb3, 0x4d, 0xa6, 0xa3, 0xce, 0x92, 0x9d,
                        0x0e, 0x0e, 0x47, 0x36,
                    ]
                    && span.parent_span_id.as_slice()
                        == [0x00, 0xf0, 0x67, 0xaa, 0x0b, 0xa9, 0x02, 0xb7];
            } else if span.name == "POST /forwarded" {
                captured
                    .forwarded_parent
                    .copy_from_slice(&span.parent_span_id);
                captured.forwarded_context = span.trace_state == "vendor=opaque"
                    && span.trace_id.as_slice()
                        == [
                            0x4b, 0xf9, 0x2f, 0x35, 0x77, 0xb3, 0x4d, 0xa6, 0xa3, 0xce, 0x92, 0x9d,
                            0x0e, 0x0e, 0x47, 0x36,
                        ];
            }
        }
    } else if metric {
        let request = opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest::decode(body).expect("metric protobuf");
        for metrics in &request.resource_metrics {
            check_resource(metrics.resource.as_ref(), captured);
        }
        for metric in request
            .resource_metrics
            .iter()
            .flat_map(|resource| &resource.scope_metrics)
            .flat_map(|scope| &scope.metrics)
        {
            captured.gateway_metrics |= match metric.name.as_str() {
                "indra.gateway.requests" => 1,
                "indra.gateway.request.duration" => 2,
                "indra.gateway.active" => 4,
                "indra.gateway.backend.requests" => 8,
                "indra.gateway.backend.duration" => 16,
                _ => 0,
            };
            if metric.name.ends_with(".duration")
                && let Some(Data::Histogram(histogram)) = &metric.data
            {
                for point in &histogram.data_points {
                    captured.histogram_points += 1;
                    captured.misplaced_durations |= !sub_ten_milliseconds(point);
                }
            }
        }
        captured.metrics += request
            .resource_metrics
            .iter()
            .flat_map(|resource| &resource.scope_metrics)
            .map(|scope| scope.metrics.len())
            .sum::<usize>();
    } else if envelope {
        for line in body.split(|byte| *byte == b'\n') {
            if let Ok(event) = serde_json::from_slice::<serde_json::Value>(line)
                && event.get("event_id").is_some()
                && event.get("exception").is_some()
            {
                captured.error_event |= event["tags"]["failure.class"] == "onnx_runtime"
                    && event["tags"]["request_id"] == "0123456789abcdef0123456789abcdef"
                    && event["tags"]["trace_id"] == "4bf92f3577b34da6a3ce929d0e0e4736"
                    && event["contexts"]["trace"]["trace_id"] == "4bf92f3577b34da6a3ce929d0e0e4736"
                    && event["contexts"]["trace"]["span_id"]
                        .as_str()
                        .is_some_and(|id| id.len() == 16)
                    && event["breadcrumbs"]["values"]
                        .as_array()
                        .is_some_and(|entries| entries.len() == 2)
                    && event["release"] == format!("indra-ocr@{}", service_version())
                    && event["environment"] == "local";
                captured.private |= event.get("request").is_some()
                    || event.get("user").is_some()
                    || event.get("server_name").is_some();
            }
        }
    } else {
        panic!("unexpected exporter route");
    }
    socket
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        .expect("collector acknowledgement");
}
