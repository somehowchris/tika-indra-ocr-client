use crate::{FailureContext, Options, TelemetryError};
use sentry::{
    ClientInitGuard, ClientOptions, Level,
    protocol::{Breadcrumb, Event, Exception},
};
use std::{borrow::Cow, num::NonZeroUsize, sync::Arc, time::Duration};

pub(crate) fn initialize(options: &Options) -> Result<ClientInitGuard, TelemetryError> {
    let timeout = Duration::from_millis(options.export_timeout_ms);
    let dsn = options
        .sentry_dsn
        .as_deref()
        .ok_or(TelemetryError::Initialize("missing Sentry DSN"))?
        .parse()
        .map_err(|_| TelemetryError::Configuration("sentry_dsn"))?;
    let mut client = ClientOptions::new()
        .release(format!(
            "{}@{}",
            options.service_name,
            env!("CARGO_PKG_VERSION")
        ))
        .environment(String::from(options.environment.as_str()))
        .sample_rate(options.sentry_sample_ratio)
        .traces_sample_rate(0.0)
        .default_integrations(false)
        .send_default_pii(false)
        .attach_stacktrace(false)
        .max_breadcrumbs(4)
        .max_request_body_size(sentry::MaxRequestBodySize::None)
        .shutdown_timeout(timeout);
    client.dsn = Some(dsn);
    client.server_name = None;
    client.transport_channel_capacity = NonZeroUsize::new(options.sentry_queue_size);
    // The SDK's default proxy conversion loses NO_PROXY and has no overall HTTP
    // deadline. Supply one concrete client so the same configured bound covers
    // connection establishment, response reads, and shutdown. Reqwest applies
    // its normal proxy/NO_PROXY policy; TLS verification cannot be disabled by
    // Sentry's legacy SSL_VERIFY environment variable.
    let http = reqwest::Client::builder()
        .connect_timeout(timeout)
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| TelemetryError::Initialize("Sentry HTTP client"))?;
    let transport_options = sentry::TransportOptions::try_from_client_options(&client)
        .ok_or(TelemetryError::Initialize("Sentry transport options"))?;
    let transport = sentry::transports::ReqwestHttpTransportOptions::from(transport_options)
        .with_client(http)
        .build();
    // Sentry requires a shared factory. Its Arc<T> implementation preserves the
    // concrete transport here; dynamic dispatch stays inside the SDK boundary.
    client = client.transport(Arc::new(transport));
    Ok(sentry::init(client))
}

pub(crate) fn report(context: &FailureContext<'_>) {
    let mut event = Event {
        level: Level::Error,
        logger: Some(String::from("indra.runtime")),
        message: Some(String::from(
            "Service operation failed; inspect the correlated trace and finite failure class",
        )),
        fingerprint: Cow::Borrowed(&[Cow::Borrowed("{{ default }}")]),
        ..Event::default()
    };
    // These SDK-owned maps and protocol lists allocate only on a reported error.
    // Never stringify EngineError: decoder diagnostics may include input data.
    event.exception.values.push(Exception {
        ty: String::from(context.class),
        value: Some(String::from("A bounded service operation failed")),
        ..Exception::default()
    });
    event
        .tags
        .insert(String::from("failure.class"), String::from(context.class));
    event.tags.insert(
        String::from("ocr.phase"),
        String::from(context.phase.name()),
    );
    if let Some(request_id) = context
        .request_id
        .filter(|value| valid_request_id(value, &context.phase))
    {
        event
            .tags
            .insert(String::from("request_id"), String::from(request_id));
    }
    if let Some(worker) = context.worker {
        event.extra.insert(String::from("worker"), worker.into());
    }
    event.extra.insert(
        String::from("elapsed_ms"),
        (context.elapsed.as_secs_f64() * 1000.0).into(),
    );
    event.breadcrumbs.values.push(Breadcrumb {
        category: Some(String::from("service.lifecycle")),
        message: Some(String::from(match context.phase {
            crate::FailurePhase::Startup => "Worker model verification or warmup did not complete",
            crate::FailurePhase::Inference => "Verified model worker began bounded OCR inference",
            crate::FailurePhase::Watchdog => {
                "A native call exceeded its request deadline and grace allowance"
            }
            crate::FailurePhase::Gateway => {
                "Tika admission stopped before terminating and reaping its process group"
            }
        })),
        level: Level::Info,
        ..Breadcrumb::default()
    });
    event.breadcrumbs.values.push(Breadcrumb {
        category: Some(String::from("service.failure")),
        message: Some(String::from(context.class)),
        level: Level::Error,
        ..Breadcrumb::default()
    });
    #[cfg(feature = "otlp")]
    {
        use opentelemetry::trace::TraceContextExt;
        use tracing_opentelemetry::OpenTelemetrySpanExt;
        let trace = tracing::Span::current().context();
        let span = trace.span();
        let ids = span.span_context();
        if ids.is_valid() {
            event
                .tags
                .insert(String::from("trace_id"), ids.trace_id().to_string());
            event
                .tags
                .insert(String::from("span_id"), ids.span_id().to_string());
        }
    }
    sentry::capture_event(event);
}

fn valid_request_id(value: &str, phase: &crate::FailurePhase) -> bool {
    // Retain the public 32-hex context contract. Only gateway failures also
    // accept the 16-hex sequence returned by that service's HTTP protocol.
    (value.len() == 32 || (value.len() == 16 && matches!(phase, crate::FailurePhase::Gateway)))
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::valid_request_id;
    use crate::FailurePhase;

    #[test]
    fn correlation_ids_preserve_existing_callers_and_reject_arbitrary_metadata() {
        let native = "0123456789abcdef0123456789abcdef";
        let gateway = "0123456789abcdef";
        for phase in [
            FailurePhase::Startup,
            FailurePhase::Inference,
            FailurePhase::Watchdog,
            FailurePhase::Gateway,
        ] {
            assert!(valid_request_id(native, &phase));
        }
        assert!(valid_request_id(gateway, &FailurePhase::Gateway));
        for phase in [
            FailurePhase::Startup,
            FailurePhase::Inference,
            FailurePhase::Watchdog,
        ] {
            assert!(!valid_request_id(gateway, &phase));
        }
        for value in [
            "",
            "0123456789abcde",
            "0123456789abcdef0",
            "0123456789abcdef0123456789abcde",
            "0123456789abcdef0123456789abcdef0",
            "DOCUMENT_SECRET!",
            "0123456789abcdef0123456789abcdeg",
            "é23456789abcdef",
        ] {
            assert!(!valid_request_id(value, &FailurePhase::Gateway));
        }
    }
}
