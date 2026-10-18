//! Strict W3C version-00 traceparent parsing; invalid context starts a new trace.
//! Baggage is deliberately not propagated because it can carry document PII.

pub use indra_trace_context::{Carrier, Parent};

/// Inject only an actual current SDK span. With exporters disabled, preserve the
/// already validated incoming carrier; never fabricate an unobservable hop.
pub fn inject_current(carrier: &mut Carrier) {
    #[cfg(feature = "otlp")]
    {
        use opentelemetry::trace::TraceContextExt;
        use tracing_opentelemetry::OpenTelemetrySpanExt;
        let context = tracing::Span::current().context();
        let span = context.span();
        let ids = span.span_context();
        if ids.is_valid() {
            carrier.set_span(
                &ids.trace_id().to_bytes(),
                &ids.span_id().to_bytes(),
                ids.is_sampled(),
                ids.trace_state().into_iter(),
            );
        }
    }
    #[cfg(not(feature = "otlp"))]
    let _ = carrier;
}

pub fn request_span(request_id: &str, parent: Option<&str>, state: Option<&str>) -> tracing::Span {
    http_span("POST", "/ocr", "POST /ocr", request_id, parent, state)
}

/// Method/route/name must come from finite application labels, never a raw URI.
pub fn http_span(
    method: &'static str,
    route: &'static str,
    name: &'static str,
    request_id: &str,
    parent: Option<&str>,
    state: Option<&str>,
) -> tracing::Span {
    let span = tracing::info_span!("http.request",
        otel.name = name, otel.kind = "server", http.request.method = method,
        http.route = route, request_id, http.response.status_code = tracing::field::Empty,
        error.type = tracing::field::Empty, trace_id = tracing::field::Empty, span_id = tracing::field::Empty);
    #[cfg(feature = "otlp")]
    if let Some(parent) = parent.and_then(Parent::parse) {
        use opentelemetry::trace::{
            SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState,
        };
        use tracing_opentelemetry::OpenTelemetrySpanExt;
        let state = state
            .and_then(indra_trace_context::State::parse)
            .and_then(|state| TraceState::from_key_value(state.pairs()).ok())
            .unwrap_or_default();
        let remote = SpanContext::new(
            TraceId::from_bytes(parent.trace),
            SpanId::from_bytes(parent.span),
            if parent.sampled {
                TraceFlags::SAMPLED
            } else {
                TraceFlags::NOT_SAMPLED
            },
            true,
            state,
        );
        // No global propagator, mutable thread-local guard, or context crossing
        // an await is required. The tracing span is instrumented per poll.
        let _ = span.set_parent(opentelemetry::Context::new().with_remote_span_context(remote));
    }
    #[cfg(feature = "otlp")]
    {
        use opentelemetry::trace::TraceContextExt;
        use tracing_opentelemetry::OpenTelemetrySpanExt;
        let context = span.context();
        let current = context.span();
        let ids = current.span_context();
        if ids.is_valid() {
            span.record("trace_id", tracing::field::display(ids.trace_id()));
            span.record("span_id", tracing::field::display(ids.span_id()));
        }
    }
    #[cfg(not(feature = "otlp"))]
    let _ = (parent, state);
    span
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn w3c_parent_is_strict_and_nonzero() {
        let valid = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        let parsed = Parent::parse(valid).expect("W3C example");
        assert_eq!(parsed.trace[0], 0x4b);
        assert!(parsed.sampled);
        for invalid in [
            "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
            "00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01",
            "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-extra",
        ] {
            assert!(Parent::parse(invalid).is_none());
        }
    }
}
