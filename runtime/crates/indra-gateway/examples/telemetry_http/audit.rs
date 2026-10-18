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
                self.data[..self.used]
                    .iter()
                    .any(|s| s.kind == Kind::NativeInference && self.descendant(s, native)),
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
        if !name.starts_with("otlp-") {
            continue;
        }
        let bytes = fs::read(entry.path())?;
        audit.otlp_bytes += bytes.len();
        for forbidden in [
            b"DOCUMENT_SECRET".as_slice(),
            b"SDK_PRIVATE_CANARY",
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
        if name.ends_with("traces.protobuf") {
            audit.trace_exports += 1;
            let request = ExportTraceServiceRequest::decode(bytes.as_slice())?;
            for resource in request.resource_spans {
                let resource = resource
                    .resource
                    .ok_or(Failure::Gate("trace resource absent"))?;
                audit.trace_role_mask |= role(&resource.attributes)?;
            }
        } else if name.ends_with("metrics.protobuf") {
            audit.metrics_exports += 1;
            let request = ExportMetricsServiceRequest::decode(bytes.as_slice())?;
            for resource in request.resource_metrics {
                audit.metric_role_mask |= role(
                    &resource
                        .resource
                        .ok_or(Failure::Gate("metric resource absent"))?
                        .attributes,
                )?;
                for scope in resource.scope_metrics {
                    for metric in scope.metrics {
                        match metric.data.ok_or(Failure::Gate("metric data absent"))? {
                            metric::Data::Sum(sum) => {
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
                                            && point.bucket_counts.iter().sum::<u64>()
                                                == point.count
                                            && point.explicit_bounds.iter().all(|b| b.is_finite()),
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
                && audit.metric_role_mask == 3,
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
        let text = fs::read_to_string(output.join(filename))?;
        check(
            !text.lines().any(|line| {
                (line.contains("opentelemetry")
                    && (line.contains("WARN") || line.contains("ERROR")))
                    || line.contains("Failed to export")
                    || line.contains("dropped spans")
            }),
            "SDK exporter error/drop evidence",
        )?;
    }
    audit.privacy_passed = true;
    Ok(audit)
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
