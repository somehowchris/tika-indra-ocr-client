//! Bounded actual Sentry envelope sink. No vendor endpoint or synthetic SDK events.
use super::{Failure, check, write_new};
use serde::Serialize;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
#[derive(Default, Serialize)]
pub(super) struct Captured {
    pub envelopes: usize,
    pub bytes: usize,
    pub gateway_events: usize,
}
pub(super) fn collect(
    listener: TcpListener,
    stop: &AtomicBool,
    output: &Path,
) -> Result<Captured, Failure> {
    listener.set_nonblocking(true)?;
    let due = Instant::now() + Duration::from_secs(300);
    let mut captured = Captured::default();
    while !stop.load(Ordering::Acquire) {
        check(Instant::now() < due, "Sentry receiver aggregate deadline")?;
        match listener.accept() {
            Ok((socket, peer)) => {
                check(peer.ip().is_loopback(), "non-loopback Sentry peer")?;
                receive(socket, output, &mut captured, due)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(2))
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(captured)
}
fn receive(
    mut socket: TcpStream,
    output: &Path,
    captured: &mut Captured,
    aggregate: Instant,
) -> Result<(), Failure> {
    let due = aggregate.min(Instant::now() + Duration::from_secs(5));
    socket.set_read_timeout(Some(Duration::from_millis(100)))?;
    socket.set_write_timeout(Some(Duration::from_secs(1)))?;
    let mut bytes = [0_u8; 73728];
    let mut used = 0;
    let boundary = loop {
        check(
            used < 8192 && Instant::now() < due,
            "Sentry header limit/deadline",
        )?;
        used += read(&mut socket, &mut bytes[used..8192], due)?;
        if let Some(at) = bytes[..used].windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
    };
    super::audit::validate_export_headers(&bytes[..boundary])?;
    let header = std::str::from_utf8(&bytes[..boundary])
        .map_err(|_| Failure::Gate("Sentry header UTF-8"))?;
    check(
        header.starts_with("POST /api/1/envelope/ HTTP/1.1\r\n"),
        "unexpected Sentry route",
    )?;
    let mut lengths = header.lines().filter_map(|line| {
        let (k, v) = line.split_once(':')?;
        k.eq_ignore_ascii_case("content-length").then_some(v.trim())
    });
    let length = lengths
        .next()
        .ok_or(Failure::Gate("Sentry requires content length"))?
        .parse::<usize>()
        .map_err(|_| Failure::Gate("invalid Sentry content length"))?;
    check(
        lengths.next().is_none() && length <= 65536,
        "duplicate/oversize Sentry length",
    )?;
    check(
        !header.lines().any(|line| {
            line.split_once(':').is_some_and(|(key, _)| {
                key.eq_ignore_ascii_case("transfer-encoding")
                    || key.eq_ignore_ascii_case("content-encoding")
            })
        }),
        "Sentry compression/chunking forbidden",
    )?;
    let end = boundary
        .checked_add(length)
        .ok_or(Failure::Gate("Sentry body overflow"))?;
    check(
        end <= bytes.len() && used <= end,
        "Sentry body cap/pipelining",
    )?;
    while used < end {
        used += read(&mut socket, &mut bytes[used..end], due)?;
    }
    let body = &bytes[boundary..end];
    verify_envelope(body)?;
    check(
        captured.envelopes < 8 && captured.bytes + length <= 262144,
        "Sentry aggregate capture cap",
    )?;
    write_new(
        &output.join(format!("sentry-{:02}.envelope", captured.envelopes)),
        body,
    )?;
    captured.envelopes += 1;
    captured.bytes += length;
    captured.gateway_events += 1;
    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
    Ok(())
}
fn read(socket: &mut TcpStream, destination: &mut [u8], due: Instant) -> Result<usize, Failure> {
    loop {
        check(Instant::now() < due, "Sentry body deadline")?;
        match socket.read(destination) {
            Ok(0) => return Err(Failure::Gate("truncated Sentry request")),
            Ok(n) => return Ok(n),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e.into()),
        }
    }
}
pub(super) fn verify_envelope(body: &[u8]) -> Result<(), Failure> {
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
            !body.windows(forbidden.len()).any(|s| s == forbidden),
            "private Sentry payload",
        )?;
    }
    let first = body
        .iter()
        .position(|b| *b == b'\n')
        .ok_or(Failure::Gate("missing Sentry envelope header"))?;
    check(first <= 8192, "Sentry envelope header cap")?;
    let _: serde_json::Value = serde_json::from_slice(&body[..first])?;
    let rest = &body[first + 1..];
    let second = rest
        .iter()
        .position(|b| *b == b'\n')
        .ok_or(Failure::Gate("missing Sentry item header"))?;
    check(second <= 1024, "Sentry item header cap")?;
    let item: serde_json::Value = serde_json::from_slice(&rest[..second])?;
    check(item["type"] == "event", "unexpected Sentry item type")?;
    let payload = &rest[second + 1..];
    let data = payload.strip_suffix(b"\n").unwrap_or(payload);
    if let Some(length) = item.get("length") {
        check(
            length.as_u64() == Some(data.len() as u64),
            "Sentry item length differs",
        )?;
    }
    let event: serde_json::Value = serde_json::from_slice(data)?;
    let fields = event
        .as_object()
        .ok_or(Failure::Gate("Sentry event is not an object"))?;
    check(
        fields.keys().all(|name| {
            matches!(
                name.as_str(),
                "event_id"
                    | "level"
                    | "version"
                    | "type"
                    | "logger"
                    | "platform"
                    | "timestamp"
                    | "release"
                    | "environment"
                    | "message"
                    | "exception"
                    | "tags"
                    | "extra"
                    | "breadcrumbs"
                    | "sdk"
                    | "contexts"
            )
        }),
        "unexpected Sentry event payload field",
    )?;
    check(
        event["level"] == "error"
            && event["logger"] == "indra.runtime"
            && event["tags"]["failure.class"] == "java_exited"
            && event["tags"]["ocr.phase"] == "gateway",
        "Sentry event not actual gateway Java failure",
    )?;
    check(
        event["message"]
            == "Service operation failed; inspect the correlated trace and finite failure class",
        "unexpected Sentry message",
    )?;
    check(
        event["exception"]["values"]
            .as_array()
            .is_some_and(|v| v.len() == 1)
            && event["exception"]["values"][0]["type"] == "java_exited"
            && event["exception"]["values"][0]["value"] == "A bounded service operation failed",
        "unexpected Sentry exception",
    )?;
    check(
        event.get("request").is_none()
            && event.get("user").is_none()
            && event.get("server_name").is_none(),
        "private Sentry request/user/server data",
    )?;
    check(
        event["exception"]["values"][0]
            .as_object()
            .is_some_and(|exception| {
                exception
                    .keys()
                    .all(|key| matches!(key.as_str(), "type" | "value"))
            }),
        "Sentry stack payload forbidden",
    )?;
    let tags = event["tags"]
        .as_object()
        .ok_or(Failure::Gate("Sentry tags absent"))?;
    check(
        tags.iter().all(|(key, value)| match key.as_str() {
            "failure.class" => value == "java_exited",
            "ocr.phase" => value == "gateway",
            "request_id" | "trace_id" => value
                .as_str()
                .is_some_and(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit())),
            "span_id" => value
                .as_str()
                .is_some_and(|s| s.len() == 16 && s.bytes().all(|b| b.is_ascii_hexdigit())),
            _ => false,
        }),
        "unbounded Sentry tags",
    )?;
    let extra = event["extra"]
        .as_object()
        .ok_or(Failure::Gate("Sentry timing context absent"))?;
    check(
        extra.len() == 1
            && extra
                .get("elapsed_ms")
                .and_then(serde_json::Value::as_f64)
                .is_some_and(|v| v.is_finite() && v >= 0.0),
        "unbounded Sentry extra context",
    )?;
    check(
        event
            .get("contexts")
            .is_none_or(|v| v.as_object().is_some_and(|m| m.is_empty())),
        "unexpected Sentry context payload",
    )?;
    let breadcrumbs = event["breadcrumbs"]["values"]
        .as_array()
        .ok_or(Failure::Gate("Sentry lifecycle breadcrumbs absent"))?;
    check(
        breadcrumbs.len() == 2
            && breadcrumbs[0]["category"] == "service.lifecycle"
            && breadcrumbs[0]["message"]
                == "Tika admission stopped before terminating and reaping its process group"
            && breadcrumbs[1]["category"] == "service.failure"
            && breadcrumbs[1]["message"] == "java_exited"
            && breadcrumbs.iter().all(|crumb| {
                crumb
                    .get("data")
                    .is_none_or(|data| data.as_object().is_some_and(|m| m.is_empty()))
            }),
        "unbounded Sentry breadcrumbs",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    static EVENT: &str = r#"{"level":"error","logger":"indra.runtime","message":"Service operation failed; inspect the correlated trace and finite failure class","extra":{"elapsed_ms":1.0},"breadcrumbs":{"values":[{"category":"service.lifecycle","message":"Tika admission stopped before terminating and reaping its process group"},{"category":"service.failure","message":"java_exited"}]},"tags":{"failure.class":"java_exited","ocr.phase":"gateway"},"exception":{"values":[{"type":"java_exited","value":"A bounded service operation failed"}]}}"#;
    fn envelope(event: &str) -> String {
        format!(
            "{{}}\n{{\"type\":\"event\",\"length\":{}}}\n{event}\n",
            event.len()
        )
    }
    #[test]
    fn domain_event_accepted_but_private_or_ambiguous_payload_rejected() {
        verify_envelope(envelope(EVENT).as_bytes()).expect("bounded classified event fixture");
        for bad in [
            EVENT.replace("java_exited", "arbitrary_document_failure"),
            EVENT.replace(
                "\"elapsed_ms\":1.0",
                "\"elapsed_ms\":1.0,\"document\":\"unseen mathematical response\"",
            ),
            EVENT.replace(
                "\"ocr.phase\":\"gateway\"",
                "\"ocr.phase\":\"gateway\",\"document\":\"unseen text\"",
            ),
            EVENT.replace("bounded service operation", "SDK_PRIVATE_CANARY"),
            EVENT.replace("\"level\":\"error\"", "\"request\":{},\"level\":\"error\""),
            EVENT.replace(
                "\"value\":\"A bounded service operation failed\"",
                "\"stacktrace\":{},\"value\":\"A bounded service operation failed\"",
            ),
        ] {
            assert!(verify_envelope(envelope(&bad).as_bytes()).is_err());
        }
        assert!(verify_envelope(b"{}\n{\"type\":\"event\",\"length\":1}\n{}\n").is_err());
        assert!(
            verify_envelope(format!("{}{}", envelope(EVENT), envelope(EVENT)).as_bytes()).is_err()
        );
    }
}
