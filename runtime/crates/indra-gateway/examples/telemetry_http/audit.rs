//! Audit only actual local SDK wire exports. Bounded arrays keep graph identity explicit.
use super::{Failure, REQUEST_PARENTS, check, sentry};
use opentelemetry_proto::tonic::collector::{
    metrics::v1::ExportMetricsServiceRequest, trace::v1::ExportTraceServiceRequest,
};
use opentelemetry_proto::tonic::{
    common::v1::any_value,
    metrics::v1::{metric, number_data_point},
};
use prost::Message;
use serde::{
    Deserialize, Serialize,
    de::{Error as _, SeqAccess, Visitor},
};
use std::{
    fmt,
    fs::{self, File},
    path::Path,
};
#[derive(Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    GatewayHttp,
    JavaParse,
    JavaOcr,
    NativeHttp,
    NativeInference,
    #[default]
    Other,
}
#[derive(Default, Clone, Copy, Deserialize)]
struct Span {
    trace: [u8; 16],
    id: [u8; 8],
    parent: [u8; 8],
    kind: Kind,
    error: bool,
}
struct Spans {
    data: [Span; 512],
    used: usize,
}
impl<'de> Deserialize<'de> for Spans {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Array;
        impl<'de> Visitor<'de> for Array {
            type Value = Spans;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("at most512 captured spans")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Spans, A::Error> {
                let mut result = Spans {
                    data: [Span::default(); 512],
                    used: 0,
                };
                while let Some(span) = sequence.next_element()? {
                    if result.used == 512 {
                        return Err(A::Error::custom("span cap"));
                    }
                    result.data[result.used] = span;
                    result.used += 1;
                }
                Ok(result)
            }
        }
        deserializer.deserialize_seq(Array)
    }
}
impl Spans {
    fn child(&self, parent: &Span, kind: Kind) -> Result<&Span, Failure> {
        let mut matches = self.data[..self.used]
            .iter()
            .filter(|s| s.trace == parent.trace && s.parent == parent.id && s.kind == kind);
        let found = matches
            .next()
            .ok_or(Failure::Gate("required exact SDK span edge absent"))?;
        check(matches.next().is_none(), "duplicate SDK span edge")?;
        Ok(found)
    }
    fn descendant<'a>(&'a self, mut child: &'a Span, parent: &Span) -> bool {
        for _ in 0..64 {
            if child.trace != parent.trace {
                return false;
            }
            if child.parent == parent.id {
                return true;
            }
            if let Some(next) = self.data[..self.used]
                .iter()
                .find(|s| s.trace == child.trace && s.id == child.parent)
            {
                child = next;
            } else {
                return false;
            }
        }
        false
    }
    fn verify(&self) -> Result<(), Failure> {
        for (index, span) in self.data[..self.used].iter().enumerate() {
            check(
                span.trace != [0; 16] && span.id != [0; 8],
                "zero SDK span identity",
            )?;
            check(
                !self.data[..index]
                    .iter()
                    .any(|other| other.trace == span.trace && other.id == span.id),
                "duplicate SDK span identity",
            )?;
        }
        for incoming in REQUEST_PARENTS {
            let parent = indra_telemetry::Parent::parse(incoming)
                .ok_or(Failure::Gate("fixed SDK parent invalid"))?;
            let mut roots = self.data[..self.used].iter().filter(|s| {
                s.kind == Kind::GatewayHttp && s.trace == parent.trace && s.parent == parent.span
            });
            let root = roots
                .next()
                .ok_or(Failure::Gate("scheduled gateway parent absent"))?;
            check(
                roots.next().is_none() && !root.error,
                "duplicate/error gateway root",
            )?;
            let java = self.child(root, Kind::JavaParse)?;
            let mut ocrs = self.data[..self.used]
                .iter()
                .filter(|s| s.kind == Kind::JavaOcr && self.descendant(s, java));
            let ocr = ocrs
                .next()
                .ok_or(Failure::Gate("real Java OCR span absent"))?;
            check(ocrs.next().is_none(), "duplicate Java OCR span")?;
            let native = self.child(ocr, Kind::NativeHttp)?;
            check(
                !java.error && !ocr.error && !native.error,
                "successful pipeline trace has error",
            )?;
            check(
                self.data[..self.used].iter().any(|s| {
                    s.kind == Kind::NativeInference && !s.error && self.descendant(s, native)
                }),
                "native inference not descendant of forwarded SDK graph",
            )?;
        }
        Ok(())
    }
}
#[derive(Serialize, Default)]
pub(super) struct Audit {
    trace_exports: usize,
    metrics_exports: usize,
    otlp_bytes: usize,
    spans: usize,
    scheduled_graphs: usize,
    trace_role_mask: u8,
    metric_role_mask: u8,
    sentry: sentry::Captured,
    privacy_passed: bool,
    cohort_exports: usize,
    gateway_requests_max: u64,
    native_requests_max: u64,
}
pub(super) fn validate_export_headers(headers: &[u8]) -> Result<(), Failure> {
    for canary in [
        b"DOCUMENT_SECRET".as_slice(),
        b"SDK_PRIVATE_CANARY",
        b"COOKIE_PRIVATE_CANARY",
    ] {
        check(
            !contains_ascii_case(headers, canary),
            "incoming client secret in exporter HTTP headers",
        )?;
    }
    Ok(())
}

pub(super) fn count_exports(path: &Path) -> Result<usize, Failure> {
    let mut count = 0;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with("otlp-") || n.starts_with("sentry-"))
        {
            count += 1;
        }
    }
    Ok(count)
}

fn role(attributes: &[opentelemetry_proto::tonic::common::v1::KeyValue]) -> Result<u8, Failure> {
    let mut service = attributes.iter().filter(|a| a.key == "service.name");
    let value = service
        .next()
        .ok_or(Failure::Gate("SDK service resource absent"))?;
    check(service.next().is_none(), "duplicate SDK service resource")?;
    match value.value.as_ref().and_then(|v| v.value.as_ref()) {
        Some(any_value::Value::StringValue(name)) => match name.as_str() {
            "telemetry-gateway" => Ok(1),
            "telemetry-native" => Ok(2),
            "tika-parser" => Ok(4),
            _ => Err(Failure::Gate("unexpected exporting role")),
        },
        _ => Err(Failure::Gate("nonstring SDK service resource")),
    }
}
pub(super) fn verify(
    output: &Path,
    enabled: bool,
    sentry: sentry::Captured,
    cohort_exports: usize,
    goldens: Option<&Path>,
) -> Result<Audit, Failure> {
    let spans: Spans = serde_json::from_reader(File::open(output.join("spans.json"))?)?;
    let mut audit = Audit {
        spans: spans.used,
        sentry,
        cohort_exports,
        ..Audit::default()
    };
    for entry in fs::read_dir(output)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with("otlp-") && !name.starts_with("sentry-") {
            continue;
        }
        let bytes = fs::read(entry.path())?;
        for forbidden in [
            b"DOCUMENT_SECRET".as_slice(),
            b"SDK_PRIVATE_CANARY",
            b"COOKIE_PRIVATE_CANARY",
            b"INVOICE",
            b"SECOND PAGE",
            b"exception.message",
            b"exception.stacktrace",
            b"baggage",
        ] {
            check(
                !bytes.windows(forbidden.len()).any(|p| p == forbidden),
                "private OTLP body/header data",
            )?;
        }
        if let Some(goldens) = goldens {
            for fixture in super::NAMES {
                let text = fs::read_to_string(goldens.join(format!("golden-{fixture}.txt")))?;
                for line in text.lines().map(str::trim).filter(|line| line.len() >= 8) {
                    check(
                        !bytes
                            .windows(line.len())
                            .any(|part| part == line.as_bytes()),
                        "OCR golden text present in OTLP export",
                    )?;
                }
            }
        }
        if name.starts_with("sentry-") {
            sentry::verify_envelope(&bytes)?;
            continue;
        }
        audit.otlp_bytes += bytes.len();
        if name.ends_with("traces.protobuf") {
            audit.trace_exports += 1;
            let request = ExportTraceServiceRequest::decode(bytes.as_slice())?;
            for resource in request.resource_spans {
                check(
                    resource
                        .scope_spans
                        .iter()
                        .any(|scope| !scope.spans.is_empty()),
                    "empty SDK trace resource",
                )?;
                let identity = role(
                    &resource
                        .resource
                        .ok_or(Failure::Gate("trace resource absent"))?
                        .attributes,
                )?;
                for span in resource.scope_spans.iter().flat_map(|scope| &scope.spans) {
                    let expected = match span.name.as_str() {
                        "Tika HTTP parse" => Some(1),
                        "POST /ocr" | "ocr.recognition" | "ocr.inference" => Some(2),
                        "tika.parse" | "tika.ocr.process" => Some(4),
                        _ => None,
                    };
                    if let Some(expected) = expected {
                        check(identity == expected, "SDK span exported under wrong role")?;
                    }
                }
                audit.trace_role_mask |= identity;
            }
        } else if name.ends_with("metrics.protobuf") {
            audit.metrics_exports += 1;
            let request = ExportMetricsServiceRequest::decode(bytes.as_slice())?;
            for resource in request.resource_metrics {
                let identity = role(
                    &resource
                        .resource
                        .ok_or(Failure::Gate("metric resource absent"))?
                        .attributes,
                )?;
                check(
                    resource.scope_metrics.iter().any(|scope| {
                        scope
                            .metrics
                            .iter()
                            .any(|metric| match metric.data.as_ref() {
                                Some(metric::Data::Sum(sum)) => !sum.data_points.is_empty(),
                                Some(metric::Data::Histogram(histogram)) => {
                                    !histogram.data_points.is_empty()
                                }
                                _ => false,
                            })
                    }),
                    "empty SDK metric resource",
                )?;
                audit.metric_role_mask |= identity;
                for scope in resource.scope_metrics {
                    for metric in scope.metrics {
                        match metric.data.ok_or(Failure::Gate("metric data absent"))? {
                            metric::Data::Sum(sum) => {
                                let required = match (identity, metric.name.as_str()) {
                                    (1, "indra.gateway.requests") => {
                                        Some(&mut audit.gateway_requests_max)
                                    }
                                    (2, "indra.ocr.requests") => {
                                        Some(&mut audit.native_requests_max)
                                    }
                                    _ => None,
                                };
                                if let Some(maximum) = required {
                                    check(
                                        sum.is_monotonic && sum.aggregation_temporality == 2,
                                        "request counter is not cumulative monotonic",
                                    )?;
                                    let mut total = 0_u64;
                                    for point in &sum.data_points {
                                        let Some(number_data_point::Value::AsInt(value)) =
                                            &point.value
                                        else {
                                            return Err(Failure::Gate(
                                                "request counter is not integer",
                                            ));
                                        };
                                        let value = u64::try_from(*value).map_err(|_| {
                                            Failure::Gate("negative request counter")
                                        })?;
                                        total = total
                                            .checked_add(value)
                                            .ok_or(Failure::Gate("request counter overflow"))?;
                                    }
                                    *maximum = (*maximum).max(total);
                                }
                                for point in sum.data_points {
                                    match point.value {
                                        Some(number_data_point::Value::AsInt(value)) => {
                                            check(value >= 0, "negative SDK counter")?
                                        }
                                        Some(number_data_point::Value::AsDouble(value)) => check(
                                            value.is_finite() && value >= 0.0,
                                            "nonfinite/negative SDK counter",
                                        )?,
                                        None => {
                                            return Err(Failure::Gate("SDK counter value absent"));
                                        }
                                    }
                                }
                            }
                            metric::Data::Histogram(histogram) => {
                                for point in histogram.data_points {
                                    check(
                                        point.sum.is_none_or(|v| v.is_finite() && v >= 0.0)
                                            && point.bucket_counts.len()
                                                == point.explicit_bounds.len() + 1
                                            && point
                                                .bucket_counts
                                                .iter()
                                                .try_fold(0_u64, |sum, n| sum.checked_add(*n))
                                                == Some(point.count)
                                            && point.explicit_bounds.iter().all(|b| b.is_finite())
                                            && point
                                                .explicit_bounds
                                                .windows(2)
                                                .all(|pair| pair[0] < pair[1]),
                                        "invalid SDK histogram",
                                    )?;
                                }
                            }
                            _ => return Err(Failure::Gate("unexpected SDK metric aggregation")),
                        }
                    }
                }
            }
        } else {
            return Err(Failure::Gate("unexpected OTLP evidence name"));
        }
    }
    if enabled {
        check(
            cohort_exports > 0,
            "no SDK export observed across cohort boundaries",
        )?;
        spans.verify()?;
        audit.scheduled_graphs = 12;
        check(
            audit.trace_exports > 0
                && audit.metrics_exports > 0
                && audit.trace_role_mask == 7
                && audit.metric_role_mask == 3
                && audit.gateway_requests_max >= 15
                && audit.native_requests_max >= 15,
            "actual SDK trace/metric exporter roles missing",
        )?;
        check(
            audit.sentry.gateway_events == 1,
            "actual Sentry gateway event absent/duplicate",
        )?;
    } else {
        check(
            audit.spans == 0
                && audit.trace_exports == 0
                && audit.metrics_exports == 0
                && audit.sentry.envelopes == 0,
            "off arm exported telemetry",
        )?;
    }
    for filename in ["gateway.stderr", "native.stderr"] {
        check(
            fs::metadata(output.join(filename))?.len() <= 16 * 1024 * 1024,
            "SDK log audit byte cap",
        )?;
        let text = fs::read_to_string(output.join(filename))?;
        check(
            !text.lines().any(export_failure),
            "SDK exporter error/drop evidence",
        )?;
    }
    audit.privacy_passed = true;
    Ok(audit)
}

fn contains_ascii_case(text: &[u8], needle: &[u8]) -> bool {
    text.windows(needle.len())
        .any(|part| part.eq_ignore_ascii_case(needle))
}
fn export_failure(line: &str) -> bool {
    let bytes = line.as_bytes();
    let sdk = [
        b"opentelemetry".as_slice(),
        b"sentry",
        b"otlp",
        b"batchspanprocessor",
        b"tika_telemetry",
    ]
    .iter()
    .any(|name| contains_ascii_case(bytes, name));
    let failure = [
        b"warn".as_slice(),
        b"error",
        b"fail",
        b"drop",
        b"queue full",
        b"queuefull",
    ]
    .iter()
    .any(|word| contains_ascii_case(bytes, word));
    (sdk && failure)
        || contains_ascii_case(bytes, b"failed to export")
        || contains_ascii_case(bytes, b"dropped spans")
        || contains_ascii_case(bytes, b"log_queue_overflow")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Spans {
        let mut spans = Spans {
            data: [Span::default(); 512],
            used: 0,
        };
        for incoming in REQUEST_PARENTS {
            let remote = indra_telemetry::Parent::parse(incoming).unwrap();
            for (index, kind) in [
                Kind::GatewayHttp,
                Kind::JavaParse,
                Kind::JavaOcr,
                Kind::NativeHttp,
                Kind::NativeInference,
            ]
            .into_iter()
            .enumerate()
            {
                let mut id = [0_u8; 8];
                id[7] = index as u8 + 1;
                let mut parent = remote.span;
                if index > 0 {
                    parent = [0; 8];
                    parent[7] = index as u8;
                }
                spans.data[spans.used] = Span {
                    trace: remote.trace,
                    id,
                    parent,
                    kind,
                    error: false,
                };
                spans.used += 1;
            }
        }
        spans
    }
    #[test]
    fn exact_parent_edges_not_trace_id_alone_admit_the_cohort() {
        let mut spans = fixture();
        spans.verify().unwrap();
        spans.data[1].parent[7] ^= 42;
        assert!(spans.verify().is_err());
        let mut spans = fixture();
        spans.data[4].parent = [42; 8];
        assert!(spans.verify().is_err());
        let mut spans = fixture();
        spans.data[60] = spans.data[0];
        spans.used = 61;
        assert!(spans.verify().is_err());
    }
    #[test]
    fn receiver_rejects_client_header_secrets_but_allows_distinct_export_credentials() {
        use std::{
            io::{Read, Write},
            net::{TcpListener, TcpStream},
            sync::atomic::{AtomicBool, AtomicUsize, Ordering},
            time::Duration,
        };
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        for (headers, accepted) in [
            (
                "Authorization: Bearer deliberately-configured-export-credential\r\n",
                true,
            ),
            ("Authorization: Bearer SDK_PRIVATE_CANARY\r\n", false),
            ("Cookie: session=COOKIE_PRIVATE_CANARY\r\n", false),
            ("Baggage: secret=DOCUMENT_SECRET\r\n", false),
        ] {
            let path = std::env::temp_dir().join(format!(
                "indra-telemetry-header-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let stop = AtomicBool::new(false);
            let (result, response) = std::thread::scope(|scope| {
                let collector = scope.spawn(|| {
                    super::super::capture::collect_with_header_validator(
                        listener,
                        &stop,
                        &path,
                        validate_export_headers,
                    )
                });
                let response = (|| -> std::io::Result<String> {
                    let mut client = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
                    client.set_read_timeout(Some(Duration::from_secs(2)))?;
                    client.set_write_timeout(Some(Duration::from_secs(2)))?;
                    write!(
                        client,
                        "POST /v1/traces HTTP/1.1\r\nHost: localhost\r\n{headers}Content-Length: 0\r\nConnection: close\r\n\r\n"
                    )?;
                    let mut response = String::new();
                    client.read_to_string(&mut response)?;
                    Ok(response)
                })();
                stop.store(true, Ordering::Release);
                (collector.join().unwrap(), response)
            });
            let captured = path.join("otlp-00-traces.protobuf").is_file();
            std::fs::remove_dir_all(path).unwrap();
            assert_eq!(result.is_ok(), accepted);
            assert_eq!(captured, accepted);
            if accepted {
                assert!(response.unwrap().starts_with("HTTP/1.1 200 OK"));
            }
        }
    }
    #[test]
    fn sdk_drop_and_shutdown_markers_are_not_case_sensitive() {
        for marker in [
            "SENTRY queue full",
            "BatchSpanProcessor dropped 1 spans",
            "tika_telemetry_shutdown_failed",
            "OTLP WARN retry",
            "{\"event\":\"log_queue_overflow\",\"dropped_lines\":1}",
        ] {
            assert!(export_failure(marker));
        }
        assert!(!export_failure("gateway ERROR failure.class=java_exited"));
    }
    #[test]
    fn captured_span_inventory_has_a_finite_bound() {
        let span = r#"{"trace":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1],"id":[0,0,0,0,0,0,0,1],"parent":[0,0,0,0,0,0,0,0],"kind":"other","error":false}"#;
        let mut body = String::from("[");
        for index in 0..513 {
            if index > 0 {
                body.push(',');
            }
            body.push_str(span);
        }
        body.push(']');
        assert!(serde_json::from_str::<Spans>(&body).is_err());
    }
}
